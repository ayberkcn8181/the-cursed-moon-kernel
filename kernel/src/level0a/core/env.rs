//! Ortam degiskenleri -- **surec basina**, oturum tablosundan miras.
//!
//! Ilk surumde tek bir sistem tablosu vardi ve her surec baslangicta
//! onun anlik goruntusunu yiginda aliyordu. Okuma icin yetiyordu, ama
//! bir sureci **kendi** ortamini degistiremez birakiyordu: `setenv`
//! yoktu, cunku yazacak yer yoktu.
//!
//! Simdi her gorev yuvasinin kendi tablosu var ve POSIX'in uc kurali
//! -- `cwd`de oldugu gibi -- oldugu gibi geciyor:
//!
//! | olay | ortam |
//! |---|---|
//! | yeni gorev yuvasi | **oturum tablosunun kopyasi** (`spawn_inner`) |
//! | `fork` | ebeveynden kopyalanir |
//! | `execve` | **degismez** -- yuva ayni kaldigi icin kendiliginden |
//!
//! Ucuncu satir yine tasarimin sonucu: sifirlama imaj yuklenirken degil
//! **yuva ayrilirken** yapiliyor, `execve` yuvayi yeniden kullaniyor.
//! Gercek POSIX'te ayni sonuca `execve(path, argv, environ)` deyimiyle
//! varilir -- cagiran kendi ortamini acikca aktarir.
//!
//! ## Oturum tablosu neden ayri bir yuva
//!
//! Tablolar `MAX_TASKS + 1` satirlik tek bir dizi; son satir
//! (`SESSION`) hicbir goreve ait degil, kabugun `set` ile duzenledigi
//! **oturum ortamidir**. Yeni bir surec dogdugunda kopyalanan odur.
//! Ayri bir tur yerine ayni dizinin bir satiri olmasi, "kopyala"
//! isleminin her yerde tek bir kalip olmasini sagliyor.
//!
//! ## Gercek POSIX'ten ayrildigi yer
//!
//! Gercek POSIX'te ortam surecin **kendi belleginde** durur ve `setenv`
//! bir sistem cagrisi **degildir** -- libc kendi dizisini duzenler.
//! TCMK'de tablo cekirdekte oldugu icin bir cagri gerekiyor. Karsiliginda
//! Win32 tarafi bedavaya dogru calisiyor: `SetEnvironmentVariableA`
//! gercek Windows'ta da sureci temsil eden bir **cekirdek/PEB** blogunu
//! degistirir, kullanici dizisini degil.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::scheduler::MAX_TASKS;

/// Tabloda tutulabilecek degisken sayisi.
pub const MAX_VARS: usize = 8;
/// `AD=deger` girdisinin en fazla uzunlugu.
pub const MAX_ENTRY: usize = 64;

/// Oturum (kabuk) tablosunun yuvasi -- hicbir goreve ait degil.
pub const SESSION: usize = MAX_TASKS;

const TABLES: usize = MAX_TASKS + 1;

static mut ENTRIES: [[[u8; MAX_ENTRY]; MAX_VARS]; TABLES] =
    [[[0; MAX_ENTRY]; MAX_VARS]; TABLES];
static mut LENGTHS: [[usize; MAX_VARS]; TABLES] = [[0; MAX_VARS]; TABLES];
static COUNTS: [AtomicUsize; TABLES] = [const { AtomicUsize::new(0) }; TABLES];

fn entry(table: usize, index: usize) -> *mut u8 {
    unsafe {
        (core::ptr::addr_of_mut!(ENTRIES) as *mut u8)
            .add((table * MAX_VARS + index) * MAX_ENTRY)
    }
}

fn lengths() -> *mut usize {
    core::ptr::addr_of_mut!(LENGTHS) as *mut usize
}

fn length_at(table: usize, index: usize) -> usize {
    unsafe { lengths().add(table * MAX_VARS + index).read() }
}

/// Bir tablodaki `index`. girdi, `AD=deger` biciminde.
pub fn entry_at(table: usize, index: usize) -> Option<&'static str> {
    if table >= TABLES || index >= COUNTS[table].load(Ordering::Relaxed) {
        return None;
    }
    unsafe {
        let len = length_at(table, index).min(MAX_ENTRY);
        core::str::from_utf8(core::slice::from_raw_parts(entry(table, index), len)).ok()
    }
}

/// Bir tablodaki degisken sayisi.
pub fn count(table: usize) -> usize {
    if table >= TABLES {
        return 0;
    }
    COUNTS[table].load(Ordering::Relaxed)
}

