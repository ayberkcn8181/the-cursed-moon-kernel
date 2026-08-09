//! i386 port G/C ve CPU ilkelleri (dokumandaki `port.h` esdegeri).

use core::arch::asm;

pub mod regs;

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

/// Faz 1 dogrulamasi: kullanici-modu uygulamasi henuz olmadigi icin
/// int 0x80 hattinin Level-0b2 dispatcher'ina kadar ulastigini kanitlamak
/// icin cekirdegin kendisi tetikler.
pub unsafe fn int80_selftest() {
    asm!("int 0x80", options(nomem, nostack));
}
