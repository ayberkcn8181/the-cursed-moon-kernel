//! i386 Sanal Bellek Yonetimi -- Faz 2: 0-4 MiB identity map (doc S.7).
//!
//! Tek bir sayfa dizini (PD) + tek bir sayfa tablosu (PT) ile ilk 4 MiB
//! 4 KiB'lik sayfalar halinde birebir eslenir. Bu araliga doc S.5'teki tum
//! kritik bolgeler girer:
//!   0x000B8000  VGA metin buffer
//!   0x00100000  cekirdek imaji (linker.ld: . = 1M)
//!   0x00200000  kmalloc heap (1 MiB)
//! Cekirdek yigini da .bss icinde oldugundan bu araliktadir.
//!
//! Faz 4'te 16 MiB'e ve LAPIC bolgesine genisletilecek; kullanici sayfa
//! korumasi (User biti) Faz 3'te eklenecek.

use crate::arch::i386::{read_cr0, write_cr0, write_cr3};

const PAGE_SIZE: usize = 4096;
const ENTRIES: usize = 1024;
/// Bir PT = 1024 sayfa x 4 KiB = 4 MiB.
const IDENTITY_MAPPED_BYTES: usize = ENTRIES * PAGE_SIZE;

const PTE_PRESENT: u32 = 1 << 0;
const PTE_WRITABLE: u32 = 1 << 1;
const CR0_PG: u32 = 1 << 31;

/// Alanlara yalnizca ham isaretci uzerinden erisilir (asagida
/// `addr_of_mut!`), bu yuzden derleyici "hic okunmadi" sanir.
#[repr(align(4096))]
struct PageTable(#[allow(dead_code)] [u32; ENTRIES]);

static mut PAGE_DIRECTORY: PageTable = PageTable([0; ENTRIES]);
static mut FIRST_PAGE_TABLE: PageTable = PageTable([0; ENTRIES]);

/// Ilk 4 MiB'i identity map eder ve sayfalamayi acar.
///
/// # Safety
/// Yalnizca bir kez, kesmeler kapaliyken ve cekirdek 0-4 MiB araliginda
/// calisirken cagrilmalidir (aksi halde PG acildigi anda kod adresi
/// cozulemez ve triple fault olur).
pub unsafe fn init() {
    let pt = core::ptr::addr_of_mut!(FIRST_PAGE_TABLE) as *mut u32;
    for i in 0..ENTRIES {
        let phys = (i * PAGE_SIZE) as u32;
        pt.add(i).write(phys | PTE_PRESENT | PTE_WRITABLE);
    }

    let pd = core::ptr::addr_of_mut!(PAGE_DIRECTORY) as *mut u32;
    for i in 0..ENTRIES {
        pd.add(i).write(0);
    }
    pd.write(pt as u32 | PTE_PRESENT | PTE_WRITABLE);

    write_cr3(pd as u32);
    write_cr0(read_cr0() | CR0_PG);
}

pub fn is_enabled() -> bool {
    read_cr0() & CR0_PG != 0
}

pub fn identity_mapped_bytes() -> usize {
    IDENTITY_MAPPED_BYTES
}
