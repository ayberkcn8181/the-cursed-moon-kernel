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

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::arch::cpu::{read_cr0, read_cr3, write_cr3};
use crate::level0a::core::frames;

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
    // Surecler bu tablodan turetilir, o yuzden saklanmasi sart.
    KERNEL_CR3.store((read_cr3() & ADDR_MASK) as usize, Ordering::Relaxed);
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

// --- Surec basina adres uzayi -------------------------------------------
//
// i386'da her surec kullanici bolgesini kaplayan tek bir PDE'yi kendine
// alir. x86_64'te ayni fikir bir seviye daha derine iner: surec kendi
// PML4/PDPT/PD ucusunu alir, cekirdek girdileri **kopyalanir** (yani
// heap, cerceve havuzu ve LAPIC her uzayda gorunur olmaya devam eder),
// yalnizca kullanici bolgesini kaplayan PD girdileri kendi sayfa
// tablolarina yonlendirilir.
//
// Kullanici bolgesi neden IKI PD girdisi: bir PD girdisi 2 MiB kapsar.
// Program imaji PD[6]'da (0x00C00000-0x00DFFFFF) durur, ama pencere
// piksel tamponlari 0x00D00000'dan baslayip dort yuvayla 0x00F00000'a
// kadar uzanir -- yani PD[7]'ye tasar. i386'da bir PDE 4 MiB kapsadigi
// icin bu sorun hic cikmamisti.
//
// Bu bolgede cekirdege ait veri yok (heap 8-12 MiB'de biter, cerceve
// havuzu 16 MiB'de baslar), bu yuzden ikisini de surece devretmek
// guvenlidir.

/// Kullanici bolgesini kaplayan PD girdileri.
const USER_PD_FIRST: usize = USER_MEM_START / LARGE_PAGE_SIZE; // 6
const USER_PD_LAST: usize = USER_PD_FIRST + 1; // 7 -- pencere yuvalari

/// Acilistaki (cekirdek) CR3; surecler bundan turetilir.
static KERNEL_CR3: AtomicUsize = AtomicUsize::new(0);

