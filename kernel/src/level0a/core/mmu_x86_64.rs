//! x86_64 Sanal Bellek Yonetimi -- 4 seviyeli sayfalama.
//!
//! Boot stub'i (bkz. `boot/x86_64.rs`) Long Mode'a girebilmek icin ilk
//! 1 GiB'i **2 MiB'lik sayfalarla** zaten birebir eslemis durumdadir. Bu
//! modul o tablolari devralir ve iki is yapar:
//!
//!   1. Kullanici bolgesini Ring 3'e acmak (PTE User biti)
//!   2. Esleme durumunu sorgulamak (`is_user_accessible`)
//!
//! Kullanici bolgesi 2 MiB'lik tek bir sayfaya sigmadigi/paylasilmamasi
//! gerektigi icin, o bolgeyi kapsayan 2 MiB'lik girdi **4 KiB'lik bir
//! sayfa tablosuna bolunur** (split). Boylece kullaniciya yalnizca kendi
//! sayfalari acilir; ayni 2 MiB icindeki cekirdek heap'i kapali kalir.

use crate::arch::cpu::{read_cr0, read_cr3, write_cr3};

const PAGE_SIZE: usize = 4096;
const LARGE_PAGE_SIZE: usize = 2 * 1024 * 1024;
const ENTRIES: usize = 512;

const PTE_PRESENT: u64 = 1 << 0;
const PTE_WRITABLE: u64 = 1 << 1;
const PTE_USER: u64 = 1 << 2;
const PTE_HUGE: u64 = 1 << 7;
const CR0_PG: u64 = 1 << 31;

const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// Kullanici (Ring 3) bellek bolgesi -- i386 ile ayni adresler.
pub const USER_MEM_START: usize = 0x00C0_0000; // 12 MiB
pub const USER_MEM_SIZE: usize = 0x0020_0000; // 2 MiB (tam bir 2 MiB sayfa)

/// Boot stub'inin ilk 1 GiB icin kurdugu esleme.
const IDENTITY_MAPPED_BYTES: usize = 1024 * 1024 * 1024;

