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

use crate::arch::cpu::{read_cr0, write_cr0, write_cr3};

const PAGE_SIZE: usize = 4096;
const ENTRIES: usize = 1024;
/// Bir PT = 1024 sayfa x 4 KiB = 4 MiB; dort tablo ile 16 MiB.
const IDENTITY_TABLES: usize = 4;
const IDENTITY_MAPPED_BYTES: usize = IDENTITY_TABLES * ENTRIES * PAGE_SIZE;

const PTE_PRESENT: u32 = 1 << 0;
const PTE_WRITABLE: u32 = 1 << 1;
const PTE_USER: u32 = 1 << 2;
const CR0_PG: u32 = 1 << 31;

/// Kullanici (Ring 3) bellek bolgesinin baslangici -- doc'taki
/// `TCMK_USER_MEM_START` ile ayni deger.
pub const USER_MEM_START: usize = 0x00C0_0000; // 12 MiB
pub const USER_MEM_SIZE: usize = 0x0020_0000; // 2 MiB

/// Alanlara yalnizca ham isaretci uzerinden erisilir (asagida
/// `addr_of_mut!`), bu yuzden derleyici "hic okunmadi" sanir.
#[repr(align(4096))]
#[derive(Clone, Copy)]
struct PageTable(#[allow(dead_code)] [u32; ENTRIES]);

static mut PAGE_DIRECTORY: PageTable = PageTable([0; ENTRIES]);
static mut IDENTITY_PAGE_TABLES: [PageTable; IDENTITY_TABLES] =
    [PageTable([0; ENTRIES]); IDENTITY_TABLES];

/// MMIO bolgeleri (framebuffer gibi) icin ek sayfa tablolari.
/// Her tablo 4 MiB'lik bir pencere esler.
const MMIO_SLOTS: usize = 2;
static mut MMIO_TABLES: [PageTable; MMIO_SLOTS] = [PageTable([0; ENTRIES]); MMIO_SLOTS];
static mut MMIO_USED: usize = 0;

/// Ilk 4 MiB'i identity map eder ve sayfalamayi acar.
///
/// # Safety
/// Yalnizca bir kez, kesmeler kapaliyken ve cekirdek 0-4 MiB araliginda
/// calisirken cagrilmalidir (aksi halde PG acildigi anda kod adresi
/// cozulemez ve triple fault olur).
pub unsafe fn init() {
    let pd = core::ptr::addr_of_mut!(PAGE_DIRECTORY) as *mut u32;
    for i in 0..ENTRIES {
        pd.add(i).write(0);
    }

    let tables = core::ptr::addr_of_mut!(IDENTITY_PAGE_TABLES) as *mut PageTable;
    for t in 0..IDENTITY_TABLES {
        let pt = tables.add(t) as *mut u32;
        for i in 0..ENTRIES {
            let phys = ((t * ENTRIES + i) * PAGE_SIZE) as u32;
            pt.add(i).write(phys | PTE_PRESENT | PTE_WRITABLE);
        }
        pd.add(t).write(pt as u32 | PTE_PRESENT | PTE_WRITABLE);
    }

    write_cr3(pd as u32);
    write_cr0(read_cr0() | CR0_PG);
}

/// Bir sanal adresin identity map icindeki PTE'sine isaretci dondurur.
unsafe fn identity_pte(page: usize) -> Option<*mut u32> {
    if page >= IDENTITY_TABLES * ENTRIES {
        return None;
    }
    let tables = core::ptr::addr_of_mut!(IDENTITY_PAGE_TABLES) as *mut PageTable;
    let pt = tables.add(page / ENTRIES) as *mut u32;
    Some(pt.add(page % ENTRIES))
}

/// Verilen araligi Ring 3'ten erisilebilir yapar (PTE User biti).
///
/// PDE'ye de User biti konur; bu tek basina cekirdegi acmaz, cunku gercek
/// erisim kontrolu PDE **ve** PTE'nin birlikte User olmasini gerektirir --
/// cekirdek sayfalarinin PTE'lerinde User biti kapali kaldigi surece
/// Ring 3 onlara erisemez (doc S.7 Faz 5: `mmu_protect_user_range`).
///
/// # Safety
/// Yalnizca gercekten kullaniciya ait olmasi gereken araliklar icin
/// cagrilmalidir; cekirdek bolgeleri verilirse izolasyon kirilir.
pub unsafe fn protect_user_range(start: usize, len: usize) {
    if len == 0 {
        return;
    }

    let first = start / PAGE_SIZE;
    let last = (start + len - 1) / PAGE_SIZE;

    for page in first..=last {
        match identity_pte(page) {
            Some(entry) => entry.write(entry.read() | PTE_USER | PTE_WRITABLE),
            None => break,
        }
    }

    // Ilgili PDE'ye de User biti gerekir (erisim PDE ve PTE ile AND'lenir).
    let pd = core::ptr::addr_of_mut!(PAGE_DIRECTORY) as *mut u32;
    for pde_index in (start >> 22)..=((start + len - 1) >> 22) {
        let e = pd.add(pde_index);
        e.write(e.read() | PTE_USER);
    }

    flush_tlb();
}

/// CR3'u yeniden yukleyerek TLB'yi bosaltir.
unsafe fn flush_tlb() {
    let pd = core::ptr::addr_of!(PAGE_DIRECTORY) as u32;
    write_cr3(pd);
}

/// Bir adresin Ring 3'e acik olup olmadigini bildirir (dogrulama amacli).
pub fn is_user_accessible(addr: usize) -> bool {
    unsafe {
        match identity_pte(addr / PAGE_SIZE) {
            Some(entry) => entry.read() & PTE_USER != 0,
            None => false,
        }
    }
}

/// Bir MMIO bolgesini (framebuffer gibi) birebir esler.
///
/// Bolge ilk 4 MiB'in disinda oldugundan kendi sayfa tablosunu gerektirir.
/// Cache devre disi (PCD) **birakilmaz**: framebuffer'a write-back cache ile
/// yazmak cok daha hizlidir ve `present()` zaten tek yonlu kopyalama yapar.
///
/// # Safety
/// `phys` gercek bir donanim bolgesi olmalidir; yanlis adres eslemek
/// rastgele bellegi bozar.
pub unsafe fn map_mmio(phys: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }

    let first_pde = phys >> 22;
    let last_pde = (phys + len - 1) >> 22;
    let needed = last_pde - first_pde + 1;

    if MMIO_USED + needed > MMIO_SLOTS {
        return false;
    }

    let pd = core::ptr::addr_of_mut!(PAGE_DIRECTORY) as *mut u32;

    for (slot_offset, pde_index) in (first_pde..=last_pde).enumerate() {
        let table = (core::ptr::addr_of_mut!(MMIO_TABLES) as *mut PageTable)
            .add(MMIO_USED + slot_offset) as *mut u32;

        let base = pde_index << 22;
        for i in 0..ENTRIES {
            let page = base + i * PAGE_SIZE;
            table.add(i).write(page as u32 | PTE_PRESENT | PTE_WRITABLE);
        }

        pd.add(pde_index)
            .write(table as u32 | PTE_PRESENT | PTE_WRITABLE);
    }

    MMIO_USED += needed;
    flush_tlb();
    true
}

pub fn is_enabled() -> bool {
    read_cr0() & CR0_PG != 0
}

pub fn identity_mapped_bytes() -> usize {
    IDENTITY_MAPPED_BYTES
}
