//! POSIX sinyalleri -- kullanici tarafi.
//!
//! Sinyal, uygulamanin kendi akisini kesip cagirdigi bir islevdir; ama
//! **uygulama onu cagirmaz**, cekirdek cagirir. Isleyici dondugunde
//! program hicbir sey olmamis gibi kaldigi yerden devam eder.
//!
//! ## Tramplen neden var
//!
//! Isleyici sirandan bir Rust fonksiyonudur; bittiginde `ret` yapar. O
//! `ret` bir yere donmek zorunda -- ama donulecek "cagiran" yoktur,
//! cunku cagri gercek bir cagri degildi. Bu yuzden cekirdek yigina bir
//! donus adresi koyar ve o adres burada tanimli tramplendir: tek isi
//! `sigreturn` cagirmaktir, boylece cekirdek saklanan baglami geri koyar.
//!
//! Tramplenin **kullanici tarafinda** olmasi bilincli: aksi halde
//! cekirdegin surecin adres uzayina kod yazmasi gerekirdi. Gercek i386
//! Linux'ta da cozum aynidir (`sigaction.sa_restorer`).
//!
//! ## Kullanim
//!
//! ```ignore
//! extern "C" fn on_usr1(signo: u32) { /* ... */ }
//! signal::install(signal::SIGUSR1, on_usr1);
//! ```
//!
//! Isleyici icinde ne yapilabilecegi sinirlidir: cekirdek isleyici
//! calisirken yeni sinyal teslim etmez, ama isleyici asil akisin ortasinda
//! calistigi icin paylasilan durumu bozabilir. Sayac artirmak, bayrak
//! kaldirmak guvenlidir.

use crate::sys;

pub const SIGHUP: u32 = 1;
pub const SIGINT: u32 = 2;
pub const SIGQUIT: u32 = 3;
pub const SIGILL: u32 = 4;
pub const SIGABRT: u32 = 6;
pub const SIGFPE: u32 = 8;
/// Yakalanamaz: `install` bu sinyal icin basarisiz olur.
pub const SIGKILL: u32 = 9;
pub const SIGUSR1: u32 = 10;
pub const SIGSEGV: u32 = 11;
pub const SIGUSR2: u32 = 12;
pub const SIGALRM: u32 = 14;
pub const SIGTERM: u32 = 15;

/// Varsayilan davranis (TCMK'de: sureci sonlandir).
pub const SIG_DFL: usize = 0;
/// Sinyali yok say.
pub const SIG_IGN: usize = 1;

// Tramplen. `global_asm!` ile yazilir cunku bir fonksiyon prologu/epilogu
// istemiyoruz: cekirdek buraya `ret` ile gelir, biz de dogrudan cekirdege
// geri gireriz. Cagri **donmez**; cekirdek baglami degistirdigi icin
// islemci baska bir noktada uyanir.
#[cfg(target_arch = "x86")]
core::arch::global_asm!(
    ".globl __tcmk_sigreturn",
    "__tcmk_sigreturn:",
    "mov eax, 119",
    "int 0x80",
);

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl __tcmk_sigreturn",
    "__tcmk_sigreturn:",
    "mov eax, 15",
    "syscall",
);

extern "C" {
    fn __tcmk_sigreturn();
}

// --- `sigaction` bayraklari (Linux ile ayni sayilar) ------------------

/// Isleyici kosarken **kendi sinyali engellenmez**.
///
/// Varsayilan POSIX davranisi tersidir: teslim edilen sinyal, isleyicisi
/// kosarken otomatik engellenir -- yani bir isleyici kendi kendini
/// yeniden cagiramaz. Bu bayrak o korumayi kaldirir.
pub const SA_NODEFER: u32 = 0x4000_0000;

/// Teslimden **once** yerlestirme `SIG_DFL`e doner: tek atimlik isleyici.
///
/// Eski `signal(2)` semantiginin ta kendisi; `sigaction` onu bayrak
/// haline getirdi.
pub const SA_RESETHAND: u32 = 0x8000_0000;

/// `sigaction`in cekirdege verdigi yapi -- dort kelime.
///
/// Gercek `struct sigaction`in sadelestirilmisi: `sa_handler`,
/// `sa_restorer`, `sa_flags`, `sa_mask`. Registerlere sigdirmak yerine
/// **isaretciyle** gecirilir, tipki `rt_sigaction` gibi; bayrak
/// eklendikce bozulmayan tek tasima bicimi budur.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SigAction {
    pub handler: usize,
    pub restorer: usize,
    pub flags: u32,
    pub mask: u32,
}

/// Ham `sigaction` cagrisi. `old` NULL olabilir.
fn sigaction_raw(signo: u32, act: *const SigAction, old: *mut usize) -> isize {
    unsafe {
        sys::syscall3(
            sys::SYS_SIGACTION,
            signo as usize,
            act as usize,
            old as usize,
        ) as isize
    }
}

