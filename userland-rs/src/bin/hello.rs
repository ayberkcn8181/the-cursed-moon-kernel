//! `/bin/hello` -- POSIX yolunun ucdan uca dogrulamasi.
//!
//! Bu program `tools/gen_hello_elf.py` ile elle uretilen ELF'in yerini
//! alir. Ayni sistem cagrilarini yapar, ama artik okunabilir Rust'tir:
//!
//!     write(1, ...)         selamlama
//!     open("/boot/msg.txt") VFS'ten dosya ac
//!     read/write            icerigi stdout'a aktar
//!     close                 tanimlayiciyi birak (Drop ile)
//!     brk(0)                mevcut program break
//!     exit(0)
//!
//! Slot 0 (taban 0x00C0_0000) -- bkz. Makefile.

#![no_std]
#![no_main]

use tcmk::{io::File, println, sys};

tcmk::entry!(main);

fn main() {
    println!("Merhaba, Ring 3 -- bu program Rust ile yazildi.");

    match File::open("/boot/msg.txt") {
        Some(mut f) => {
            println!("[hello] /boot/msg.txt acildi (fd={}).", f.fd());
            let mut buf = [0u8; 128];
            let n = f.read(&mut buf);
            // Icerigi oldugu gibi stdout'a aktar.
            sys::write(sys::STDOUT, &buf[..n]);
            println!("[hello] {} bayt okundu.", n);
            // f burada Drop ile kapanir -- fd sizmaz.
        }
        None => println!("[hello] /boot/msg.txt acilamadi."),
    }

    let brk = sys::brk(0);
    println!("[hello] program break = {:#010x}", brk);

    // brk'i buyutmeyi dene. Ust sinir kullanici yigininin tabanidir; imaj
    // yigina yakinsa 4 KiB'lik istek reddedilir -- bu da dogru davranistir.
    let want = brk + 0x1000;
    let grown = sys::brk(want);
    if grown == want {
        println!("[hello] brk(+4 KiB) -> {:#010x} (buyudu)", grown);
    } else {
        let small = sys::brk(brk + 0x40);
        println!(
            "[hello] brk(+4 KiB) reddedildi (yigin tabani sinir); +64 B -> {:#010x}",
            small
        );
    }

    println!("[hello] cikiliyor.");
}
