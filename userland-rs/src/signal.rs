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

/// Bir **cocuk surecin durumu degisti**: cikti, olduruldu, durdu ya da
/// devam etti.
///
/// Iki ozelligiyle ayri duruyor:
///
///   * Varsayilani **yok saymak**. Cocugu olan her surec onu aliyor ve
///     cogu umursamiyor; varsayilani olum olsaydi `fork` eden her
///     program cocugu bitince olurdu.
///   * `si_code` (`CLD_*`) ve `si_status` birlikte, `waitpid`
///     cagirmadan da cevap veriyor.
///
/// Windows'ta karsiligi yok, ve bu bir eksiklik degil -- baska bir
/// secim:
///
/// ```text
///   POSIX    cocuk oldu  ->  ebeveyne SIGCHLD GONDERILIR   (itme)
///   Windows  cocuk oldu  ->  surec nesnesi ISARETLENIR     (cekme)
/// ```
pub const SIGCHLD: u32 = 17;

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

// --- Gercek-zamanli sinyaller -----------------------------------------

/// Ilk gercek-zamanli sinyal.
///
/// Buradan itibaren sinyaller **kuyruga** girer: uc kez gonderilen sinyal
/// uc kez teslim edilir ve her teslim kendi `si_value`siyla gelir.
/// 1..=31 arasindakiler ise bir bit maskesinde **birlesir** -- uc gonderim
/// tek teslime duser.
///
/// Ayrim POSIX'in en az bilinen kurallarindan biri ve kaynak temelli: bit
/// maskesi sabit yer tutar ve dolamaz, kuyruk dolabilir ve doldugunda
/// `sigqueue` `EAGAIN` doner.
pub const SIGRTMIN: u32 = 32;
/// Son gercek-zamanli sinyal.
pub const SIGRTMAX: u32 = 63;

/// Sinyal gercek-zamanli mi -- yani kuyruga mi girer?
pub const fn is_rt(signo: u32) -> bool {
    signo >= SIGRTMIN && signo <= SIGRTMAX
}

/// 64 bitlik `sigset_t`, makine kelimeleri halinde.
///
/// Neden bir `u64` degil: yapi cekirdege **isaretciyle** gidiyor ve
/// cekirdek onu kelime kelime okuyor. `u64`un hizalamasi iki mimaride
/// ayni degil (Rust i386'da da 8'e hizalar), yani araya dolgu girip
/// girmedigi hedefe gore degisirdi -- bir ABI'nin tasiyamayacagi tur bir
/// belirsizlik. Kelime dizisi bu soruyu ortadan kaldiriyor.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SigSet {
    #[cfg(target_arch = "x86")]
    words: [usize; 2],
    #[cfg(target_arch = "x86_64")]
    words: [usize; 1],
}

impl SigSet {
    /// Hicbir sinyal iceren bos kume.
    pub const EMPTY: Self = SigSet::from_bits(0);

    /// 64 bitlik maskeyi kelimelere boler.
    pub const fn from_bits(bits: u64) -> Self {
        #[cfg(target_arch = "x86")]
        {
            SigSet {
                words: [bits as u32 as usize, (bits >> 32) as u32 as usize],
            }
        }
        #[cfg(target_arch = "x86_64")]
        {
            SigSet {
                words: [bits as usize],
            }
        }
    }

    /// Kelimeleri 64 bitlik maskede birlestirir.
    pub const fn bits(&self) -> u64 {
        #[cfg(target_arch = "x86")]
        {
            self.words[0] as u64 | ((self.words[1] as u64) << 32)
        }
        #[cfg(target_arch = "x86_64")]
        {
            self.words[0] as u64
        }
    }

    /// Tek bir sinyalden olusan kume.
    pub const fn of(signo: u32) -> Self {
        SigSet::from_bits(1u64 << signo)
    }

    /// Bu sinyal kumede var mi?
    pub const fn has(&self, signo: u32) -> bool {
        self.bits() & (1u64 << signo) != 0
    }
}

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

