//! Duz (flat) GDT: Ring 0 ve Ring 3 segmentlerini donanimsal olarak
//! ayirir (doc S.4). Faz 1'de sadece Ring 0 segmentleri kullanilir; Ring 3
//! girdileri Faz 3'teki kullanici-modu gecisi icin simdiden hazirlanir.

use core::arch::asm;
use core::mem::size_of;

pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
#[allow(dead_code)] // load() icinde 0x10 olarak sabit gomulu, bkz. asagidaki not
pub const KERNEL_DATA_SELECTOR: u16 = 0x10;
#[allow(dead_code)]
pub const USER_CODE_SELECTOR: u16 = 0x1B; // index 3, RPL 3
#[allow(dead_code)]
pub const USER_DATA_SELECTOR: u16 = 0x23; // index 4, RPL 3

#[derive(Clone, Copy)]
#[repr(C, packed)]
struct GdtEntry {
    limit_low: u16,
    base_low: u16,
    base_middle: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
}

impl GdtEntry {
    const fn new(base: u32, limit: u32, access: u8, flags: u8) -> Self {
        GdtEntry {
            limit_low: (limit & 0xFFFF) as u16,
            base_low: (base & 0xFFFF) as u16,
            base_middle: ((base >> 16) & 0xFF) as u8,
            access,
            granularity: (flags << 4) | (((limit >> 16) & 0x0F) as u8),
            base_high: ((base >> 24) & 0xFF) as u8,
        }
    }

    const fn null() -> Self {
        GdtEntry::new(0, 0, 0, 0)
    }
}

#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u32,
}

const FLAGS_32BIT_4K: u8 = 0b1100; // G=1 (4 KiB granularity), D/B=1 (32-bit)

/// TSS secicisi (GDT index 5).
pub const TSS_SELECTOR: u16 = 0x28;

static mut GDT: [GdtEntry; 6] = [
    GdtEntry::null(),                                   // 0x00 null
    GdtEntry::new(0, 0xFFFFF, 0x9A, FLAGS_32BIT_4K),     // 0x08 kernel code (ring0)
    GdtEntry::new(0, 0xFFFFF, 0x92, FLAGS_32BIT_4K),     // 0x10 kernel data (ring0)
    GdtEntry::new(0, 0xFFFFF, 0xFA, FLAGS_32BIT_4K),     // 0x18 user code (ring3)
    GdtEntry::new(0, 0xFFFFF, 0xF2, FLAGS_32BIT_4K),     // 0x20 user data (ring3)
    GdtEntry::null(),                                   // 0x28 TSS (init()'te doldurulur)
];

/// i386 Gorev Durum Segmenti. Faz 3'te tek amaci `esp0`/`ss0`'dir:
/// Ring 3'ten bir kesme (int 0x80) geldiginde CPU otomatik olarak buradaki
/// cekirdek yiginina gecer. Donanimsal gorev degistirme kullanilmaz.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Tss {
    prev_tss: u32,
    esp0: u32,
    ss0: u32,
    esp1: u32,
    ss1: u32,
    esp2: u32,
    ss2: u32,
    cr3: u32,
    eip: u32,
    eflags: u32,
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esp: u32,
    ebp: u32,
    esi: u32,
    edi: u32,
    es: u32,
    cs: u32,
    ss: u32,
    ds: u32,
    fs: u32,
    gs: u32,
    ldt: u32,
    trap: u16,
    iomap_base: u16,
}

impl Tss {
    const fn empty() -> Self {
        Tss {
            prev_tss: 0,
            esp0: 0,
            ss0: 0,
            esp1: 0,
            ss1: 0,
            esp2: 0,
            ss2: 0,
            cr3: 0,
            eip: 0,
            eflags: 0,
            eax: 0,
            ecx: 0,
            edx: 0,
            ebx: 0,
            esp: 0,
            ebp: 0,
            esi: 0,
            edi: 0,
            es: 0,
            cs: 0,
            ss: 0,
            ds: 0,
            fs: 0,
            gs: 0,
            ldt: 0,
            trap: 0,
            // I/O izin haritasi yok: limit'in otesini gosterir.
            iomap_base: size_of::<Tss>() as u16,
        }
    }
}

static mut TSS: Tss = Tss::empty();

pub fn init() {
    unsafe {
        let ptr = GdtPointer {
            limit: (size_of::<[GdtEntry; 6]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u32,
        };
        load(&ptr);
    }
}

/// TSS'i GDT'ye yazar, `ltr` ile yukler ve Ring 3 -> Ring 0 gecislerinde
/// kullanilacak cekirdek yiginini ayarlar (doc S.7 Faz 3: "TSS + ESP0").
///
/// # Safety
/// `kernel_stack_top` gecerli, cekirdege ait bir yigin tepesi olmalidir.
/// `init()` cagrildiktan sonra ve yalnizca bir kez cagrilmalidir.
pub unsafe fn install_tss(kernel_stack_top: usize) {
    let tss_ptr = core::ptr::addr_of_mut!(TSS);
    (*tss_ptr).ss0 = KERNEL_DATA_SELECTOR as u32;
    (*tss_ptr).esp0 = kernel_stack_top as u32;

    let base = tss_ptr as u32;
    let limit = (size_of::<Tss>() - 1) as u32;

    // access 0x89 = P=1, DPL=0, type=1001b (kullanilabilir 32-bit TSS).
    let gdt = core::ptr::addr_of_mut!(GDT) as *mut GdtEntry;
    gdt.add(5).write(GdtEntry::new(base, limit, 0x89, 0b0000));

    // GDT yeniden yuklenmeli ki yeni girdi gorulsun, sonra TSS yuklenir.
    let ptr = GdtPointer {
        limit: (size_of::<[GdtEntry; 6]>() - 1) as u16,
        base: core::ptr::addr_of!(GDT) as u32,
    };
    asm!("lgdt [{0}]", in(reg) &ptr, options(nostack));
    asm!("ltr {0:x}", in(reg) TSS_SELECTOR, options(nostack));
}

/// Her Ring 3'e girmeden once cagrilir: bir sonraki syscall/kesmenin
/// hangi cekirdek yigininda karsilanacagini belirler.
///
/// # Safety
/// `kernel_stack_top` gecerli bir cekirdek yigini tepesi olmalidir.
pub unsafe fn set_kernel_stack(kernel_stack_top: usize) {
    core::ptr::addr_of_mut!((*core::ptr::addr_of_mut!(TSS)).esp0).write(kernel_stack_top as u32);
}

unsafe fn load(ptr: *const GdtPointer) {
    // Secici degerleri (0x10 / 0x08) yukaridaki KERNEL_*_SELECTOR sabitleriyle
    // ayni tutulmalidir; register-genisligi karmasasindan kacinmak icin
    // sabit olarak gomulur.
    asm!(
        "lgdt [{gdt}]",
        "mov ax, 0x10",
        "mov ds, ax",
        "mov es, ax",
        "mov fs, ax",
        "mov gs, ax",
        "mov ss, ax",
        "push 0x08",
        "lea {tmp}, [2f]",
        "push {tmp}",
        "retf",
        "2:",
        gdt = in(reg) ptr,
        tmp = out(reg) _,
        options(nostack),
    );
}
