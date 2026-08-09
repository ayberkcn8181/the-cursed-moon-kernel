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

static mut GDT: [GdtEntry; 5] = [
    GdtEntry::null(),                                   // 0x00 null
    GdtEntry::new(0, 0xFFFFF, 0x9A, FLAGS_32BIT_4K),     // 0x08 kernel code (ring0)
    GdtEntry::new(0, 0xFFFFF, 0x92, FLAGS_32BIT_4K),     // 0x10 kernel data (ring0)
    GdtEntry::new(0, 0xFFFFF, 0xFA, FLAGS_32BIT_4K),     // 0x18 user code (ring3)
    GdtEntry::new(0, 0xFFFFF, 0xF2, FLAGS_32BIT_4K),     // 0x20 user data (ring3)
];

pub fn init() {
    unsafe {
        let ptr = GdtPointer {
            limit: (size_of::<[GdtEntry; 5]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u32,
        };
        load(&ptr);
    }
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