/// Yeni bir kullanici adres uzayi kurar; CR3 degerini dondurur.
///
/// Bes cerceve gerekir: PML4, PDPT, PD ve iki kullanici sayfa tablosu.
/// Cekirdek girdileri kopyalandigi icin surec calisirken cekirdek hala
/// kendi heap'ini ve cerceve havuzunu gorur -- ELF imajini kopyalayan da
/// zaten odur.
///
/// # Safety
/// Sayfalama kurulmus olmalidir; kesmeler kapali cagrilmalidir.
pub unsafe fn create_user_space() -> Option<usize> {
    let kernel = kernel_cr3();
    if kernel == 0 {
        return None;
    }

    let pml4 = frames::alloc()?;
    let pdpt = match frames::alloc() {
        Some(f) => f,
        None => {
            frames::free(pml4);
            return None;
        }
    };
    let pd = match frames::alloc() {
        Some(f) => f,
        None => {
            frames::free(pml4);
            frames::free(pdpt);
            return None;
        }
    };

    // Cekirdek tablolarini oldugu gibi devral.
    let kernel_pdpt = (table_entry(kernel as u64, 0).read() & ADDR_MASK) as usize;
    let kernel_pd = (table_entry(kernel_pdpt as u64, 0).read() & ADDR_MASK) as usize;
    copy_table(kernel, pml4);
    copy_table(kernel_pdpt, pdpt);
    copy_table(kernel_pd, pd);

    // Zinciri surecin kendi tablolarina yonlendir. User biti her
    // seviyede gerekir: erisim haklari AND'lenir.
    table_entry(pml4 as u64, 0).write(pdpt as u64 | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
    table_entry(pdpt as u64, 0).write(pd as u64 | PTE_PRESENT | PTE_WRITABLE | PTE_USER);

    // Kullanici bolgesinin PD girdileri: bos (sayfa yok) sayfa tablolari.
    for pd_index in USER_PD_FIRST..=USER_PD_LAST {
        let pt = match frames::alloc() {
            Some(f) => f,
            None => {
                destroy_user_space(pml4);
                return None;
            }
        };
        // `frames::alloc` cerceveyi sifirlar, yani tablo bos baslar.
        table_entry(pd as u64, pd_index)
            .write(pt as u64 | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
    }

    Some(pml4)
}

/// Bir adres uzayini ve **ona ait** cerceveleri birakir.
///
/// Yalnizca surecin kendi tablolari ve kullanici sayfalari serbest
/// kalir; cekirdek girdileri kopya oldugu icin dokunulmaz.
///
/// # Safety
/// `cr3` bu modulun urettigi bir uzay olmalidir ve **o anda yuklu
/// olmamalidir**.
pub unsafe fn destroy_user_space(cr3: usize) {
    if cr3 == 0 || cr3 == kernel_cr3() {
        return;
    }

    let pdpt = (table_entry(cr3 as u64, 0).read() & ADDR_MASK) as usize;
    if pdpt != 0 {
        let pd = (table_entry(pdpt as u64, 0).read() & ADDR_MASK) as usize;
        if pd != 0 {
            for pd_index in USER_PD_FIRST..=USER_PD_LAST {
                let entry = table_entry(pd as u64, pd_index).read();
                if entry & PTE_PRESENT == 0 || entry & PTE_HUGE != 0 {
                    continue;
                }
                let pt = (entry & ADDR_MASK) as usize;
                // Sayfalar: yalnizca surecin tahsis ettikleri. Pencere
                // tamponlari cekirdege aittir ve havuzun disindadir;
                // `frames::free` havuz disi adresleri yok sayar.
                for i in 0..ENTRIES {
                    let page = table_entry(pt as u64, i).read();
                    if page & PTE_PRESENT != 0 {
                        frames::free((page & ADDR_MASK) as usize);
                    }
                }
                frames::free(pt);
            }
            frames::free(pd);
        }
        frames::free(pdpt);
    }
    frames::free(cr3);
}

/// Adres uzayini yukler (baglam degisimi).
///
/// # Safety
/// `cr3` gecerli bir uzay ya da `kernel_cr3()` olmalidir.
pub unsafe fn switch_to(cr3: usize) {
    if cr3 == 0 {
        write_cr3(kernel_cr3() as u64);
    } else {
        write_cr3(cr3 as u64);
    }
}

pub fn kernel_cr3() -> usize {
    KERNEL_CR3.load(Ordering::Relaxed)
}

/// Surecin kullanici bolgesine cerceve tahsis edip esler.
///
/// # Safety
/// `cr3` bu modulun urettigi bir uzay olmalidir.
pub unsafe fn map_user_range(cr3: usize, start: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    if cr3 == 0 || cr3 == kernel_cr3() {
        // Paylasimli yol (adres uzayi olmayan cagiran).
        protect_user_range(start, len);
        return true;
    }

    let first = start / PAGE_SIZE;
    let last = (start + len - 1) / PAGE_SIZE;

    for page in first..=last {
        let addr = page * PAGE_SIZE;
        let entry = match user_pte(cr3, addr) {
            Some(e) => e,
            None => return false,
        };
        if entry.read() & PTE_PRESENT != 0 {
            continue;
        }
        let frame = match frames::alloc() {
            Some(f) => f,
            None => return false,
        };
        entry.write(frame as u64 | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
    }

    flush_tlb();
    true
}

/// **Var olan** fiziksel sayfalari surecin adres uzayina esler
/// (pencere piksel tamponlari).
///
/// # Safety
/// `phys` cekirdege ait, omru pencereden uzun bir bolge olmalidir.
pub unsafe fn map_user_frames(cr3: usize, vaddr: usize, phys: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    if cr3 == 0 || cr3 == kernel_cr3() {
        protect_user_range(phys, len);
        return true;
    }
    if vaddr % PAGE_SIZE != 0 || phys % PAGE_SIZE != 0 {
        return false;
    }

    let pages = (len + PAGE_SIZE - 1) / PAGE_SIZE;
    for i in 0..pages {
        let entry = match user_pte(cr3, vaddr + i * PAGE_SIZE) {
            Some(e) => e,
            None => return false,
        };
        entry.write((phys + i * PAGE_SIZE) as u64 | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
    }

    flush_tlb();
    true
}

/// Kullanici bolgesindeki bir adresin PTE'sine isaretci; bolge disiysa
/// `None`.
unsafe fn user_pte(cr3: usize, addr: usize) -> Option<*mut u64> {
    let pd_index = addr / LARGE_PAGE_SIZE;
    if !(USER_PD_FIRST..=USER_PD_LAST).contains(&pd_index) {
        return None;
    }
    let pdpt = (table_entry(cr3 as u64, 0).read() & ADDR_MASK) as usize;
    let pd = (table_entry(pdpt as u64, 0).read() & ADDR_MASK) as usize;
    let entry = table_entry(pd as u64, pd_index).read();
    if entry & PTE_PRESENT == 0 || entry & PTE_HUGE != 0 {
        return None;
    }
    let pt = entry & ADDR_MASK;
    Some(table_entry(pt, (addr / PAGE_SIZE) % ENTRIES))
}

/// Bir surecin kullandigi kullanici sayfasi sayisi (kabuk raporu).
pub fn user_pages(cr3: usize) -> usize {
    if cr3 == 0 || cr3 == kernel_cr3() {
        return 0;
    }
    unsafe {
        let pdpt = (table_entry(cr3 as u64, 0).read() & ADDR_MASK) as usize;
        let pd = (table_entry(pdpt as u64, 0).read() & ADDR_MASK) as usize;
        let mut count = 0;
        for pd_index in USER_PD_FIRST..=USER_PD_LAST {
            let entry = table_entry(pd as u64, pd_index).read();
            if entry & PTE_PRESENT == 0 || entry & PTE_HUGE != 0 {
                continue;
            }
            let pt = entry & ADDR_MASK;
            count += (0..ENTRIES)
                .filter(|&i| table_entry(pt, i).read() & PTE_PRESENT != 0)
                .count();
        }
        count
    }
}

/// Bir tabloyu (512 girdi) oldugu gibi kopyalar.
unsafe fn copy_table(src: usize, dst: usize) {
    core::ptr::copy_nonoverlapping(src as *const u64, dst as *mut u64, ENTRIES);
}

/// Surec basina gercekten eslenen pencere (i386 ile ayni).
///
/// Bolgenin tamami degil: her sayfa bir cerceve demek ve havuz sinirli.
/// 512 KiB, imaj + yigin icin fazlasiyla yeter; pencere tamponlari bunun
/// disinda, kendi yuvalarina eslenir.
pub const USER_MAP_SIZE: usize = 512 * 1024;
