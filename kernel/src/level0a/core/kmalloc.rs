//! Cekirdek ici bump allocator (doc S.5: Heap 0x00200000-0x002FFFFF, 1 MiB).
//!
//! Faz 2 kapsaminda kasten en basit haliyle: tahsis edilen bellek geri
//! verilmez (`kfree` yok). Gercek bir serbest-liste/slab allocator Faz 9+
//! icin planlidir; su an tek tuketici scheduler'in gorev yiginlaridir.

use core::sync::atomic::{AtomicUsize, Ordering};

pub const HEAP_START: usize = 0x0020_0000;
pub const HEAP_SIZE: usize = 1024 * 1024; // 1 MiB
pub const HEAP_END: usize = HEAP_START + HEAP_SIZE;

static NEXT_FREE: AtomicUsize = AtomicUsize::new(HEAP_START);

/// `align` bayt hizali `size` baytlik blok ayirir; heap tukendiyse `None`.
///
/// Tek cekirdekli oldugumuz icin kesmeler kapatilarak atomik yapilir --
/// `AtomicUsize` burada yalnizca `static mut` uyarilarindan kacinmak icindir.
pub fn kmalloc_aligned(size: usize, align: usize) -> Option<*mut u8> {
    if size == 0 || !align.is_power_of_two() {
        return None;
    }

    crate::arch::cpu::without_interrupts(|| {
        let current = NEXT_FREE.load(Ordering::Relaxed);
        let start = (current + align - 1) & !(align - 1);
        let end = start.checked_add(size)?;

        if end > HEAP_END {
            return None;
        }

        NEXT_FREE.store(end, Ordering::Relaxed);
        Some(start as *mut u8)
    })
}

/// 16 bayt hizali varsayilan tahsis.
pub fn kmalloc(size: usize) -> Option<*mut u8> {
    kmalloc_aligned(size, 16)
}

pub fn used_bytes() -> usize {
    NEXT_FREE.load(Ordering::Relaxed) - HEAP_START
}

pub fn free_bytes() -> usize {
    HEAP_END - NEXT_FREE.load(Ordering::Relaxed)
}
