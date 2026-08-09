//! i386 port G/C ve CPU ilkelleri (dokumandaki `port.h` esdegeri).

use core::arch::asm;

pub mod context;
pub mod regs;
pub mod usermode;

#[inline(always)]
pub unsafe fn outb(port: u16, val: u8) {
    asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

#[inline(always)]
pub fn enable_interrupts() {
    unsafe { asm!("sti", options(nomem, nostack)) };
}

#[inline(always)]
pub fn disable_interrupts() {
    unsafe { asm!("cli", options(nomem, nostack)) };
}

#[inline(always)]
pub fn halt() {
    unsafe { asm!("hlt", options(nomem, nostack)) };
}

/// Kesmeleri devre disi birakip `f`'yi calistirir, onceki IF durumunu geri
/// yukler. VGA yazici gibi tek-cekirdekli paylasimli durumu IRQ handler'larina
/// karsi korumak icin kullanilir (harici spinlock crate'i yok).
pub fn without_interrupts<F: FnOnce() -> R, R>(f: F) -> R {
    let flags: u32;
    unsafe {
        asm!("pushfd", "pop {0}", out(reg) flags, options(nomem));
    }
    let were_enabled = flags & (1 << 9) != 0;

    disable_interrupts();
    let ret = f();
    if were_enabled {
        enable_interrupts();
    }
    ret
}

/// i386 Linux ABI ile `int 0x80` sistem cagrisi tetikler
/// (EAX=numara, EBX/ECX/EDX=arg1..3, donus EAX -- doc S.6).
///
/// Faz 2'de henuz Ring 3 kullanici programi yok; cagriyi cekirdegin kendisi
/// yapar, ancak gectigi yol gercek olanla ayni: IDT vektor 128 -> Level-0b2
/// dispatcher -> Level-0b1 POSIX subsystem -> Level-0a.
///
/// # Safety
/// Isaretcı argumanlari gecerli bellek gostermelidir.
pub unsafe fn syscall3(number: u32, arg1: u32, arg2: u32, arg3: u32) -> u32 {
    let ret: u32;
    asm!(
        "int 0x80",
        inout("eax") number => ret,
        in("ebx") arg1,
        in("ecx") arg2,
        in("edx") arg3,
        options(nostack),
    );
    ret
}

/// CR3'e sayfa dizini fiziksel adresini yukler.
///
/// # Safety
/// `pd_phys` gecerli, 4 KiB hizali bir sayfa dizinini gostermelidir.
pub unsafe fn write_cr3(pd_phys: u32) {
    asm!("mov cr3, {0}", in(reg) pd_phys, options(nostack, preserves_flags));
}

pub fn read_cr0() -> u32 {
    let value: u32;
    unsafe { asm!("mov {0}, cr0", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// # Safety
/// CR0.PG'yi acmak, cagirmadan once gecerli bir sayfa tablosunun CR3'e
/// yuklenmis ve calisan kodun identity-map edilmis olmasini gerektirir.
pub unsafe fn write_cr0(value: u32) {
    asm!("mov cr0, {0}", in(reg) value, options(nostack, preserves_flags));
}
