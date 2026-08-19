//! Is-parcacigi yerel deposu -- **iki mimari, iki mekanizma**.
//!
//! Bu modulden onceki durum: FS/GS tabanlari her zaman sifirdi ve hicbir
//! program onlari degistiremiyordu. Kendi userland'imiz icin sorun
//! degildi (TLS kullanmiyoruz), ama derleyicinin urettigi gercek bir
//! Linux ikilisi icin yapisal bir engel: glibc'nin ilk isi TLS blogunu
//! kurmaktir ve yigin koruyucusu her fonksiyon girisinde onu okur.
//!
//! ```text
//!   i386     set_thread_area(user_desc*)   bir GDT TANIMLAYICISI ister;
//!            cekirdek girdi ayirir, program seciciyi GS'e yukler
//!   x86_64   arch_prctl(ARCH_SET_FS, adr)  long mode segmentasyonu
//!            kaldirdi; taban dogrudan bir MSR
//! ```
//!
//! Register secimi de mimariye gore degisiyor -- ve Windows'unkiyle
//! **capraz**: Linux i386'da GS, x86_64'te FS kullanir; Windows tam
//! tersini yapar (bkz. cekirdekteki `core::tls`).

use crate::sys;

/// `struct user_desc` -- i386'nin `set_thread_area` yapisi.
///
/// Yalnizca ilk iki alan anlam tasiyor; TCMK limit ve bayraklari sabit
/// tutuyor (duz 4 GiB, ring 3 verisi).
#[cfg(target_arch = "x86")]
#[repr(C)]
struct UserDesc {
    entry_number: u32,
    base_addr: u32,
    limit: u32,
    flags: u32,
}

/// Is-parcacigi tabanini ayarlar.
///
/// Basarili olursa bundan sonra `read(offset)` o blogu okur.
#[cfg(target_arch = "x86")]
pub fn set(base: usize) -> bool {
    // -1: "sen bir girdi ayir". Cekirdek ayirdigi numarayi geri yazar.
    let mut desc = UserDesc {
        entry_number: u32::MAX,
        base_addr: base as u32,
        limit: 0xFFFFF,
        flags: 0,
    };
    if unsafe { sys::syscall1(sys::SYS_SET_THREAD_AREA, &mut desc as *mut _ as usize) as isize }
        != 0
    {
        return false;
    }
    // Secici = (girdi << 3) | RPL 3. Tanimlayiciyi kurmak yetmez;
    // registeri yuklemek de programin isi -- Linux'ta da oyle.
    let selector = ((desc.entry_number << 3) | 3) as u16;
    unsafe {
        core::arch::asm!("mov gs, {0:x}", in(reg) selector, options(nostack, preserves_flags));
    }
    true
}

/// x86_64: taban dogrudan MSR'ye yaziliyor, secici yok.
#[cfg(target_arch = "x86_64")]
pub fn set(base: usize) -> bool {
    const ARCH_SET_FS: usize = 0x1002;
    unsafe { sys::syscall2(sys::SYS_ARCH_PRCTL, ARCH_SET_FS, base) as isize == 0 }
}

/// Is-parcacigi blogundan `offset` baytindaki kelimeyi okur.
///
/// Erisim segment onekiyle yapiliyor: i386'da `gs:`, x86_64'te `fs:`.
/// Ayni kod, iki ayri register -- ve fark derleme aninda kapaniyor.
pub fn read(offset: usize) -> usize {
    let value: usize;
    unsafe {
        #[cfg(target_arch = "x86")]
        core::arch::asm!("mov {0}, gs:[{1}]", out(reg) value, in(reg) offset,
                         options(nostack, preserves_flags, readonly));
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov {0}, fs:[{1}]", out(reg) value, in(reg) offset,
                         options(nostack, preserves_flags, readonly));
    }
    value
}
