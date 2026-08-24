//! `mapped` -- dosyayi okumak yerine **adreslemek**.
//!
//! `mmap` buraya kadar yalnizca anonimdi: bos sayfa veriyordu, `brk`in
//! yapamadigi seyi (geri verme) yapiyordu ama bir dosyayla ilgisi yoktu.
//! Artik dosya destekli esleme de var.
//!
//! ## Neden onemli
//!
//! Bir dosyayi adreslemek, onu okumaktan farkli bir sozlesmedir:
//!
//! ```text
//!   read()  -> tampona KOPYALA, imleci ilerlet
//!   mmap()  -> dosyanin kendisini ADRESLE, imlec yok
//! ```
//!
//! Ikincisi rastgele erisimi bedava yapar ve dizin/veri tabani
//! dosyalarini okuyan gercek yazilimlarin ilk tercihidir.
//!
//! ## Windows'taki karsiligi
//!
//! ```text
//!   POSIX   mmap(NULL, len, PROT_READ, MAP_PRIVATE, fd, offset)
//!             -> tek cagri, dogrudan adres
//!
//!   Win32   CreateFileMappingA(hFile, ..) -> esleme NESNESI
//!           MapViewOfFile(nesne, ..)      -> adres
//! ```
//!
//! Aradaki nesne bosuna degil: Windows'ta adlandirilabilir ve surecler
//! arasinda paylasilabilir. PE ikizi `winmap.exe` ayni sinavlari o
//! yoldan yapiyor.
//!
//! ## Dort sinav
//!
//! ```text
//!   A  icerik        -> eslenen baytlar read() ile AYNI
//!   B  hizasiz ofset -> sayfa hizali olmayan istek REDDEDILIR
//!   C  dosya sonu    -> sinirin otesi SIFIR gelir
//!   D  munmap        -> bolge birakilir, anonim esleme calismaya devam
//! ```
//!
//! C, POSIX'in acikca soyledigi bir kural: eslenen bolge dosyadan
//! uzunsa kalan kisim sifirdir. Cop birakmak, dosyanin sonundan sonra
//! onceki surecin verisini gostermek olurdu.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0010_1A20;
const PANEL: u32 = 0x001C_2C36;
const FG: u32 = 0x00E0_ECF0;
const DIM: u32 = 0x0080_949C;
const ACCENT: u32 = 0x0060_D0C0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Eslenecek dosya -- RAMFS'te duruyor, yani disk gerekmiyor.
const PATH: &[u8] = b"/boot/msg.txt\0";

/// B sinavinin basladigi ofset. Sayfa sinirinda **degil**: i386'da
/// cekirdege giden ofset sayfa cinsinden, yani hizasiz bir deger
/// donusumun dogru yapildigini da olcuyor.
const OFFSET: usize = 0;

#[derive(Clone, Copy)]
struct Check {
    name: &'static str,
    detail: &'static str,
    passed: bool,
}

const EMPTY: Check = Check {
    name: "",
    detail: "",
    passed: false,
};

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;
    let mut checks = [EMPTY; 4];

    // Once siradan yoldan oku: karsilastirmanin dogru tarafi bu.
    let mut expected = [0u8; 128];
    let mut expected_len = 0usize;
    let fd = unsafe { sys::open_raw(PATH.as_ptr(), 0) };
    if fd >= 0 {
        let read = sys::read(fd as usize, &mut expected);
        if read > 0 {
            expected_len = read as usize;
        }
    }

    // --- A: icerik ---
    let view = if fd >= 0 && expected_len > 0 {
        sys::mmap_file(expected_len, fd as isize, OFFSET)
    } else {
        None
    };
    let a = match view {
        Some(addr) => (0..expected_len).all(|i| unsafe { *addr.add(i) } == expected[i]),
        None => false,
    };
    checks[0] = Check {
        name: "A eslenen icerik",
        detail: if fd < 0 {
            "dosya acilamadi"
        } else if view.is_none() {
            "esleme basarisiz"
        } else if a {
            "eslenen baytlar read() ile ayni"
        } else {
            "icerik AYRISIYOR"
        },
        passed: a,
    };

    // --- B: hizasiz ofset reddediliyor mu ---
    //
    // POSIX'in kurali net: `mmap`in ofseti **sayfa hizali** olmak
    // zorunda. i386'da bu ABI'ye gomulu (`mmap2` ofseti sayfa cinsinden
    // alir), yani sekiz bayt sessizce sifira yuvarlanirdi ve cagiran
    // dosyanin basini gordugunu fark etmezdi. Kontrol bu yuzden
    // bolmeden once yapiliyor -- glibc de aynisini yapar.
    let unaligned = if fd >= 0 {
        sys::mmap_file(4096, fd as isize, 8)
    } else {
        None
    };
    let b = fd >= 0 && unaligned.is_none();
    checks[1] = Check {
        name: "B hizasiz ofset",
        detail: if fd < 0 {
            "dosya acilamadi"
        } else if b {
            "sayfa hizali olmayan ofset reddedildi"
        } else {
            "hizasiz ofset KABUL edildi"
        },
        passed: b,
    };

    // --- C: dosya sonundan sonrasi ---
    //
    // Dosyadan uzun bir bolge isteniyor; kalan kisim sifir olmali.
    let padded = if fd >= 0 && expected_len > 0 {
        sys::mmap_file(expected_len + 64, fd as isize, OFFSET)
    } else {
        None
    };
    let c = match padded {
        Some(addr) => (expected_len..expected_len + 64)
            .all(|i| unsafe { *addr.add(i) } == 0),
        None => false,
    };
    checks[2] = Check {
        name: "C dosya sonu",
        detail: if padded.is_none() {
            "esleme basarisiz"
        } else if c {
            "sinirin otesi sifir geldi"
        } else {
            "sinirin otesinde COP var"
        },
        passed: c,
    };

    // --- D: munmap ve anonim esleme ---
    let released = match view {
        Some(addr) => sys::munmap(addr, expected_len) == 0,
        None => false,
    };
    let anon = sys::mmap(4096);
    let anon_ok = match anon {
        Some(addr) => {
            unsafe { addr.write(0xA5) };
            unsafe { addr.read() == 0xA5 }
        }
        None => false,
    };
    let d = released && anon_ok;
    checks[3] = Check {
        name: "D munmap + anonim",
        detail: if !released {
            "munmap basarisiz"
        } else if !anon_ok {
            "anonim esleme bozuldu"
        } else {
            "bolge birakildi, anonim esleme saglam"
        },
        passed: d,
    };

    if fd >= 0 {
        sys::close(fd as usize);
    }

    for check in &checks {
        let _ = writeln!(
            out,
            "[mapped] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }

    let mut win = match Window::open("mapped -- dosya destekli mmap", 280, 200, 440, 150) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks, expected_len);
        win.flush();
    }
}

fn draw(win: &mut Window, checks: &[Check; 4], size: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "okumak degil, adreslemek", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            300,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, "dosya boyu:", DIM);
    win.number(110, h - 30, size, FG);
    win.text(
        6,
        h - 14,
        if passed == checks.len() {
            "hepsi gecti   q cik"
        } else {
            "BIR SINAV KALDI   q cik"
        },
        if passed == checks.len() { OK } else { WARN },
    );
}
