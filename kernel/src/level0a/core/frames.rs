//! Fiziksel cerceve (page frame) ayiricisi.
//!
//! `kmalloc` bir **bump** ayiricidir: serbest birakma yoktur, cunku
//! cekirdek yapilari acilista bir kez ayrilir ve omur boyu yasar. Surec
//! basina adres uzayi ise bunun tersini gerektirir: her surec kendi sayfa
//! dizinini ve veri cercevelerini alir, sonlandiginda **geri verir**.
//!
//! Bu yuzden ayri bir ayirici: 4 KiB'lik sabit boyutlu cerceveler uzerinde
//! bit haritasi. Sabit boyut, parcalanma (fragmentation) sorununu tumden
//! ortadan kaldirir -- sayfalama zaten 4 KiB tanecikli calisiyor.
//!
//! ## Havuz neden identity map icinde
//!
//! Cekirdek, bir surecin cercevesine ELF imajini **kopyalamak** zorundadir.
//! Cerceve identity map disinda olsaydi cekirdegin ona gecici bir esleme
//! kurmasi (ve her kopyalamada TLB temizlemesi) gerekirdi. Havuzu identity
//! araliginda tutmak bunun tamamini gereksiz kilar: cerceveye fiziksel
//! adresinden dogrudan yazilir.
//!
//! ## Referans sayaci (copy-on-write icin)
//!
//! Bit haritasi "ayrilmis mi?" sorusuna cevap verir; copy-on-write ise
//! "**kac surec** bakiyor?" sorusunu sorar. `fork` sonrasi ayni cerceve
//! iki adres uzayinda birden gorunur ve yalnizca **son** birakan onu
//! havuza geri vermelidir. Bu yuzden her cercevenin bir sayaci var:
//! `alloc` 1'den baslatir, `share` artirir, `free` azaltir ve sifira
//! inince cerceveyi gercekten serbest birakir.

use core::sync::atomic::{AtomicUsize, Ordering};

pub const FRAME_SIZE: usize = 4096;

/// Havuz 16 MiB'de baslar: cekirdek imaji (~5 MiB), heap (8-12 MiB) ve
/// kullanici bolgesinin sanal adresi (12-14 MiB) bunun altindadir.
pub const POOL_START: usize = 0x0100_0000;
/// 16 MiB havuz = 4096 cerceve.
pub const POOL_FRAMES: usize = 4096;

static mut BITMAP: [u8; POOL_FRAMES / 8] = [0; POOL_FRAMES / 8];
/// Cerceve basina referans sayaci. `u8` yeter: en fazla `MAX_TASKS` (8)
/// surec ayni cerceveyi paylasabilir. Yine de tasma denetleniyor --
/// tasarsa `share` reddeder ve cagiran tam kopyaya duser.
static mut REFCOUNT: [u8; POOL_FRAMES] = [0; POOL_FRAMES];
static USED: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// COW hatasinda gercekten kopyalanan sayfa sayisi.
static COW_COPIES: AtomicUsize = AtomicUsize::new(0);
/// COW hatasinda kopyalamaya **gerek kalmayan** sayfa sayisi: son sahip
/// kaldigi icin sayfa yalnizca yeniden yazilabilir yapildi.
static COW_REUSES: AtomicUsize = AtomicUsize::new(0);
/// Talep uzerine doldurulan sayfa sayisi: ayrilmis bir adrese ilk kez
/// dokunuldugu icin cerceve verildi.
static DEMAND_FILLS: AtomicUsize = AtomicUsize::new(0);

fn bit(index: usize) -> bool {
    unsafe {
        let bitmap = core::ptr::addr_of!(BITMAP) as *const u8;
        bitmap.add(index / 8).read() & (1 << (index % 8)) != 0
    }
}

fn index_of(phys: usize) -> Option<usize> {
    if phys < POOL_START {
        return None;
    }
    let index = (phys - POOL_START) / FRAME_SIZE;
    if index >= POOL_FRAMES {
        None
    } else {
        Some(index)
    }
}

fn refs(index: usize) -> u8 {
    unsafe { (core::ptr::addr_of!(REFCOUNT) as *const u8).add(index).read() }
}

