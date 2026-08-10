//! x86_64 port G/C ve CPU ilkelleri (i386 tarafiyla ayni arayuz).

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

/// 16-bit port okuma/yazma -- ATA PIO veri portu (0x1F0) 16 bitliktir.
#[inline(always)]
pub unsafe fn outw(port: u16, val: u16) {
    asm!("out dx, ax", in("dx") port, in("ax") val, options(nomem, nostack, preserves_flags));
}

#[inline(always)]
pub unsafe fn inw(port: u16) -> u16 {
    let val: u16;
    asm!("in ax, dx", out("ax") val, in("dx") port, options(nomem, nostack, preserves_flags));
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
/// yukler (i386 tarafiyla ayni sozlesme).
pub fn without_interrupts<F: FnOnce() -> R, R>(f: F) -> R {
    let flags: u64;
    unsafe {
        asm!("pushfq", "pop {0}", out(reg) flags, options(nomem));
    }
    let were_enabled = flags & (1 << 9) != 0;

    disable_interrupts();
    let ret = f();
    if were_enabled {
        enable_interrupts();
    }
    ret
}

/// Cekirdek icinden sistem cagrisi yapar (RAX=numara, RDI/RSI/RDX=arg1..3).
///
/// **`syscall` komutu degil, `int 0x80` kullanilir** ve bu bilincli bir
/// tercihtir: `syscall`'in donusu `sysretq`'tir ve `sysretq` her zaman
/// **Ring 3'e** doner. Ring 0'daki bir cagiran (ornegin Faz 2 worker
/// gorevi) `syscall` kullanirsa kendini Ring 3'te bulur. `int 0x80` ise
/// `iretq` ile cagiranin ayricalik seviyesine geri doner.
///
/// Ring 3 kullanici programlari elbette gercek `syscall` komutunu kullanir
/// (bkz. `level0a::syscall_msr`); iki yol da ayni Level-0b2 kapisina cikar.
///
/// # Safety
/// Isaretci argumanlari gecerli bellek gostermelidir.
pub unsafe fn syscall3(number: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    let ret: u64;
    asm!(
        "int 0x80",
        inout("rax") number => ret,
        in("rdi") arg1,
        in("rsi") arg2,
        in("rdx") arg3,
        options(nostack),
    );
    ret
}

// --- Kontrol registerlari ve MSR'ler ---

/// # Safety
/// `pd_phys` gecerli, 4 KiB hizali bir PML4 tablosunu gostermelidir.
pub unsafe fn write_cr3(pml4_phys: u64) {
    asm!("mov cr3, {0}", in(reg) pml4_phys, options(nostack, preserves_flags));
}

pub fn read_cr3() -> u64 {
    let value: u64;
    unsafe { asm!("mov {0}, cr3", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

pub fn read_cr0() -> u64 {
    let value: u64;
    unsafe { asm!("mov {0}, cr0", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// # Safety
/// CR0'i degistirmek sayfalama/koruma davranisini dogrudan etkiler.
///
/// Long Mode'da sayfalama zaten acik geldigi icin su an kullanilmiyor;
/// mimari ilkel kumesini tam tutmak icin korunuyor.
#[allow(dead_code)]
pub unsafe fn write_cr0(value: u64) {
    asm!("mov cr0, {0}", in(reg) value, options(nostack, preserves_flags));
}

/// # Safety
/// Gecerli bir MSR numarasi verilmelidir; aksi halde #GP olusur.
pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (high, low): (u32, u32);
    asm!("rdmsr", in("ecx") msr, out("edx") high, out("eax") low,
         options(nomem, nostack, preserves_flags));
    ((high as u64) << 32) | (low as u64)
}

/// # Safety
/// Yanlis MSR/deger yazmak CPU'yu tanimsiz duruma sokabilir.
pub unsafe fn wrmsr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    asm!("wrmsr", in("ecx") msr, in("edx") high, in("eax") low,
         options(nostack, preserves_flags));
}

pub const MSR_EFER: u32 = 0xC000_0080;
pub const MSR_STAR: u32 = 0xC000_0081;
pub const MSR_LSTAR: u32 = 0xC000_0082;
pub const MSR_SFMASK: u32 = 0xC000_0084;
