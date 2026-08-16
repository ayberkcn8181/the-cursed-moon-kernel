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
    // Once yerel katman: bu surecin kendi `set`i yigindaki anlik
    // goruntuyu degistiremez, o yuzden onun uzerine biner.
    if let Some(found) = overlay_get(name) {
        return found;
    }
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

// --- Yazma ------------------------------------------------------------
//
// Iki tarafta iki ayri is yapiliyor, cunku okuma yollari farkli:
//
//   POSIX  okuma yigindaki **anlik goruntuden**. Cekirdege yazmak onu
//          degistirmez, o yuzden yerel bir katman da tutuluyor -- gercek
//          libc'nin `setenv`i de tam olarak bunu yapar (kendi
//          dizisindeki isaretciyi degistirir).
//   Win32  okuma zaten cekirdege soruluyor; yazinca sonraki okuma yeni
//          degeri gorur. Yerel katmana gerek yok.

/// POSIX tarafinda yerel katman -- `setenv`in kendi surecte gorunen
/// yuzu.
#[cfg(not(target_os = "windows"))]
const OVERLAY_VARS: usize = 4;
#[cfg(not(target_os = "windows"))]
const OVERLAY_ENTRY: usize = 64;
#[cfg(not(target_os = "windows"))]
static mut OVERLAY: [[u8; OVERLAY_ENTRY]; OVERLAY_VARS] = [[0; OVERLAY_ENTRY]; OVERLAY_VARS];
#[cfg(not(target_os = "windows"))]
static mut OVERLAY_LEN: [usize; OVERLAY_VARS] = [0; OVERLAY_VARS];
#[cfg(not(target_os = "windows"))]
static OVERLAY_COUNT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Yerel katmandan okur -- `get`ten **once** bakilir.
#[cfg(not(target_os = "windows"))]
fn overlay_get(name: &str) -> Option<Option<&'static str>> {
    use core::sync::atomic::Ordering;
    let total = OVERLAY_COUNT.load(Ordering::Relaxed);
    for i in 0..total {
        unsafe {
            let base = core::ptr::addr_of!(OVERLAY) as *const u8;
            let len = (core::ptr::addr_of!(OVERLAY_LEN) as *const usize).add(i).read();
            let text = core::str::from_utf8(core::slice::from_raw_parts(
                base.add(i * OVERLAY_ENTRY),
                len,
            ))
            .ok()?;
            if name_of(text) == name {
                let value = &text[name.len() + 1..];
                // Bos deger "silindi" demek: dis katmandaki eski deger
                // artik gorunmemeli, o yuzden `Some(None)` doner.
                return Some(if value.is_empty() { None } else { Some(value) });
            }
        }
    }
    None
}

/// POSIX: cekirdege bildirir **ve** yerel katmani gunceller.
///
/// Iki adim de gerekli ve ayri seyler icin: cekirdek kaydi `fork`/
/// `execve` ile dogacak sureclerin gorecegi ortam, yerel katman ise bu
/// surecin kendi `get`inin gorecegi deger.
#[cfg(not(target_os = "windows"))]
pub fn set(name: &str, value: &str) -> bool {
    use core::sync::atomic::Ordering;
    if name.is_empty() || name.contains('=') || name.len() + 1 + value.len() >= OVERLAY_ENTRY {
        return false;
    }

    let mut name_buf = [0u8; OVERLAY_ENTRY];
    let mut value_buf = [0u8; OVERLAY_ENTRY];
    name_buf[..name.len()].copy_from_slice(name.as_bytes());
    value_buf[..value.len()].copy_from_slice(value.as_bytes());
    if unsafe { crate::sys::setenv(name_buf.as_ptr(), value_buf.as_ptr()) } != 0 {
        return false;
    }

    unsafe {
        let total = OVERLAY_COUNT.load(Ordering::Relaxed);
        let base = core::ptr::addr_of_mut!(OVERLAY) as *mut u8;
        let lengths = core::ptr::addr_of_mut!(OVERLAY_LEN) as *mut usize;

        // Ayni ad varsa uzerine yaz; yoksa yeni yuva.
        let mut index = total;
        for i in 0..total {
            let len = lengths.add(i).read();
            let text = core::str::from_utf8(core::slice::from_raw_parts(
                base.add(i * OVERLAY_ENTRY),
                len,
            ));
            if text.map(name_of) == Ok(name) {
                index = i;
                break;
            }
        }
        if index == total {
            if total >= OVERLAY_VARS {
                // Cekirdek yazildi ama yerel katman doldu: bu surecin
                // kendi `get`i eski degeri gorur. Sessiz kalmak yerine
                // basarisiz bildiriliyor.
                return false;
            }
            OVERLAY_COUNT.store(total + 1, Ordering::Relaxed);
        }

        let slot = base.add(index * OVERLAY_ENTRY);
        let mut at = 0usize;
        for byte in name.bytes() {
            slot.add(at).write(byte);
            at += 1;
        }
        slot.add(at).write(b'=');
        at += 1;
        for byte in value.bytes() {
            slot.add(at).write(byte);
            at += 1;
        }
        lengths.add(index).write(at);
    }
    true
}

/// Win32: `SetEnvironmentVariableA` -- okuma zaten cekirdege sordugu
/// icin yerel katmana gerek yok.
#[cfg(target_os = "windows")]
pub fn set(name: &str, value: &str) -> bool {
    unsafe {
        if name.len() >= 32 || value.len() >= 64 {
            return false;
        }
        let name_buffer = core::ptr::addr_of_mut!(NAME) as *mut u8;
        for (i, byte) in name.bytes().enumerate() {
            name_buffer.add(i).write(byte);
        }
        name_buffer.add(name.len()).write(0);

        let mut value_buffer = [0u8; 64];
        value_buffer[..value.len()].copy_from_slice(value.as_bytes());
        crate::winapi::SetEnvironmentVariableA(name_buffer, value_buffer.as_ptr()) != 0
    }
}

/// Degiskeni siler (`unsetenv` / `SetEnvironmentVariableA(ad, NULL)`).
pub fn unset(name: &str) -> bool {
    set(name, "")
}
