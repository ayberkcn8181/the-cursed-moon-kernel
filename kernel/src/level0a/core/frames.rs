// x86_64 portu tek adres uzayinda calistigi icin (bkz. `mmu_x86_64`)
// cerceve ayiricisini kullanmaz; orada bu modul bilerek olu kod olur.
#![cfg_attr(target_arch = "x86_64", allow(dead_code))]

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

use core::sync::atomic::{AtomicUsize, Ordering};

pub const FRAME_SIZE: usize = 4096;

/// Havuz 16 MiB'de baslar: cekirdek imaji (~5 MiB), heap (8-12 MiB) ve
/// kullanici bolgesinin sanal adresi (12-14 MiB) bunun altindadir.
pub const POOL_START: usize = 0x0100_0000;
/// 16 MiB havuz = 4096 cerceve.
pub const POOL_FRAMES: usize = 4096;

static mut BITMAP: [u8; POOL_FRAMES / 8] = [0; POOL_FRAMES / 8];
static USED: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn bit(index: usize) -> bool {
    unsafe {
        let bitmap = core::ptr::addr_of!(BITMAP) as *const u8;
        bitmap.add(index / 8).read() & (1 << (index % 8)) != 0
    }
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
            let used = USED.fetch_add(1, Ordering::Relaxed) + 1;
            PEAK.fetch_max(used, Ordering::Relaxed);

            let phys = POOL_START + i * FRAME_SIZE;
            unsafe { core::ptr::write_bytes(phys as *mut u8, 0, FRAME_SIZE) };
            return Some(phys);
        }
        None
    })
}

/// Cerceveyi havuza geri verir. Havuz disi adresler sessizce yok sayilir
/// (cekirdek yapilari `kmalloc`'tan gelir, buraya ait degildir).
pub fn free(phys: usize) {
    if phys < POOL_START {
        return;
    }
    let index = (phys - POOL_START) / FRAME_SIZE;
    if index >= POOL_FRAMES {
        return;
    }
    crate::arch::cpu::without_interrupts(|| {
        if bit(index) {
            set_bit(index, false);
            USED.fetch_sub(1, Ordering::Relaxed);
        }
    });
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