/// Bir girdinin ad kismi (`=` isaretine kadar).
fn name_of(text: &str) -> &str {
    match text.find('=') {
        Some(i) => &text[..i],
        None => text,
    }
}

/// Adi verilen degiskenin degeri.
pub fn get(table: usize, name: &str) -> Option<&'static str> {
    for i in 0..count(table) {
        let text = entry_at(table, i)?;
        if name_of(text) == name {
            return Some(&text[name.len() + 1..]);
        }
    }
    None
}

/// Degiskeni ayarlar; varsa uzerine yazar.
///
/// Deger bos verilirse girdi **silinir** -- kabuktaki `set AD=` bicimi
/// ve Win32'nin `SetEnvironmentVariableA(name, NULL)` cagrisi ayni
/// anlama geliyor.
pub fn set(table: usize, name: &str, value: &str) -> bool {
    if table >= TABLES || name.is_empty() || name.contains('=') {
        return false;
    }
    if name.len() + 1 + value.len() >= MAX_ENTRY {
        return false;
    }

    crate::arch::cpu::without_interrupts(|| unsafe {
        let total = COUNTS[table].load(Ordering::Relaxed);
        let existing = (0..total).find(|i| entry_at(table, *i).map(name_of) == Some(name));

        if value.is_empty() {
            // Silme: son girdiyi bosalan yere tasi (sira onemli degil).
            if let Some(index) = existing {
                let last = total - 1;
                if index != last {
                    core::ptr::copy_nonoverlapping(
                        entry(table, last),
                        entry(table, index),
                        MAX_ENTRY,
                    );
                    let base = lengths();
                    let value = base.add(table * MAX_VARS + last).read();
                    base.add(table * MAX_VARS + index).write(value);
                }
                COUNTS[table].store(last, Ordering::Relaxed);
            }
            return true;
        }

        let index = match existing {
            Some(i) => i,
            None => {
                if total >= MAX_VARS {
                    return false;
                }
                COUNTS[table].store(total + 1, Ordering::Relaxed);
                total
            }
        };

        let slot = entry(table, index);
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
        lengths().add(table * MAX_VARS + index).write(at);
        true
    })
}

/// Bir tablodaki her seyi siler.
///
/// `execve` cagirana kendi `envp`sini verdiginde gerekiyor: yeni ortam
/// eskisinin **uzerine eklenmez**, onun **yerine gecer**. Gercek POSIX
/// de boyledir -- `execve`ye verilen dizi ortamin tamamidir.
pub fn clear(table: usize) {
    if table < TABLES {
        COUNTS[table].store(0, Ordering::Relaxed);
    }
}

/// `AD=deger` biciminde tek bir girdiyi yerlestirir.
///
/// `envp` dizisindeki her satir zaten bu bicimdedir; ayristirmayi tek
/// bir yerde tutmak, cagiranin ad ile degeri elle ayirmasini onluyor.
/// `=` icermeyen satirlar POSIX'te de yok sayilir.
pub fn set_entry(table: usize, text: &str) -> bool {
    match text.find('=') {
        Some(i) => set(table, &text[..i], &text[i + 1..]),
        None => false,
    }
}

/// Bir tabloyu digerinin uzerine kopyalar.
///
/// Tek kalip: yuva ayrilirken oturumdan, `fork`ta ebeveynden. Ikisi de
/// **anlik goruntu** -- kopyadan sonra iki taraf birbirini etkilemez.
fn copy_table(from: usize, to: usize) {
    if from >= TABLES || to >= TABLES || from == to {
        return;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let total = COUNTS[from].load(Ordering::Relaxed);
        for i in 0..total {
            core::ptr::copy_nonoverlapping(entry(from, i), entry(to, i), MAX_ENTRY);
            let base = lengths();
            let len = base.add(from * MAX_VARS + i).read();
            base.add(to * MAX_VARS + i).write(len);
        }
        COUNTS[to].store(total, Ordering::Relaxed);
    });
}

/// Yeni bir gorev yuvasi: ortam **oturum tablosundan** dogar.
///
/// `scheduler::spawn_inner`dan cagriliyor -- yani `execve` bu yola
/// ugramiyor ve ortam exec'ten sonra yasiyor.
pub fn reset(task: usize) {
    copy_table(SESSION, task);
}

/// `fork`: cocuk ebeveynin ortamini devralir.
pub fn clone_into(child: usize, parent: usize) {
    copy_table(parent, child);
}

/// Acilis ortamini kurar (oturum tablosu).
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
    set(SESSION, "HOME", home);
    set(SESSION, "PATH", "/bin");
    set(SESSION, "SHELL", "tcmk");
}
