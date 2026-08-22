//! Modul tablosu: `GetModuleHandleA`, `GetProcAddress`, `LoadLibraryA`.
//!
//! Bir Windows programi calisirken kendi imajini ve yuklu DLL'leri
//! **sorabilir**. Uc soru vardir ve ucu de siradan:
//!
//! ```text
//!   GetModuleHandleA(NULL)          -> ben nereye yuklendim?
//!   GetModuleHandleA("KERNEL32.dll") -> su DLL yuklu mu?
//!   GetProcAddress(h, "Sleep")       -> su fonksiyonun adresi ne?
//! ```
//!
//! Ucuncusu pratikte cok kullanilir: bir program ithal tablosuna
//! koyamayacagi bir fonksiyonu (yeni bir Windows surumunde gelen, ya da
//! istege bagli bir ozellik) calisma zamaninda arar. Bulamazsa eski
//! yolu kullanir. Bu, "ithal edilmeyen sey yok" varsayimini kiran tek
//! Win32 yoludur.
//!
//! ## TCMK'de DLL diye bir dosya yok
//!
//! `KERNEL32.dll` diskte durmuyor; gomulu bir tablo (bkz. `dll.rs`) adi
//! bir NT servis numarasina ceviriyor ve yukleyici surecin adres
//! uzayina o servisi cagiran kucuk bir thunk yaziyor. Program IAT'de
//! gordugu seyi normal bir DLL girisinden ayirt edemiyor.
//!
//! Bunun iki sonucu var:
//!
//!   * **Tanitici (HMODULE) gercek bir adres degil.** Gercek Windows'ta
//!     HMODULE, DLL'in yuklendigi tabandir. Ortada imaj olmadigi icin
//!     burada etiketlenmis bir sayi kullaniliyor. Programin onu
//!     dereferans etmesi zaten desteklenmeyen bir davranistir.
//!   * **`GetProcAddress` thunk'i o anda uretir.** Ithal tablosundakiler
//!     yukleme aninda uretilir; buradan istenen ise imajin arkasinda
//!     ayrilmis kucuk bir alana yazilir (bkz. `pe32::RUNTIME_THUNKS`).
//!     Ayni fonksiyon tekrar istenirse onbellekten donulur -- yoksa bir
//!     dongu icindeki cagri alani tuketirdi.
//!
//! Surecin **kendi** imaji icin tanitici gercek bir adrestir: imaj
//! bellekte duruyor, yani Windows'un sozlesmesi orada aynen gecerli.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::scheduler;

use super::dll;

/// Sentetik DLL taniticilarinin taban degeri.
///
/// Gercek bir HMODULE yuklenmis imajin tabanidir. Burada imaj yok, o
/// yuzden kullanici bolgesine **hic denk gelmeyecek** bir aralik
/// seciliyor: boylece bir tanitici yanlislikla adres gibi kullanilirsa
/// sessizce yanlis veri okumak yerine hemen hata verir.
const SYNTHETIC_BASE: usize = 0x0D11_0000;
const SYNTHETIC_STRIDE: usize = 0x0001_0000;

/// Bir gorevin `GetProcAddress` onbelleginde kac giris tutulur.
const CACHE_SLOTS: usize = 16;

/// Surecin imaj tabani (`GetModuleHandleA(NULL)`), boyu ve giris noktasi.
static IMAGE_BASE: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
static IMAGE_SIZE: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
static IMAGE_ENTRY: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Calisma zamani thunk alani: siradaki bos adres ve sinir.
static ARENA_NEXT: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
static ARENA_END: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// `GetProcAddress` onbellegi: servis numarasi -> uretilmis thunk.
static CACHE_SERVICE: [[AtomicUsize; CACHE_SLOTS]; scheduler::MAX_TASKS] =
    [const { [const { AtomicUsize::new(0) }; CACHE_SLOTS] }; scheduler::MAX_TASKS];
static CACHE_ADDRESS: [[AtomicUsize; CACHE_SLOTS]; scheduler::MAX_TASKS] =
    [const { [const { AtomicUsize::new(0) }; CACHE_SLOTS] }; scheduler::MAX_TASKS];

/// Yeni bir PE imaji bu goreve yuklendi.
pub fn set_image(task: usize, base: usize, size: usize, entry: usize, arena: usize, arena_end: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    IMAGE_BASE[task].store(base, Ordering::Relaxed);
    IMAGE_SIZE[task].store(size, Ordering::Relaxed);
    IMAGE_ENTRY[task].store(entry, Ordering::Relaxed);
    ARENA_NEXT[task].store(arena, Ordering::Relaxed);
    ARENA_END[task].store(arena_end, Ordering::Relaxed);
    for slot in &CACHE_SERVICE[task] {
        slot.store(0, Ordering::Relaxed);
    }
}

/// Gorevin PE bilgisi silinir (ELF surecleri, yeni imaj).
pub fn reset(task: usize) {
    if task < scheduler::MAX_TASKS {
        IMAGE_BASE[task].store(0, Ordering::Relaxed);
        ARENA_NEXT[task].store(0, Ordering::Relaxed);
    }
}

pub fn image_base(task: usize) -> usize {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    IMAGE_BASE[task].load(Ordering::Relaxed)
}