fn set_refs(index: usize, value: u8) {
    unsafe {
        (core::ptr::addr_of_mut!(REFCOUNT) as *mut u8)
            .add(index)
            .write(value)
    };
}

fn set_bit(index: usize, value: bool) {
    unsafe {
        let bitmap = core::ptr::addr_of_mut!(BITMAP) as *mut u8;
        let byte = bitmap.add(index / 8);
        let mask = 1u8 << (index % 8);
        if value {
            byte.write(byte.read() | mask);
        } else {
            byte.write(byte.read() & !mask);
        }
    }
}

/// Bir cerceve ayirir ve **sifirlar**; fiziksel adresini doner.
///
/// Sifirlama pazarlik konusu degil: ayrilan cerceve bir sonraki surecin
/// adres uzayina girecek. Sifirlanmazsa onceki surecin verisi yeni surece
/// gorunur -- izolasyonun tam da engellemesi gereken sey.
pub fn alloc() -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| {
        for i in 0..POOL_FRAMES {
            if bit(i) {
                continue;
            }
            set_bit(i, true);
            set_refs(i, 1);
            let used = USED.fetch_add(1, Ordering::Relaxed) + 1;
            PEAK.fetch_max(used, Ordering::Relaxed);

            let phys = POOL_START + i * FRAME_SIZE;
            unsafe { core::ptr::write_bytes(phys as *mut u8, 0, FRAME_SIZE) };
            return Some(phys);
        }
        None
    })
}

/// Cerceveye bir referans daha ekler (`fork`'ta paylasim).
///
/// Havuz disi adreslerde ve sayac tasmasinda `false` doner; cagiran o
/// zaman tam kopyaya duser. Sessizce basarili donmek, birakilmayan bir
/// cerceve (sizinti) ya da erken serbest birakma demek olurdu.
pub fn share(phys: usize) -> bool {
    let index = match index_of(phys) {
        Some(i) => i,
        None => return false,
    };
    crate::arch::cpu::without_interrupts(|| {
        let current = refs(index);
        if !bit(index) || current == 0 || current == u8::MAX {
            return false;
        }
        set_refs(index, current + 1);
        true
    })
}

/// Cerceveye kac referans var. Ayrilmamis cerceve icin 0.
pub fn refcount(phys: usize) -> u8 {
    match index_of(phys) {
        Some(i) => crate::arch::cpu::without_interrupts(|| refs(i)),
        None => 0,
    }
}

/// Bir referansi birakir; **son** referans gidince cerceve havuza doner.
///
/// Havuz disi adresler sessizce yok sayilir (cekirdek yapilari
/// `kmalloc`'tan gelir, buraya ait degildir).
pub fn free(phys: usize) {
    let index = match index_of(phys) {
        Some(i) => i,
        None => return,
    };
    crate::arch::cpu::without_interrupts(|| {
        if !bit(index) {
            return;
        }
        let current = refs(index);
        if current > 1 {
            set_refs(index, current - 1);
            return;
        }
        set_refs(index, 0);
        set_bit(index, false);
        USED.fetch_sub(1, Ordering::Relaxed);
    });
}

/// Birden fazla adres uzayinda gorunen cerceve sayisi (kabuk raporu).
pub fn shared() -> usize {
    crate::arch::cpu::without_interrupts(|| {
        (0..POOL_FRAMES)
            .filter(|&i| bit(i) && refs(i) > 1)
            .count()
    })
}

pub fn note_cow_copy() {
    COW_COPIES.fetch_add(1, Ordering::Relaxed);
}

pub fn note_cow_reuse() {
    COW_REUSES.fetch_add(1, Ordering::Relaxed);
}

pub fn cow_copies() -> usize {
    COW_COPIES.load(Ordering::Relaxed)
}

pub fn cow_reuses() -> usize {
    COW_REUSES.load(Ordering::Relaxed)
}

pub fn note_demand_fill() {
    DEMAND_FILLS.fetch_add(1, Ordering::Relaxed);
}

pub fn demand_fills() -> usize {
    DEMAND_FILLS.load(Ordering::Relaxed)
}

pub fn used() -> usize {
    USED.load(Ordering::Relaxed)
}

pub fn free_count() -> usize {
    POOL_FRAMES - used()
}

pub fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

pub fn total() -> usize {
    POOL_FRAMES
}
