//! Program argumanlari -- **iki ABI, tek arayuz**.
//!
//! Argumanlarin cekirdekten uygulamaya gecis bicimi POSIX ile Win32'de
//! yapisal olarak farklidir, ve TCMK ikisini de oldugu gibi korur:
//!
//! ```text
//!   POSIX (ELF)   yiginda:  [argc][argv0][argv1]..[NULL][envp NULL]
//!                 bir DIZI -- her arguman ayri, NUL ile biter
//!
//!   Win32 (PE)    GetCommandLineA() -> "browse /notlar"
//!                 tek bir DIZE -- bolmek cagirana kalir
//! ```
//!
//! Gercek Windows'ta o bolme isini CRT yapar (`CommandLineToArgvW`);
//! burada `init_win32` yapiyor. Yani bu modul, iki ayri sozlesmenin
//! ustune ortak bir yuz koyuyor -- uygulama hangi ikili bicimde
//! derlendigini bilmeden `args::get(1)` diyebiliyor.
//!
//! ```ignore
//! fn main() {
//!     match tcmk::args::get(1) {
//!         Some(path) => open(path),
//!         None => open("/"),
//!     }
//! }
//! ```

use core::sync::atomic::{AtomicUsize, Ordering};

/// Saklanabilecek en fazla arguman (`argv[0]` dahil).
const MAX_ARGS: usize = 8;
/// Win32 tarafinda komut satirinin kopyalandigi tampon.
const CMDLINE_MAX: usize = 160;

/// Her argumanin baslangic isaretcisi ve uzunlugu.
static POINTERS: [AtomicUsize; MAX_ARGS] = [const { AtomicUsize::new(0) }; MAX_ARGS];
static LENGTHS: [AtomicUsize; MAX_ARGS] = [const { AtomicUsize::new(0) }; MAX_ARGS];
static COUNT: AtomicUsize = AtomicUsize::new(0);

/// Win32 komut satirinin kopyasi.
///
/// Bolme **yerinde** yapiliyor: bosluklar NUL'a cevriliyor ve her
/// parcanin basi kaydediliyor. Cekirdegin verdigi dizeyi degistirmemek
/// icin once buraya kopyalaniyor.
static mut CMDLINE: [u8; CMDLINE_MAX] = [0; CMDLINE_MAX];

/// POSIX baslangic yigininda `environ` dizisinin adresi.
///
/// Argumanlarla ayni yiginda, `argv`nin NULL sonlandiricisindan hemen
/// sonra durur -- bu yuzden onu bulan yer de burasi. `env` modulu
/// diziyi buradan okur.
static ENVIRON: AtomicUsize = AtomicUsize::new(0);

fn record(pointer: usize, length: usize) {
    let index = COUNT.load(Ordering::Relaxed);
    if index >= MAX_ARGS {
        return;
    }
    POINTERS[index].store(pointer, Ordering::Relaxed);
    LENGTHS[index].store(length, Ordering::Relaxed);
    COUNT.store(index + 1, Ordering::Relaxed);
}

/// POSIX: SysV baslangic yigininden `argc`/`argv` okur.
///
/// `stack` giris anindaki yigin isaretcisidir; orada once `argc`, hemen
/// ardindan `argv` isaretcileri durur.
///
/// # Safety
/// Yalnizca `_start`tan, gercek giris yigin isaretcisiyle cagrilmalidir.
pub unsafe fn init_posix(stack: *const usize) {
    if stack.is_null() {
        return;
    }
    let argc = stack.read();
    for i in 0..argc.min(MAX_ARGS) {
        let pointer = stack.add(1 + i).read();
        if pointer == 0 {
            break;
        }
        // Uzunluk NUL'a kadar okunarak bulunur -- `argv` dizisi
        // uzunluk tasimaz, C dizesi sozlesmesi budur.
        let mut length = 0usize;
        while length < CMDLINE_MAX && (pointer as *const u8).add(length).read() != 0 {
            length += 1;
        }
        record(pointer, length);
    }

    // `environ`: argv'nin NULL sonlandiricisindan hemen sonrasi.
    // Konum **gercek** `argc`ye gore hesaplanir; MAX_ARGS budamasi
    // yalnizca kac argumani sakladigimizi etkiler, yigin duzenini degil.
    ENVIRON.store(stack.add(1 + argc + 1) as usize, Ordering::Relaxed);
}

/// POSIX `environ` dizisinin basi -- yoksa bos isaretci.
pub fn environ() -> *const usize {
    ENVIRON.load(Ordering::Relaxed) as *const usize
}