pub fn image_size(task: usize) -> usize {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    IMAGE_SIZE[task].load(Ordering::Relaxed)
}

pub fn image_entry(task: usize) -> usize {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    IMAGE_ENTRY[task].load(Ordering::Relaxed)
}

/// `index`. sentetik DLL'in taniticisi.
pub fn synthetic_handle(index: usize) -> usize {
    SYNTHETIC_BASE + index * SYNTHETIC_STRIDE
}

/// Bir taniticiyi gomulu tablodaki sirasina cevirir.
fn synthetic_index(handle: usize) -> Option<usize> {
    if handle < SYNTHETIC_BASE {
        return None;
    }
    let offset = handle - SYNTHETIC_BASE;
    if offset % SYNTHETIC_STRIDE != 0 {
        return None;
    }
    let index = offset / SYNTHETIC_STRIDE;
    if index < dll::count() {
        Some(index)
    } else {
        None
    }
}

/// `GetModuleHandleA`. `name` `None` ise surecin kendi imaji.
///
/// Bulunamazsa sifir -- Windows'un sozlesmesi de bu (`GetModuleHandle`
/// yuklu **olmayan** bir DLL'i yuklemez, yalnizca bakar).
pub fn handle_for(task: usize, name: Option<&str>) -> usize {
    let Some(name) = name else {
        return image_base(task);
    };
    // Uzantisiz istenmis olabilir: `GetModuleHandleA("kernel32")`
    // gercek Windows'ta da calisir.
    for index in 0..dll::count() {
        let Some(full) = dll::name_at(index) else {
            continue;
        };
        if eq_ignore_case(full, name) || eq_ignore_case(strip_extension(full), name) {
            return synthetic_handle(index);
        }
    }
    // Surecin kendi adi da sorulabilir; imaj adiyla karsilastirmak icin
    // program yolu kullanilir.
    let program = crate::level0b1::process::program_path_of(task);
    if !program.is_empty() && eq_ignore_case(base_name(program), name) {
        return image_base(task);
    }
    0
}

/// `GetProcAddress`. `by_ordinal` doluysa ad yerine ordinal kullanilir.
///
/// Windows'ta ordinal ile arama, isaretcinin ust 16 biti sifir oldugunda
/// devreye girer (`MAKEINTRESOURCE`). Cagiran tarafi bunu ayirdigi icin
/// burada iki ayri parametre var.
///
/// # Safety
/// Cagiran gorevin adres uzayi etkin olmalidir: thunk **kullanicinin**
/// bellegine yazilir.
pub unsafe fn proc_address(
    task: usize,
    handle: usize,
    name: Option<&str>,
    by_ordinal: Option<u16>,
) -> usize {
    let Some(index) = synthetic_index(handle) else {
        // Surecin kendi imaji: ihracat tablosu yok. Gercek Windows'ta
        // bir EXE de ihracat yapabilir, ama TCMK'nin ureticileri
        // yapmiyor -- var gibi davranmak yaniltirdi.
        return 0;
    };
    let export = match (name, by_ordinal) {
        (Some(function), _) => dll::resolve_in(index, function),
        (None, Some(ordinal)) => dll::resolve_ordinal_in(index, ordinal),
        _ => None,
    };
    let Some(export) = export else {
        return 0;
    };

    // Ayni fonksiyon daha once istendiyse ayni adresi dondur. Gercek
    // Windows'ta da `GetProcAddress` her cagride ayni adresi verir --
    // programlar bunu karsilastirma icin kullanir.
    if let Some(cached) = cached_address(task, export.service) {
        return cached;
    }

    let next = ARENA_NEXT[task].load(Ordering::Relaxed);
    let end = ARENA_END[task].load(Ordering::Relaxed);
    if next == 0 || next + dll::THUNK_SIZE > end {
        return 0;
    }
    dll::emit_thunk(next, &export);
    ARENA_NEXT[task].store(next + dll::THUNK_SIZE, Ordering::Relaxed);
    remember_address(task, export.service, next);
    next
}

fn cached_address(task: usize, service: u32) -> Option<usize> {
    if task >= scheduler::MAX_TASKS {
        return None;
    }
    for slot in 0..CACHE_SLOTS {
        if CACHE_SERVICE[task][slot].load(Ordering::Relaxed) == service as usize + 1 {
            return Some(CACHE_ADDRESS[task][slot].load(Ordering::Relaxed));
        }
    }
    None
}

fn remember_address(task: usize, service: u32, address: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    for slot in 0..CACHE_SLOTS {
        if CACHE_SERVICE[task][slot].load(Ordering::Relaxed) == 0 {
            // +1: sifir "bos yuva" anlamina geliyor, ama servis numarasi
            // sifir olabilir. Kaydirma iki durumu ayiriyor.
            CACHE_SERVICE[task][slot].store(service as usize + 1, Ordering::Relaxed);
            CACHE_ADDRESS[task][slot].store(address, Ordering::Relaxed);
            return;
        }
    }
}

/// Bir dosya adindan uzantiyi atar (`KERNEL32.dll` -> `KERNEL32`).
fn strip_extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) => &name[..i],
        None => name,
    }
}

/// Bir yoldan yalnizca dosya adini alir.
pub fn base_name(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

fn eq_ignore_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}
