//! Is-parcacigi yerel deposu -- **surec basina segment tabani**.
//!
//! Bu cagriya kadar TCMK'de FS/GS tabanlari her zaman sifirdi ve hicbir
//! program onlari degistiremiyordu. Kendi userland'imiz icin sorun
//! degildi (TLS kullanmiyor), ama derleyicinin urettigi gercek bir
//! ikili icin **yapisal bir engel**: glibc'nin ilk yaptigi islerden biri
//! TLS blogunu kurmaktir, ve yigin koruyucusu (`stack protector`) her
//! fonksiyon girisinde o blogu okur.
//!
//! ## Iki isletim sistemi, iki register -- ve ikisi de ters
//!
//! Ayni donanim mekanizmasi (segment tabani) iki dunyada da kullaniliyor
//! ama **secilen registerlar capraz**:
//!
//! ```text
//!             i386            x86_64
//!   Linux     GS              FS
//!   Windows   FS (TEB)        GS (TEB)
//! ```
//!
//! Yani bir mimaride Linux'un kullandigi registeri oteki mimaride
//! Windows kullaniyor. TCMK ikisini de tasimak zorunda, o yuzden gorev
//! basina **iki** taban tutuluyor.
//!
//! ## Mimariler mekanizmayi ayri cozuyor
//!
//! * **i386**: taban bir GDT tanimlayicisinda. Program bir **secici**
//!   yukler; taban degisince tanimlayici yeniden yazilip register
//!   yeniden yuklenmeli (bkz. `gdt::i386::set_tls_bases`).
//! * **x86_64**: long mode segmentasyonu **kaldirdi** -- kalan tek
//!   istisna FS/GS tabanlari ve onlar birer MSR. Tanimlayici yok,
//!   secici yok, yalnizca `wrmsr`.
//!
//! Ayrimin gorunur sonucu: iki ABI'nin cagri isimleri de farkli.
//! POSIX'te i386 `set_thread_area` (bir tanimlayici ister), x86_64
//! `arch_prctl` (dogrudan adres alir).

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::scheduler::MAX_TASKS;

static FS_BASE: [AtomicUsize; MAX_TASKS] = [const { AtomicUsize::new(0) }; MAX_TASKS];
static GS_BASE: [AtomicUsize; MAX_TASKS] = [const { AtomicUsize::new(0) }; MAX_TASKS];

/// Bir gorevin FS tabani.
///
/// i386'da su an yalnizca Win32 tarafi kullanacak (TEB); POSIX orada
/// GS'i kullaniyor. Bu yuzden bir mimaride "kullanilmiyor" gorunuyor.
#[allow(dead_code)]
pub fn fs_of(task: usize) -> usize {
    if task >= MAX_TASKS {
        return 0;
    }
    FS_BASE[task].load(Ordering::Relaxed)
}

/// Bir gorevin GS tabani.
#[allow(dead_code)]
pub fn gs_of(task: usize) -> usize {
    if task >= MAX_TASKS {
        return 0;
    }
    GS_BASE[task].load(Ordering::Relaxed)
}

/// FS tabanini ayarlar ve **hemen etkinlestirir**.
///
/// Etkinlestirme burada olmali: cagiran surec Ring 3'e donunce yeni
/// tabani gormeyi bekler, bir sonraki gorev degisimini degil.
#[allow(dead_code)]
pub fn set_fs(task: usize, base: usize) {
    if task >= MAX_TASKS {
        return;
    }
    FS_BASE[task].store(base, Ordering::Relaxed);
    activate(task);
}

/// GS tabanini ayarlar ve hemen etkinlestirir.
#[allow(dead_code)]
pub fn set_gs(task: usize, base: usize) {
    if task >= MAX_TASKS {
        return;
    }
    GS_BASE[task].store(base, Ordering::Relaxed);
    activate(task);
}

/// Yeni bir gorev yuvasi: tabanlar sifirlanir.
///
/// `cwd` ve ortamdan farkli olarak `execve` bunu **korumamali**: yeni
/// imajin TLS blogu yok ve eski imajin adresini gostermeye devam etmek,
/// serbest kalmis bellege isaret eden bir segment birakirdi. Bu yuzden
/// sifirlama yuva ayrilirken degil, **imaj yuklenirken** cagriliyor
/// (bkz. `process::enter_ring3`).
pub fn reset(task: usize) {
    if task >= MAX_TASKS {
        return;
    }
    FS_BASE[task].store(0, Ordering::Relaxed);
    GS_BASE[task].store(0, Ordering::Relaxed);
}

/// `fork`: cocuk ebeveynin tabanlarini devralir.
///
/// Adres uzayi kopyalandigi icin ayni sanal adres cocukta da gecerli --
/// yani taban degeri oldugu gibi tasinabilir.
pub fn clone_into(child: usize, parent: usize) {
    if child >= MAX_TASKS || parent >= MAX_TASKS {
        return;
    }
    FS_BASE[child].store(FS_BASE[parent].load(Ordering::Relaxed), Ordering::Relaxed);
    GS_BASE[child].store(GS_BASE[parent].load(Ordering::Relaxed), Ordering::Relaxed);
}

/// Verilen gorevin tabanlarini donanima yazar.
///
/// Gorev degisiminde cagriliyor. Cekirdek FS/GS kullanmadigi icin
/// aradaki surede yanlis bir tabanin yuklu olmasi zarar vermiyor.
pub fn activate(task: usize) {
    if task >= MAX_TASKS {
        return;
    }
    let fs = FS_BASE[task].load(Ordering::Relaxed);
    let gs = GS_BASE[task].load(Ordering::Relaxed);

    #[cfg(target_arch = "x86")]
    crate::level0a::gdt::set_tls_bases(fs as u32, gs as u32);

    #[cfg(target_arch = "x86_64")]
    unsafe {
        // Long mode'da tanimlayici yok: taban dogrudan MSR'den geliyor.
        crate::arch::x86_64::wrmsr(MSR_FS_BASE, fs as u64);
        crate::arch::x86_64::wrmsr(MSR_GS_BASE, gs as u64);
    }
}

/// `IA32_FS_BASE` -- long mode'da FS tabani.
#[cfg(target_arch = "x86_64")]
const MSR_FS_BASE: u32 = 0xC000_0100;
/// `IA32_GS_BASE` -- long mode'da GS tabani.
#[cfg(target_arch = "x86_64")]
const MSR_GS_BASE: u32 = 0xC000_0101;
