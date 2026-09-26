//! Takas alani: bir sayfayi diske atip cerceveyi geri almak.
//!
//! Cerceve havuzu 16 MiB. Uzun sure bunun tukenmesinin tek cevabi
//! **reddetmekti**: `frames::alloc` `None` doner, `fork`/`execve`
//! basarisiz olur, talep uzerine sayfalama hatayi normal yoluna
//! birakirdi. Yani sistem, hic dokunulmamis sayfalar yuzunden yeni bir
//! surec acamayabilirdi.
//!
//! Takas o cevabi degistiriyor: bir sayfa diske yazilir, cercevesi
//! havuza doner, ve sayfaya bir daha dokunulunca geri okunur.
//!
//! ## Alan nerede
//!
//! TCMKFS bolumunun **sonunda**. Bu secim bilincli: onde olsaydi veri
//! bloklarinin basladigi sektor kayardi ve eski imajlar sessizce yanlis
//! okunurdu. Sondan ayirmak yalnizca kapasiteyi kuculttugu icin eski
//! bir imaj (takas alani olmayan) aynen baglanmaya devam ediyor --
//! superblock'taki yuva sayisi sifir okunur ve takas kapali kalir.
//!
//! ```text
//!   ... veri bloklari ...  |  takas yuvalari (1024 x 4 KiB = 4 MiB)
//!                          ^
//!                          superblock'ta `swap_start`
//! ```
//!
//! ## Yuva neden dosya degil
//!
//! Takas dosya sisteminin **altinda** durmali: sayfa atmak, dosya
//! sistemi meta verisini degistirmemeli. Bir inode kullanmak, takas
//! yazmasinin inode tablosunu da diske yazmasi demekti -- ve bellek
//! sikisikken en son istenecek sey budur. Yuvalar ham sektor
//! araliklaridir, tek sahipleri bu modul.
//!
//! ## Es zamanlilik
//!
//! Disk islemleri sirasinda `yield` cagrilmiyor (bkz. `tcmkfs`), yani
//! isbirlikci zamanlamada ayrica kilit gerekmiyor. Bitmap islemleri
//! yine de kesmeler kapaliyken yapiliyor.

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::level0a::core::frames::FRAME_SIZE;
use crate::level0a::drivers::block::{self, SECTOR_SIZE};

/// Yuva basina sektor sayisi (4096 / 512).
const SECTORS_PER_SLOT: u32 = (FRAME_SIZE / SECTOR_SIZE) as u32;

/// Ayrilan yuva sayisi -- 1024 x 4 KiB = 4 MiB.
///
/// Cerceve havuzunun dortte biri. Daha buyugu diski bosuna yer, daha
/// kucugu baski altinda hemen dolar; bu oran "birkac surec sikisabilsin"
/// demek, "sinirsiz bellek" degil.
pub const SLOTS: u32 = 1024;

/// Takas alaninin bolum icindeki ilk sektoru (0 = takas yok).
static START_SECTOR: AtomicU32 = AtomicU32::new(0);
/// Bolumun disk uzerindeki ilk sektoru.
static PART_LBA: AtomicU32 = AtomicU32::new(0);
static SLOT_COUNT: AtomicU32 = AtomicU32::new(0);
static USED_SLOTS: AtomicU32 = AtomicU32::new(0);

static PAGES_OUT: AtomicUsize = AtomicUsize::new(0);
static PAGES_IN: AtomicUsize = AtomicUsize::new(0);
static FAILURES: AtomicUsize = AtomicUsize::new(0);

/// Yuva basina bir bit. 1024 yuva = 128 bayt.
static mut BITMAP: [u8; (SLOTS / 8) as usize] = [0; (SLOTS / 8) as usize];

/// Takas alanini bildirir; `slots` sifirsa takas kapatilir.
///
/// Bitmap **her baglamada sifirlaniyor**: takasin diskte kalici bir
/// anlami yok. Bir sayfa ancak onu bekleyen bir adres uzayi varken
/// anlamlidir ve o uzaylar yeniden baslatmayi gecmez; eski yuvalari
/// dolu saymak yalnizca alani kaybettirirdi.
pub fn init(part_lba: u32, start_sector: u32, slots: u32) {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let bitmap = core::ptr::addr_of_mut!(BITMAP) as *mut u8;
        core::ptr::write_bytes(bitmap, 0, (SLOTS / 8) as usize);
    });
    PART_LBA.store(part_lba, Ordering::Relaxed);
    START_SECTOR.store(if slots == 0 { 0 } else { start_sector }, Ordering::Relaxed);
    SLOT_COUNT.store(slots.min(SLOTS), Ordering::Relaxed);
    USED_SLOTS.store(0, Ordering::Relaxed);
}

