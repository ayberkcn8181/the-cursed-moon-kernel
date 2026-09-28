//! POSIX sinyalleri (doc S.7 Faz 8) -- `kill`, `signal`, `sigreturn`.
//!
//! ## Sinyal nedir, cekirdek acisindan
//!
//! Bir surece "su an ne yapiyorsan birak, once sunu calistir" demenin
//! yoludur. Cekirdek surecin **Ring 3 baglamini** kenara koyar, yigininin
//! ustune sahte bir cagri cercevesi kurar ve donusu isleyicinin adresine
//! cevirir. Surec, hicbir sey cagirmamis olmasina ragmen kendini
//! isleyicinin icinde bulur; isleyici `ret` ettiginde kucuk bir tramplen
//! `sigreturn` cagirir ve cekirdek saklanan baglami geri koyar. Surec
//! kaldigi yerden, **hicbir seyi fark etmemis gibi** devam eder.
//!
//! Bu, diger butun syscall'larin tersidir: orada kullanici cagirir,
//! cekirdek doner. Burada cekirdek cagirir, kullanici doner.
//!
//! ## Teslim ne zaman olur
//!
//! Sinyal aninda calismaz; **beklemeye alinir** (`PENDING` bit maskesi) ve
//! surec bir dahaki sefer cekirdekten Ring 3'e donerken teslim edilir
//! (`level0b2::dispatcher`). Yani teslim noktasi bir syscall donusudur.
//!
//! Bunun pratikteki anlami: hicbir syscall yapmayan, saf hesap yapan bir
//! dongu sinyali gormez. TCMK uygulamalari her karede `win_flush`
//! cagirdigi icin gecikme bir kareden kucuktur; `spin` gibi bilerek
//! kilitlenen bir program ise ancak `SIGKILL` ile durur -- ki o zaten
//! surecin isbirligini gerektirmez (asagi bkz.).
//!
//! ## `SIGKILL` neden farkli
//!
//! Yakalanamaz, yok sayilamaz ve **beklemeye alinmaz**: gonderen taraf
//! hedefi dogrudan sonlandirir. Beklemeye alinsaydi, hicbir syscall
//! yapmayan bir surec oldurulemezdi -- yani "her seyi durdurabilen komut"
//! olma ozelligi kaybolurdu.
//!
//! ## Iki sinif sinyal: biri birlesir, oteki kuyruga girer
//!
//! POSIX'in en az bilinen ayrimi burada. **Standart** sinyaller (1..=31)
//! bir **bit maskesinde** bekler, yani bir sinyal iki kez gonderilirse
//! bir kez teslim edilir -- ikinci gonderim birinciyle *birlesir*.
//! **Gercek-zamanli** sinyaller (`SIGRTMIN..=SIGRTMAX`) bir **kuyrukta**
//! bekler: N kez gonderilen sinyal N kez teslim edilir ve her kopya
//! kendi `si_value`siyla gelir.
//!
//! ```text
//!   kill(pid, SIGUSR1) x3   ->  isleyici 1 kez kosar   (bit birlesti)
//!   sigqueue(pid, SIGRTMIN) x3 -> isleyici 3 kez kosar (kuyruk)
//! ```
//!
//! Ayrim uydurma degil, bir **kaynak** karari: bit maskesi sabit yer
//! tutar ve hicbir zaman dolmaz; kuyruk ise sinirli ve dolabilir --
//! doldugunda `sigqueue` `EAGAIN` doner. Standart sinyaller isletim
//! sisteminin *bildirim* araci (bir sey oldu), gercek-zamanlilar ise
//! *mesaj* araci (su oldu, su degerle) oldugu icin ikisinin maliyeti de
//! farkli olmak zorunda.
//!
//! Windows tarafinda en yakin karsilik **APC kuyruklari**dir
//! (`QueueUserAPC`): onlar *her zaman* kuyruklu ve *her zaman* deger
//! tasir, yani Windows'ta bu iki sinifin ayrimi hic yoktur.
//!
//! ## Bilerek yapilmayanlar
//!
//! * **`sigwaitinfo`/`sigtimedwait` yok.** Kuyruktan **senkron** okuma
//!   yuzu; teslim yalnizca isleyici uzerinden oluyor.
//! * **`si_uid` her zaman 0.** TCMK'de kullanici kimligi yok.

use crate::arch::cpu::regs::{SyscallFrame, UserContext};
use crate::arch::cpu::usermode;
use crate::level0a::core::{mmu, scheduler};
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

pub const SIG_DFL: usize = 0;
pub const SIG_IGN: usize = 1;

pub const SIGHUP: u32 = 1;
pub const SIGINT: u32 = 2;
pub const SIGQUIT: u32 = 3;
pub const SIGILL: u32 = 4;
pub const SIGABRT: u32 = 6;
pub const SIGFPE: u32 = 8;
pub const SIGKILL: u32 = 9;
pub const SIGUSR1: u32 = 10;
pub const SIGSEGV: u32 = 11;
pub const SIGUSR2: u32 = 12;
/// Okuyan ucu kapali bir boruya yazmak.
///
/// POSIX'in en sert varsayilani: yakalanmazsa **surec oler**. Sebebi
/// kabuk boru hatlari -- `uretici | head` kaliginda `head` ilk on satiri
/// alip cikinca, uretici yazmaya devam etse sonsuza kadar kosardi.
/// Sinyal onu sessizce durduruyor.
///
/// Windows'ta bunun karsiligi **yok**: `WriteFile` yalnizca
/// `ERROR_BROKEN_PIPE` doner ve surec yasamaya devam eder. Ayni olay,
/// birinde olum, otekinde bir hata kodu.
pub const SIGPIPE: u32 = 13;
pub const SIGALRM: u32 = 14;
pub const SIGTERM: u32 = 15;

// --- Is denetimi (job control) sinyalleri ---
//
// Numaralar Linux'un i386 ve x86_64'teki degerleriyle ayni; ikisinde de
// farklilasmiyorlar (farklilasan seyler `SIGCHLD` oncesi eski
// numaralandirmadan kalma).

/// Durmus bir sureci **devam ettirir**.
///
/// Iki ozelligi var ve ikisi de diger sinyallerden ayri:
///
/// * Durmus bir surec sinyal **teslim alamaz** (kosmuyor), o yuzden
///   `SIGCONT` kuyruga girmeden once hedefi kaldirir.
/// * Varsayilani "oldur" degil "devam et", yani yakalanmadiginda da
///   zararsiz.
pub const SIGCONT: u32 = 18;

/// Sureci **durdurur** ve yakalanamaz.
///
/// `SIGKILL` ile ayni ayricalikta: isleyici kurulamaz, yok sayilamaz,
/// maskelenemez. Gerekce de ayni -- bir sureci durdurabilmek isletim
/// sisteminin son sozu olmali. Yakalanabilseydi kacan bir surec
/// durdurulamazdi.
pub const SIGSTOP: u32 = 19;

/// Terminalden gelen durdurma istegi (Ctrl-Z).
///
/// `SIGSTOP`tan farki tek cumlede: **yakalanabilir**. Bir duzenleyici
/// Ctrl-Z'yi yakalayip once dosyasini kaydedebilsin diye.
pub const SIGTSTP: u32 = 20;

/// **Standart** sinyallerin en buyugu. Bu sinira kadar olanlar bit
/// maskesinde birlesir (bkz. modul basi).
pub const MAX_STANDARD: u32 = 31;

/// Ilk gercek-zamanli sinyal.
///
/// Linux'ta 32 ve 33 libc'nin kendi isi (is parcaciklari) oldugu icin
/// uygulamalara 34'ten baslar; TCMK'de libc yok, o yuzden sinir gercek
/// sinirin kendisi.
pub const SIGRTMIN: u32 = 32;
/// Son gercek-zamanli sinyal.
pub const SIGRTMAX: u32 = 63;

/// Desteklenen en buyuk sinyal numarasi.
///
/// 31 idi ve maske `u32`du: gercek-zamanli sinyaller icin **yer yoktu**.
/// Simdi 63 ve maske `u64`. Buyume sadece bir sayi degisikligi degil,
/// bir tasima degisikligi de getirdi: i386'da 64 bitlik maske tek bir
/// registera sigmiyor, o yuzden `sigprocmask`/`sigsuspend` artik
/// gercek Linux gibi **isaretciyle** calisiyor (bkz. `posix_syscalls`).
pub const MAX_SIGNAL: u32 = SIGRTMAX;

/// Sinyal gercek-zamanli mi -- yani kuyruga mi girer?
pub fn rt(signo: u32) -> bool {
    signo >= SIGRTMIN && signo <= SIGRTMAX
}

/// Bir sinyalin bir surecteki karsiligi.
#[derive(Clone, Copy)]
struct Disposition {
    /// `SIG_DFL`, `SIG_IGN` ya da Ring 3'teki isleyicinin adresi.
    handler: usize,
    /// Isleyici dondugunde ziplanacak tramplen (kullanici tarafi verir).
    restorer: usize,
    /// `SA_*` bayraklari (bkz. `SA_NODEFER`, `SA_RESETHAND`).
    flags: u32,
    /// Isleyici kosarken **ek olarak** engellenecek sinyaller
    /// (`sigaction`in `sa_mask` alani).
    mask: u64,
}

impl Disposition {
    const DEFAULT: Self = Disposition {
        handler: SIG_DFL,
        restorer: 0,
        flags: 0,
        mask: 0,
    };
}

// --- `sigaction` bayraklari (Linux ile ayni sayilar) ------------------

/// Isleyici kosarken **kendi sinyali engellenmez**.
///
/// Varsayilan POSIX davranisi tersidir: teslim edilen sinyal, isleyicisi
/// kosarken otomatik engellenir. Bu bayrak o korumayi kaldirir, yani
/// sinyal kendi isleyicisinin icinde yeniden teslim edilebilir.
pub const SA_NODEFER: u32 = 0x4000_0000;

/// Teslimden **once** yerlestirme `SIG_DFL`e doner (tek atimlik isleyici).
///
/// Eski `signal(2)` semantiginin ta kendisi; `sigaction` onu bayrak
/// haline getirdi.
pub const SA_RESETHAND: u32 = 0x8000_0000;

