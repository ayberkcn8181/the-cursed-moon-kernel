//! Dosya esleme nesneleri: `CreateFileMapping` / `MapViewOfFile`.
//!
//! Bir dosyayi okumak yerine **adreslemek** iki dunyada da var, ama
//! sekilleri farkli:
//!
//! ```text
//!   POSIX   mmap(NULL, len, PROT_READ, MAP_PRIVATE, fd, offset)
//!             -> tek cagri, dogrudan adres
//!
//!   Win32   CreateFileMappingA(hFile, ..) -> esleme NESNESI
//!           MapViewOfFile(nesne, ..)      -> adres
//!             -> iki cagri, arada bir tanitici
//! ```
//!
//! Aradaki tanitici bosuna degil. Windows'ta esleme nesnesi **adlandirilabilir**
//! ve baska surecler ayni adla acip ayni belleği paylasabilir; POSIX'te
//! ayni is `shm_open` ile ayri bir yoldan yapilir. TCMK adlandirmayi
//! desteklemiyor (paylasimli bellek yok), ama iki adimli yapiyi
//! **koruyor**: tek cagriya indirmek, gercek bir Windows programinin
//! bekledigi sirayi bozardi.
//!
//! ## Gorunum (view) nedir
//!
//! Esleme nesnesi "su dosya, su boy" der; **gorunum** o dosyanin bir
//! parcasini adres uzayina koyar. Bir nesnenin birden cok gorunumu
//! olabilir. TCMK gorev basina dorde kadar gorunum tutuyor.
//!
//! `UnmapViewOfFile` yalnizca **adres** alir, uzunluk almaz -- yani
//! uzunlugu cekirdek hatirlamak zorunda. POSIX `munmap`in ikisini birden
//! istemesinin sebebi de bu farkin tersi: orada cekirdek bir sey
//! hatirlamaz.
//!
//! ## Ne kopyalaniyor
//!
//! Icerik esleme aninda okunuyor; TCMK'de sayfa onbellegi yok. Yani
//! `PAGE_READWRITE` ile yapilan bir yazma **dosyaya gitmez** -- POSIX
//! tarafindaki `MAP_PRIVATE` ile ayni davranis. Bu, README'de acikca
//! yazili bir sadelestirme.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::scheduler;

/// Gorev basina esleme nesnesi ve gorunum sayisi.
const MAX_OBJECTS: usize = 4;
const MAX_VIEWS: usize = 4;

/// Esleme taniticilarini fd'lerden ve surec taniticilarindan ayiran bit.
pub const MAPPING_HANDLE_FLAG: usize = 0x4000_0000;

/// Nesne yuvalari: `fd + 1` (sifir = bos), ve nesnenin boyu.
static OBJECT_FD: [[AtomicUsize; MAX_OBJECTS]; scheduler::MAX_TASKS] =
    [const { [const { AtomicUsize::new(0) }; MAX_OBJECTS] }; scheduler::MAX_TASKS];
static OBJECT_SIZE: [[AtomicUsize; MAX_OBJECTS]; scheduler::MAX_TASKS] =
    [const { [const { AtomicUsize::new(0) }; MAX_OBJECTS] }; scheduler::MAX_TASKS];

/// Gorunum yuvalari: adres (sifir = bos) ve uzunluk.
static VIEW_ADDR: [[AtomicUsize; MAX_VIEWS]; scheduler::MAX_TASKS] =
    [const { [const { AtomicUsize::new(0) }; MAX_VIEWS] }; scheduler::MAX_TASKS];
static VIEW_LEN: [[AtomicUsize; MAX_VIEWS]; scheduler::MAX_TASKS] =
    [const { [const { AtomicUsize::new(0) }; MAX_VIEWS] }; scheduler::MAX_TASKS];

/// Yeni bir imaj bu yuvaya geldi: butun esleme durumu silinir.
pub fn reset(task: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    for slot in 0..MAX_OBJECTS {
        OBJECT_FD[task][slot].store(0, Ordering::Relaxed);
    }
    for slot in 0..MAX_VIEWS {
        VIEW_ADDR[task][slot].store(0, Ordering::Relaxed);
    }
}

