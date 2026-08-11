//! i386 Sanal Bellek Yonetimi -- identity map + **surec basina adres uzayi**.
//!
//! ## Yerlesim
//!
//! Ilk 32 MiB 4 KiB'lik sayfalarla birebir eslenir; buraya doc S.5'teki tum
//! kritik bolgeler girer:
//!   0x000B8000  VGA metin buffer
//!   0x00100000  cekirdek imaji (linker: . = 1M)
//!   0x00800000  kmalloc heap
//!   0x00C00000  kullanici bolgesinin SANAL adresi
//!   0x01000000  fiziksel cerceve havuzu (bkz. `core::frames`)
//!
//! ## Surec basina adres uzayi
//!
//! Kullanici bolgesi (0x00C00000, 2 MiB) tam olarak **tek bir PDE'nin**
//! (indeks 3) icine duser. Bu, izolasyonu ucuz kilar: her surec yalnizca
//! kendi PDE 3 sayfa tablosunu alir, geri kalan her sey (cekirdek, heap,
//! MMIO) tum adres uzaylarinda **paylasilir**.
//!
//! Onceki model "slot"lardi: her uygulama derleme aninda farkli bir taban
//! adrese linkleniyordu, cunku hepsi ayni adres uzayini paylasiyordu. Bunun
//! iki bedeli vardi -- ayni ikilinin iki kopyasi birbirinin kodunu eziyordu
//! ve her uygulama digerinin bellegini okuyabiliyordu. Ikisi de bitti.
//!
//! ## PDE'lerde User biti neden acilista aciliyor
//!
//! Erisim denetimi PDE **ve** PTE'nin birlikte User olmasini gerektirir.
//! Identity PDE'lerine User bitini acilista koymak izolasyonu bozmaz
//! (cekirdek PTE'lerinde User kapali kalir) ama onemli bir sorunu cozer:
//! surec sayfa dizinleri PDE'lerin **kopyasini** tasir, oysa sayfa
//! tablolari paylasilir. PDE bayraklari sonradan degistirilseydi, o
//! degisiklik mevcut sureclere ulasmazdi.

use crate::arch::cpu::{read_cr0, read_cr3, write_cr0, write_cr3};
use crate::level0a::core::frames;

const PAGE_SIZE: usize = 4096;
const ENTRIES: usize = 1024;
/// Bir PT = 1024 sayfa x 4 KiB = 4 MiB; sekiz tablo ile 32 MiB.
/// Cerceve havuzu (16-32 MiB) da bu araligin icindedir; cekirdek
/// cercevelere fiziksel adreslerinden dogrudan yazabilsin diye.
const IDENTITY_TABLES: usize = 8;
const IDENTITY_MAPPED_BYTES: usize = IDENTITY_TABLES * ENTRIES * PAGE_SIZE;

const PTE_PRESENT: u32 = 1 << 0;
const PTE_WRITABLE: u32 = 1 << 1;
const PTE_USER: u32 = 1 << 2;
const CR0_PG: u32 = 1 << 31;

/// Kullanici (Ring 3) bellek bolgesinin baslangici -- doc'taki
/// `TCMK_USER_MEM_START` ile ayni deger.
pub const USER_MEM_START: usize = 0x00C0_0000; // 12 MiB
pub const USER_MEM_SIZE: usize = 0x0020_0000; // 2 MiB

/// Kullanici bolgesinin dustugu sayfa dizini girdisi.
const USER_PDE: usize = USER_MEM_START >> 22;

