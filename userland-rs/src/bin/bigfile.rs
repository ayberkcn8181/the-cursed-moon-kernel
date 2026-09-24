//! `bigfile` -- TCMKFS'in kalkan tavanlari.
//!
//! Dosya sistemi uzun sure iki sert sinirla yasadi ve ikisi de
//! yerlesimden geliyordu:
//!
//! ```text
//!   64 inode        -> diskte en fazla 64 girdi
//!   160 KiB/dosya   -> inode'da yalnizca 40 DOGRUDAN blok isaretcisi
//! ```
//!
//! Ikincisi klasik Unix cozumuyle kalkti: **dolayli blok**. Dogrudan
//! isaretciler bittiginde inode tek bir bloga isaret ediyor ve o blok
//! 4096/4 = 1024 blok numarasi tasiyor.
//!
//! ```text
//!   mantiksal blok 0..39     -> inode.blocks[n]         (dogrudan)
//!   mantiksal blok 40..1063  -> dolayli_blok[n - 40]    (bir okuma daha)
//! ```
//!
//! Tavan boylece 160 KiB'dan 4 MiB + 160 KiB'a cikti.
//!
//! Ilki yerlesim degisikligi istedi: 512 inode'luk tablo 256 sektor
//! tutuyor, oysa metaveri 1. sektorde basliyor ve 40. sektorde
//! onyukleyici alani geliyordu. Tabloyu buyutmek onyukleyiciyi ezerdi,
//! o yuzden metaveri onyukleyici alaninin **arkasina** tasindi.
//!
//! ## Alti sinav
//!
//! ```text
//!   A  dolayli alan    -> 200 KiB yazildi (eski tavan 160 KiB)
//!   B  sinir           -> 160 KiB'in iki yaninda da icerik dogru
//!   C  derin okuma     -> dosyanin sonu dogru geldi
//!   D  blok sizmiyor   -> silince bos blok sayisi TAM geri geliyor
//!   E  inode tavani    -> 80 dosya yaratilabiliyor (eski tavan 64)
//!   F  tavan duruyor   -> azami boyun otesi hala reddediliyor
//! ```
//!
//! B en onemlisi: A yalnizca "yazma hata vermedi" der, oysa dolayli
//! alanin **dogru okundugu** ancak sinirin iki yani karsilastirilinca
//! gorulur. Bir blok numarasi yanlis yerden okunsaydi yazma yine
//! basarili gorunur, veri sessizce baska bir blogtan gelirdi.
//!
//! D ayri bir sinav olmayi hak ediyor cunku dolayli blok da bir blok:
//! dosya silinirken **o da** geri verilmeli. Verilmeseydi hicbir sey
//! bozulmaz, disk yavasca dolardi -- gorunmesi en zor hata turu.
//!
//! F, tavanin kalkmadigini kaldirilmadigini gosteriyor: sinir hala var,
//! yalnizca 26 kat oteye tasindi.
//!
//! ## Disk yoksa
//!
//! RAMFS salt okunur, yani disksiz acilista (yalnizca ISO) bu
//! sinavlarin hicbiri kurulamaz. O zaman "gecti" degil **"atlandi"**
//! yaziliyor -- calismayan bir yetenegi calisiyor saymak, sinavi
//! degersiz kilardi.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::{File, Stdout};
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0010_1A16;
const PANEL: u32 = 0x001C_2C26;
const FG: u32 = 0x00E0_ECE6;
const DIM: u32 = 0x0086_9A94;
const ACCENT: u32 = 0x0070_D0B0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;
const SKIP: u32 = 0x00C0_B060;

/// Eski tavan: 40 dogrudan blok x 4 KiB.
const OLD_LIMIT: usize = 160 * 1024;
/// Yazilacak boy -- eski tavanin otesinde, ama olcumu uzatmayacak kadar.
const BIG: usize = 200 * 1024;
const CHUNK: usize = 4096;
const PATH: &str = "/home/buyuk.bin";

/// E sinavinda yaratilacak dosya sayisi -- eski inode tavaninin otesi.
const MANY: usize = 80;

/// Konuma bagli desen.
///
/// Hem blok icinde hem bloklar arasinda degisiyor: sabit bir bayt
/// kullanilsaydi yanlis blogtan okunan veri de dogru gorunurdu.
fn pattern(at: usize) -> u8 {
    ((at >> 8) ^ at ^ 0x5A) as u8
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Copy)]
struct Check {
    name: &'static str,
    detail: &'static str,
    verdict: Verdict,
}

const EMPTY: Check = Check {
    name: "",
    detail: "",
    verdict: Verdict::Failed,
};