/// Bir sinyal icin isleyici kurar -- **bayraklariyla**.
///
/// `install` bunun bayraksiz halidir; libc'de `signal()`in `sigaction()`
/// uzerine kurulmus olmasiyla ayni iliski.
pub fn action(signo: u32, handler: extern "C" fn(u32), flags: u32, mask: u32) -> isize {
    let act = SigAction {
        handler: handler as usize,
        restorer: __tcmk_sigreturn as *const () as usize,
        flags,
        mask,
    };
    sigaction_raw(signo, &act, core::ptr::null_mut())
}

/// Bir sinyal icin isleyici kurar; onceki isleyiciyi doner.
///
/// `SIGKILL` icin basarisizdir (negatif doner) -- yakalanamaz.
pub fn install(signo: u32, handler: extern "C" fn(u32)) -> isize {
    let mut previous = 0usize;
    let act = SigAction {
        handler: handler as usize,
        restorer: __tcmk_sigreturn as *const () as usize,
        flags: 0,
        mask: 0,
    };
    let result = sigaction_raw(signo, &act, &mut previous);
    if result < 0 {
        result
    } else {
        previous as isize
    }
}

/// Yerlestirmeyi degistirmeden **sorar**.
pub fn current_handler(signo: u32) -> usize {
    let mut previous = 0usize;
    if sigaction_raw(signo, core::ptr::null(), &mut previous) < 0 {
        return SIG_DFL;
    }
    previous
}

/// Sinyali yok saydirir.
pub fn ignore(signo: u32) -> isize {
    let act = SigAction {
        handler: SIG_IGN,
        restorer: 0,
        flags: 0,
        mask: 0,
    };
    sigaction_raw(signo, &act, core::ptr::null_mut())
}

/// Varsayilan davranisa dondurur.
pub fn default(signo: u32) -> isize {
    let act = SigAction {
        handler: SIG_DFL,
        restorer: 0,
        flags: 0,
        mask: 0,
    };
    sigaction_raw(signo, &act, core::ptr::null_mut())
}

/// Bir surece sinyal gonderir. POSIX'te `kill` "oldur" degil "sinyal
/// gonder" demektir; oldurme, sinyalin varsayilan davranisidir.
pub fn kill(pid: usize, signo: u32) -> isize {
    unsafe { sys::syscall2(sys::SYS_KILL, pid, signo as usize) as isize }
}

/// Calisan surecin kimligi.
pub fn getpid() -> usize {
    unsafe { sys::syscall0(sys::SYS_GETPID) }
}

// --- Engel maskesi ---

/// Verilen sinyalleri **ekle** (engelle).
pub const SIG_BLOCK: usize = 0;
/// Verilen sinyalleri **cikar** (engeli kaldir).
pub const SIG_UNBLOCK: usize = 1;
/// Maskeyi verilenle **degistir**.
pub const SIG_SETMASK: usize = 2;

/// Sinyal numarasini maske bitine cevirir.
pub const fn mask_of(signo: u32) -> u32 {
    1 << signo
}

/// POSIX `sigprocmask`: engel maskesini degistirir, **eskisini** doner.
///
/// Bloke bir sinyal kaybolmaz -- bekler ve maske acilinca teslim edilir.
/// Kritik bolge kalibi budur: maskele, isi yap, maskeyi ac.
///
/// Tasima farki: gercek POSIX iki `sigset_t` isaretcisi alir; burada
/// maske deger olarak gecer (32 sinyal tek kelimeye sigiyor).
pub fn sigprocmask(how: usize, set: u32) -> u32 {
    unsafe { sys::syscall2(sys::SYS_SIGPROCMASK, how, set as usize) as u32 }
}

/// Mevcut engel maskesini okur (hicbir seyi degistirmeden).
pub fn current_mask() -> u32 {
    sigprocmask(SIG_BLOCK, 0)
}

/// POSIX `pause`: teslim edilebilir bir sinyal gelene kadar **uyur**.
///
/// Bu cagriya kadar sinyal beklemenin tek yolu, `yield_now` ile donen
/// bir yoklama donguysu -- yani sinyal gelene kadar CPU yakmak. `pause`
/// sirasinda gorev hic zamanlanmaz; kabugun `ps` tablosunda `sinyal`
/// durumunda gorunur ve `cpu` sayaci artmaz.
pub fn pause() -> isize {
    crate::sys::pause()
}

/// POSIX `sigsuspend`: maskeyi **gecici** degistirip sinyal bekler.
///
/// `sigprocmask` + `pause` ikilisinden farki bolunmez olmasi: ayri
/// cagrilarda sinyal tam aradaki pencerede gelirse `pause` onu kacirir
/// ve surec sonsuza kadar uyar. Maske, isleyici dondukten sonra eski
/// haline doner.
pub fn sigsuspend(mask: u32) -> isize {
    crate::sys::sigsuspend(mask)
}

/// POSIX `alarm`: `seconds` sonra kendine `SIGALRM` gonderir.
///
/// Onceki alarmdan kalan saniyeyi doner; `0` alarmi iptal eder.
pub fn alarm(seconds: u32) -> u32 {
    unsafe { sys::syscall1(sys::SYS_ALARM, seconds as usize) as u32 }
}