/// Surec basina gercekten eslenen alan. Tum bolgeyi (2 MiB = 512 cerceve)
/// pesin eslemek havuzu bosuna tuketirdi; talep uzerine sayfalama (demand
/// paging) gelene kadar sabit ve comert bir pencere yeterli.
pub const USER_MAP_SIZE: usize = 512 * 1024;

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
        // PDE'de User acik, PTE'lerde kapali: cekirdek sayfalari Ring 3'e
        // kapali kalir ama PTE bayraklari sonradan degistirilebilir hale
        // gelir (bkz. modul basligi).
        pd.add(t).write(pt as u32 | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
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
///
/// **Mevcut** CR3 geri yazilir, cekirdek dizini degil: bir surec adres
/// uzayindayken cekirdek dizinine gecmek kullanici eslemesini bir anda
/// yok ederdi.
unsafe fn flush_tlb() {
    write_cr3(read_cr3());
}

/// Bir adresin Ring 3'e acik olup olmadigini bildirir.
///
/// Kullanici bolgesi surec basina degistigi icin sorgu **mevcut CR3**
/// uzerinden yapilir; identity bolgesi ise tum adres uzaylarinda ortaktir.
/// POSIX cevirmeni kullanici isaretcilerini bununla dogrular, dolayisiyla
/// yanlis adres uzayina bakmak gercek bir guvenlik acigi olurdu.
pub fn is_user_accessible(addr: usize) -> bool {
    unsafe {
        if addr >> 22 == USER_PDE {
            return user_pte(read_cr3() as usize, addr).map_or(false, |e| {
                let value = e.read();
                value & PTE_PRESENT != 0 && value & PTE_USER != 0
            });
        }
        match identity_pte(addr / PAGE_SIZE) {
            Some(entry) => entry.read() & PTE_USER != 0,
            None => false,
        }
    }
}

// --- Surec basina adres uzayi ---------------------------------------------

/// Cekirdegin (ve kullanici uzayi olmayan gorevlerin) CR3 degeri.
pub fn kernel_cr3() -> usize {
    core::ptr::addr_of!(PAGE_DIRECTORY) as usize
}

/// Verilen adres uzayinda kullanici bolgesine ait PTE isaretcisi.
unsafe fn user_pte(cr3: usize, addr: usize) -> Option<*mut u32> {
    if addr >> 22 != USER_PDE {
        return None;
    }
    let pd = cr3 as *const u32;
    let pde = pd.add(USER_PDE).read();
    if pde & PTE_PRESENT == 0 {
        return None;
    }
    let pt = (pde & !0xFFF) as *mut u32;
    Some(pt.add((addr >> 12) & (ENTRIES - 1)))
}

/// Yeni bir kullanici adres uzayi olusturur; CR3 degerini doner.
///
/// Cekirdek PDE'leri **kopyalanir** (sayfa tablolarinin kendisi paylasilir),
/// yalnizca kullanici PDE'si sifirdan olusturulur. Boylece cekirdek her
/// adres uzayinda ayni yerdedir -- kesme ve syscall yollari surec
/// degistiginde kirilmaz.
///
/// # Safety
/// Sayfalama acik olmalidir.
pub unsafe fn create_user_space() -> Option<usize> {
    let pd_phys = frames::alloc()?;
    let pd = pd_phys as *mut u32;
    let kernel_pd = core::ptr::addr_of!(PAGE_DIRECTORY) as *const u32;
    for i in 0..ENTRIES {
        pd.add(i).write(kernel_pd.add(i).read());
    }

    let pt_phys = match frames::alloc() {
        Some(p) => p,
        None => {
            frames::free(pd_phys);
            return None;
        }
    };
    // frames::alloc sifirlanmis cerceve verir; PTE'ler bos baslar.
    pd.add(USER_PDE)
        .write(pt_phys as u32 | PTE_PRESENT | PTE_WRITABLE | PTE_USER);

    Some(pd_phys)
}

/// Adres uzayini ve tum kullanici cercevelerini havuza geri verir.
///
/// # Safety
/// `cr3` bu modulun urettigi bir adres uzayi olmali ve **su an yuklu
/// olmamalidir**.
pub unsafe fn destroy_user_space(cr3: usize) {
    if cr3 == 0 || cr3 == kernel_cr3() {
        return;
    }
    let pd = cr3 as *mut u32;
    let pde = pd.add(USER_PDE).read();
    if pde & PTE_PRESENT != 0 {
        let pt = (pde & !0xFFF) as *mut u32;
        for i in 0..ENTRIES {
            let entry = pt.add(i).read();
            if entry & PTE_PRESENT != 0 {
                frames::free((entry & !0xFFF) as usize);
            }
        }
        frames::free((pde & !0xFFF) as usize);
    }
    frames::free(cr3);
}

/// Verilen adres uzayini yukler.
///
/// # Safety
/// `cr3` gecerli bir sayfa dizini olmalidir.
pub unsafe fn switch_to(cr3: usize) {
    if cr3 != 0 && read_cr3() as usize != cr3 {
        write_cr3(cr3 as u32);
    }
}

/// Kullanici bolgesinde `[start, start+len)` icin cerceve tahsis edip esler.
///
/// Zaten eslenmis sayfalar atlanir; bu sayede cagri yinelenebilir.
///
/// # Safety
/// `cr3` bu modulun urettigi bir adres uzayi olmalidir.
pub unsafe fn map_user_range(cr3: usize, start: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let first = start / PAGE_SIZE;
    let last = (start + len - 1) / PAGE_SIZE;

    for page in first..=last {
        let addr = page * PAGE_SIZE;
        let entry = match user_pte(cr3, addr) {
            Some(e) => e,
            None => return false, // kullanici bolgesi disinda
        };
        if entry.read() & PTE_PRESENT != 0 {
            continue;
        }
        let frame = match frames::alloc() {
            Some(f) => f,
            None => return false,
        };
        entry.write(frame as u32 | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
    }

    flush_tlb();
    true
}

/// **Var olan** fiziksel sayfalari surecin adres uzayina esler.
///
/// `map_user_range`'den farki: cerceve tahsis etmez, verilen fiziksel
/// bolgeyi (ornegin bir pencere piksel tamponunu) kullanici sanal
/// adresine baglar. Cekirdek ayni bellegi identity adresinden gormeye
/// devam eder -- kompozitor tamponu oradan okur.
///
/// Bu, paylasimli tamponlarin **tek surece** acilmasini saglar: eskiden
/// tampon identity haritasinda Ring 3'e aciliyordu, yani her uygulama her
/// pencerenin piksellerini okuyabiliyordu.
///
/// # Safety
/// `phys` gercekten cekirdege ait, omru pencereden uzun bir bolge
/// olmalidir; `cr3` bu modulun urettigi bir adres uzayi olmalidir.
pub unsafe fn map_user_frames(cr3: usize, vaddr: usize, phys: usize, len: usize) -> bool {
    if len == 0 {
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
        entry.write((phys + i * PAGE_SIZE) as u32 | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
    }

    flush_tlb();
    true
}

/// Bir adres uzayinin kullanici bolgesini **icerigiyle birlikte**
/// kopyalar (`fork`).
///
/// Kopyalama sanal adresler uzerinden yapilamaz: kaynak ve hedef ayni
/// sanal adreste (0x00C00000) ama farkli CR3'lerde durur, yani ikisi ayni
/// anda gorunmez. Bunun yerine **fiziksel** adresler kullanilir --
/// cerceve havuzu cekirdegin identity haritasinin icinde oldugu icin
/// (bkz. `frames`) her iki cerceveye de dogrudan yazilabilir. CR3
/// degistirmek ya da gecici pencere esleme gerekmez.
///
/// Kopyalanan sey ebeveynin yalnizca **var olan** sayfalaridir; hic
/// dokunulmamis sayfalar cocukta da yoktur.
///
/// Not: bu **tam kopyadir**, copy-on-write degil. COW icin sayfalari salt
/// okunur isaretleyip page fault'ta ayirmak gerekir; hata isleyicisi
/// bunun icin ayri bir asamadir. Tam kopya, `fork` semantigi acisindan
/// dogru sonucu verir -- yalnizca daha pahalidir.
///
/// # Safety
/// `src_cr3` bu modulun urettigi bir adres uzayi olmalidir.
pub unsafe fn clone_user_space(src_cr3: usize) -> Option<usize> {
    if src_cr3 == 0 || src_cr3 == kernel_cr3() {
        return None;
    }

    let dst_cr3 = create_user_space()?;

    let src_pd = src_cr3 as *const u32;
    let src_pde = src_pd.add(USER_PDE).read();
    if src_pde & PTE_PRESENT == 0 {
        // Ebeveynin hic kullanici sayfasi yok; bos uzay dogru sonuctur.
        return Some(dst_cr3);
    }
    let src_pt = (src_pde & !0xFFF) as *const u32;

    for i in 0..ENTRIES {
        let entry = src_pt.add(i).read();
        if entry & PTE_PRESENT == 0 {
            continue;
        }

        let vaddr = (USER_PDE << 22) | (i << 12);
        let dst_entry = match user_pte(dst_cr3, vaddr) {
            Some(e) => e,
            None => {
                destroy_user_space(dst_cr3);
                return None;
            }
        };

        let frame = match frames::alloc() {
            Some(f) => f,
            None => {
                destroy_user_space(dst_cr3);
                return None;
            }
        };

        core::ptr::copy_nonoverlapping(
            (entry & !0xFFF) as *const u8,
            frame as *mut u8,
            PAGE_SIZE,
        );
        dst_entry.write(frame as u32 | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
    }

    Some(dst_cr3)
}

/// Bir surecin kullandigi kullanici sayfasi sayisi (kabuk raporu icin).
pub fn user_pages(cr3: usize) -> usize {
    if cr3 == 0 || cr3 == kernel_cr3() {
        return 0;
    }
    unsafe {
        let pd = cr3 as *const u32;
        let pde = pd.add(USER_PDE).read();
        if pde & PTE_PRESENT == 0 {
            return 0;
        }
        let pt = (pde & !0xFFF) as *const u32;
        (0..ENTRIES)
            .filter(|i| pt.add(*i).read() & PTE_PRESENT != 0)
            .count()
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