/// Win32: `GetCommandLineA`nin dondugu tek dizeyi parcalara ayirir.
///
/// Bu, `CommandLineToArgvW`nin isidir ve kurallari **uydurma degildir**:
///
///   * Cift tirnak "tirnak icinde" durumunu degistirir; o durumdayken
///     bosluk siradan bir karakterdir.
///   * Bir tirnaktan onceki `2n` ters bolu -> `n` ters bolu + durum
///     degisir; `2n+1` -> `n` ters bolu + **gercek** bir tirnak.
///   * Ters bolu yalnizca tirnaktan onceyken ozeldir, yani
///     `C:\dizin\dosya` hicbir kacisa ugramaz.
///
/// Kurallara uymak gerekiyor cunku cekirdek komut satirini kurarken
/// **ayni** kurallarla alintiliyor (bkz. `level0b1::argv`). Eskiden
/// burada yalnizca bosluktan bolunuyordu ve `"iki kelime"` iki arguman
/// oluyordu -- yani bosluklu bir yol yolun yarisina donusuyordu.
///
/// # Safety
/// `line` NUL sonlandirmali gecerli bir dize olmalidir.
pub unsafe fn init_win32(line: *const u8) {
    if line.is_null() {
        return;
    }
    // Once kopyala: cekirdegin verdigi dize degistirilmemeli.
    let base = core::ptr::addr_of_mut!(CMDLINE) as *mut u8;
    let mut length = 0usize;
    while length < CMDLINE_MAX - 1 {
        let byte = line.add(length).read();
        if byte == 0 {
            break;
        }
        base.add(length).write(byte);
        length += 1;
    }
    base.add(length).write(0);

    // Cozulen metin **ayni tamponun icine**, yerinde yazilir. Kacislar
    // yalnizca karakter siler (`\"` -> `"`, `""` -> ``), hic eklemez, o
    // yuzden yazma imleci okuma imlecini asla gecemez.
    let mut read = 0usize;
    let mut write = 0usize;
    let mut in_quotes = false;
    let mut started = false;
    let mut start = 0usize;

    let finish = |start: usize, write: usize, base: *mut u8| {
        if write > start {
            record(base.add(start) as usize, write - start);
        } else {
            // Bos arguman (`""`): gecerli ve korunmali.
            record(base.add(start) as usize, 0);
        }
    };

    while read < length {
        let c = base.add(read).read();

        if !in_quotes && (c == b' ' || c == b'\t') {
            if started {
                finish(start, write, base);
                started = false;
                // Ayirici olarak bir NUL birak, sonraki arguman ondan
                // sonra baslasin -- dilimler hep bu tampona bakiyor.
                base.add(write).write(0);
                write += 1;
                start = write;
            }
            read += 1;
            continue;
        }

        if c == b'\\' {
            let mut slashes = 0usize;
            while read + slashes < length && base.add(read + slashes).read() == b'\\' {
                slashes += 1;
            }
            let quote_follows = read + slashes < length && base.add(read + slashes).read() == b'"';
            let emit = if quote_follows { slashes / 2 } else { slashes };
            for _ in 0..emit {
                base.add(write).write(b'\\');
                write += 1;
            }
            if emit > 0 {
                started = true;
            }
            read += slashes;
            if quote_follows {
                if slashes % 2 == 1 {
                    base.add(write).write(b'"');
                    write += 1;
                } else {
                    in_quotes = !in_quotes;
                }
                started = true;
                read += 1;
            }
            continue;
        }

        if c == b'"' {
            in_quotes = !in_quotes;
            started = true;
            read += 1;
            continue;
        }

        base.add(write).write(c);
        write += 1;
        started = true;
        read += 1;
    }

    if started {
        finish(start, write, base);
        base.add(write).write(0);
    }
}

/// Kac arguman var (`argv[0]` dahil).
pub fn count() -> usize {
    COUNT.load(Ordering::Relaxed)
}

/// `index`. arguman. `0` programin kendi adidir.
pub fn get(index: usize) -> Option<&'static str> {
    if index >= count() {
        return None;
    }
    let pointer = POINTERS[index].load(Ordering::Relaxed);
    let length = LENGTHS[index].load(Ordering::Relaxed);
    if pointer == 0 {
        return None;
    }
    unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(pointer as *const u8, length)).ok()
    }
}

/// Ilk gercek arguman (`argv[1]`) -- yoksa `None`.
///
/// Uygulamalarin cogunun istedigi tam olarak bu: "bana bir yol verildi mi?"
pub fn first() -> Option<&'static str> {
    get(1)
}