/// Bolunen bir sistem cagrisi, isleyici dondukten sonra **yeniden
/// calistirilir**.
///
/// POSIX'in en ince ayrintilarindan biri. Bloke eden bir cagri (bos
/// borudan `read` gibi) sirasinda sinyal gelirse iki secenek var:
///
/// ```text
///   SA_RESTART yok -> cagri -EINTR ile doner, program kendisi yeniden dener
///   SA_RESTART var -> cekirdek cagriyi kendisi yeniden baslatir
/// ```
///
/// Ikincisi `EINTR`i **gorunmez** kilar ve cogu program bunu ister --
/// `EINTR` denetlemeyi unutan kod, sinyal geldiginde sessizce bozulur.
/// Eski `signal(2)` yuzu bu yuzden bayragi kendiliginden koyar; ham
/// `sigaction` koymaz.
///
/// Uygulamasi: cagri bolununce cerceve **geri sariliyor** -- komut
/// isaretcisi iki bayt geri aliniyor (`int 0x80` da `syscall` da iki
/// bayt) ve cagri numarasi geri yaziliyor. Sonra `deliver_pending` o
/// cerceveyi kaydedip isleyiciye atliyor; `sigreturn` onu geri
/// yukleyince cagri kendiliginden yeniden calisiyor. Gercek Linux'un
/// `ERESTARTSYS` mekanizmasi da budur.
pub const SA_RESTART: u32 = 0x1000_0000;

/// Isleyici **uc** arguman alir: `(signo, siginfo_t*, ucontext_t*)`.
///
/// Tek argumanli yuz yalnizca "hangi sinyal" der. Uc argumanli yuz iki
/// soruya daha cevap veriyor:
///
/// ```text
///   siginfo_t  NEDEN geldi  -- si_code, si_addr, si_pid
///   ucontext_t NEREDE kesildi -- butun registerlar
/// ```
///
/// Ikincisi bir okuma yuzeyi degil: `ucontext_t` **yazilabilir**.
/// Isleyici bir registeri duzeltip donerse cekirdek duzeltilmis baglami
/// geri yukler ve hatali komut tekrarlanir. Windows'ta bunun adi
/// `EXCEPTION_CONTINUE_EXECUTION`; POSIX'te ayri bir adi yok, cunku
/// mekanizma zaten donusun kendisidir (bkz. `sigreturn`).
pub const SA_SIGINFO: u32 = 0x0000_0004;

/// Isleyici **ayri** bir yiginda kossun (`sigaltstack` ile kurulan).
///
/// Tek bir sey icin var ve o sey onemli: **yigin tasmasini yakalamak**.
/// Tasma aninda yigin isaretcisi artik gecerli bir yeri gostermiyor;
/// sinyal cercevesi oraya kurulamaz, yani sinyal teslim edilemez ve
/// surec tanisiz oler. Ayri yigin bu dongunun disina cikmanin tek yolu.
///
/// ```text
///   bayrak yok  ->  cerceve kesilen yiginin ustune  (tasmada YAZILAMAZ)
///   bayrak var  ->  cerceve AYRI yigina             (tasmada yazilabilir)
/// ```
pub const SA_ONSTACK: u32 = 0x0800_0000;

/// Cekirdegin tanidigi bayraklar. Digerleri sessizce yok sayilir --
/// `SA_RESTART` gibi, karsiligi olmayan bir bayragi kabul ediyormus gibi
/// yapmak yaniltici olurdu (bkz. README).
pub const SUPPORTED_FLAGS: u32 =
    SA_NODEFER | SA_RESETHAND | SA_RESTART | SA_SIGINFO | SA_ONSTACK;

// --- `sigaltstack` ---------------------------------------------------

/// `ss_flags`: su an o yiginin **ustunde** kosuluyor.
pub const SS_ONSTACK: u32 = 1;
/// `ss_flags`: ayri yigini kaldir.
pub const SS_DISABLE: u32 = 2;

/// Ayri yigin icin kabul edilen en kucuk olcu.
///
/// Linux'un `MINSIGSTKSZ`i 2 KiB'dir ve TCMK'nin cercevesi de oraya
/// siginiyor: `siginfo_t` (128) + `ucontext_t` (348/968) + cagri
/// cercevesi. Daha kucugunu kabul etmek, isleyiciye girer girmez
/// tasacak bir yigin vermek olurdu -- yani sorunu cozmek yerine
/// tasimak.
pub const MINSIGSTKSZ: usize = 2048;

/// Ayri yiginin tabani ve olcusu; taban 0 = kurulu degil.
static ALT_SP: [core::sync::atomic::AtomicUsize; scheduler::MAX_TASKS] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; scheduler::MAX_TASKS];
static ALT_SIZE: [core::sync::atomic::AtomicUsize; scheduler::MAX_TASKS] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Kac isleyici su an ayri yiginin ustunde kosuyor.
///
/// Sayac, cunku ic ice teslim mumkun: ayri yiginda kosan bir isleyicinin
/// icinde ikinci bir sinyal teslim edilirse o da **ayni** yigini
/// kullanmali -- yeniden tepeye donmek, alttaki cerceveyi ezmek olurdu.
static ALT_DEPTH: [core::sync::atomic::AtomicUsize; scheduler::MAX_TASKS] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Hangi katman ayri yigina **gecti** -- `sigreturn` geri sayarken bakar.
static USED_ALT: [[core::sync::atomic::AtomicUsize; NEST_DEPTH]; scheduler::MAX_TASKS] =
    [const { [const { core::sync::atomic::AtomicUsize::new(0) }; NEST_DEPTH] };
        scheduler::MAX_TASKS];

// --- `si_code`: sinyalin **kaynagi** (Linux ile ayni sayilar) ---------
//
// Sinyal numarasi ne oldugunu, `si_code` nereden geldigini soyler. Ayrim
// gercek programlarda onemli: `SIGSEGV`i yakalayan bir kod, hatanin
// eslenmemis bir sayfadan mi yoksa izin ihlalinden mi geldigine gore
// farkli davranir (birincisi tembel bir ayirici icin normal, ikincisi
// degil).

/// Bir surec `kill` ile gonderdi. `si_pid` gonderenin kimligidir.
pub const SI_USER: i32 = 0;
/// Cekirdek gonderdi (ornegin `SIGPIPE`).
pub const SI_KERNEL: i32 = 0x80;
/// Bir surec `sigqueue` ile gonderdi -- **degeriyle birlikte**.
///
/// Negatif olmasi bir kaza degil: POSIX, `si_code`in isaretini bir ayrim
/// olarak kullanir. Sifir ve pozitif kodlari **cekirdek** uretir (bir
/// hata olustu, bir zamanlayici doldu), negatifleri ise bir **surec**.
/// `SI_USER`in 0 olmasi bu kuraldan once kaldigi icin istisnadir.
pub const SI_QUEUE: i32 = -1;
/// `SIGSEGV`: adres **eslenmemis**.
pub const SEGV_MAPERR: i32 = 1;
/// `SIGSEGV`: adres eslenmis ama erisim izni yok.
pub const SEGV_ACCERR: i32 = 2;
/// `SIGFPE`: tam sayi sifira bolme.
pub const FPE_INTDIV: i32 = 1;
/// `SIGILL`: gecersiz islem.
pub const ILL_ILLOPN: i32 = 2;

/// Bir sinyalin **neden** geldigi.
///
/// `sigaction`in `SA_SIGINFO` yuzunde Ring 3'e `siginfo_t` olarak
/// gidiyor. Cekirdek icinde ayri bir tip olmasi kasitli: `siginfo_t`nin
/// ikili duzeni mimariye gore degisiyor (bkz. `usermode.rs`), oysa
/// tasidigi anlam degismiyor.
#[derive(Clone, Copy)]
pub struct SigInfo {
    /// `si_code` -- sinyalin kaynagi.
    pub code: i32,
    /// `si_addr` -- `SIGSEGV`/`SIGBUS`/`SIGFPE`/`SIGILL`'de hataya yol
    /// acan adres. Digerlerinde sifir.
    pub addr: usize,
    /// `si_pid` -- `kill` ile gelenlerde **gonderenin** kimligi.
    pub pid: usize,
    /// `si_value` -- yalnizca `sigqueue` ile gelenlerde (`SI_QUEUE`).
    ///
    /// Sinyalin **yuku**: gonderen tarafin isleyiciye ilettigi tek
    /// kelime. Standart sinyallerde boyle bir alan yok, cunku standart
    /// sinyaller birlesiyor -- iki gonderim tek teslime dusunce hangi
    /// degerin tasinacagi cevapsiz kalirdi. Kuyruk, degeri anlamli
    /// kilan seydir.
    pub value: usize,
}

impl SigInfo {
    /// Bilgi tasimayan kayit: `si_code` disinda her sey sifir.
    pub const fn from_kernel() -> Self {
        SigInfo {
            code: SI_KERNEL,
            addr: 0,
            pid: 0,
            value: 0,
        }
    }
}

/// Yerlestirme tablosunun bir gorevdeki genisligi (0 dahil, 63 dahil).
const SLOTS: usize = MAX_SIGNAL as usize + 1;

/// Surec basina yerlestirmeler. Gorev kimligiyle indekslenir; `fork`
/// bunlari kopyalar (`clone_into`), `execve`/cikis sifirlar (`reset`).
static mut DISPOSITIONS: [[Disposition; SLOTS]; scheduler::MAX_TASKS] =
    [[Disposition::DEFAULT; SLOTS]; scheduler::MAX_TASKS];

/// Bekleyen sinyaller: bit N = sinyal N.
///
/// Atomik, cunku gonderen baska bir gorevdir; okuma-degistirme-yazma
/// arasinda zamanlayici araya girerse sinyal kaybolurdu.
///
/// Gercek-zamanli sinyaller icin bu bit **"kuyrukta en az bir kopya
/// var"** demektir; kac kopya oldugunu `QUEUE` bilir. Standart
/// sinyallerde bitin kendisi butun hikayedir -- birlesmenin somut
/// karsiligi da budur.
#[allow(clippy::declare_interior_mutable_const)]
const ZERO_MASK: AtomicU64 = AtomicU64::new(0);
static PENDING: [AtomicU64; scheduler::MAX_TASKS] = [ZERO_MASK; scheduler::MAX_TASKS];

