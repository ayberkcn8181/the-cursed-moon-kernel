//! Cekirdek ici bump allocator.
//!
//! Heap 8 MiB'e tasindi: framebuffer'in 3 MiB'lik arka tamponu cekirdek
//! `.bss`'ini ~5 MiB'e cikardi ve eski 2 MiB'lik heap konumu imajin
//! uzerine biniyordu (doc S.5'teki harita Faz 13 ile guncellendi).
//!
//! Faz 2 kapsaminda kasten en basit haliyle: tahsis edilen bellek geri
//! verilmez (`kfree` yok). Gercek bir serbest-liste/slab allocator Faz 9+
//! icin planlidir; su an tek tuketici scheduler'in gorev yiginlaridir.

use core::sync::atomic::{AtomicUsize, Ordering};

pub const HEAP_START: usize = 0x0080_0000; // 8 MiB
pub const HEAP_SIZE: usize = 4 * 1024 * 1024; // 4 MiB
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
