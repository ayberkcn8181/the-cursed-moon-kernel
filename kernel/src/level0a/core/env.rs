//! Ortam degiskenleri -- sistem tablosu.
//!
//! Argumanlar geldiginde yigina bir `envp` sonlandiricisi konmustu ama
//! **icerik yoktu**: onu okuyan bir program bos bir dizi goruyordu. Bu
//! modul o diziyi dolduruyor.
//!
//! ## Neden tek tablo, surec basina degil
//!
//! Gercek POSIX'te ortam surecin adres uzayindadir: `fork` onu
//! kopyalar, `execve` cagiranin verdigi `envp` ile degistirir,
//! `setenv` yalnizca **kendi** kopyasini degistirir.
//!
//! TCMK'de cekirdek tek bir tablo tutuyor ve her yeni surec baslangicta
//! onun **anlik goruntusunu** yiginda aliyor. Sonuclari:
//!
//!   * Bir surecin kendi kopyasinda yaptigi degisiklik kardeslerine
//!     yayilmaz -- POSIX'te de oyle.
//!   * Ama `execve` cagiranin `envp`sini tasimadigi icin, degisiklik
//!     exec'ten sonra da yasamaz. Gercek POSIX'te yasardi. Bilincli
//!     sadelestirme (bkz. README).
//!
//! Tabloyu kabuk `set` komutuyla degistiriyor; boylece "oturum ortami"
//! kavrami kabukta, surecler de ondan miras aliyor.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Tabloda tutulabilecek degisken sayisi.
pub const MAX_VARS: usize = 8;
/// `AD=deger` girdisinin en fazla uzunlugu.
pub const MAX_ENTRY: usize = 64;

static mut ENTRIES: [[u8; MAX_ENTRY]; MAX_VARS] = [[0; MAX_ENTRY]; MAX_VARS];
static mut LENGTHS: [usize; MAX_VARS] = [0; MAX_VARS];
static COUNT: AtomicUsize = AtomicUsize::new(0);

fn entry(index: usize) -> *mut u8 {
    unsafe { (core::ptr::addr_of_mut!(ENTRIES) as *mut u8).add(index * MAX_ENTRY) }
}

/// `index`. girdi, `AD=deger` biciminde.
pub fn entry_at(index: usize) -> Option<&'static str> {
    if index >= COUNT.load(Ordering::Relaxed) {
        return None;
    }
    unsafe {
        let len = (core::ptr::addr_of!(LENGTHS) as *const usize).add(index).read();
        core::str::from_utf8(core::slice::from_raw_parts(entry(index), len)).ok()
    }
}

/// Kayitli degisken sayisi.
pub fn count() -> usize {
    COUNT.load(Ordering::Relaxed)
}

/// Bir girdinin ad kismi (`=` isaretine kadar).
fn name_of(text: &str) -> &str {
    match text.find('=') {
        Some(i) => &text[..i],
        None => text,
    }
}

/// Adi verilen degiskenin degeri.
pub fn get(name: &str) -> Option<&'static str> {
    for i in 0..count() {
        let text = entry_at(i)?;
        if name_of(text) == name {
            return Some(&text[name.len() + 1..]);
        }
    }
    None
}

/// Degiskeni ayarlar; varsa uzerine yazar.
///
/// Deger bos verilirse girdi **silinir** -- kabuktaki `set AD=` bicimi
/// boylece "kaldir" anlamina geliyor.
pub fn set(name: &str, value: &str) -> bool {
    if name.is_empty() || name.contains('=') {
        return false;
    }
    let needed = name.len() + 1 + value.len();
    if needed >= MAX_ENTRY {
        return false;
    }

    crate::arch::cpu::without_interrupts(|| unsafe {
        let total = COUNT.load(Ordering::Relaxed);
        let existing = (0..total).find(|i| entry_at(*i).map(name_of) == Some(name));

        if value.is_empty() {
            // Silme: son girdiyi bosalan yere tasi (sira onemli degil).
            if let Some(index) = existing {
                let last = total - 1;
                if index != last {
                    let source = entry(last);
                    let target = entry(index);
                    core::ptr::copy_nonoverlapping(source, target, MAX_ENTRY);
                    let lengths = core::ptr::addr_of_mut!(LENGTHS) as *mut usize;
                    lengths.add(index).write(lengths.add(last).read());
                }
                COUNT.store(last, Ordering::Relaxed);
            }
            return true;
        }

        let index = match existing {
            Some(i) => i,
            None => {
                if total >= MAX_VARS {
                    return false;
                }
                COUNT.store(total + 1, Ordering::Relaxed);
                total
            }
        };

        let slot = entry(index);
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
        (core::ptr::addr_of_mut!(LENGTHS) as *mut usize)
            .add(index)
            .write(at);
        true
    })
}

/// Acilis ortamini kurar.
///
/// Degerler bilerek az: bunlar "varsayilan oturum", uygulamalarin
/// bulmayi bekledigi en yalin kume. `HOME` disk bagliysa anlamli bir
/// dizin gosterir, degilse koke duser -- var olmayan bir dizin
/// vermek, `cd $HOME` diyen ilk programi bozardi.
pub fn init() {
    let home = if crate::level0a::core::tcmkfs::mounted()
        && crate::level0a::core::tcmkfs::resolve("/home").is_some()
    {
        "/home"
    } else {
        "/"
    };
    set("HOME", home);
    set("PATH", "/bin");
    set("SHELL", "tcmk");
}