/// Ic ice gecebilecek en fazla isleyici sayisi.
///
/// Onceden **tek** katman vardi ve bu, "isleyici icindeyken hicbir sinyal
/// teslim edilmez" kuralini zorunlu kiliyordu: ikinci bir teslim tek
/// yuvayi ezer ve surec asla eski baglamina donemezdi. Yani bir SIGALRM,
/// suren bir SIGUSR1 isleyicisi yuzunden bekliyordu -- POSIX'te ise
/// **yalnizca ayni sinyal** engellenir, farkli sinyaller ic ice teslim
/// edilir.
///
/// Dort katman bilincli bir sinir: her katman bir `UserContext` +
/// bir maske tutuyor, ve gercek programlarda ikiden derin ic ice sinyal
/// zaten patolojiktir. Sinira dayanildiginda teslim **ertelenir**
/// (sinyal `PENDING`de kalir), kaybolmaz.
const NEST_DEPTH: usize = 4;

/// Isleyiciye girilmeden onceki Ring 3 baglamlari -- **yigin**.
/// `sigreturn` en usttekini geri koyar.
static mut SAVED: [[UserContext; NEST_DEPTH]; scheduler::MAX_TASKS] =
    [[UserContext::ZERO; NEST_DEPTH]; scheduler::MAX_TASKS];

/// Her katmanda, isleyiciye girmeden **once** yururlukte olan maske.
///
/// `sigreturn` bunu geri koyar: POSIX'te isleyicinin ek engelleri
/// (`sa_mask` + sinyalin kendisi) yalnizca isleyici suresince gecerlidir.
static mut SAVED_MASK: [[u64; NEST_DEPTH]; scheduler::MAX_TASKS] =
    [[0; NEST_DEPTH]; scheduler::MAX_TASKS];

/// Her katmanda Ring 3'teki `ucontext_t`nin adresi; 0 = yok.
///
/// Yalnizca `SA_SIGINFO` isleyicileri icin dolu. `sigreturn` buraya
/// bakiyor: kayit varsa baglam **oradan** okunuyor, yani isleyicinin
/// yaptigi degisiklikler yurumus oluyor. Yoksa `SAVED`den, yani
/// isleyici hicbir sey degistirememis gibi.
static UCONTEXT_AT: [[core::sync::atomic::AtomicUsize; NEST_DEPTH]; scheduler::MAX_TASKS] =
    [const { [const { core::sync::atomic::AtomicUsize::new(0) }; NEST_DEPTH] };
        scheduler::MAX_TASKS];

/// Bekleyen **standart** sinyalin neden geldigi -- gorev basina, sinyal
/// basina tek yuva.
///
/// `PENDING` yalnizca bir bit tasiyor; bu tablo o bitin yanindaki
/// hikayedir. Sinyal basina **tek** yuva olmasi bir eksiklik degil,
/// birlesmenin dogal sonucu: ayni standart sinyal iki kez gonderilirse
/// bir kez teslim edilir, yani saklanacak tek bir neden vardir.
///
/// Tablo bu yuzden yalnizca 1..=31'i kapsiyor. Gercek-zamanli sinyaller
/// birlesmedigi icin nedenlerini burada degil `QUEUE`da tutuyor -- ve
/// kuyrukta her kopyanin kendi nedeni var.
const INFO_SLOTS: usize = MAX_STANDARD as usize + 1;
static mut INFO: [[SigInfo; INFO_SLOTS]; scheduler::MAX_TASKS] =
    [[SigInfo::from_kernel(); INFO_SLOTS]; scheduler::MAX_TASKS];

// --- Gercek-zamanli sinyal kuyrugu ------------------------------------

/// Bir gorevin kuyruguna sigan en fazla kopya.
///
/// Sinir **olmak zorunda**: kuyruk cekirdek bellegidir ve bir surec
/// baska bir surece sinirsiz sinyal gonderebilseydi, gonderen taraf
/// cekirdegi tuketirdi. POSIX bu yuzden `sigqueue`a bir hata kodu verir
/// (`EAGAIN`) -- yani "kuyruk dolu" bir arizanin degil, **sozlesmenin**
/// parcasi. Gercek Linux'un siniri surec basina `RLIMIT_SIGPENDING`dir;
/// TCMK'de sabit, cunku kaynak sinirlari (`rlimit`) henuz yok.
pub const SIGQUEUE_LEN: usize = 8;

/// Kuyruktaki bir kopya: hangi sinyal, hangi nedenle.
#[derive(Clone, Copy)]
struct Queued {
    signo: u32,
    info: SigInfo,
}

impl Queued {
    const EMPTY: Self = Queued {
        signo: 0,
        info: SigInfo::from_kernel(),
    };
}

/// Gorev basina kuyruk -- **varis sirasinda** dolu bir on ek.
///
/// Halka (ring) degil, sikistiran bir dizi. Sebep teslim kurali: teslim
/// sirasi once **sinyal numarasina** bakar (kucuk olan once), sonra ayni
/// numara icinde varis sirasina. Yani cikarilan oge her zaman basta
/// olmaz; halka olsaydi ortadan cikarma yine kaydirma gerektirecekti.
/// Sekiz ogeyle kaydirmanin maliyeti de olcume girmeyecek kadar kucuk.
static mut QUEUE: [[Queued; SIGQUEUE_LEN]; scheduler::MAX_TASKS] =
    [[Queued::EMPTY; SIGQUEUE_LEN]; scheduler::MAX_TASKS];

/// Kuyrukta duran kopya sayisi (gorev basina).
static QUEUED: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Kuyruk dolu oldugu icin **reddedilen** gonderim sayisi (olcum).
static QUEUE_DROPPED: AtomicU32 = AtomicU32::new(0);
/// Kuyrugun gordugu en buyuk derinlik (olcum).
static QUEUE_PEAK: AtomicU32 = AtomicU32::new(0);

/// Kuyruga bir kopya ekler; yer yoksa `false`.
///
/// Kesmeler kapali: `QUEUED` ile `QUEUE` birlikte tutarli olmak zorunda
/// ve ikisi tek bir atomik islemle guncellenemez.
fn queue_push(task: usize, signo: u32, info: SigInfo) -> bool {
    crate::arch::cpu::without_interrupts(|| {
        let used = QUEUED[task].load(Ordering::SeqCst);
        if used >= SIGQUEUE_LEN {
            return false;
        }
        // SAFETY: yuva gorev ve indekse ozel, sinir yukarida denetlendi.
        unsafe {
            (core::ptr::addr_of_mut!(QUEUE) as *mut Queued)
                .add(task * SIGQUEUE_LEN + used)
                .write(Queued { signo, info });
        }
        QUEUED[task].store(used + 1, Ordering::SeqCst);
        QUEUE_PEAK.fetch_max(used as u32 + 1, Ordering::Relaxed);
        true
    })
}

/// Kuyruktan `signo`nun **en eski** kopyasini cikarir.
fn queue_pop(task: usize, signo: u32) -> Option<SigInfo> {
    crate::arch::cpu::without_interrupts(|| {
        let used = QUEUED[task].load(Ordering::SeqCst);
        // SAFETY: yalnizca 0..used araligi okunuyor.
        unsafe {
            let base = core::ptr::addr_of_mut!(QUEUE) as *mut Queued;
            let row = base.add(task * SIGQUEUE_LEN);
            for i in 0..used {
                if row.add(i).read().signo != signo {
                    continue;
                }
                let found = row.add(i).read().info;
                // Kalanlar bir asagi kayiyor: varis sirasi korunmali.
                for j in i..used - 1 {
                    row.add(j).write(row.add(j + 1).read());
                }
                row.add(used - 1).write(Queued::EMPTY);
                QUEUED[task].store(used - 1, Ordering::SeqCst);
                return Some(found);
            }
        }
        None
    })
}

/// Kuyrukta bu sinyalden baska kopya kaldi mi?
fn queue_has(task: usize, signo: u32) -> bool {
    crate::arch::cpu::without_interrupts(|| {
        let used = QUEUED[task].load(Ordering::SeqCst);
        // SAFETY: yalnizca 0..used araligi okunuyor.
        unsafe {
            let row = (core::ptr::addr_of!(QUEUE) as *const Queued).add(task * SIGQUEUE_LEN);
            (0..used).any(|i| row.add(i).read().signo == signo)
        }
    })
}

/// Kuyrugu bosaltir (`fork` cocugu, `execve`, cikis).
fn queue_clear(task: usize) {
    crate::arch::cpu::without_interrupts(|| {
        QUEUED[task].store(0, Ordering::SeqCst);
        // SAFETY: butun satir kendi yuvasi.
        unsafe {
            let row = (core::ptr::addr_of_mut!(QUEUE) as *mut Queued).add(task * SIGQUEUE_LEN);
            for i in 0..SIGQUEUE_LEN {
                row.add(i).write(Queued::EMPTY);
            }
        }
    });
}

/// Gorevin kuyrugunda bekleyen kopya sayisi (kabuk raporu).
pub fn queued_count(task: usize) -> usize {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    QUEUED[task].load(Ordering::Relaxed)
}

/// `(en derin kuyruk, reddedilen gonderim)` -- kabuk raporu.
pub fn queue_stats() -> (u32, u32) {
    (
        QUEUE_PEAK.load(Ordering::Relaxed),
        QUEUE_DROPPED.load(Ordering::Relaxed),
    )
}

/// Kac isleyici ic ice suruyor (0 = normal akis).
static DEPTH: [core::sync::atomic::AtomicUsize; scheduler::MAX_TASKS] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// En derin ic ice teslim -- olcum icin.
static MAX_NESTED: AtomicU32 = AtomicU32::new(0);
/// Sinir dolu oldugu icin ertelenen teslim sayisi.
static NEST_DEFERRED: AtomicU32 = AtomicU32::new(0);

/// Surec basina **engellenen** sinyaller: bit N = sinyal N bloke.
///
/// Bloke bir sinyal kaybolmaz; `PENDING`'de bekler ve maske acildiginda
/// teslim edilir. POSIX'in "kritik bolge" araci budur -- uygulama
/// bolunmemesi gereken isi maskeyi kapatarak yapar.
static BLOCKED: [AtomicU64; scheduler::MAX_TASKS] = [ZERO_MASK; scheduler::MAX_TASKS];

/// `alarm` icin uyanma ani (PIT tik'i, mutlak). 0 = kurulu degil.
///
/// Maskelerle ayni sabiti paylasiyordu; maske 64 bite cikinca ayrildi --
/// bu bir maske degil, bir **tik sayisi**.
static ALARM_AT: [AtomicU32; scheduler::MAX_TASKS] =
    [const { AtomicU32::new(0) }; scheduler::MAX_TASKS];

