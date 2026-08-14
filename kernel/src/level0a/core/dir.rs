//! Acik dizinler -- `opendir`/`getdents` icin yan tablo.
//!
//! ## Neden ayri bir tablo?
//!
//! Bir dosya tanimlayicisi (`core::fd`) yalnizca iki sayi tasir: `node`
//! ve `offset`. Dosyalarda `node` VFS dugumu, borularda boru indeksidir.
//! Dizinlerde ise gezinmek icin **yol adi** gerekir: TCMK'nin dizin
//! kavrami iki arka uca yayilmistir -- agac diskte (TCMKFS inode'lari),
//! dosyalar duz isim uzayinda (VFS) yasar. `/bin` gibi yalnizca RAMFS'te
//! var olan dizinlerin karsiligi olan bir inode hic yoktur, dolayisiyla
//! "dizin = inode numarasi" varsayimi bastan yanlis olurdu.
//!
//! Cozum: yol adlari burada, kucuk bir havuzda durur; tanimlayici
//! yalnizca havuz indeksini tasir. Boru havuzuyla ayni kalip -- sayacli,
//! `fork`/`dup` ile paylasilabilir, son sahip birakinca serbest kalir.
//!
//! Gezinme imleci (kacinci girdideyiz) burada **tutulmaz**: o,
//! tanimlayicinin kendi `offset` alanindadir. Boylece ayni dizini iki kez
//! acan bir surec iki bagimsiz imlec alir.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::tcmkfs;

/// Ayni anda acik tutulabilen dizin sayisi.
///
/// Dizin gezmek kisa omurlu bir istir (ac, listele, kapat); 8 yuva
/// butun sureclerin ayni anda birer dizin gezmesine yeter.
pub const MAX_OPEN_DIRS: usize = 8;

#[derive(Clone, Copy)]
struct OpenDir {
    path: [u8; tcmkfs::PATH_MAX],
    len: usize,
    /// Bu yuvayi kac tanimlayici gosteriyor (`fork`/`dup` sonrasi >1).
    refs: usize,
}

impl OpenDir {
    const fn empty() -> Self {
        OpenDir {
            path: [0; tcmkfs::PATH_MAX],
            len: 0,
            refs: 0,
        }
    }
}

static mut DIRS: [OpenDir; MAX_OPEN_DIRS] = [OpenDir::empty(); MAX_OPEN_DIRS];
static OPEN_COUNT: AtomicUsize = AtomicUsize::new(0);

fn slot(index: usize) -> *mut OpenDir {
    unsafe { (core::ptr::addr_of_mut!(DIRS) as *mut OpenDir).add(index) }
}

/// Bir yol adi icin yuva ayirir; havuz doluysa `None`.
pub fn open(path: &str) -> Option<usize> {
    if path.len() >= tcmkfs::PATH_MAX {
        return None;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        for i in 0..MAX_OPEN_DIRS {
            let entry = &mut *slot(i);
            if entry.refs != 0 {
                continue;
            }
            *entry = OpenDir::empty();
            entry.path[..path.len()].copy_from_slice(path.as_bytes());
            entry.len = path.len();
            entry.refs = 1;
            OPEN_COUNT.fetch_add(1, Ordering::Relaxed);
            return Some(i);
        }
        None
    })
}

/// Yuvanin sayacini artirir (`fork` / `dup`).
pub fn add_ref(index: usize) {
    if index >= MAX_OPEN_DIRS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let entry = &mut *slot(index);
        if entry.refs != 0 {
            entry.refs += 1;
        }
    });
}

/// Sayaci dusurur; sifira inince yuva serbest kalir.
pub fn release(index: usize) {
    if index >= MAX_OPEN_DIRS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let entry = &mut *slot(index);
        if entry.refs == 0 {
            return;
        }
        entry.refs -= 1;
        if entry.refs == 0 {
            *entry = OpenDir::empty();
            OPEN_COUNT.fetch_sub(1, Ordering::Relaxed);
        }
    });
}

/// Yuvanin tuttugu yol adi.
///
/// Havuz `static` oldugu icin dilim `'static`: yuva serbest kalana kadar
/// gecerlidir, ki cagiran zaten acik bir tanimlayici uzerinden gelir.
pub fn path_of(index: usize) -> Option<&'static str> {
    if index >= MAX_OPEN_DIRS {
        return None;
    }
    unsafe {
        let entry = &*slot(index);
        if entry.refs == 0 {
            return None;
        }
        core::str::from_utf8(&entry.path[..entry.len]).ok()
    }
}

/// Su an acik dizin sayisi (kabuk `df` raporu icin).
pub fn open_count() -> usize {
    OPEN_COUNT.load(Ordering::Relaxed)
}