/// Bir 2 MiB'lik girdinin yerine gecen 4 KiB'lik sayfa tablosu.
#[repr(align(4096))]
struct PageTable(#[allow(dead_code)] [u64; ENTRIES]);

/// Bolunmus 2 MiB girdileri icin tablo havuzu.
///
/// **Neden havuz:** basta tek bir statik tablo vardi ve "kullanici
/// bolgesi" icin yeterli sanilmisti. Degildi -- Ring 3'e acilan tek sey
/// program imaji degil: pencere piksel tamponlari da aciliyor ve onlar
/// cekirdek yiginindan (baska bir 2 MiB bolgesinden) geliyor. Tek tablo
/// paylasildiginda **ikinci bolme birincinin tablosunu caliyordu**: PD[6]
/// hala eski tabloyu gosterirken tablonun icerigi baska bir bolgeye
/// ait fiziksel adreslerle doluyor, yani calisan programin kod sayfasi
/// ayagindan kayiyordu. Belirti, uygulamanin ilk pencere cagrisindan
/// hemen sonra kendi kodunda page fault almasiydi.
const MAX_SPLITS: usize = 8;
static mut SPLIT_TABLES: [PageTable; MAX_SPLITS] =
    [const { PageTable([0; ENTRIES]) }; MAX_SPLITS];
/// Her tablonun hangi PD girdisine ait oldugu (`usize::MAX` = bos).
static mut SPLIT_OWNER: [usize; MAX_SPLITS] = [usize::MAX; MAX_SPLITS];

/// `pd_index` icin tablo dondurur; yoksa havuzdan ayirir.
///
/// # Safety
/// Yalnizca sayfalama kurulumu sirasinda, kesmeler kapaliyken.
unsafe fn split_table_for(pd_index: usize) -> Option<*mut u64> {
    let owners = core::ptr::addr_of_mut!(SPLIT_OWNER) as *mut usize;
    let tables = core::ptr::addr_of_mut!(SPLIT_TABLES) as *mut PageTable;

    for i in 0..MAX_SPLITS {
        if owners.add(i).read() == pd_index {
            return Some(tables.add(i) as *mut u64);
        }
    }
    for i in 0..MAX_SPLITS {
        if owners.add(i).read() == usize::MAX {
            owners.add(i).write(pd_index);
            return Some(tables.add(i) as *mut u64);
        }
    }
    None
}

/// Long Mode'da sayfalama zaten aciktir (aksi halde buraya gelinemezdi).
/// Bu fonksiyon yalnizca durumu dogrular ve raporlar.
///
/// # Safety
/// Yalnizca bir kez, kesmeler kapaliyken cagrilmalidir.
pub unsafe fn init() {
    // Boot stub'i CR3'u kurdu; burada dogrulamaktan baska bir sey gerekmez.
    debug_assert!(is_enabled());
}

pub fn is_enabled() -> bool {
    read_cr0() & CR0_PG != 0
}

pub fn identity_mapped_bytes() -> usize {
    IDENTITY_MAPPED_BYTES
}

/// Bir seviyedeki tablo girdisine isaretci dondurur.
unsafe fn table_entry(table_phys: u64, index: usize) -> *mut u64 {
    (table_phys as *mut u64).add(index)
}

/// Verilen araligi Ring 3'ten erisilebilir yapar.
///
/// Aralik 2 MiB'lik bir buyuk sayfanin icindeyse o girdi once 4 KiB'lik
/// bir tabloya bolunur; boylece ayni 2 MiB'i paylasan cekirdek bellegi
/// (ornegin heap) Ring 3'e acilmaz.
///
/// # Safety
/// Yalnizca gercekten kullaniciya ait araliklar icin cagrilmalidir.
pub unsafe fn protect_user_range(start: usize, len: usize) {
    if len == 0 {
        return;
    }

    let pml4 = read_cr3() & ADDR_MASK;
    // Ilk 1 GiB icinde oldugumuz icin PML4[0] -> PDPT[0] -> PD sabittir.
    let pdpt = table_entry(pml4, 0).read() & ADDR_MASK;
    let pd = table_entry(pdpt, 0).read() & ADDR_MASK;

    let first_page = start / PAGE_SIZE;
    let last_page = (start + len - 1) / PAGE_SIZE;

    // Aralik hangi 2 MiB'lik girdilere denk geliyor?
    let first_pd = start / LARGE_PAGE_SIZE;
    let last_pd = (start + len - 1) / LARGE_PAGE_SIZE;

    for pd_index in first_pd..=last_pd {
        let pd_entry_ptr = table_entry(pd, pd_index);
        let pd_entry = pd_entry_ptr.read();

        let pt_phys = if pd_entry & PTE_HUGE != 0 {
            // 2 MiB'lik girdiyi 4 KiB'lik tabloya bol. Her PD girdisi
            // KENDI tablosunu alir (bkz. `split_table_for`).
            let pt = match split_table_for(pd_index) {
                Some(p) => p,
                None => {
                    crate::println!(
                        "[LEVEL-0a] mmu: bolme tablosu havuzu doldu (PD #{}).",
                        pd_index
                    );
                    return;
                }
            };
            let base = (pd_index * LARGE_PAGE_SIZE) as u64;
            for i in 0..ENTRIES {
                let phys = base + (i * PAGE_SIZE) as u64;
                // Varsayilan: cekirdege ait, Ring 3'e KAPALI.
                pt.add(i).write(phys | PTE_PRESENT | PTE_WRITABLE);
            }
            pd_entry_ptr.write(pt as u64 | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
            pt as u64
        } else {
            pd_entry & ADDR_MASK
        };

        // Bu 2 MiB icindeki, istenen aralikla kesisen sayfalari ac.
        let page_lo = (pd_index * LARGE_PAGE_SIZE) / PAGE_SIZE;
        for page in first_page.max(page_lo)..=last_page.min(page_lo + ENTRIES - 1) {
            let idx = page - page_lo;
            let entry = table_entry(pt_phys, idx);
            entry.write(entry.read() | PTE_USER | PTE_WRITABLE);
        }
    }

    // PML4/PDPT/PD zincirinde de User biti olmali (erisim AND'lenir).
    let pml4_e = table_entry(pml4, 0);
    pml4_e.write(pml4_e.read() | PTE_USER);
    let pdpt_e = table_entry(pdpt, 0);
    pdpt_e.write(pdpt_e.read() | PTE_USER);

    flush_tlb();
}

unsafe fn flush_tlb() {
    write_cr3(read_cr3());
}

/// Bir adresin Ring 3'e acik olup olmadigini bildirir.
pub fn is_user_accessible(addr: usize) -> bool {
    unsafe {
        let pml4 = read_cr3() & ADDR_MASK;
        let pml4_e = table_entry(pml4, (addr >> 39) & 0x1FF).read();
        if pml4_e & PTE_PRESENT == 0 || pml4_e & PTE_USER == 0 {
            return false;
        }

        let pdpt = pml4_e & ADDR_MASK;
        let pdpt_e = table_entry(pdpt, (addr >> 30) & 0x1FF).read();
        if pdpt_e & PTE_PRESENT == 0 || pdpt_e & PTE_USER == 0 {
            return false;
        }

        let pd = pdpt_e & ADDR_MASK;
        let pd_e = table_entry(pd, (addr >> 21) & 0x1FF).read();
        if pd_e & PTE_PRESENT == 0 || pd_e & PTE_USER == 0 {
            return false;
        }
        if pd_e & PTE_HUGE != 0 {
            return true; // 2 MiB sayfa, User biti zaten dogrulandi
        }

        let pt = pd_e & ADDR_MASK;
        let pt_e = table_entry(pt, (addr >> 12) & 0x1FF).read();
        pt_e & PTE_PRESENT != 0 && pt_e & PTE_USER != 0
    }
}

// --- Surec basina adres uzayi (x86_64: henuz yok) --------------------------
//
// i386 tarafinda her surec kendi PDE'sini alir (bkz. `mmu_i386`). x86_64'te
// dort seviyeli tablo ve 2 MiB huge page bolunmesi ayni isi daha fazla
// muhasebe ile yapmayi gerektiriyor; bu port su an **tek adres uzayi**
// modelinde kaliyor. Ust katmanlar farki gormesin diye ayni API burada da
// var: `create_user_space` `None` doner ve cagiran paylasimli yola duser.

/// x86_64'te surec basina adres uzayi henuz yok.
///
/// # Safety
/// Cagri guvenlidir; imza i386 ile ayni kalsin diye `unsafe`.
pub unsafe fn create_user_space() -> Option<usize> {
    None
}

/// # Safety
/// Bkz. `create_user_space`.
pub unsafe fn destroy_user_space(_cr3: usize) {}

/// # Safety
/// Bkz. `create_user_space`.
pub unsafe fn switch_to(_cr3: usize) {}

pub fn kernel_cr3() -> usize {
    0
}

/// Paylasimli modelde esleme zaten hazir; yalnizca Ring 3'e acilir.
///
/// # Safety
/// `protect_user_range` ile ayni onkosullar.
pub unsafe fn map_user_range(_cr3: usize, start: usize, len: usize) -> bool {
    protect_user_range(start, len);
    true
}

pub fn user_pages(_cr3: usize) -> usize {
    0
}

/// Paylasimli modelde tampon zaten cekirdek haritasinda; yalnizca acilir.
///
/// # Safety
/// `protect_user_range` ile ayni onkosullar.
pub unsafe fn map_user_frames(_cr3: usize, _vaddr: usize, phys: usize, len: usize) -> bool {
    protect_user_range(phys, len);
    true
}

/// Paylasimli modelde tum kullanici bolgesi tek surece aittir.
pub const USER_MAP_SIZE: usize = USER_MEM_SIZE;