/// Teslim edilen sinyal sayaci (kabuk `sigs` komutu icin).
static DELIVERED: AtomicU32 = AtomicU32::new(0);
static SENT: AtomicU32 = AtomicU32::new(0);
/// Bloke oldugu icin bekletilen teslim sayisi.
static BLOCKED_HITS: AtomicU32 = AtomicU32::new(0);

/// `sigprocmask` islemleri (Linux ile ayni sayilar).
pub const SIG_BLOCK: usize = 0;
pub const SIG_UNBLOCK: usize = 1;
pub const SIG_SETMASK: usize = 2;

/// Engellenemeyen sinyaller. POSIX `SIGKILL` ve `SIGSTOP`'u maskeye
/// almaz; alsaydi bir surec kendini oldurulemez ya da
/// durdurulamaz yapabilirdi.
///
/// Yorum uzun sure `SIGSTOP`tan soz ediyordu ama maskede yalnizca
/// `SIGKILL` vardi -- cunku `SIGSTOP` henuz yoktu. Artik ikisi de var.
const UNBLOCKABLE: u64 = (1 << SIGKILL) | (1 << SIGSTOP);

/// PIT tik'i basina saniye (100 Hz).
const TICKS_PER_SECOND: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalError {
    /// Sinyal numarasi 1..=63 disinda.
    InvalidSignal,
    /// Hedef gorev yok ya da zaten sonlanmis.
    NoSuchTask,
    /// `SIGKILL`/`SIGSTOP` yakalanamaz.
    Uncatchable,
    /// Gercek-zamanli sinyal kuyrugu dolu (`EAGAIN`).
    ///
    /// Yalnizca gercek-zamanli sinyallerde olabilir: standart sinyaller
    /// bir bite yazildigi icin **hicbir zaman** yer bulamamazlik
    /// etmiyor. Iki sinifin maliyet farki en somut haliyle burada.
    QueueFull,
}

fn valid(signo: u32) -> bool {
    signo >= 1 && signo <= MAX_SIGNAL
}

/// Sinyalin varsayilan davranisi surec sonlandirmak mi?
///
/// Isleyici kurulmamis bir sinyalin varsayilan davranisi.
///
/// Uzun sure bu tek bir satirdi -- "varsayilan her zaman oldur" -- ve o
/// zaman icin dogruydu: yok sayilan ya da durduran hicbir sinyal
/// yoktu. Is denetimi geldiginde varsayim cokuyor, cunku `SIGSTOP`
/// oldurmuyor ve `SIGCONT` hicbir sey yapmiyor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DefaultAction {
    /// Sureci sonlandirir (POSIX'te cogu sinyalin varsayilani).
    Terminate,
    /// Sureci durdurur.
    Stop,
    /// Durmussa devam ettirir; degilse hicbir sey yapmaz.
    Continue,
}

pub fn default_action(signo: u32) -> DefaultAction {
    match signo {
        SIGSTOP | SIGTSTP => DefaultAction::Stop,
        SIGCONT => DefaultAction::Continue,
        _ => DefaultAction::Terminate,
    }
}

/// Bu sinyal yakalanabilir/maskelenebilir mi?
///
/// Ikisi de "isletim sisteminin son sozu": biri sureci oldurur, oteki
/// durdurur, ve ikisi de surecin isbirligini beklemez.
pub fn uncatchable(signo: u32) -> bool {
    signo == SIGKILL || signo == SIGSTOP
}

/// Bir goreve sinyal gonderir.
///
/// `SIGKILL` beklemeye alinmaz: hedef **hemen** sonlandirilir. Digerleri
/// bekleyenler maskesine yazilir ve hedef Ring 3'e donerken teslim edilir.
/// Sinyali kuyruga koyar; bekleyen bir `pause`/`sigsuspend` varsa uyandirir.
pub fn raise(target: usize, signo: u32) -> Result<(), SignalError> {
    // `kill`in sozlesmesi: kaynak **bir surectir** ve kim oldugu
    // soylenir. `SA_SIGINFO` isleyicisi bunu `si_pid`de goruyor.
    raise_with(
        target,
        signo,
        SigInfo {
            code: SI_USER,
            addr: 0,
            pid: scheduler::current_id(),
            value: 0,
        },
    )
}

/// POSIX `sigqueue`: sinyali **bir degerle** gonderir.
///
/// `kill`den iki farki var ve ikisi birbirine bagli:
///
/// ```text
///   kill      -> yalnizca "hangi sinyal"; ayni sinyal birlesebilir
///   sigqueue  -> sinyal + bir kelime deger; kopyalar KUYRUKTA durur
/// ```
///
/// Deger ancak kuyruk varsa anlamli: birlesen bir sinyalde iki
/// gonderimden hangisinin degerinin tasinacagi cevapsiz kalirdi. Bu
/// yuzden `sigqueue` standart bir sinyale de uygulanabilir ama degeri
/// yalnizca gercek-zamanlilarda guvenilirdir -- POSIX'in kendi kurali da
/// budur.
pub fn sigqueue(target: usize, signo: u32, value: usize) -> Result<(), SignalError> {
    raise_with(
        target,
        signo,
        SigInfo {
            code: SI_QUEUE,
            addr: 0,
            pid: scheduler::current_id(),
            value,
        },
    )
}

/// Cekirdegin kendi gonderdigi sinyal (`SIGPIPE` gibi).
///
/// `raise`ten tek farki `si_code`: gonderen bir surec degil. Ayrimi
/// yapmamak, `SIGPIPE`i yakalayan bir isleyiciye "bunu sana 3 numarali
/// surec gonderdi" demek olurdu.
pub fn raise_kernel(target: usize, signo: u32) -> Result<(), SignalError> {
    raise_with(target, signo, SigInfo::from_kernel())
}

/// `raise`in govdesi: sinyali **nedeniyle birlikte** kuyruga koyar.
pub fn raise_with(target: usize, signo: u32, info: SigInfo) -> Result<(), SignalError> {
    if !valid(signo) {
        return Err(SignalError::InvalidSignal);
    }
    if target >= scheduler::MAX_TASKS {
        return Err(SignalError::NoSuchTask);
    }
    match scheduler::state_of(target) {
        scheduler::TaskState::Unused | scheduler::TaskState::Terminated => {
            return Err(SignalError::NoSuchTask)
        }
        _ => {}
    }

    // Gercek-zamanli sinyal **kuyruga** giriyor ve kuyruk dolabilir.
    //
    // Ekleme sayaclardan **once** yapiliyor: yer yoksa gonderim hic
    // olmamis sayilmali, yoksa `sigs` raporu teslim edilmeyecek bir
    // sinyali gonderilmis gosterirdi. Asagidaki uc ozel durum
    // (`SIGKILL`/`SIGSTOP`/`SIGCONT`) hepsi standart oldugu icin buraya
    // bir kayit birakmis olamaz.
    if rt(signo) && !queue_push(target, signo, info) {
        QUEUE_DROPPED.fetch_add(1, Ordering::Relaxed);
        return Err(SignalError::QueueFull);
    }

    SENT.fetch_add(1, Ordering::Relaxed);

    if signo == SIGKILL {
        // Yakalanamaz: surecin isbirligini beklemeden sonlandirilir.
        // Kendi kendine gonderilmisse cagri zincirinden cikmak gerekir,
        // bu yuzden ayri yol.
        if target == scheduler::current_id() {
            scheduler::set_current_exit_signal(SIGKILL);
            crate::level0a::kernel_api::exit_current_task(0);
        }
        scheduler::set_exit_signal(target, SIGKILL);
        return scheduler::terminate(target)
            .map_err(|_| SignalError::NoSuchTask);
    }

    if signo == SIGSTOP {
        // `SIGKILL` gibi yakalanamaz ve kuyruga girmez: hedef **hemen**
        // durur. Kuyruga girseydi hic kosmayan bir gorev (ornegin uzun
        // bir `waitpid` icinde) onu teslim alamaz ve asla durmazdi --
        // oysa durdurmak tam da kosmayan bir sureci hedef alabilmeli.
        scheduler::stop_task(target, SIGSTOP);
        return Ok(());
    }

    if signo == SIGCONT {
        // Durmus bir surec sinyal teslim alamaz, cunku kosmuyor.
        // O yuzden once kaldirilir; kuyruga girmesi yine de sart,
        // isleyici kurulmussa calismali.
        scheduler::continue_task(target);
    }

    // Standart sinyalin nedeni tek yuvaya yaziliyor; gercek-zamanlinin
    // nedeni zaten kuyruktaki kopyanin yaninda.
    //
    // Neden, bitten **once** yaziliyor: ters sirada olsaydi hedef
    // sinyali eski nedeniyle teslim alabilirdi.
    if !rt(signo) {
        // SAFETY: yuva gorev ve sinyal numarasina ozel.
        unsafe {
            (core::ptr::addr_of_mut!(INFO) as *mut SigInfo)
                .add(target * INFO_SLOTS + signo as usize)
                .write(info)
        };
    }
    PENDING[target].fetch_or(1u64 << signo, Ordering::SeqCst);

    // `pause`/`sigsuspend` ile uyuyan bir gorev varsa kaldirilir. Tek
    // hedeflidir: yalnizca sinyalin gittigi gorev uyanir. Bloke bir
    // sinyalse `deliverable` yine `false` doner ve gorev geri uyur --
    // sahte uyanma dongude ele aliniyor.
    scheduler::wake_signal_waiter(target);
    Ok(())
}

/// Bir **surec grubuna** sinyal gonderir; ulasilan gorev sayisini doner.
///
/// POSIX'te `kill(-pgid, sig)` budur ve kabugun Ctrl-C'si tam olarak
/// bunu yapar: bir boru hatti uc ayri surectir, ama tek bir istir.
///
/// Hedefler once bir kopyaya aliniyor: yayin sirasinda sinyal bir
/// gorevi oldurebilir ya da durdurabilir, yani tablo gezilirken
/// degisir. Kopya olmasaydi ayni sinyal bazi uyelere hic gitmeyebilirdi.
pub fn raise_group(pgid: usize, signo: u32) -> Result<usize, SignalError> {
    if !valid(signo) {
        return Err(SignalError::InvalidSignal);
    }
    let mut targets = [0usize; scheduler::MAX_TASKS];
    let found = scheduler::group_tasks(pgid, &mut targets);
    if found == 0 {
        return Err(SignalError::NoSuchTask);
    }
    let mut sent = 0usize;
    for &target in targets.iter().take(found) {
        if raise(target, signo).is_ok() {
            sent += 1;
        }
    }
    if sent == 0 {
        Err(SignalError::NoSuchTask)
    } else {
        Ok(sent)
    }
}