/// Yolu NUL sonlandirmali bir tampona kopyalar.
fn c_path(path: &str, buf: &mut [u8; 64]) -> bool {
    if path.len() >= buf.len() {
        return false;
    }
    buf.fill(0);
    buf[..path.len()].copy_from_slice(path.as_bytes());
    true
}

fn unlink(path: &str) -> bool {
    let mut buf = [0u8; 64];
    if !c_path(path, &mut buf) {
        return false;
    }
    unsafe { sys::unlink(buf.as_ptr()) >= 0 }
}

/// `/tmp/bNN` adini uretir.
fn numbered(index: usize, buf: &mut [u8; 16]) -> &str {
    buf.fill(0);
    buf[..7].copy_from_slice(b"/tmp/b\0");
    buf[6] = b'0' + (index / 10) as u8;
    buf[7] = b'0' + (index % 10) as u8;
    core::str::from_utf8(&buf[..8]).unwrap_or("/tmp/b00")
}

fn main() {
    let mut out = Stdout;
    let mut checks = [EMPTY; 6];

    // Disk yoksa hicbir sinav kurulamaz.
    if sys::fs_total_blocks() == 0 {
        for (i, name) in [
            "A dolayli alan",
            "B sinir",
            "C derin okuma",
            "D blok sizmiyor",
            "E inode tavani",
            "F tavan duruyor",
        ]
        .iter()
        .enumerate()
        {
            checks[i] = Check {
                name,
                detail: "disk bagli degil",
                verdict: Verdict::Skipped,
            };
        }
        report(&mut out, &checks, 0, 0);
        show(&checks, 0);
        return;
    }

    // Onceki kosudan kalmis olabilir.
    unlink(PATH);

    let free_before = sys::fs_free_blocks();

    // --- A: dolayli alana yazma ---
    let mut written = 0usize;
    let mut chunk = [0u8; CHUNK];
    {
        let mut file = File::create(PATH);
        if let Some(f) = file.as_mut() {
            while written < BIG {
                let len = CHUNK.min(BIG - written);
                for (i, slot) in chunk[..len].iter_mut().enumerate() {
                    *slot = pattern(written + i);
                }
                let n = f.write(&chunk[..len]);
                if n != len {
                    break;
                }
                written += n;
            }
        }
    }
    let a = written == BIG;
    checks[0] = Check {
        name: "A dolayli alan",
        detail: if a {
            "200 KiB yazildi (eski tavan 160 KiB)"
        } else if written > 0 && written <= OLD_LIMIT {
            "eski tavanda durdu -- dolayli blok yok"
        } else {
            "yazma basarisiz"
        },
        verdict: if a { Verdict::Passed } else { Verdict::Failed },
    };

    // --- B: sinirin iki yani ---
    //
    // 160 KiB, dogrudan isaretcilerin bittigi yer. Sekiz bayt oncesi
    // son dogrudan bloktan, sekiz bayt sonrasi ILK dolayli bloktan
    // geliyor; ikisinin de dogru olmasi gerekiyor.
    let mut edge = [0u8; 16];
    let b = a && read_at(PATH, OLD_LIMIT - 8, &mut edge) && matches(OLD_LIMIT - 8, &edge);
    checks[1] = Check {
        name: "B sinir",
        detail: if b {
            "160 KiB'in iki yani da dogru"
        } else if !a {
            "A gectikten sonra anlamli"
        } else {
            "sinirda veri YANLIS geldi"
        },
        verdict: if b { Verdict::Passed } else { Verdict::Failed },
    };

    // --- C: derin okuma ---
    let mut tail = [0u8; 16];
    let c = a && read_at(PATH, BIG - 16, &mut tail) && matches(BIG - 16, &tail);
    checks[2] = Check {
        name: "C derin okuma",
        detail: if c {
            "dosyanin sonu dogru geldi"
        } else if !a {
            "A gectikten sonra anlamli"
        } else {
            "son blok YANLIS geldi"
        },
        verdict: if c { Verdict::Passed } else { Verdict::Failed },
    };

    // --- D: blok sizmiyor ---
    //
    // Dolayli blok da bir bloktur. Silerken geri verilmezse disk her
    // buyuk dosyada bir blok kaybeder -- sessizce.
    let removed = unlink(PATH);
    let free_after = sys::fs_free_blocks();
    let d = removed && free_after == free_before;
    checks[3] = Check {
        name: "D blok sizmiyor",
        detail: if d {
            "bos blok sayisi tam geri geldi"
        } else if !removed {
            "dosya silinemedi"
        } else if free_after < free_before {
            "blok SIZDI (dolayli blok birakilmadi?)"
        } else {
            "bos blok sayisi buyudu"
        },
        verdict: if d { Verdict::Passed } else { Verdict::Failed },
    };

    // --- E: inode tavani ---
    let free_before_many = sys::fs_free_blocks();
    let mut made = 0usize;
    for i in 0..MANY {
        let mut buf = [0u8; 16];
        let name = numbered(i, &mut buf);
        match File::create(name) {
            Some(_) => made += 1,
            None => break,
        }
    }
    // Hepsi gercekten acilabiliyor mu -- yaratildi sanmak yetmez.
    let mut reopened = 0usize;
    for i in 0..made {
        let mut buf = [0u8; 16];
        if File::open(numbered(i, &mut buf)).is_some() {
            reopened += 1;
        }
    }
    for i in 0..made {
        let mut buf = [0u8; 16];
        unlink(numbered(i, &mut buf));
    }
    let free_after_many = sys::fs_free_blocks();
    let e = made == MANY && reopened == MANY && free_after_many == free_before_many;
    checks[4] = Check {
        name: "E inode tavani",
        detail: if e {
            "80 dosya yaratildi ve geri acildi"
        } else if made <= 64 {
            "64'te tikandi -- eski tavan duruyor"
        } else if reopened != made {
            "yaratildi ama geri ACILAMADI"
        } else {
            "silince blok sayisi degisti"
        },
        verdict: if e { Verdict::Passed } else { Verdict::Failed },
    };

    // --- F: tavan duruyor ---
    //
    // Sinir kalkmadi, yalnizca 26 kat oteye tasindi. Cekirdek boyu
    // asan yazmayi **tahsisten once** reddediyor, yani bu sinav
    // diskten hicbir sey harcamiyor.
    let max = sys::fs_max_file_size();
    let f = max > 0 && write_at(PATH, max - 4, &[1u8; 16]) < 0;
    unlink(PATH);
    checks[5] = Check {
        name: "F tavan duruyor",
        detail: if f {
            "azami boyun otesi reddedildi"
        } else if max == 0 {
            "azami boy okunamadi"
        } else {
            "tavan ASILDI"
        },
        verdict: if f { Verdict::Passed } else { Verdict::Failed },
    };

    report(&mut out, &checks, max, written);
    show(&checks, max);
}

