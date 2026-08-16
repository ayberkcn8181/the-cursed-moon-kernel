//! Ortam degiskenleri -- **iki ABI, tek arayuz**.
//!
//! Argumanlarda oldugu gibi, ortamin uygulamaya gecis bicimi de iki
//! tarafta yapisal olarak farkli, ve TCMK ikisini de oldugu gibi
//! koruyor:
//!
//! ```text
//!   POSIX (ELF)   yiginda:  [argv NULL]["HOME=/home"]["PATH=/bin"][NULL]
//!                 bir DIZI -- arama uygulamanin isi (`getenv` libc'de)
//!
//!   Win32 (PE)    GetEnvironmentVariableA("HOME", buf, 64)
//!                 ADLA SORULUR -- arama cekirdekte, degeri tampona yazar
//! ```
//!
//! Fark yalnizca bicim degil, **kimin aradigi**: POSIX'te tabloyu
//! uygulama tarar, Win32'de cekirdek. Bu modul ikisinin ustune ortak
//! bir yuz koyuyor.
//!
//! ```ignore
//! let home = tcmk::env::get("HOME").unwrap_or("/");
//! ```

/// Win32 tarafinda degerin kopyalandigi tampon.
///
/// `GetEnvironmentVariableA` bir tampon istiyor; POSIX tarafinda ise
/// deger zaten yiginda duruyor ve dogrudan gosterilebiliyor. Ortak
/// arayuzun tek sozlesmesi bu yuzden **en dar** olani: dondurulen dilim
/// bir sonraki `get` cagrisina kadar gecerlidir.
#[cfg(target_os = "windows")]
static mut VALUE: [u8; 64] = [0; 64];
/// Adin NUL sonlandirmali kopyasi -- Win32 C dizesi bekliyor.
#[cfg(target_os = "windows")]
static mut NAME: [u8; 32] = [0; 32];

/// Bir girdinin ad kismi (`=` isaretine kadar).
#[cfg(not(target_os = "windows"))]
fn name_of(text: &str) -> &str {
    match text.find('=') {
        Some(i) => &text[..i],
        None => text,
    }
}

/// POSIX: baslangic yiginindaki `environ` dizisini tarar.
///
/// Gercek libc'de `getenv` de tam olarak bunu yapar -- cekirdegin
/// verdigi diziyi dogrusal arar.
#[cfg(not(target_os = "windows"))]
pub fn get(name: &str) -> Option<&'static str> {
    let environ = crate::args::environ();
    if environ.is_null() {
        return None;
    }
    let mut index = 0usize;
    loop {
        let pointer = unsafe { environ.add(index).read() };
        if pointer == 0 {
            return None;
        }
        let mut length = 0usize;
        while length < 128 && unsafe { (pointer as *const u8).add(length).read() } != 0 {
            length += 1;
        }
        let text = unsafe {
            core::str::from_utf8(core::slice::from_raw_parts(pointer as *const u8, length)).ok()
        };
        if let Some(text) = text {
            if name_of(text) == name {
                return Some(&text[name.len() + 1..]);
            }
        }
        index += 1;
    }
}

/// Win32: `GetEnvironmentVariableA` ile adla sorar.
///
/// Aramayi cekirdek yapiyor; burada yalnizca ad NUL sonlandirmali hale
/// getiriliyor ve donen uzunluk dilime cevriliyor.
#[cfg(target_os = "windows")]
pub fn get(name: &str) -> Option<&'static str> {
    unsafe {
        let name_buffer = core::ptr::addr_of_mut!(NAME) as *mut u8;
        if name.len() >= 32 {
            return None;
        }
        for (i, byte) in name.bytes().enumerate() {
            name_buffer.add(i).write(byte);
        }
        name_buffer.add(name.len()).write(0);

        let value = core::ptr::addr_of_mut!(VALUE) as *mut u8;
        let written = crate::winapi::GetEnvironmentVariableA(name_buffer, value, 64);
        // Sifir = degisken yok. Tamponun boyundan buyuk bir sayi =
        // "gereken uzunluk" -- degeri sigdiramadik demektir.
        if written == 0 || written as usize >= 64 {
            return None;
        }
        core::str::from_utf8(core::slice::from_raw_parts(value, written as usize)).ok()
    }
}

/// Degiskeni ver, yoksa yedegi kullan.
///
/// `env::get("HOME").unwrap_or("/")` kalibi her uygulamada tekrar
/// ediyordu.
pub fn get_or(name: &str, fallback: &'static str) -> &'static str {
    get(name).unwrap_or(fallback)
}