/// Bir sinyal icin isleyici kaydeder; onceki isleyiciyi doner.
///
/// `restorer`, kullanici tarafinin `sigreturn` cagiran kucuk tramplenidir.
/// Cekirdegin kullanici adres uzayina kod yazmasindan boylece kacinilir --
/// i386 Linux'un `sa_restorer` alaninin varlik sebebi de budur.
pub fn set_handler(
    task: usize,
    signo: u32,
    handler: usize,
    restorer: usize,
    flags: u32,
    mask: u64,
) -> Result<usize, SignalError> {
    if !valid(signo) {
        return Err(SignalError::InvalidSignal);
    }
    // `SIGKILL` ve `SIGSTOP` yakalanamaz. Ikisinin de gerekcesi ayni:
    // biri sureci oldurur, oteki durdurur, ve ikisi de surecin
    // isbirligini beklemez. Yakalanabilselerdi kacan bir surec ne
    // oldurulebilir ne durdurulabilirdi.
    if uncatchable(signo) {
        return Err(SignalError::Uncatchable);
    }
    if task >= scheduler::MAX_TASKS {
        return Err(SignalError::NoSuchTask);
    }
    // Gercek bir isleyici veriliyorsa hem kendisi hem tramplen kullanici
    // alaninda olmali; aksi halde cekirdek, surecin istegiyle kendi
    // kodunun icine dallanirdi.
    if handler != SIG_DFL && handler != SIG_IGN {
        if !mmu::is_user_accessible(handler) || !mmu::is_user_accessible(restorer) {
            return Err(SignalError::InvalidSignal);
        }
    }

    crate::arch::cpu::without_interrupts(|| unsafe {
        let slot = (core::ptr::addr_of_mut!(DISPOSITIONS) as *mut Disposition)
            .add(task * SLOTS + signo as usize);
        let old = slot.read().handler;
        slot.write(Disposition {
            handler,
            restorer,
            // Taninmayan bayraklar **atilir**. Kabul ediyormus gibi
            // saklamak, karsiligi olmayan bir bayragin calistigi
            // izlenimini verirdi.
            flags: flags & SUPPORTED_FLAGS,
            // SIGKILL hicbir yolla engellenemez; `sa_mask` de bir yol.
            mask: mask & !UNBLOCKABLE,
        });
        Ok(old)
    })
}

/// Bir sinyalin su anki isleyicisi (`sigaction`in `oldact` alani icin).
pub fn handler_of(task: usize, signo: u32) -> usize {
    if task >= scheduler::MAX_TASKS || !valid(signo) {
        return SIG_DFL;
    }
    unsafe {
        (core::ptr::addr_of!(DISPOSITIONS) as *const Disposition)
            .add(task * SLOTS + signo as usize)
            .read()
            .handler
    }
}

/// En derin ic ice teslim ve sinir yuzunden ertelenen teslim sayisi.
pub fn nesting_stats() -> (u32, u32) {
    (
        MAX_NESTED.load(Ordering::Relaxed),
        NEST_DEFERRED.load(Ordering::Relaxed),
    )
}

/// POSIX `sigprocmask`: engel maskesini okur/degistirir; **eski** maskeyi
/// doner.
///
/// ## Tasima farki
///
/// Gercek POSIX iki `sigset_t` **isaretcisi** alir. TCMK maskeyi
/// dogrudan deger olarak gecirir ve eskisini donus degeriyle verir:
/// 32 sinyal tek bir kelimeye sigdigi icin isaretci dogrulamak gereksiz
/// bir yol olurdu (ayni sadelestirme `pipe`'ta da yapildi).
pub fn sigprocmask(task: usize, how: usize, set: u64) -> Option<u64> {
    if task >= scheduler::MAX_TASKS {
        return None;
    }
    let old = BLOCKED[task].load(Ordering::SeqCst);
    let next = match how {
        SIG_BLOCK => old | set,
        SIG_UNBLOCK => old & !set,
        SIG_SETMASK => set,
        _ => return None,
    };
    // SIGKILL maskeye giremez: girebilseydi bir surec kendini
    // oldurulemez yapabilirdi.
    BLOCKED[task].store(next & !UNBLOCKABLE, Ordering::SeqCst);
    Some(old)
}

/// `sigsuspend` sirasinda saklanan **eski** maske.
static SUSPEND_SAVE: [AtomicU64; scheduler::MAX_TASKS] = [ZERO_MASK; scheduler::MAX_TASKS];
/// `SUSPEND_SAVE` gecerli mi -- yani geri yuklenmeyi bekleyen bir maske
/// var mi? Ayri bir bayrak sart: sifir da gecerli bir maskedir ("hicbir
/// sey bloke degil"), yani sentinel deger kullanilamaz.
static RESTORE_PENDING: [core::sync::atomic::AtomicBool; scheduler::MAX_TASKS] =
    [const { core::sync::atomic::AtomicBool::new(false) }; scheduler::MAX_TASKS];

/// Kac kez sinyal beklendi (kabuk `sigs` raporu icin).
static SUSPENDS: AtomicU32 = AtomicU32::new(0);

/// Teslim edilebilir bir sinyal bekliyor mu?
///
/// "Bekleyen" yetmez, **bloke olmayan** bekleyen gerekir: maskelenmis bir
/// sinyal `pause`i uyandirmaz, cunku teslim de edilmez.
pub fn deliverable(task: usize) -> bool {
    if task >= scheduler::MAX_TASKS {
        return false;
    }
    PENDING[task].load(Ordering::SeqCst) & !BLOCKED[task].load(Ordering::SeqCst) != 0
}

/// Bekleyen sinyal, bloke eden bir cagriyi **bolmeli** mi?
///
/// `deliverable`dan farki: yok sayilan (`SIG_IGN`) sinyaller cagriyi
/// bolmez. POSIX'in kurali bu ve mantikli -- yok sayilan bir sinyal
/// hicbir sey yapmiyor demektir, bekleyen bir okumayi kaldirmasinin
/// sebebi yok. Ayrimi yapmasaydik, yok sayilan bir sinyal bekleyen
/// programi bosuna `EINTR` ile uyandirirdi.
pub fn interrupts_call(task: usize) -> bool {
    if task >= scheduler::MAX_TASKS {
        return false;
    }
    let mask = PENDING[task].load(Ordering::SeqCst) & !BLOCKED[task].load(Ordering::SeqCst);
    if mask == 0 {
        return false;
    }
    unsafe {
        let table = core::ptr::addr_of!(DISPOSITIONS) as *const Disposition;
        for signo in 0..=MAX_SIGNAL {
            if mask & (1u64 << signo) == 0 {
                continue;
            }
            if table.add(task * SLOTS + signo as usize).read().handler != SIG_IGN {
                return true;
            }
        }
    }
    false
}

/// Bolunen cagri, isleyici dondukten sonra yeniden calistirilmali mi?
///
/// Bolen sinyalin `SA_RESTART` bayragina bakar. Birden fazla sinyal
/// bekliyorsa **ilk teslim edilecek** olan belirler -- `deliver_pending`
/// de en dusuk numarayi once teslim ediyor, yani ikisi ayni sinyale
/// bakiyor.
pub fn restart_after_signal(task: usize) -> bool {
    if task >= scheduler::MAX_TASKS {
        return false;
    }
    let mask = PENDING[task].load(Ordering::SeqCst) & !BLOCKED[task].load(Ordering::SeqCst);
    if mask == 0 {
        return false;
    }
    unsafe {
        let table = core::ptr::addr_of!(DISPOSITIONS) as *const Disposition;
        for signo in 0..=MAX_SIGNAL {
            if mask & (1u64 << signo) == 0 {
                continue;
            }
            let d = table.add(task * SLOTS + signo as usize).read();
            if d.handler == SIG_IGN {
                continue;
            }
            // Varsayilan davranis sureci oldurecek: yeniden baslatmanin
            // anlami yok, zaten geri donulmeyecek.
            if d.handler == SIG_DFL {
                return false;
            }
            return d.flags & SA_RESTART != 0;
        }
    }
    false
}

/// POSIX `pause`: teslim edilebilir bir sinyal gelene kadar uyur.
///
/// Bu cagriya kadar bir surec sinyali **bekleyemiyordu**: tek yol,
/// bayragi yoklayan bir dongu kurmakti -- yani sinyal gelene kadar CPU
/// yakmak. Olcu de bu: `pause` sirasinda gorevin `cpu` sayaci artmamali.
pub fn pause(task: usize) {
    SUSPENDS.fetch_add(1, Ordering::Relaxed);
    scheduler::wait_for_signal(|| deliverable(task));
}

/// POSIX `sigsuspend`: maskeyi **gecici** olarak degistirip bekler.
///
/// ## Neden `sigprocmask` + `pause` yetmez
///
/// Klasik kalip sudur: sinyali bloke et, bayragi kontrol et, sinyal
/// gelmemisse bekle. Ikisini ayri cagirmak arada bir **pencere** birakir
/// -- sinyal tam o aralikta gelirse `pause` onu kacirir ve surec sonsuza
/// kadar uyur. `sigsuspend` maskeyi degistirmeyi ve beklemeyi tek,
/// bolunmez bir adimda yapar; varlik sebebi tam olarak budur.
///
/// ## Maske ne zaman geri yuklenir
///
/// POSIX: isleyici `sigsuspend` maskesiyle kosar, **isleyici dondukten
/// sonra** eski maske geri gelir. TCMK'de teslim noktasi sistem cagrisi
/// donusudur, o yuzden geri yukleme `sigreturn`da yapilir. Isleyici
/// yoksa (varsayilan davranis / yok sayma) `deliver_pending` sonunda
/// yapilir -- iki yol da ayni `restore_mask`i cagirir.
pub fn sigsuspend(task: usize, mask: u64) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    let saved = BLOCKED[task].load(Ordering::SeqCst);
    SUSPEND_SAVE[task].store(saved, Ordering::SeqCst);
    RESTORE_PENDING[task].store(true, Ordering::SeqCst);
    // SIGKILL asla bloke edilemez; `sigprocmask` ile ayni kural.
    BLOCKED[task].store(mask & !UNBLOCKABLE, Ordering::SeqCst);

    SUSPENDS.fetch_add(1, Ordering::Relaxed);
    scheduler::wait_for_signal(|| deliverable(task));
}

