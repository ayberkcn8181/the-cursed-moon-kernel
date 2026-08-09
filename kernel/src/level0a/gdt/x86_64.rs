//! x86_64 GDT + 64-bit TSS.
//!
//! Long Mode'da segmentasyon buyuk olcude devre disidir (taban/limit yok
//! sayilir), ama **ayricalik seviyesi ve L biti hala GDT'den okunur**;
//! ayrica `syscall`/`sysret` ve `iretq` belirli bir secici duzeni bekler.
//!
//! Secici duzeni `syscall`/`sysret` sozlesmesine gore secilmistir
//! (STAR MSR: SYSCALL -> CS=base, SS=base+8; SYSRET -> CS=base+16, SS=base+8):
//!
//!   0x00 null
//!   0x08 cekirdek kod   (Ring 0, L=1)
//!   0x10 cekirdek veri  (Ring 0)
//!   0x18 kullanici veri (Ring 3)   <- sysret icin once veri gelmeli
//!   0x20 kullanici kod  (Ring 3, L=1)
//!   0x28 TSS (16 bayt, iki GDT girdisi kaplar)

use core::arch::asm;
use core::mem::size_of;

pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
/// Asagidaki asm bloklarinda ve usermode.rs'te sabit olarak gomulu;
/// burada tek dogru kaynak olarak tutulur.
#[allow(dead_code)]
pub const KERNEL_DATA_SELECTOR: u16 = 0x10;
#[allow(dead_code)]
pub const USER_DATA_SELECTOR: u16 = 0x18 | 3; // 0x1B
#[allow(dead_code)]
pub const USER_CODE_SELECTOR: u16 = 0x20 | 3; // 0x23
pub const TSS_SELECTOR: u16 = 0x28;

#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u64,
}

/// 64-bit TSS. Long Mode'da donanimsal gorev degistirme yoktur; yapinin
/// tek islevi `rsp0` (Ring 3 -> Ring 0 gecisinde kullanilacak yigin) ve
/// IST girdileridir.
#[repr(C, packed)]
struct Tss {
    reserved0: u32,
    rsp0: u64,
    rsp1: u64,
    rsp2: u64,
    reserved1: u64,
    ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    iomap_base: u16,
}

impl Tss {
    const fn empty() -> Self {
        Tss {
            reserved0: 0,
            rsp0: 0,
            rsp1: 0,
            rsp2: 0,
            reserved1: 0,
            ist: [0; 7],
            reserved2: 0,
            reserved3: 0,
            iomap_base: size_of::<Tss>() as u16,
        }
    }
}

static mut TSS: Tss = Tss::empty();

/// 7 girdi: null + 4 segment + TSS (2 girdi kaplar).
static mut GDT: [u64; 7] = [
    0,
    // Cekirdek kod: P=1 DPL=0 S=1 Type=Code/Read, L=1 (64-bit)
    0x00AF_9A00_0000_FFFF,
    // Cekirdek veri: P=1 DPL=0 S=1 Type=Data/Write
    0x00CF_9200_0000_FFFF,
    // Kullanici veri: DPL=3
    0x00CF_F200_0000_FFFF,
    // Kullanici kod: DPL=3, L=1
    0x00AF_FA00_0000_FFFF,
    // TSS (init sirasinda doldurulur -- 16 bayt, iki slot)
    0,
    0,
];

/// GDT'yi yukler ve segment registerlarini tazeler.
///
/// Boot stub'i zaten gecici bir GDT64 yuklemisti; burada kalici olan
/// (kullanici segmentleri ve TSS iceren) tabloya geciyoruz.
pub fn init() {
    unsafe {
        let ptr = GdtPointer {
            limit: (size_of::<[u64; 7]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u64,
        };
        asm!("lgdt [{0}]", in(reg) &ptr, options(nostack));

        // Veri segmentleri dogrudan yuklenebilir.
        asm!(
            "mov ax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov ss, ax",
            "mov fs, ax",
            "mov gs, ax",
            out("ax") _,
            options(nostack),
        );

        // CS dogrudan yazilamaz; uzak donus (retfq) ile tazelenir.
        asm!(
            "push 0x08",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            tmp = out(reg) _,
            options(nostack),
        );
    }
}

/// TSS'i GDT'ye yazar, `ltr` ile yukler ve Ring 3 -> Ring 0 gecislerinde
/// kullanilacak cekirdek yiginini ayarlar.
///
/// # Safety
/// `init()` cagrildiktan sonra ve yalnizca bir kez cagrilmalidir.
pub unsafe fn install_tss(kernel_stack_top: usize) {
    let tss_ptr = core::ptr::addr_of_mut!(TSS);
    (*tss_ptr).rsp0 = kernel_stack_top as u64;

    let base = tss_ptr as u64;
    let limit = (size_of::<Tss>() - 1) as u64;

    // 64-bit sistem descriptor'u 16 bayttir: dusuk yari klasik duzen,
    // yuksek yari yalnizca tabanin ust 32 bitini tasir.
    let low = (limit & 0xFFFF)
        | ((base & 0xFFFF) << 16)
        | (((base >> 16) & 0xFF) << 32)
        | (0x89u64 << 40)                 // P=1 DPL=0 Type=9 (kullanilabilir 64-bit TSS)
        | (((limit >> 16) & 0xF) << 48)
        | (((base >> 24) & 0xFF) << 56);
    let high = (base >> 32) & 0xFFFF_FFFF;

    let gdt = core::ptr::addr_of_mut!(GDT) as *mut u64;
    gdt.add(5).write(low);
    gdt.add(6).write(high);

    let ptr = GdtPointer {
        limit: (size_of::<[u64; 7]>() - 1) as u16,
        base: core::ptr::addr_of!(GDT) as u64,
    };
    asm!("lgdt [{0}]", in(reg) &ptr, options(nostack));
    asm!("ltr {0:x}", in(reg) TSS_SELECTOR, options(nostack));
}

/// Her Ring 3'e girmeden once cagrilir.
///
/// # Safety
/// `kernel_stack_top` gecerli bir cekirdek yigini tepesi olmalidir.
pub unsafe fn set_kernel_stack(kernel_stack_top: usize) {
    core::ptr::addr_of_mut!((*core::ptr::addr_of_mut!(TSS)).rsp0).write(kernel_stack_top as u64);
}
