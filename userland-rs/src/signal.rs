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
/// Okuyan ucu kapali bir boruya yazmak.
///
/// POSIX'in en sert varsayilani: yakalanmazsa **surec oler**. Windows'ta
/// karsiligi yok -- orada `WriteFile` yalnizca `ERROR_BROKEN_PIPE`
/// doner ve surec yasar.
pub const SIGPIPE: u32 = 13;
pub const SIGALRM: u32 = 14;
pub const SIGTERM: u32 = 15;

// --- Is denetimi ---

/// Durmus bir sureci devam ettirir.
///
/// Iki ozelligi diger sinyallerden ayri: durmus bir surec teslim
/// alamadigi icin cekirdek onu **once kaldiriyor**, ve varsayilani
/// "oldur" degil "devam et".
pub const SIGCONT: u32 = 18;
/// Sureci durdurur -- `SIGKILL` gibi yakalanamaz ve maskelenemez.
pub const SIGSTOP: u32 = 19;
/// Terminalden gelen durdurma istegi (Ctrl-Z) -- **yakalanabilir**.
pub const SIGTSTP: u32 = 20;

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

// x86_64'te donus **kesme kapisindan** yapiliyor, `syscall`dan degil.
//
// Fark ince ama belirleyici: `syscall` komutu donus adresini `RCX`e,
// bayraklari `R11`e koyar. Yani o yoldan donen bir cagri **o iki
// registeri geri yukleyemez** -- ikisi donus bilgisinin kendisini
// tasir. Siradan bir cagri icin bu sorun degil (ABI zaten ikisini
// "cagri tarafindan bozulur" sayar), ama `sigreturn` siradan bir cagri
// degil: isleyicinin `ucontext_t`de yaptigi duzeltmeyi geri yuklemesi
// gerekiyor ve duzeltilen sey `RCX` olabilir.
//
// `int 0x80` bir kesme kapisidir ve `iretq` ile doner; cerceve butun
// registerlari tasir. Bu yuzden TCMK'nin x86_64 cekirdegi o vektoru
// de bagli tutuyor (bkz. `idt::x86_64`). Windows yuzu ayni sebeple
// zaten `int 0x2E` kullaniyordu -- POSIX yuzu de artik esit.
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl __tcmk_sigreturn",
    "__tcmk_sigreturn:",
    "mov eax, 15",
    "int 0x80",
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

/// Bolunen bir sistem cagrisi, isleyici dondukten sonra **yeniden
/// calistirilir**.
///
/// Bloke eden bir cagri (bos borudan `read` gibi) sirasinda sinyal
/// gelirse iki secenek var:
///
/// ```text
///   SA_RESTART yok -> cagri -EINTR ile doner, program kendisi dener
///   SA_RESTART var -> cekirdek cagriyi kendisi yeniden baslatir
/// ```
///
/// Ikincisi `EINTR`i **gorunmez** kilar. Eski `signal(2)` yuzu bayragi
/// kendiliginden koyar (bkz. `install`); ham `sigaction` koymaz -- ve
/// bu fark, `EINTR` denetlemeyi unutan kodun neden bazen calisip bazen
/// bozuldugunun tarihsel sebebi.
pub const SA_RESTART: u32 = 0x1000_0000;

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
///
/// Bayrak **konmaz**: bolunen cagrilar `-EINTR` doner. `SA_RESTART`
/// isteyen `install_with` kullanmali -- ayrim bilincli, cunku ikisi
/// gercekten farkli davraniyor ve hangisinin istendigi cagiranin
/// bilmesi gereken bir sey.
pub fn install(signo: u32, handler: extern "C" fn(u32)) -> isize {
    install_with(signo, handler, 0)
}

/// Isleyici **uc** arguman alir: `(signo, *const SigInfo, *mut UContext)`.
///
/// Tek argumanli yuz yalnizca "hangi sinyal" der. Uc argumanli yuz
/// "neden" ve "nerede" sorularina da cevap veriyor -- ve ikincisi
/// **yazilabilir**: isleyici bir registeri duzeltip donerse hatali komut
/// duzeltilmis haliyle tekrarlanir.
pub const SA_SIGINFO: u32 = 0x0000_0004;

/// Isleyici **ayri** bir yiginda kossun (`sigaltstack` ile kurulan).
///
/// Tek bir sey icin var ve o sey onemli: **yigin tasmasini yakalamak**.
/// Tasma aninda yigin isaretcisi artik gecerli bir yeri gostermiyor;
/// sinyal cercevesi oraya kurulamaz, yani sinyal teslim edilemez ve
/// surec tanisiz oler.
pub const SA_ONSTACK: u32 = 0x0800_0000;

/// `ss_flags`: su an o yiginin **ustunde** kosuluyor.
pub const SS_ONSTACK: u32 = 1;
/// `ss_flags`: ayri yigini kaldir.
pub const SS_DISABLE: u32 = 2;

/// Ayri yigin icin kabul edilen en kucuk olcu (Linux ile ayni).
pub const MINSIGSTKSZ: usize = 2048;