/// Dosyanin `at` konumundan okur.
fn read_at(path: &str, at: usize, buf: &mut [u8]) -> bool {
    match File::open(path) {
        Some(mut f) => {
            if sys::lseek(f.fd(), at, sys::SEEK_SET) < 0 {
                return false;
            }
            f.read(buf) == buf.len()
        }
        None => false,
    }
}

/// `at` konumuna yazar; `write`in donus degerini aynen verir.
fn write_at(path: &str, at: usize, data: &[u8]) -> isize {
    match File::create(path) {
        Some(f) => {
            if sys::lseek(f.fd(), at, sys::SEEK_SET) < 0 {
                return -1;
            }
            sys::write(f.fd(), data)
        }
        None => -1,
    }
}

fn matches(at: usize, buf: &[u8]) -> bool {
    buf.iter().enumerate().all(|(i, b)| *b == pattern(at + i))
}

fn report(out: &mut Stdout, checks: &[Check; 6], max: usize, written: usize) {
    use core::fmt::Write;
    for check in checks {
        let _ = writeln!(
            out,
            "[bigfile] {}: {} ({})",
            check.name,
            match check.verdict {
                Verdict::Passed => "gecti",
                Verdict::Failed => "KALDI",
                Verdict::Skipped => "atlandi",
            },
            check.detail
        );
    }
    let _ = writeln!(
        out,
        "[bigfile] yazilan: {} bayt, azami: {} bayt, blok: {} bayt",
        written,
        max,
        sys::fs_block_size()
    );
}

fn show(checks: &[Check; 6], max: usize) {
    let mut win = match Window::open("bigfile -- kalkan tavanlar", 260, 160, 470, 190) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, checks, max);
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 6], max: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "dolayli blok: 160 KiB -> 4 MiB", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        let (word, color) = match check.verdict {
            Verdict::Passed => ("gecti", OK),
            Verdict::Failed => ("KALDI", WARN),
            Verdict::Skipped => ("atlandi", SKIP),
        };
        win.text(350, y, word, color);
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.verdict == Verdict::Passed).count();
    let skipped = checks.iter().filter(|c| c.verdict == Verdict::Skipped).count();
    win.text(6, h - 30, "azami dosya (KiB):", DIM);
    win.number(170, h - 30, max / 1024, FG);
    win.text(
        6,
        h - 14,
        if skipped == checks.len() {
            "disk yok -- hepsi atlandi   q cik"
        } else if passed == checks.len() {
            "hepsi gecti   q cik"
        } else {
            "BIR SINAV KALDI   q cik"
        },
        if skipped == checks.len() {
            SKIP
        } else if passed == checks.len() {
            OK
        } else {
            WARN
        },
    );
}