/// Takas kullanilabilir mi.
pub fn available() -> bool {
    START_SECTOR.load(Ordering::Relaxed) != 0 && SLOT_COUNT.load(Ordering::Relaxed) > 0
}

fn bit_get(slot: u32) -> bool {
    unsafe {
        let bitmap = core::ptr::addr_of!(BITMAP) as *const u8;
        bitmap.add(slot as usize / 8).read() & (1 << (slot % 8)) != 0
    }
}

fn bit_set(slot: u32, value: bool) {
    unsafe {
        let bitmap = core::ptr::addr_of_mut!(BITMAP) as *mut u8;
        let byte = bitmap.add(slot as usize / 8);
        let mask = 1u8 << (slot % 8);
        if value {
            byte.write(byte.read() | mask);
        } else {
            byte.write(byte.read() & !mask);
        }
    }
}

/// Bos bir yuva ayirir.
pub fn alloc_slot() -> Option<u32> {
    if !available() {
        return None;
    }
    crate::arch::cpu::without_interrupts(|| {
        let count = SLOT_COUNT.load(Ordering::Relaxed);
        for slot in 0..count {
            if !bit_get(slot) {
                bit_set(slot, true);
                USED_SLOTS.fetch_add(1, Ordering::Relaxed);
                return Some(slot);
            }
        }
        None
    })
}

/// Yuvayi geri verir.
///
/// Icerigi silinmiyor: yuva yeniden verildiginde uzerine tam bir sayfa
/// yaziliyor, yani eski veri hicbir zaman **okunmuyor**. Silmek bir
/// sayfalik gereksiz yazma olurdu.
pub fn free_slot(slot: u32) {
    crate::arch::cpu::without_interrupts(|| {
        if slot < SLOT_COUNT.load(Ordering::Relaxed) && bit_get(slot) {
            bit_set(slot, false);
            USED_SLOTS.fetch_sub(1, Ordering::Relaxed);
        }
    });
}

fn slot_lba(slot: u32) -> Option<u32> {
    if slot >= SLOT_COUNT.load(Ordering::Relaxed) {
        return None;
    }
    let start = START_SECTOR.load(Ordering::Relaxed);
    if start == 0 {
        return None;
    }
    Some(PART_LBA.load(Ordering::Relaxed) + start + slot * SECTORS_PER_SLOT)
}

/// Cerceveyi yuvaya yazar.
///
/// `phys` identity haritasinin icinde oldugu icin dogrudan okunabiliyor
/// (bkz. `frames`): cerceve havuzu bilerek oraya konmustu.
///
/// # Safety
/// `phys` gecerli, havuza ait bir cerceve olmali.
pub unsafe fn write_frame(slot: u32, phys: usize) -> bool {
    let lba = match slot_lba(slot) {
        Some(l) => l,
        None => return false,
    };
    let bytes = core::slice::from_raw_parts(phys as *const u8, FRAME_SIZE);
    match block::write(lba, SECTORS_PER_SLOT as u8, bytes) {
        Ok(()) => {
            PAGES_OUT.fetch_add(1, Ordering::Relaxed);
            true
        }
        Err(_) => {
            FAILURES.fetch_add(1, Ordering::Relaxed);
            false
        }
    }
}

/// Yuvayi cerceveye geri okur.
///
/// # Safety
/// `phys` gecerli, havuza ait bir cerceve olmali.
pub unsafe fn read_frame(slot: u32, phys: usize) -> bool {
    let lba = match slot_lba(slot) {
        Some(l) => l,
        None => return false,
    };
    let bytes = core::slice::from_raw_parts_mut(phys as *mut u8, FRAME_SIZE);
    match block::read(lba, SECTORS_PER_SLOT as u8, bytes) {
        Ok(()) => {
            PAGES_IN.fetch_add(1, Ordering::Relaxed);
            true
        }
        Err(_) => {
            FAILURES.fetch_add(1, Ordering::Relaxed);
            false
        }
    }
}

pub fn total_slots() -> u32 {
    SLOT_COUNT.load(Ordering::Relaxed)
}

pub fn used_slots() -> u32 {
    USED_SLOTS.load(Ordering::Relaxed)
}

/// Diske atilan sayfa sayisi.
pub fn pages_out() -> usize {
    PAGES_OUT.load(Ordering::Relaxed)
}

/// Diskten geri okunan sayfa sayisi.
pub fn pages_in() -> usize {
    PAGES_IN.load(Ordering::Relaxed)
}

/// Basarisiz disk islemi sayisi.
///
/// Ayri bir sayac olmasi bilincli: takasin **sessizce** calismamasi,
/// calismamasindan daha kotudur.
pub fn failures() -> usize {
    FAILURES.load(Ordering::Relaxed)
}