/// `stack_t` -- `sigaltstack`in aldigi ve dondurdugu yapi.
///
/// Alan sirasi Linux ile birebir: `sigaltstack` cagiran derlenmis bir
/// kod onu tam bu duzende yazar.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AltStack {
    pub sp: usize,
    pub flags: u32,
    pub size: usize,
}

impl AltStack {
    /// Kurulu olmayan yigin.
    pub const NONE: Self = AltStack {
        sp: 0,
        flags: SS_DISABLE,
        size: 0,
    };
}

/// Ayri sinyal yigini kurar ve/veya oncekini okur.
///
/// `new` `None` ise yalnizca sorulur; `old` `None` ise yalnizca kurulur.
/// Doner: 0 ya da negatif hata.
pub fn sigaltstack(new: Option<&AltStack>, old: Option<&mut AltStack>) -> isize {
    let new_ptr = new.map_or(core::ptr::null(), |s| s as *const AltStack);
    let old_ptr = old.map_or(core::ptr::null_mut(), |s| s as *mut AltStack);
    unsafe {
        sys::syscall2(
            sys::SYS_SIGALTSTACK,
            new_ptr as usize,
            old_ptr as usize,
        ) as isize
    }
}

/// Uc argumanli isleyicinin imzasi.
pub type SigActionHandler = extern "C" fn(u32, *const SigInfo, *mut UContext);

/// Isleyiciyi `SA_SIGINFO` ile kurar.
///
/// `action`dan farki yalnizca imzasi ve bayragin kendiliginden
/// konmasi. Ayri bir fonksiyon olmasi bilincli: iki imza ikili duzeyde
/// uyumsuz, yani bayragi yanlislikla unutmak ya da fazladan koymak
/// isleyiciyi cop argumanlarla cagirirdi.
pub fn action_info(signo: u32, handler: SigActionHandler, flags: u32, mask: u32) -> isize {
    let act = SigAction {
        handler: handler as *const () as usize,
        restorer: __tcmk_sigreturn as *const () as usize,
        flags: flags | SA_SIGINFO,
        mask,
    };
    sigaction_raw(signo, &act, core::ptr::null_mut())
}

// --- `si_code`: sinyalin kaynagi (Linux ile ayni sayilar) ------------
pub const SI_USER: i32 = 0;
pub const SI_KERNEL: i32 = 0x80;
/// `SIGSEGV`: adres **eslenmemis**.
pub const SEGV_MAPERR: i32 = 1;
/// `SIGSEGV`: adres eslenmis ama erisim izni yok.
pub const SEGV_ACCERR: i32 = 2;
/// `SIGFPE`: tam sayi sifira bolme.
pub const FPE_INTDIV: i32 = 1;
/// `SIGILL`: gecersiz islem.
pub const ILL_ILLOPN: i32 = 2;

/// `siginfo_t`nin okunan bolumu.
///
/// Gercek `siginfo_t` 128 bayttir ve sonrasi sinyale gore degisen bir
/// birlesimdir. Burada yalnizca **ortak bas** alaniyla birlesimin ilk
/// kelimesi tanimli; geri kalani okunmuyor, o yuzden yazilmasina da
/// gerek yok.
///
/// Birlesimin tek alanla temsil edilmesi bir sadelestirme degil, ikili
/// gercek: `si_addr` ile `si_pid` **ayni ofsette** durur. Hangisinin
/// gecerli oldugunu sinyal ve `si_code` belirler.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SigInfo {
    pub signo: i32,
    pub errno: i32,
    pub code: i32,
    /// x86_64'te birlesim 8'e hizali basliyor; bu dolgu onun yeri.
    #[cfg(target_arch = "x86_64")]
    #[allow(dead_code)]
    _pad: i32,
    /// Birlesimin ilk kelimesi.
    field: usize,
}

impl SigInfo {
    /// Hataya yol acan adres (`SIGSEGV`, `SIGBUS`, `SIGFPE`, `SIGILL`).
    pub fn addr(&self) -> usize {
        self.field
    }

    /// Gonderenin kimligi (`si_code == SI_USER`).
    pub fn pid(&self) -> usize {
        self.field & 0xFFFF_FFFF
    }
}

/// `ucontext_t` -- sinyal kesildiginde registerlarin durumu.
///
/// Ham bayt blogu olarak tutuluyor ve alanlara ofsetle erisiliyor;
/// gerekce `winapi::Context` ile ayni (bkz. orada): kaydin buyuk
/// bolumu bu programin dokunmadigi bolgeler ve ic duzenlerini burada
/// yazmak, iki yerde ayrisabilen bir ABI birakirdi.
///
/// **Yazilabilir** olmasi asil nokta: isleyici bir registeri
/// degistirip donerse cekirdek degistirilmis baglami geri yukler.
#[repr(C)]
pub struct UContext {
    _opaque: [u8; 16],
}

