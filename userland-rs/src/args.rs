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
}

/// Win32: `GetCommandLineA`nin dondugu tek dizeyi parcalara ayirir.
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

    // Bosluklari NUL'a cevirerek yerinde bol; her parcanin basini kaydet.
    let mut i = 0usize;
    while i < length {
        while i < length && base.add(i).read() == b' ' {
            base.add(i).write(0);
            i += 1;
        }
        if i >= length {
            break;
        }
        let start = i;
        while i < length && base.add(i).read() != b' ' {
            i += 1;
        }
        record(base.add(start) as usize, i - start);
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