/// `sigsuspend`in sakladigi maskeyi geri yukler (varsa).
fn restore_mask(task: usize) {
    if task < scheduler::MAX_TASKS
        && RESTORE_PENDING[task].swap(false, Ordering::SeqCst)
    {
        BLOCKED[task].store(SUSPEND_SAVE[task].load(Ordering::SeqCst), Ordering::SeqCst);
    }
}

/// Kac kez `pause`/`sigsuspend` ile sinyal beklendi.
pub fn suspend_count() -> u32 {
    SUSPENDS.load(Ordering::Relaxed)
}

pub fn blocked_mask(task: usize) -> u64 {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    BLOCKED[task].load(Ordering::Relaxed)
}

pub fn blocked_hits() -> u32 {
    BLOCKED_HITS.load(Ordering::Relaxed)
}

/// POSIX `alarm`: `seconds` sonra `SIGALRM` gonderilmesini ister.
///
/// Onceki alarmdan **kalan saniyeyi** doner (POSIX boyle tanimlar);
/// `seconds == 0` alarmi iptal eder. Cozunurluk PIT tik'idir (10 ms).
pub fn alarm(task: usize, seconds: u32) -> u32 {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    let now = crate::level0a::pit::ticks();
    let previous = ALARM_AT[task].load(Ordering::SeqCst);
    let remaining = if previous > now {
        (previous - now + TICKS_PER_SECOND - 1) / TICKS_PER_SECOND
    } else {
        0
    };

    if seconds == 0 {
        ALARM_AT[task].store(0, Ordering::SeqCst);
    } else {
        ALARM_AT[task].store(now + seconds * TICKS_PER_SECOND, Ordering::SeqCst);
    }
    remaining
}

/// Kalan alarm suresi, tik cinsinden (kabuk raporu). 0 = kurulu degil.
pub fn alarm_remaining(task: usize) -> u32 {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    let at = ALARM_AT[task].load(Ordering::Relaxed);
    let now = crate::level0a::pit::ticks();
    if at > now {
        at - now
    } else {
        0
    }
}

/// PIT kesmesinden cagrilir: suresi dolan alarmlar `SIGALRM` uretir.
///
/// Kesme baglamindan cagrildigi icin yalnizca atomik islem yapiyor --
/// `raise` de bir bit koymaktan ibarettir; teslim, hedef Ring 3'e
/// donerken olur.
pub fn on_tick(now: u32) {
    for task in 0..scheduler::MAX_TASKS {
        let at = ALARM_AT[task].load(Ordering::Relaxed);
        if at == 0 || now < at {
            continue;
        }
        // Once sifirla, sonra gonder: `raise` sirasinda uygulama yeni bir
        // alarm kurarsa ezmemek icin karsilastirmali degisim.
        if ALARM_AT[task]
            .compare_exchange(at, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            continue;
        }
        let _ = raise(task, SIGALRM);
    }
}

/// `fork`: yerlestirmeler cocuga **kopyalanir** (POSIX boyle der), ama
/// bekleyen sinyaller kopyalanmaz -- cocuk temiz baslar.
pub fn clone_into(child: usize) {
    let parent = scheduler::current_id();
    if child >= scheduler::MAX_TASKS || parent >= scheduler::MAX_TASKS {
        return;
    }
    // Ayri sinyal yigini **devralinir**: adres uzayi kopyalandigi icin
    // ayni adres cocukta da gecerli. `execve` ise onu birakir (bkz.
    // `reset`), cunku orada adres artik baska bir seyin olabilir.
    ALT_SP[child].store(ALT_SP[parent].load(Ordering::SeqCst), Ordering::SeqCst);
    ALT_SIZE[child].store(ALT_SIZE[parent].load(Ordering::SeqCst), Ordering::SeqCst);
    ALT_DEPTH[child].store(0, Ordering::SeqCst);
    crate::arch::cpu::without_interrupts(|| unsafe {
        let base = core::ptr::addr_of_mut!(DISPOSITIONS) as *mut Disposition;
        core::ptr::copy_nonoverlapping(
            base.add(parent * SLOTS),
            base.add(child * SLOTS),
            SLOTS,
        );
        DEPTH[child].store(0, Ordering::SeqCst);
    });
    PENDING[child].store(0, Ordering::SeqCst);
    // Kuyruk da kopyalanmiyor: POSIX'te cocuk **hicbir** bekleyen sinyal
    // devralmaz, ve kuyruk tam olarak bekleyen sinyallerin listesidir.
    queue_clear(child);
    // POSIX: cocuk ebeveynin engel maskesini **devralir**, ama bekleyen
    // sinyalleri ve alarmini almaz.
    BLOCKED[child].store(BLOCKED[parent].load(Ordering::SeqCst), Ordering::SeqCst);
    ALARM_AT[child].store(0, Ordering::SeqCst);
}

/// `execve` ve gorev cikisi: yeni imaj eski isleyicileri devralmaz --
/// adresleri zaten baska bir programa aittir.
pub fn reset(task: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    // Askida kalan bir `sigsuspend` geri yuklemesi yeni imaja tasinmaz:
    // maske o programin degil, oncekinin karariydi.
    RESTORE_PENDING[task].store(false, Ordering::SeqCst);
    // Ayri yigin eski imajin adres uzayinda duruyordu: yeni imajda o
    // adres baska bir seyin olabilir. `cwd` ve ortamdan farkli olarak
    // **korunmamali**.
    forget_alt_stack(task);
    crate::arch::cpu::without_interrupts(|| unsafe {
        let base = core::ptr::addr_of_mut!(DISPOSITIONS) as *mut Disposition;
        for i in 0..SLOTS {
            base.add(task * SLOTS + i).write(Disposition::DEFAULT);
        }
        DEPTH[task].store(0, Ordering::SeqCst);
    });
    PENDING[task].store(0, Ordering::SeqCst);
    queue_clear(task);
    // Engel maskesi POSIX'te `execve`'yi **asar** (yeni imaj ayni
    // maskeyle baslar); isleyiciler asmaz, cunku adresleri eski imaja
    // aitti. Alarm da korunur -- kurulmus bir zamanlayici, program
    // degisti diye iptal olmaz.
}

/// Bekleyen sinyal maskesi (kabuk icin).
pub fn pending_mask(task: usize) -> u64 {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    PENDING[task].load(Ordering::Relaxed)
}

/// Gorevin kayitli isleyicisi olan sinyallerin maskesi (kabuk icin).
pub fn handled_mask(task: usize) -> u64 {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    let mut mask = 0u64;
    crate::arch::cpu::without_interrupts(|| unsafe {
        let base = core::ptr::addr_of!(DISPOSITIONS) as *const Disposition;
        for signo in 1..=MAX_SIGNAL as usize {
            let d = base.add(task * SLOTS + signo).read();
            if d.handler != SIG_DFL && d.handler != SIG_IGN {
                mask |= 1u64 << signo;
            }
        }
    });
    mask
}

pub fn delivered_count() -> u32 {
    DELIVERED.load(Ordering::Relaxed)
}

pub fn sent_count() -> u32 {
    SENT.load(Ordering::Relaxed)
}

/// Sinyalin kisa adi (kabuk ciktilari icin).
pub fn name_of(signo: u32) -> &'static str {
    match signo {
        SIGHUP => "SIGHUP",
        SIGINT => "SIGINT",
        SIGQUIT => "SIGQUIT",
        SIGILL => "SIGILL",
        SIGABRT => "SIGABRT",
        SIGFPE => "SIGFPE",
        SIGKILL => "SIGKILL",
        SIGUSR1 => "SIGUSR1",
        SIGSEGV => "SIGSEGV",
        SIGUSR2 => "SIGUSR2",
        SIGPIPE => "SIGPIPE",
        SIGALRM => "SIGALRM",
        SIGTERM => "SIGTERM",
        SIGCONT => "SIGCONT",
        SIGSTOP => "SIGSTOP",
        SIGTSTP => "SIGTSTP",
        // Gercek-zamanlilarin tek tek adi yok; POSIX onlari zaten
        // `SIGRTMIN+n` diye adlandirir, yani ad bir **sayi ifadesi**.
        _ if rt(signo) => "SIGRT",
        _ => "SIG?",
    }
}