/// `ucontext_t` icindeki register ofsetleri.
///
/// i386'da `uc_mcontext` 0x14'te bir `struct sigcontext`tir; x86_64'te
/// 0x28'te 23 kelimelik bir `gregs` dizisi. Sayilar Linux ABI'sinin
/// parcasidir.
#[cfg(target_arch = "x86")]
mod uc {
    const M: usize = 0x14;
    pub const EDI: usize = M + 16;
    pub const ESI: usize = M + 20;
    pub const EBP: usize = M + 24;
    pub const ESP: usize = M + 28;
    pub const EBX: usize = M + 32;
    pub const EDX: usize = M + 36;
    pub const ECX: usize = M + 40;
    pub const EAX: usize = M + 44;
    pub const EIP: usize = M + 56;
    pub const CR2: usize = M + 84;
}

#[cfg(target_arch = "x86_64")]
mod uc {
    const M: usize = 0x28;
    pub const EDI: usize = M + 64;
    pub const ESI: usize = M + 72;
    pub const EBP: usize = M + 80;
    pub const EBX: usize = M + 88;
    pub const EDX: usize = M + 96;
    pub const EAX: usize = M + 104;
    pub const ECX: usize = M + 112;
    pub const ESP: usize = M + 120;
    pub const EIP: usize = M + 128;
    pub const CR2: usize = M + 176;
}

/// `ucontext_t`de adlandirilmis bir register.
///
/// Adlar i386 yuzunden geliyor ve x86_64'te 64 bitlik ikizine denk
/// duser (`Cx` -> `ecx`/`rcx`). `winapi`nin `Reg`i ile ayni desen:
/// kaydin duzeni mimariye gore degisiyor ama **hangi register** oldugu
/// degismiyor.
#[derive(Clone, Copy)]
pub enum Reg {
    Ax,
    Bx,
    Cx,
    Dx,
    Si,
    Di,
    Bp,
    Sp,
    Ip,
    /// Sayfa hatasinin adresi -- register degil, ama ayni kayitta.
    Cr2,
}

fn reg_offset(reg: Reg) -> usize {
    match reg {
        Reg::Ax => uc::EAX,
        Reg::Bx => uc::EBX,
        Reg::Cx => uc::ECX,
        Reg::Dx => uc::EDX,
        Reg::Si => uc::ESI,
        Reg::Di => uc::EDI,
        Reg::Bp => uc::EBP,
        Reg::Sp => uc::ESP,
        Reg::Ip => uc::EIP,
        Reg::Cr2 => uc::CR2,
    }
}

/// `ucontext_t`den bir register okur.
///
/// # Safety
/// `context` cekirdegin kurdugu gecerli bir `ucontext_t` olmalidir --
/// yani yalnizca bir `SA_SIGINFO` isleyicisinin icinde.
pub unsafe fn get_reg(context: *const UContext, reg: Reg) -> usize {
    ((context as usize + reg_offset(reg)) as *const usize).read_unaligned()
}

/// `ucontext_t`ye bir register yazar.
///
/// Yazilan deger isleyici **dondugunde** yururluge girer: cekirdek
/// baglami bu kayittan geri okur. Hatali bir komutu duzeltmenin yolu
/// budur -- Windows'ta ayni isi `CONTEXT` + `EXCEPTION_CONTINUE_EXECUTION`
/// yapiyor.
///
/// # Safety
/// `get_reg` ile ayni kosul.
pub unsafe fn set_reg(context: *mut UContext, reg: Reg, value: usize) {
    ((context as usize + reg_offset(reg)) as *mut usize).write_unaligned(value)
}

/// Isleyiciyi **bayraklarla** kurar (`SA_RESTART`, `SA_NODEFER`, ...).
pub fn install_with(signo: u32, handler: extern "C" fn(u32), flags: u32) -> isize {
    let mut previous = 0usize;
    let act = SigAction {
        handler: handler as usize,
        restorer: __tcmk_sigreturn as *const () as usize,
        flags,
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

/// Bir **surec grubuna** sinyal gonderir (POSIX `kill(-pgid, sig)`).
///
/// Kabugun Ctrl-C'si budur: bir boru hatti uc ayri surectir ama tek bir
/// istir, ve hepsi birden bitmelidir.
pub fn kill_group(pgid: usize, signo: u32) -> isize {
    // Cekirdek isaret bitine bakiyor; negatif deger grup demek.
    let target = -(pgid as isize);
    unsafe { sys::syscall2(sys::SYS_KILL, target as usize, signo as usize) as isize }
}

/// Surecin grubunu degistirir (POSIX `setpgid`).
///
/// Ikisi de sifir olabiliyor ve anlamlari ayri: `pid = 0` "kendim",
/// `pgid = 0` "kendi numaramla yeni bir grup kur".
pub fn setpgid(pid: usize, pgid: usize) -> isize {
    unsafe { sys::syscall2(sys::SYS_SETPGID, pid, pgid) as isize }
}

/// Surecin grubunu okur (POSIX `getpgid`); `pid = 0` "kendim".
pub fn getpgid(pid: usize) -> isize {
    unsafe { sys::syscall1(sys::SYS_GETPGID, pid) as isize }
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
