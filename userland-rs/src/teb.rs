//! Is Parcacigi Ortam Blogu (TEB) -- Windows'un surece bakan yuzu.
//!
//! Bir Windows programi cok sey icin cekirdege **hic sormaz**: kendi
//! kimligini, yigin sinirlarini, son hata kodunu bir bellek yapisindan
//! okur. O yapinin adresi bir segment tabaninda durur ve register secimi
//! mimariye gore degisir:
//!
//! ```text
//!   i386     fs:[0x18] -> TEB'in kendi adresi (NtTib.Self)
//!   x86_64   gs:[0x30] -> ayni alan, 64-bit yerlesimde
//! ```
//!
//! Bu, `GetLastError`in gercek Windows'ta neden bir sistem cagrisi
//! **olmadigini** aciklar: tek satirdir --
//! `return NtCurrentTeb()->LastErrorValue;`
//!
//! POSIX tarafinda karsiligi yok. Orada TLS blogunun **icerigini** program
//! belirler; Windows'ta yerlesim cekirdegin sozlesmesidir ve ofsetler
//! derlenmis kodun icine gomuludur.

/// `NtTib.Self` alaninin ofseti.
#[cfg(target_arch = "x86")]
pub const SELF_OFFSET: usize = 0x18;
#[cfg(target_arch = "x86_64")]
pub const SELF_OFFSET: usize = 0x30;

/// `ClientId.UniqueProcess`.
#[cfg(target_arch = "x86")]
pub const UNIQUE_PROCESS_OFFSET: usize = 0x20;
#[cfg(target_arch = "x86_64")]
pub const UNIQUE_PROCESS_OFFSET: usize = 0x40;

/// `LastErrorValue`.
#[cfg(target_arch = "x86")]
pub const LAST_ERROR_OFFSET: usize = 0x34;
#[cfg(target_arch = "x86_64")]
pub const LAST_ERROR_OFFSET: usize = 0x68;

/// TEB'den `offset` baytindaki kelimeyi okur.
///
/// Erisim segment onekiyle: i386'da `fs:`, x86_64'te `gs:`. Ayni kod,
/// iki ayri register -- fark derleme aninda kapaniyor.
pub fn read(offset: usize) -> usize {
    let value: usize;
    unsafe {
        #[cfg(target_arch = "x86")]
        core::arch::asm!("mov {0}, fs:[{1}]", out(reg) value, in(reg) offset,
                         options(nostack, preserves_flags, readonly));
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov {0}, gs:[{1}]", out(reg) value, in(reg) offset,
                         options(nostack, preserves_flags, readonly));
    }
    value
}

/// TEB'den 32-bitlik bir alan okur (`LastErrorValue` gibi).
pub fn read32(offset: usize) -> u32 {
    let value: u32;
    unsafe {
        #[cfg(target_arch = "x86")]
        core::arch::asm!("mov {0:e}, fs:[{1}]", out(reg) value, in(reg) offset,
                         options(nostack, preserves_flags, readonly));
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov {0:e}, gs:[{1}]", out(reg) value, in(reg) offset,
                         options(nostack, preserves_flags, readonly));
    }
    value
}

/// `NtCurrentTeb()` -- TEB'in kendi adresi.
pub fn current() -> usize {
    read(SELF_OFFSET)
}