/// `sigaction`in cekirdege verdigi yapi.
///
/// Gercek `struct sigaction`in sadelestirilmisi: `sa_handler`,
/// `sa_restorer`, `sa_flags`, `sa_mask`. Registerlere sigdirmak yerine
/// **isaretciyle** gecirilir, tipki `rt_sigaction` gibi; bayrak
/// eklendikce bozulmayan tek tasima bicimi budur.
///
/// Butun alanlar makine kelimesi genisliginde, cunku cekirdek yapiyi
/// kelime kelime okuyor. Bir onceki duzende `flags: u32` ve `mask: u32`
/// yan yanaydi ve x86_64'te ikisi **tek bir** 8-baytlik kelimeye
/// oturuyordu; cekirdek ise dort kelime bekliyordu, yani yapinin
/// bittigi yerin otesini okuyup `sa_mask` sayiyordu. Butun cagiranlarin
/// maskeyi sifir gecmesi hatayi gorunmez kilmisti.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SigAction {
    pub handler: usize,
    pub restorer: usize,
    /// `SA_*` bayraklari; yalnizca alt 32 bit anlamli.
    pub flags: usize,
    pub mask: SigSet,
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
pub fn action(signo: u32, handler: extern "C" fn(u32), flags: u32, mask: u64) -> isize {
    let act = SigAction {
        handler: handler as usize,
        restorer: __tcmk_sigreturn as *const () as usize,
        flags: flags as usize,
        mask: SigSet::from_bits(mask),
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

/// Cocuk **durdugunda** `SIGCHLD` gonderilmesin -- yalnizca olumde.
///
/// Cogu program bunu ister: bir cocugun durmasi is denetimi
/// meselesidir ve onunla yalnizca kabuk ilgilenir.
pub const SA_NOCLDSTOP: u32 = 0x0000_0001;

/// Cocuklar **zombi birakmasin**.
///
/// POSIX'in en bilinen tuhafliklarindan biri: `SIGCHLD`i `SIG_IGN`
/// yapmak "umursamiyorum" demenin otesinde bir sey yapar -- cekirdek
/// cocuklari kendisi toplar ve `waitpid` artik `ECHILD` doner.
/// `SA_NOCLDWAIT` ayni etkiyi bir isleyici kuruluyken saglar.
///
/// Yani yok saymak burada sinyali degil **kaydi** siliyor.
pub const SA_NOCLDWAIT: u32 = 0x0000_0002;

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
pub fn action_info(signo: u32, handler: SigActionHandler, flags: u32, mask: u64) -> isize {
    let act = SigAction {
        handler: handler as *const () as usize,
        restorer: __tcmk_sigreturn as *const () as usize,
        flags: (flags | SA_SIGINFO) as usize,
        mask: SigSet::from_bits(mask),
    };
    sigaction_raw(signo, &act, core::ptr::null_mut())
}

// --- `si_code`: sinyalin kaynagi (Linux ile ayni sayilar) ------------
pub const SI_USER: i32 = 0;
pub const SI_KERNEL: i32 = 0x80;
/// `sigqueue` ile geldi -- `si_value` gecerli.
///
/// Negatif olmasi bir kaza degil: POSIX `si_code`in **isaretini** bir
/// ayrim olarak kullanir. Sifir ve pozitif kodlari cekirdek uretir,
/// negatifleri bir surec. (`SI_USER`in 0 olmasi kuraldan once kaldigi
/// icin istisnadir.)
pub const SI_QUEUE: i32 = -1;
/// `SIGSEGV`: adres **eslenmemis**.
pub const SEGV_MAPERR: i32 = 1;
/// `SIGSEGV`: adres eslenmis ama erisim izni yok.
pub const SEGV_ACCERR: i32 = 2;
/// `SIGFPE`: tam sayi sifira bolme.
pub const FPE_INTDIV: i32 = 1;
/// `SIGILL`: gecersiz islem.
pub const ILL_ILLOPN: i32 = 2;

// --- `SIGCHLD`in `si_code`lari ---
//
// Burada `si_code` bir yan bilgi degil **asil** bilgi: hangi olayin
// oldugunu yalnizca o soyluyor, ve `si_status`un anlami da ona bagli.

/// Cocuk kendi cikti; `si_status` cikis kodu.
pub const CLD_EXITED: i32 = 1;
/// Cocuk bir sinyalle olduruldu; `si_status` o sinyal.
pub const CLD_KILLED: i32 = 2;
/// Olum bir cekirdek dokumu birakti (TCMK dokum almiyor).
pub const CLD_DUMPED: i32 = 3;
/// Cocuk durdu; `si_status` durduran sinyal.
pub const CLD_STOPPED: i32 = 5;
/// Durmus cocuk devam etti.
pub const CLD_CONTINUED: i32 = 6;

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
    /// Birlesimin ikinci kelimesi.
    ///
    /// i386'da `si_uid` (0x10), x86_64'te `si_value` (0x18). Ayni
    /// kelimenin iki mimaride iki ayri alan olmasi, birlesimin
    /// hizalanmasinin bir sonucu: x86_64'te `si_pid`+`si_uid` (4+4)
    /// **tek** bir 8-baytlik kelimeye oturuyor, i386'da iki ayri
    /// kelimeye.
    second: usize,
    /// i386'da `si_value` (0x14); x86_64'te birlesimin bir sonraki
    /// kelimesi ve okunmuyor.
    third: usize,
}

impl SigInfo {
    /// Hataya yol acan adres (`SIGSEGV`, `SIGBUS`, `SIGFPE`, `SIGILL`).
    pub fn addr(&self) -> usize {
        self.field
    }

    /// Gonderenin kimligi (`si_code == SI_USER` ya da `SI_QUEUE`).
    pub fn pid(&self) -> usize {
        self.field & 0xFFFF_FFFF
    }

    /// `SIGCHLD`in tasidigi durum: cikis kodu, olduren ya da durduran
    /// sinyal -- hangisi oldugunu `si_code` soyluyor.
    ///
    /// `si_value` ile **ayni alani** okuyor ve bu bir kisayol degil:
    /// gercek `siginfo_t`de ikisi ayni birlesimin ucuncu yuvasidir.
    /// Ayri bir alan tanimlamak, olmayan bir ayrimi varmis gibi
    /// gostermek olurdu.
    pub fn status(&self) -> u32 {
        self.value() as u32
    }

    /// `sigqueue`in tasidigi deger -- yalnizca `si_code == SI_QUEUE`de.
    ///
    /// Ofset mimariye gore ayri kelimelere dusuyor: i386'da birlesimin
    /// uc alani (pid, uid, value) 4+4+4 bayt oldugu icin `value`
    /// **ucuncu** kelimedir; x86_64'te pid ile uid tek bir 8-baytlik
    /// kelimeye oturdugu icin `value` **ikinci** kelimeye kayar.
    pub fn value(&self) -> usize {
        #[cfg(target_arch = "x86")]
        {
            // { pid, uid, value } = 4+4+4 bayt: value ucuncu kelime.
            self.third
        }
        #[cfg(target_arch = "x86_64")]
        {
            // Birlesim 8'e hizali basliyor ve pid+uid tek kelimede;
            // value o yuzden ikinci kelime.
            self.second
        }
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
        flags: flags as usize,
        mask: SigSet::EMPTY,
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
        mask: SigSet::EMPTY,
    };
    sigaction_raw(signo, &act, core::ptr::null_mut())
}

/// Varsayilan davranisa dondurur.
pub fn default(signo: u32) -> isize {
    let act = SigAction {
        handler: SIG_DFL,
        restorer: 0,
        flags: 0,
        mask: SigSet::EMPTY,
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
///
/// `u64` doner: gercek-zamanli sinyaller 32..=63 arasinda ve `u32`ye
/// sigmiyorlar. Maskenin genislemesinin en gorunur izi bu imza.
pub const fn mask_of(signo: u32) -> u64 {
    1u64 << signo
}

/// POSIX `sigprocmask`: engel maskesini degistirir, **eskisini** doner.
///
/// Bloke bir sinyal kaybolmaz -- bekler ve maske acilinca teslim edilir.
/// Kritik bolge kalibi budur: maskele, isi yap, maskeyi ac.
///
/// Cekirdege iki `sigset_t` **isaretcisi** gidiyor, tipki gercek
/// `rt_sigprocmask` gibi. Maske uzun sure deger olarak geciyordu ve o
/// zaman icin dogruydu -- 32 sinyal tek bir registera siginiyordu.
/// Gercek-zamanli sinyaller maskeyi 64 bite cikarinca i386'da tek
/// registera sigmadi ve eski maskenin donus degeriyle verilmesi de
/// imkansizlasti.
pub fn sigprocmask(how: usize, set: u64) -> u64 {
    let new = SigSet::from_bits(set);
    let mut old = SigSet::EMPTY;
    unsafe {
        sys::syscall3(
            sys::SYS_SIGPROCMASK,
            how,
            &new as *const SigSet as usize,
            &mut old as *mut SigSet as usize,
        );
    }
    old.bits()
}

/// Mevcut engel maskesini okur (hicbir seyi degistirmeden).
///
/// `set` NULL geciliyor: "hicbir sey ekleme" ile "hicbir sey degistirme"
/// ayni sonucu verse de ikincisi cagrinin **niyetini** soyluyor.
pub fn current_mask() -> u64 {
    let mut old = SigSet::EMPTY;
    unsafe {
        sys::syscall3(
            sys::SYS_SIGPROCMASK,
            SIG_BLOCK,
            0,
            &mut old as *mut SigSet as usize,
        );
    }
    old.bits()
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
pub fn sigsuspend(mask: u64) -> isize {
    crate::sys::sigsuspend(&SigSet::from_bits(mask))
}

/// POSIX `sigqueue`: sinyali **bir degerle** gonderir.
///
/// `kill`den farki bir arguman degil, bir sozlesme:
///
/// ```text
///   kill(pid, SIGRTMIN) x3      ->  3 kopya kuyrukta, 3 teslim
///   kill(pid, SIGUSR1)  x3      ->  1 bit, 1 teslim
///   sigqueue(pid, SIGRTMIN, v)  ->  kopya + DEGER
/// ```
///
/// Kuyruk sinirli: dolduysa `-EAGAIN` doner. Bu bir ariza degil,
/// POSIX'in yazili sozlesmesi -- kuyruk cekirdek bellegi oldugu icin
/// gonderen taraf sinirsiz yer isteyemez.
///
/// Deger, isleyicide `SigInfo::value()` ile okunur ve isleyicinin
/// `SA_SIGINFO` ile kurulmus olmasi gerekir: tek argumanli yuz yalnizca
/// sinyal numarasini gorur, yani degeri **hic** gormez.
pub fn sigqueue(pid: usize, signo: u32, value: usize) -> isize {
    unsafe { sys::syscall3(sys::SYS_SIGQUEUE, pid, signo as usize, value) as isize }
}

// --- Sinyali beklemek: ucuncu yuz --------------------------------------

/// `sigtimedwait` icin **suresiz** bekleme.
///
/// Gercek Linux ucuncu argumani `struct timespec*` alir ve `NULL`
/// suresiz demektir; TCMK'nin cozunurlugu 10 ms oldugu icin sure
/// dogrudan milisaniye geciyor ve bu deger `NULL`un yerini tutuyor.
pub const WAIT_FOREVER: usize = usize::MAX;

/// `sigtimedwait`in yazdigi kaydin tam olcusu.
///
/// Gercek `siginfo_t` 128 bayttir ve cekirdek tamponun tamamini
/// sifirlayip yaziyor. [`SigInfo`] yalnizca **okunan** on kismi
/// tanimliyor, o yuzden tampon ayrica ayrilmali.
pub const SIGINFO_SIZE: usize = 128;

/// `siginfo_t` icin hizali bir tampon.
///
/// Hizalama bilincli: kaydin icinde 8 baytlik alanlar var (x86_64'te
/// `si_value`), ve hizasiz bir tampon onlari sayfa siniri yakininda
/// boler.
#[repr(C, align(16))]
pub struct SigInfoBuf([u8; SIGINFO_SIZE]);

impl SigInfoBuf {
    pub const fn new() -> Self {
        SigInfoBuf([0; SIGINFO_SIZE])
    }

    /// Kaydin okunabilir bolumu.
    ///
    /// # Safety
    /// Yalnizca basarili bir `sigtimedwait`ten sonra anlamlidir;
    /// oncesinde alanlar sifirdir.
    pub fn info(&self) -> &SigInfo {
        // SAFETY: `SigInfo` tamponun on ekidir ve tampon 16'ya hizali.
        unsafe { &*(self.0.as_ptr() as *const SigInfo) }
    }
}

impl Default for SigInfoBuf {
    fn default() -> Self {
        SigInfoBuf::new()
    }
}

/// POSIX `sigtimedwait`: kumedeki bir sinyali **senkron** alir.
///
/// ## Sinyalin ucuncu yuzu
///
/// Bir sinyalin bir surece yapabilecegi iki sey vardi: varsayilan
/// davranis (cogunlukla olum) ya da bir **isleyici** -- yani akisi
/// kesen bir cagri. Bu cagri ucuncusu: sinyali bir **mesaj gibi
/// okumak**.
///
/// ```text
///   sigaction + teslim  ->  cekirdek CAGIRIR, program bolunur
///   sigtimedwait        ->  program OKUR, hicbir sey bolunmez
/// ```
///
/// ## Sinyal once ENGELLENMELI
///
/// Engellenmezse isleyiciye (ya da varsayilan davranisa) gider ve buraya
/// hic ulasmaz. Kalip su:
///
/// ```ignore
/// signal::sigprocmask(signal::SIG_BLOCK, signal::mask_of(SIGRTMIN));
/// loop {
///     let mut buf = signal::SigInfoBuf::new();
///     let signo = signal::sigtimedwait(
///         signal::mask_of(SIGRTMIN), Some(&mut buf), signal::WAIT_FOREVER);
///     // ... isleyici degil, siradan kod
/// }
/// ```
///
/// Kazanc buyuk: sinyal isleyicisinin butun yeniden-girilebilirlik
/// sinirlari ortadan kalkar. Isleyicide `malloc` cagirmak yasaktir,
/// burada serbesttir -- cunku burasi bir isleyici degil, siradan bir
/// dongu.
///
/// Windows'un APC sozlesmesi de aynidir: teslim ani programin secimi.
/// Fark, Windows'ta bunun **varsayilan** olmasi; POSIX'te program
/// varsayilanin disina cikmak icin ozel olarak calisiyor.
///
/// Doner: sinyal numarasi, ya da negatif hata --
/// `-EAGAIN` sure doldu, `-EINTR` kume disi bir sinyal bekleme bolundu.
pub fn sigtimedwait(set: u64, info: Option<&mut SigInfoBuf>, timeout_ms: usize) -> isize {
    let mask = SigSet::from_bits(set);
    let info_ptr = info.map_or(core::ptr::null_mut(), |b| b as *mut SigInfoBuf);
    unsafe {
        sys::syscall3(
            sys::SYS_SIGTIMEDWAIT,
            &mask as *const SigSet as usize,
            info_ptr as usize,
            timeout_ms,
        ) as isize
    }
}

/// `sigtimedwait`in suresiz bicimi (POSIX `sigwaitinfo`).
pub fn sigwaitinfo(set: u64, info: Option<&mut SigInfoBuf>) -> isize {
    sigtimedwait(set, info, WAIT_FOREVER)
}

/// POSIX `alarm`: `seconds` sonra kendine `SIGALRM` gonderir.
///
/// Onceki alarmdan kalan saniyeyi doner; `0` alarmi iptal eder.
pub fn alarm(seconds: u32) -> u32 {
    unsafe { sys::syscall1(sys::SYS_ALARM, seconds as usize) as u32 }
}
