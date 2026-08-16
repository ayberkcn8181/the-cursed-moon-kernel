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
//! ## Bilerek yapilmayanlar
//!
//! * **Maskeleme (`sigprocmask`) yok.** Isleyici calisirken yeni sinyal
//!   teslim edilmez (ic ice cagri yok), ama secmeli bloklama yoktur.
//! * **`siginfo`/`sigaction` bayraklari yok.** Isleyici yalnizca sinyal
//!   numarasini alir -- klasik `signal()` semantigi.
//! * **Sira yok.** Ayni sinyal iki kez gelirse bir kez teslim edilir
//!   (bit maskesi); gercek POSIX'te standart sinyaller icin de boyledir.

use crate::arch::cpu::regs::{SyscallFrame, UserContext};
use crate::arch::cpu::usermode;
use crate::level0a::core::{mmu, scheduler};
use core::sync::atomic::{AtomicU32, Ordering};

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
pub const SIGALRM: u32 = 14;
pub const SIGTERM: u32 = 15;

/// Desteklenen en buyuk sinyal numarasi (1..=31, gercek-zamanli sinyaller
/// yok).
pub const MAX_SIGNAL: u32 = 31;

/// Bir sinyalin bir surecteki karsiligi.
#[derive(Clone, Copy)]
struct Disposition {
    /// `SIG_DFL`, `SIG_IGN` ya da Ring 3'teki isleyicinin adresi.
    handler: usize,
    /// Isleyici donduğunde ziplanacak tramplen (kullanici tarafi verir).
    restorer: usize,
    /// `SA_*` bayraklari (bkz. `SA_NODEFER`, `SA_RESETHAND`).
    flags: u32,
    /// Isleyici kosarken **ek olarak** engellenecek sinyaller
    /// (`sigaction`in `sa_mask` alani).
    mask: u32,
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

/// Cekirdegin tanidigi bayraklar. Digerleri sessizce yok sayilir --
/// `SA_RESTART` gibi, karsiligi olmayan bir bayragi kabul ediyormus gibi
/// yapmak yaniltici olurdu (bkz. README).
pub const SUPPORTED_FLAGS: u32 = SA_NODEFER | SA_RESETHAND;

/// Surec basina yerlestirmeler. Gorev kimligiyle indekslenir; `fork`
/// bunlari kopyalar (`clone_into`), `execve`/cikis sifirlar (`reset`).
static mut DISPOSITIONS: [[Disposition; MAX_SIGNAL as usize + 1]; scheduler::MAX_TASKS] =
    [[Disposition::DEFAULT; MAX_SIGNAL as usize + 1]; scheduler::MAX_TASKS];

/// Bekleyen sinyaller: bit N = sinyal N.
///
/// `AtomicU32`, cunku gonderen baska bir gorevdir; okuma-degistirme-yazma
/// arasinda zamanlayici araya girerse sinyal kaybolurdu.
#[allow(clippy::declare_interior_mutable_const)]
const ZERO_PENDING: AtomicU32 = AtomicU32::new(0);
static PENDING: [AtomicU32; scheduler::MAX_TASKS] = [ZERO_PENDING; scheduler::MAX_TASKS];

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
static mut SAVED_MASK: [[u32; NEST_DEPTH]; scheduler::MAX_TASKS] =
    [[0; NEST_DEPTH]; scheduler::MAX_TASKS];

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
static BLOCKED: [AtomicU32; scheduler::MAX_TASKS] = [ZERO_PENDING; scheduler::MAX_TASKS];

/// `alarm` icin uyanma ani (PIT tik'i, mutlak). 0 = kurulu degil.
static ALARM_AT: [AtomicU32; scheduler::MAX_TASKS] = [ZERO_PENDING; scheduler::MAX_TASKS];

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
/// almaz; alsaydi bir surec kendini oldurulemez yapabilirdi.
const UNBLOCKABLE: u32 = 1 << SIGKILL;

/// PIT tik'i basina saniye (100 Hz).
const TICKS_PER_SECOND: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalError {
    /// Sinyal numarasi 1..=31 disinda.
    InvalidSignal,
    /// Hedef gorev yok ya da zaten sonlanmis.
    NoSuchTask,
    /// `SIGKILL`/`SIGSTOP` yakalanamaz.
    Uncatchable,
}

fn valid(signo: u32) -> bool {
    signo >= 1 && signo <= MAX_SIGNAL
}

/// Sinyalin varsayilan davranisi surec sonlandirmak mi?
///
/// TCMK'de yok sayilan (ignore) varsayilani olan bir sinyal yok --
/// `SIGCHLD`/`SIGURG` gibi sinyaller uygulanmadi. Yani varsayilan her
/// zaman "oldur"dur.
fn default_terminates(_signo: u32) -> bool {
    true
}

/// Bir goreve sinyal gonderir.
///
/// `SIGKILL` beklemeye alinmaz: hedef **hemen** sonlandirilir. Digerleri
/// bekleyenler maskesine yazilir ve hedef Ring 3'e donerken teslim edilir.
/// Sinyali kuyruga koyar; bekleyen bir `pause`/`sigsuspend` varsa uyandirir.
pub fn raise(target: usize, signo: u32) -> Result<(), SignalError> {
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

    SENT.fetch_add(1, Ordering::Relaxed);

    if signo == SIGKILL {
        // Yakalanamaz: surecin isbirligini beklemeden sonlandirilir.
        // Kendi kendine gonderilmisse cagri zincirinden cikmak gerekir,
        // bu yuzden ayri yol.
        if target == scheduler::current_id() {
            crate::level0a::kernel_api::exit_current_task(128 + SIGKILL);
        }
        return scheduler::terminate(target)
            .map_err(|_| SignalError::NoSuchTask);
    }

    PENDING[target].fetch_or(1 << signo, Ordering::SeqCst);

    // `pause`/`sigsuspend` ile uyuyan bir gorev varsa kaldirilir. Tek
    // hedeflidir: yalnizca sinyalin gittigi gorev uyanir. Bloke bir
    // sinyalse `deliverable` yine `false` doner ve gorev geri uyur --
    // sahte uyanma dongude ele aliniyor.
    scheduler::wake_signal_waiter(target);
    Ok(())
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
    mask: u32,
) -> Result<usize, SignalError> {
    if !valid(signo) {
        return Err(SignalError::InvalidSignal);
    }
    if signo == SIGKILL {
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
            .add(task * (MAX_SIGNAL as usize + 1) + signo as usize);
        let old = slot.read().handler;
        slot.write(Disposition {
            handler,
            restorer,
            // Taninmayan bayraklar **atilir**. Kabul ediyormus gibi
            // saklamak, `SA_RESTART` gibi karsiligi olmayan bir bayragin
            // calistigi izlenimini verirdi.
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
            .add(task * (MAX_SIGNAL as usize + 1) + signo as usize)
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
pub fn sigprocmask(task: usize, how: usize, set: u32) -> Option<u32> {
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
static SUSPEND_SAVE: [core::sync::atomic::AtomicU32; scheduler::MAX_TASKS] =
    [const { core::sync::atomic::AtomicU32::new(0) }; scheduler::MAX_TASKS];
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
pub fn sigsuspend(task: usize, mask: u32) {
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

pub fn blocked_mask(task: usize) -> u32 {
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
    crate::arch::cpu::without_interrupts(|| unsafe {
        let base = core::ptr::addr_of_mut!(DISPOSITIONS) as *mut Disposition;
        let width = MAX_SIGNAL as usize + 1;
        core::ptr::copy_nonoverlapping(
            base.add(parent * width),
            base.add(child * width),
            width,
        );
        DEPTH[child].store(0, Ordering::SeqCst);
    });
    PENDING[child].store(0, Ordering::SeqCst);
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
    crate::arch::cpu::without_interrupts(|| unsafe {
        let base = core::ptr::addr_of_mut!(DISPOSITIONS) as *mut Disposition;
        let width = MAX_SIGNAL as usize + 1;
        for i in 0..width {
            base.add(task * width + i).write(Disposition::DEFAULT);
        }
        DEPTH[task].store(0, Ordering::SeqCst);
    });
    PENDING[task].store(0, Ordering::SeqCst);
    // Engel maskesi POSIX'te `execve`'yi **asar** (yeni imaj ayni
    // maskeyle baslar); isleyiciler asmaz, cunku adresleri eski imaja
    // aitti. Alarm da korunur -- kurulmus bir zamanlayici, program
    // degisti diye iptal olmaz.
}

/// Bekleyen sinyal maskesi (kabuk icin).
pub fn pending_mask(task: usize) -> u32 {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    PENDING[task].load(Ordering::Relaxed)
}

/// Gorevin kayitli isleyicisi olan sinyallerin maskesi (kabuk icin).
pub fn handled_mask(task: usize) -> u32 {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    let mut mask = 0;
    crate::arch::cpu::without_interrupts(|| unsafe {
        let base = core::ptr::addr_of!(DISPOSITIONS) as *const Disposition;
        let width = MAX_SIGNAL as usize + 1;
        for signo in 1..=MAX_SIGNAL as usize {
            let d = base.add(task * width + signo).read();
            if d.handler != SIG_DFL && d.handler != SIG_IGN {
                mask |= 1 << signo;
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
        SIGALRM => "SIGALRM",
        SIGTERM => "SIGTERM",
        _ => "SIG?",
    }
}

/// Ring 3'e donmeden hemen once cagrilir: bekleyen bir sinyal varsa
/// cerceveyi isleyiciye cevirir.
///
/// # Safety
/// `frame` Ring 3'ten gelen gecerli bir syscall cercevesi olmalidir ve
/// cagiran gorevin adres uzayi etkin olmalidir.
pub unsafe fn deliver_pending(frame: &mut SyscallFrame) {
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
        let signo = mask.trailing_zeros();
        PENDING[task].fetch_and(!(1 << signo), Ordering::SeqCst);

        let width = MAX_SIGNAL as usize + 1;
        let entry = (core::ptr::addr_of_mut!(DISPOSITIONS) as *mut Disposition)
            .add(task * width + signo as usize);
        let d = entry.read();

        // `SA_RESETHAND`: yerlestirme **teslimden once** varsayilana
        // doner, yani isleyici tek atimliktir. Eski `signal(2)`
        // semantiginin ta kendisi.
        if d.flags & SA_RESETHAND != 0 {
            entry.write(Disposition::DEFAULT);
        }

        match d.handler {
            SIG_IGN => continue,
            SIG_DFL => {
                if default_terminates(signo) {
                    crate::println!(
                        "[LEVEL-0b1] sinyal: gorev #{} {} ile sonlandiriliyor (varsayilan davranis).",
                        task,
                        name_of(signo)
                    );
                    DELIVERED.fetch_add(1, Ordering::Relaxed);
                    crate::level0a::kernel_api::exit_current_task(128 + signo);
                }
                continue;
            }
            handler => {
                let depth = DEPTH[task].load(Ordering::SeqCst);
                let mut context = frame.user_context();

                // Baglam ve **o anki maske** birlikte saklanir: isleyici
                // dondugunde ikisi de geri gelmeli.
                let saved = core::ptr::addr_of_mut!(SAVED) as *mut UserContext;
                saved.add(task * NEST_DEPTH + depth).write(context);
                let saved_mask = core::ptr::addr_of_mut!(SAVED_MASK) as *mut u32;
                saved_mask
                    .add(task * NEST_DEPTH + depth)
                    .write(blocked);
                if usermode::build_signal_frame(&mut context, signo, handler, d.restorer)
                    .is_none()
                {
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
                    extra |= 1 << signo;
                }
                BLOCKED[task].store((blocked | extra) & !UNBLOCKABLE, Ordering::SeqCst);

                let depth = depth + 1;
                DEPTH[task].store(depth, Ordering::SeqCst);
                MAX_NESTED.fetch_max(depth as u32, Ordering::Relaxed);

                frame.set_user_context(&context);
                DELIVERED.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
    }
}

/// `sigreturn`: isleyiciden donusu tamamlar, saklanan baglami geri koyar.
///
/// Donus degeri diye bir sey yoktur -- kullanici bu cagriyi kendi yazmaz,
/// tramplen yapar ve cagri **donmez** (baglam degistigi icin surec baska
/// bir noktada uyanir).
///
/// # Safety
/// Ring 3'ten gelen gecerli bir cerceve ile cagrilmalidir.
pub unsafe fn sigreturn(frame: &mut SyscallFrame) -> bool {
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

    let context = (core::ptr::addr_of!(SAVED) as *const UserContext)
        .add(task * NEST_DEPTH + depth)
        .read();
    frame.set_user_context(&context);

    // Isleyicinin ek engelleri yalnizca isleyici suresince gecerliydi.
    let saved_mask = (core::ptr::addr_of!(SAVED_MASK) as *const u32)
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