/// Ring 3'e donmeden hemen once cagrilir: bekleyen bir sinyal varsa
/// cerceveyi isleyiciye cevirir.
///
/// # Safety
/// `frame` Ring 3'ten gelen gecerli bir syscall cercevesi olmalidir ve
/// cagiran gorevin adres uzayi etkin olmalidir.
pub unsafe fn deliver_pending(frame: &mut SyscallFrame, from_interrupt: bool) {
    if !usermode::in_user_mode() {
        return;
    }
    let task = scheduler::current_id();
    if task >= scheduler::MAX_TASKS {
        return;
    }
    // Ic ice teslim artik **serbest** -- ayni sinyal degilse. POSIX
    // kurali budur: teslim edilen sinyal kendi isleyicisi suresince
    // engellenir (bkz. asagida `SA_NODEFER`), digerleri engellenmez.
    //
    // Tek sinir yigin derinligi. Dolduysa teslim ertelenir: sinyal
    // `PENDING`de kalir ve bir isleyici dondugunde yeniden denenir.
    if DEPTH[task].load(Ordering::SeqCst) >= NEST_DEPTH {
        if PENDING[task].load(Ordering::SeqCst) & !BLOCKED[task].load(Ordering::SeqCst) != 0 {
            NEST_DEFERRED.fetch_add(1, Ordering::Relaxed);
        }
        return;
    }

    loop {
        // Bloke olanlar **elenir**, silinmez: maskede duran bir sinyal
        // kaybolmaz, `sigprocmask` acildiginda teslim edilir.
        let blocked = BLOCKED[task].load(Ordering::SeqCst);
        let mask = PENDING[task].load(Ordering::SeqCst) & !blocked;
        if mask == 0 {
            if PENDING[task].load(Ordering::SeqCst) != 0 {
                BLOCKED_HITS.fetch_add(1, Ordering::Relaxed);
            }
            // Isleyici calismadan buraya gelindiyse (varsayilan davranis
            // ya da yok sayma) `sigreturn` hic olmayacak; `sigsuspend`in
            // maskesi burada geri yuklenir.
            restore_mask(task);
            return;
        }
        // En kucuk numara once. Standart sinyaller 1..=31'de durdugu icin
        // bu, POSIX'in "standart sinyaller gercek-zamanlilardan once"
        // kuralini kendiliginden veriyor.
        let signo = mask.trailing_zeros();

        // Bekleyen bitin **ne zaman silindigi** iki sinifi ayiran yer.
        //
        //   standart      -> bit hemen silinir; tek yuva, tek teslim
        //   gercek-zamanli -> kuyruktan bir kopya cikar; kuyrukta
        //                     baskasi kaldiysa bit DURUR ve sinyal
        //                     yeniden teslim edilir
        //
        // Birlesme ile kuyruklanma arasindaki butun fark bu kosulda.
        let info = if rt(signo) {
            match queue_pop(task, signo) {
                Some(info) => {
                    if !queue_has(task, signo) {
                        PENDING[task].fetch_and(!(1u64 << signo), Ordering::SeqCst);
                    }
                    info
                }
                None => {
                    // Bit var, kayit yok. Olmamasi gereken bir hal ama
                    // biti birakmak sonsuz donguye girmek olurdu.
                    PENDING[task].fetch_and(!(1u64 << signo), Ordering::SeqCst);
                    continue;
                }
            }
        } else {
            PENDING[task].fetch_and(!(1u64 << signo), Ordering::SeqCst);
            (core::ptr::addr_of!(INFO) as *const SigInfo)
                .add(task * INFO_SLOTS + signo as usize)
                .read()
        };

        let entry = (core::ptr::addr_of_mut!(DISPOSITIONS) as *mut Disposition)
            .add(task * SLOTS + signo as usize);
        let d = entry.read();

        // `SA_RESETHAND`: yerlestirme **teslimden once** varsayilana
        // doner, yani isleyici tek atimliktir. Eski `signal(2)`
        // semantiginin ta kendisi.
        if d.flags & SA_RESETHAND != 0 {
            entry.write(Disposition::DEFAULT);
        }

        match d.handler {
            SIG_IGN => continue,
            SIG_DFL => match default_action(signo) {
                DefaultAction::Terminate => {
                    crate::println!(
                        "[LEVEL-0b1] sinyal: gorev #{} {} ile sonlandiriliyor (varsayilan davranis).",
                        task,
                        name_of(signo)
                    );
                    DELIVERED.fetch_add(1, Ordering::Relaxed);
                    // Cikis kodu degil, **olum sebebi** kaydediliyor.
                    // `128 + signo` kabuk gelenegidir; cekirdek ikisini
                    // ayri tutmak zorunda, yoksa `WIFSIGNALED` soran bir
                    // program yanilir.
                    scheduler::set_current_exit_signal(signo);
                    // Donmez.
                    crate::level0a::kernel_api::exit_current_task(0);
                }
                DefaultAction::Stop => {
                    // `SIGTSTP` yakalanmadi: varsayilan durdurmak.
                    // `stop_task` calisan gorev icin donmez-gibi
                    // davranir -- `SIGCONT` gelene kadar burada beklenir.
                    crate::println!(
                        "[LEVEL-0b1] sinyal: gorev #{} {} ile durduruldu.",
                        task,
                        name_of(signo)
                    );
                    DELIVERED.fetch_add(1, Ordering::Relaxed);
                    scheduler::stop_task(task, signo);
                    continue;
                }
                // `SIGCONT` hedefi zaten `raise` aninda kaldirdi;
                // burada yapacak is yok. Yine de kuyruga girmesi
                // gerekiyordu: isleyici kurulmussa o calismali.
                DefaultAction::Continue => continue,
            },
            _ => {
                let depth = DEPTH[task].load(Ordering::SeqCst);
                let mut context = frame.user_context_via(from_interrupt);

                // Baglam ve **o anki maske** birlikte saklanir: isleyici
                // dondugunde ikisi de geri gelmeli.
                let saved = core::ptr::addr_of_mut!(SAVED) as *mut UserContext;
                saved.add(task * NEST_DEPTH + depth).write(context);
                let saved_mask = core::ptr::addr_of_mut!(SAVED_MASK) as *mut u64;
                saved_mask
                    .add(task * NEST_DEPTH + depth)
                    .write(blocked);
                if enter_handler(task, depth, &mut context, signo, &d, &info).is_none() {
                    // Yigin gecerli degil: sinyali teslim etmeye calisirken
                    // sureci bozmaktansa varsayilan davranisa dusulur.
                    crate::println!(
                        "[LEVEL-0b1] sinyal: gorev #{} yigini gecersiz, {} varsayilana dusuyor.",
                        task,
                        name_of(signo)
                    );
                    crate::level0a::kernel_api::exit_current_task(128 + signo);
                }
                // POSIX: isleyici kosarken **kendi sinyali** engellenir
                // (`SA_NODEFER` bunu kaldirir) ve `sa_mask`teki sinyaller
                // de eklenir. Ayni sinyalin kendi isleyicisinde yeniden
                // teslim edilmesi boylece engellenmis oluyor -- eskiden
                // bu isi "hicbir sinyal teslim edilmez" kurali yapiyordu.
                let mut extra = d.mask;
                if d.flags & SA_NODEFER == 0 {
                    extra |= 1u64 << signo;
                }
                BLOCKED[task].store((blocked | extra) & !UNBLOCKABLE, Ordering::SeqCst);

                let depth = depth + 1;
                DEPTH[task].store(depth, Ordering::SeqCst);
                MAX_NESTED.fetch_max(depth as u32, Ordering::Relaxed);

                frame.set_user_context_via(from_interrupt, &context);
                DELIVERED.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
    }
}

// --- Ayri yigin: sorgu ve kurulum ------------------------------------

/// Gorevin ayri yigini: `(taban, olcu, ss_flags)`.
///
/// `ss_flags` uc halden birini soyler ve ucu de ayri bir cevap:
///
/// ```text
///   SS_DISABLE   kurulu degil
///   SS_ONSTACK   kurulu ve su an USTUNDE kosuluyor
///   0            kurulu, ama su an kullanilmiyor
/// ```
pub fn alt_stack_of(task: usize) -> (usize, usize, u32) {
    if task >= scheduler::MAX_TASKS {
        return (0, 0, SS_DISABLE);
    }
    let sp = ALT_SP[task].load(Ordering::SeqCst);
    if sp == 0 {
        return (0, 0, SS_DISABLE);
    }
    let flags = if ALT_DEPTH[task].load(Ordering::SeqCst) > 0 {
        SS_ONSTACK
    } else {
        0
    };
    (sp, ALT_SIZE[task].load(Ordering::SeqCst), flags)
}

/// Calisan gorevin ayri yigini -- `ucontext_t`nin `uc_stack` alani icin.
pub fn current_alt_stack() -> (usize, usize, u32) {
    alt_stack_of(scheduler::current_id())
}

/// `sigaltstack`in reddedilme sebepleri.
///
/// Hata **numarasi** degil, sebep donuyor: errno sayilari tek bir yerde
/// (`posix_syscalls`) duruyor ve ikinci bir kopya, iki yerde
/// ayrisabilen bir sozlesme birakirdi.
#[derive(Debug, Clone, Copy)]
pub enum AltStackError {
    /// Ustunde kosulurken degistirilemez.
    Busy,
    /// Taninmayan `ss_flags`.
    BadFlags,
    /// `MINSIGSTKSZ`den kucuk.
    TooSmall,
    /// Ring 3'ten erisilemeyen bolge.
    BadAddress,
}

/// `sigaltstack`in kurulum yarisi.
pub fn set_alt_stack(
    task: usize,
    sp: usize,
    size: usize,
    flags: u32,
) -> Result<(), AltStackError> {
    if task >= scheduler::MAX_TASKS {
        return Err(AltStackError::BadFlags);
    }
    // Ustunde kosulurken degistirmek yasak: cerceve tam orada duruyor.
    // POSIX'in kurali bu ve gerekcesi somut -- degisim kabul edilseydi
    // isleyici kendi altindaki zemini cekmis olurdu.
    if ALT_DEPTH[task].load(Ordering::SeqCst) > 0 {
        return Err(AltStackError::Busy);
    }
    if flags & SS_DISABLE != 0 {
        ALT_SP[task].store(0, Ordering::SeqCst);
        ALT_SIZE[task].store(0, Ordering::SeqCst);
        return Ok(());
    }
    if flags != 0 {
        return Err(AltStackError::BadFlags);
    }
    if size < MINSIGSTKSZ {
        // Linux burada `ENOMEM` der: istek gecersiz degil, **yetersiz**.
        return Err(AltStackError::TooSmall);
    }
    if sp == 0 || !mmu::is_user_accessible(sp) || !mmu::is_user_accessible(sp + size - 1) {
        return Err(AltStackError::BadAddress);
    }
    ALT_SP[task].store(sp, Ordering::SeqCst);
    ALT_SIZE[task].store(size, Ordering::SeqCst);
    Ok(())
}

/// Ayri yigini kaldirir (`execve`, gorev cikisi).
fn forget_alt_stack(task: usize) {
    ALT_SP[task].store(0, Ordering::SeqCst);
    ALT_SIZE[task].store(0, Ordering::SeqCst);
    ALT_DEPTH[task].store(0, Ordering::SeqCst);
}

/// Cerceve kurulacak yigin tepesi -- ayri yigina gecilecekse.
///
/// `None` donuyorsa kesilen yigin kullanilacak. Uc sebepten biriyle:
/// ayri yigin kurulu degil, ya da zaten onun ustundeyiz (o zaman
/// **tepeye donmek** alttaki cerceveyi ezerdi).
fn alt_top(task: usize) -> Option<usize> {
    let sp = ALT_SP[task].load(Ordering::SeqCst);
    if sp == 0 || ALT_DEPTH[task].load(Ordering::SeqCst) > 0 {
        return None;
    }
    let size = ALT_SIZE[task].load(Ordering::SeqCst);
    Some((sp + size) & !0xF)
}

/// Baglami isleyiciye cevirir; `ucontext_t` adresini de kaydeder.
///
/// Iki yuz arasindaki tek fark burada secilir:
///
/// ```text
///   SA_SIGINFO yok  ->  handler(signo)
///   SA_SIGINFO var  ->  handler(signo, &siginfo, &ucontext)
/// ```
///
/// Ikincisi ayrica bir **geri yol** aciyor: kurulan `ucontext_t`nin
/// adresi saklaniyor ve `sigreturn` baglami oradan okuyor. Yani
/// isleyicinin registerlarda yaptigi degisiklik yururlukte kaliyor.
///
/// Doner: `None` ise yigin gecersiz, cerceve kurulamadi.
///
/// # Safety
/// Cagiran gorevin adres uzayi etkin olmalidir.
unsafe fn enter_handler(
    task: usize,
    depth: usize,
    context: &mut UserContext,
    signo: u32,
    d: &Disposition,
    info: &SigInfo,
) -> Option<()> {
    // `SA_ONSTACK`: cerceve **ayri** yigina kuruluyor.
    //
    // Secilen taban `context`e yazilmiyor, ayri bir arguman olarak
    // gecirilyor -- ve bu bir uslup tercihi degil. Ilk yazilista taban
    // dogrudan `context.sp`ye konuyordu ve `ucontext_t` o baglamdan
    // dolduruldugu icin `uc_mcontext.esp` **ayri yigini** gosteriyordu.
    // Isleyici donunce cekirdek o degeri geri yukluyor, yani surec ayri
    // yiginin ustunde devam ediyordu: kesilen yigin kayboluyordu.
    // Olcum bunu, `SA_ONSTACK`siz bir isleyicinin de ayri yiginda
    // gorunmesiyle yakaladi.
    USED_ALT[task][depth].store(0, Ordering::SeqCst);
    let mut stack = context.stack_pointer();
    if d.flags & SA_ONSTACK != 0 {
        if let Some(top) = alt_top(task) {
            stack = top;
            USED_ALT[task][depth].store(1, Ordering::SeqCst);
            ALT_DEPTH[task].fetch_add(1, Ordering::SeqCst);
        }
    }

    if d.flags & SA_SIGINFO == 0 {
        UCONTEXT_AT[task][depth].store(0, Ordering::SeqCst);
        return usermode::build_signal_frame(context, stack, signo, d.handler, d.restorer);
    }

    // Neden artik **cagiran** tarafindan veriliyor, tablodan okunmuyor.
    // Sebep kuyruk: gercek-zamanli bir sinyalin nedeni sinyal basina tek
    // yuvada degil, teslim edilen **kopyanin** yaninda duruyor. Burada
    // tabloya bakmak, kuyruktan cekilen kopyanin degerini kaybetmek
    // olurdu.
    let ucontext_at =
        usermode::build_siginfo_frame(context, stack, signo, d.handler, d.restorer, info)?;
    UCONTEXT_AT[task][depth].store(ucontext_at, Ordering::SeqCst);
    Some(())
}

/// Bir CPU hatasini POSIX sinyali olarak teslim eder.
///
/// Bu, `deliver_pending`in kardesi ve ayrilmalarinin sebebi **cerceve
/// turu**: sinyaller normalde bir sistem cagrisinin donus yolunda
/// teslim edilir (`SyscallFrame`), ama bir sayfa hatasi sistem cagrisi
/// degil -- kesme kapisindan gelir ve cercevesi `ExceptionFrame`tir.
///
/// Yapilan is ayni: kullanici yigininin ustune bir cerceve kurulur ve
/// baglam isleyiciye cevrilir. Buradan `true` donulunce istisna
/// isleyicisi `iret` eder ve CPU hatali komuta degil, **isleyiciye**
/// doner. Windows tarafindaki ikizi `seh::dispatch` (bkz. orada).
///
/// Doner: teslim edildi mi. `false` ise cagiran olumcul yola devam
/// eder -- yani surec sonlanir ve izolasyon korunur.
///
/// Uc durumda bilerek `false` donuyor ve ucu de ayni sebebe cikiyor:
/// **senkron** bir hata yok sayilamaz, cunku donuldugunde ayni komut
/// ayni hatayi verir ve surec sonsuz donguye girer.
///
/// ```text
///   isleyici yok (SIG_DFL)  -> varsayilan davranis: sonlan
///   isleyici SIG_IGN        -> yok saymak mumkun degil: sonlan
///   sinyal engellenmis      -> ertelemek mumkun degil: sonlan
/// ```
///
/// Ucuncusu bir yan fayda daha veriyor: sinyal kendi isleyicisi
/// suresince engellendigi icin (`SA_NODEFER` yoksa), `SIGSEGV`
/// isleyicisinin **kendi** urettigi bir sayfa hatasi burada `false`
/// donuyor ve surec sonlaniyor. Gercek Linux de aynisini yapar; aksi
/// halde hatali bir isleyici sistemi sonsuz teslim dongusune sokardi.
///
/// # Safety
/// `frame` Ring 3'ten gelen gecerli bir istisna cercevesi olmalidir ve
/// cagiran gorevin adres uzayi etkin olmalidir.
pub unsafe fn deliver_fault(
    frame: &mut crate::arch::cpu::regs::ExceptionFrame,
    signo: u32,
    info: SigInfo,
) -> bool {
    let task = scheduler::current_id();
    if task >= scheduler::MAX_TASKS || !valid(signo) {
        return false;
    }
    if BLOCKED[task].load(Ordering::SeqCst) & (1u64 << signo) != 0 {
        return false;
    }
    let depth = DEPTH[task].load(Ordering::SeqCst);
    if depth >= NEST_DEPTH {
        return false;
    }

    let entry = (core::ptr::addr_of_mut!(DISPOSITIONS) as *mut Disposition)
        .add(task * SLOTS + signo as usize);
    let d = entry.read();
    if d.handler == SIG_DFL || d.handler == SIG_IGN {
        return false;
    }
    if d.flags & SA_RESETHAND != 0 {
        entry.write(Disposition::DEFAULT);
    }

    let blocked = BLOCKED[task].load(Ordering::SeqCst);
    let mut context = frame.user_context();

    let saved = core::ptr::addr_of_mut!(SAVED) as *mut UserContext;
    saved.add(task * NEST_DEPTH + depth).write(context);
    let saved_mask = core::ptr::addr_of_mut!(SAVED_MASK) as *mut u64;
    saved_mask.add(task * NEST_DEPTH + depth).write(blocked);

    if enter_handler(task, depth, &mut context, signo, &d, &info).is_none() {
        // Yigin yazilamiyor. Sinyali teslim etmeye calisirken sureci
        // bozmaktansa olumcul yola birakiliyor.
        return false;
    }

    let mut extra = d.mask;
    if d.flags & SA_NODEFER == 0 {
        extra |= 1u64 << signo;
    }
    BLOCKED[task].store((blocked | extra) & !UNBLOCKABLE, Ordering::SeqCst);

    DEPTH[task].store(depth + 1, Ordering::SeqCst);
    MAX_NESTED.fetch_max(depth as u32 + 1, Ordering::Relaxed);

    frame.set_user_context(&context);
    DELIVERED.fetch_add(1, Ordering::Relaxed);
    FAULTS_CAUGHT.fetch_add(1, Ordering::Relaxed);
    true
}

/// Sinyale cevrilip **yakalanan** CPU hatasi sayisi (kabuk raporu).
static FAULTS_CAUGHT: AtomicU32 = AtomicU32::new(0);

pub fn faults_caught() -> u32 {
    FAULTS_CAUGHT.load(Ordering::Relaxed)
}

/// `sigreturn`: isleyiciden donusu tamamlar, saklanan baglami geri koyar.
///
/// Donus degeri diye bir sey yoktur -- kullanici bu cagriyi kendi yazmaz,
/// tramplen yapar ve cagri **donmez** (baglam degistigi icin surec baska
/// bir noktada uyanir).
///
/// # Safety
/// Ring 3'ten gelen gecerli bir cerceve ile cagrilmalidir.
pub unsafe fn sigreturn(frame: &mut SyscallFrame, from_interrupt: bool) -> bool {
    let task = scheduler::current_id();
    if task >= scheduler::MAX_TASKS {
        return false;
    }
    let depth = DEPTH[task].load(Ordering::SeqCst);
    if depth == 0 {
        // Isleyici icinde degilken `sigreturn` cagirmak, kullanicinin
        // rastgele bir baglama zipladigi anlamina gelirdi.
        return false;
    }
    let depth = depth - 1;
    DEPTH[task].store(depth, Ordering::SeqCst);

    let mut context = (core::ptr::addr_of!(SAVED) as *const UserContext)
        .add(task * NEST_DEPTH + depth)
        .read();

    // `SA_SIGINFO` isleyicisinin gordugu `ucontext_t` **yazilabilirdi**.
    // Saklanan baglam yerine onu okumak, isleyicinin yaptigi
    // duzeltmenin yurumesi demek: hatali bir komutu duzeltip donmek
    // ancak boyle bir sey ifade eder.
    //
    // Kaydin kendisi kullanici yigininda duruyor, yani icerigi
    // guvenilmez. `read_ucontext` bunu bilerek yaziyor: yalnizca genel
    // registerlar aliniyor ve bayraklarin sistem bitleri cekirdegin
    // degeriyle kaliyor (bkz. `usermode.rs`).
    // Ayri yigina gecilmisse sayac geri aliniyor: bu katman artik onun
    // ustunde degil.
    if USED_ALT[task][depth].swap(0, Ordering::SeqCst) != 0 {
        let previous = ALT_DEPTH[task].load(Ordering::SeqCst);
        ALT_DEPTH[task].store(previous.saturating_sub(1), Ordering::SeqCst);
    }

    let ucontext_at = UCONTEXT_AT[task][depth].swap(0, Ordering::SeqCst);
    if ucontext_at != 0 && mmu::is_user_accessible(ucontext_at) {
        usermode::read_ucontext(ucontext_at, &mut context);
    }
    frame.set_user_context_via(from_interrupt, &context);

    // Isleyicinin ek engelleri yalnizca isleyici suresince gecerliydi.
    let saved_mask = (core::ptr::addr_of!(SAVED_MASK) as *const u64)
        .add(task * NEST_DEPTH + depth)
        .read();
    BLOCKED[task].store(saved_mask, Ordering::SeqCst);

    // POSIX: `sigsuspend`in maskesi isleyici **dondukten sonra** kalkar.
    // Yalnizca **en distaki** donuste: ic katmanlarda geri yuklenirse
    // `sigsuspend` maskesi daha isleyici bitmeden kalkardi.
    if depth == 0 {
        restore_mask(task);
    }
    true
}