/// Bir tanitici esleme nesnesi mi?
pub fn is_mapping(handle: usize) -> bool {
    handle >= MAPPING_HANDLE_FLAG && handle - MAPPING_HANDLE_FLAG < MAX_OBJECTS
}

fn index_of(handle: usize) -> Option<usize> {
    if is_mapping(handle) {
        Some(handle - MAPPING_HANDLE_FLAG)
    } else {
        None
    }
}

/// `CreateFileMappingA`. Doner: tanitici, ya da sifir.
///
/// `size` sifirsa dosyanin tamami eslenir -- Windows'un kurali da bu
/// (`dwMaximumSizeHigh/Low` sifir = "dosya kadar").
pub fn create(task: usize, fd: u32, size: usize) -> usize {
    if task >= scheduler::MAX_TASKS || size == 0 {
        return 0;
    }
    for slot in 0..MAX_OBJECTS {
        if OBJECT_FD[task][slot].load(Ordering::Relaxed) == 0 {
            OBJECT_FD[task][slot].store(fd as usize + 1, Ordering::Relaxed);
            OBJECT_SIZE[task][slot].store(size, Ordering::Relaxed);
            return MAPPING_HANDLE_FLAG + slot;
        }
    }
    0
}

/// Nesnenin tanimlayicisi ve boyu.
pub fn object(task: usize, handle: usize) -> Option<(u32, usize)> {
    if task >= scheduler::MAX_TASKS {
        return None;
    }
    let slot = index_of(handle)?;
    let fd = OBJECT_FD[task][slot].load(Ordering::Relaxed);
    if fd == 0 {
        return None;
    }
    Some((
        (fd - 1) as u32,
        OBJECT_SIZE[task][slot].load(Ordering::Relaxed),
    ))
}

/// `CloseHandle` bir esleme nesnesine geldiginde.
///
/// Gercek Windows'ta nesne kapansa bile **gorunumler yasar**: bellek,
/// son gorunum kaldirilana kadar durur. TCMK de ayni: burada yalnizca
/// yuva bosalir, `unmap_view` ayri calisir.
pub fn close(task: usize, handle: usize) -> bool {
    if task >= scheduler::MAX_TASKS {
        return false;
    }
    let Some(slot) = index_of(handle) else {
        return false;
    };
    if OBJECT_FD[task][slot].load(Ordering::Relaxed) == 0 {
        return false;
    }
    OBJECT_FD[task][slot].store(0, Ordering::Relaxed);
    true
}

/// Bir gorunumu kaydeder. Doner: yer varsa `true`.
pub fn remember_view(task: usize, addr: usize, len: usize) -> bool {
    if task >= scheduler::MAX_TASKS {
        return false;
    }
    for slot in 0..MAX_VIEWS {
        if VIEW_ADDR[task][slot].load(Ordering::Relaxed) == 0 {
            VIEW_ADDR[task][slot].store(addr, Ordering::Relaxed);
            VIEW_LEN[task][slot].store(len, Ordering::Relaxed);
            return true;
        }
    }
    false
}

/// `UnmapViewOfFile`: adresten uzunlugu bulur ve yuvayi bosaltir.
///
/// Uzunlugun cekirdekte tutulmasi sart: Win32 cagrisi yalnizca adres
/// alir. POSIX `munmap` ikisini birden ister, yani orada hatirlanacak
/// bir sey yok -- ayni isin iki sozlesmesi.
pub fn forget_view(task: usize, addr: usize) -> Option<usize> {
    if task >= scheduler::MAX_TASKS {
        return None;
    }
    for slot in 0..MAX_VIEWS {
        if VIEW_ADDR[task][slot].load(Ordering::Relaxed) == addr {
            VIEW_ADDR[task][slot].store(0, Ordering::Relaxed);
            return Some(VIEW_LEN[task][slot].load(Ordering::Relaxed));
        }
    }
    None
}
