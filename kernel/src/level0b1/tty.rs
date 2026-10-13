//! Denetleyen terminal ve is denetimi.
//!
//! Surec gruplari bir onceki batida geldi: `setpgid`, `kill(-pgid)`,
//! `waitpid(-pgid)`, `SIGSTOP`/`SIGCONT`. Eksik kalan **terminal**
//! tarafiydi, ve is denetiminin asil anlami orada:
//!
//! ```text
//!   $ make &          <- arka plan isi
//!   $ vim             <- on plan isi, klavyeyi o okuyor
//! ```
//!
//! Iki is ayni terminali paylasiyor. Klavyenin `vim`e gitmesi, `make`in
//! ise klavyeye **dokunamamasi** gerekiyor. POSIX'in cevabi uc parcali:
//!
//! ```text
//!   oturum (sid)         bir terminali paylasan islerin kumesi
//!   on plan grubu        terminali OKUYABILEN tek grup
//!   SIGTTIN / SIGTTOU    arka plandan okumaya / ayara dokunmaya
//!                        calisan grubu durdurur
//! ```
//!
//! ## TCMK'de terminal ne
//!
//! TCMK'de seri bir tty yok; klavye pencerelere gidiyor. Uzun sure
//! `read(0)` gorevin **kendi** penceresini okuyordu ve penceresi
//! olmayan bir `fork` cocugu dosya sonu aliyordu. Bu, paylasilan bir
//! terminalin olmamasi demekti -- yani is denetiminin uzerine
//! kurulacagi zemin de yoktu.
//!
//! Model artik su: **bir oturumun terminali liderinin penceresi**.
//! Kabuktan baslatilan bir program kendi oturumunun lideri; actigi ilk
//! pencere onun terminali, ve `fork` cocuklari -- kendi pencereleri
//! yoksa -- o terminali okuyor. Gercek bir sistemde bir kabugun
//! cocuklari da tam olarak boyle ayni tty'yi paylasir.
//!
//! Kendi penceresi olan bir cocuk eskisi gibi **kendi** penceresini
//! okuyor: o pencere onun kendi terminali, oturumunki degil.
//!
//! ## Iki karar, zit yonlerde
//!
//! ```text
//!   arka plandan OKUMA        SIGTTIN yok sayili/engelli -> EIO
//!                             degilse grup SIGTTIN ile durur
//!
//!   arka plandan AYAR         SIGTTOU yok sayili/engelli -> IZIN VER
//!   (tcsetpgrp)               degilse grup SIGTTOU ile durur
//! ```
//!
//! Asimetri bilincli ve kabuklar ona dayaniyor: bir isi on plana
//! verdikten sonra kabuk **kendisi** arka plandadir, ve terminali geri
//! almak icin `SIGTTOU`yu engelleyip `tcsetpgrp` cagirir. Okumada ayni
//! kural olsaydi arka plan isleri sinyali engelleyerek klavyeyi
//! calabilirdi.

use crate::level0a::core::scheduler;
use crate::level0b1::signal;

/// Gorevin **denetleyen terminali**: oturum liderinin ilk penceresi.
///
/// `None` uc durumda ve ucu de ayri sebep:
///
/// * oturum terminalini **kaybetmis** (lider cikti),
/// * lider artik bu oturumun lideri degil (yuva baskasina gecti),
/// * liderin hic penceresi yok (`setsid` cagirmis bir arka plan sureci
///   ya da henuz pencere acmamis bir program).
pub fn ctty_of(task: usize) -> Option<usize> {
    if scheduler::ctty_lost(task) {
        return None;
    }
    let sid = scheduler::sid_of(task);
    if scheduler::sid_of(sid) != sid {
        return None;
    }
    crate::level0a::wm::first_window_of(sid)
}

/// `read(0)` / `poll(0)` hangi pencereyi okuyacak.
///
/// Gorevin **kendi** penceresi varsa o -- eski davranis aynen suruyor.
/// Yoksa oturumun terminali. Ikisi de yoksa `None` ve okuma dosya sonu
/// doner: tus gelecek bir yer yok, beklemek sonsuza kadar beklemek olur.
pub fn input_window_of(task: usize) -> Option<usize> {
    crate::level0a::wm::first_window_of(task).or_else(|| ctty_of(task))
}

/// Terminalden okuma kararinin sonucu.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReadVerdict {
    /// On planda ya da okunan pencere oturum terminali degil.
    Allowed,
    /// Arka planda: grup `SIGTTIN` aldi, cagri bolunmeli.
    Stopped,
    /// Arka planda ve `SIGTTIN` yok sayili/engelli: `EIO`.
    Denied,
}

/// Bir gorevin bu pencereden okumasi izinli mi.
///
/// Denetim **veri olup olmadigindan once** yapiliyor -- Linux'ta da
/// boyle. Yoksa arka plan isi, tampon dolu oldugu surece klavyeyi
/// okuyabilir ve yalnizca bos oldugunda durdurulurdu: kullanicinin
/// yazdigi satirlar rastgele bir sekilde iki is arasinda bolunurdu.
pub fn check_read(task: usize, window: usize) -> ReadVerdict {
    if ctty_of(task) != Some(window) {
        return ReadVerdict::Allowed;
    }
    let sid = scheduler::sid_of(task);
    let pgid = scheduler::pgid_of(task);
    if pgid == scheduler::foreground_of(sid) {
        return ReadVerdict::Allowed;
    }
    if signal::ignored_or_blocked(task, signal::SIGTTIN) {
        READS_DENIED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        return ReadVerdict::Denied;
    }
    // Yetim grup: `SIGTTIN` gonderilmiyor, okuma `EIO` aliyor.
    //
    // Sebep yetimligin tanimi: grubu disaridan devam ettirebilecek kimse
    // yok. Burada durdurulsa sonsuza kadar durur -- kabuk gitmis, `fg`
    // diyecek kimse kalmamis.
    if scheduler::pgrp_orphaned(pgid) {
        READS_DENIED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        return ReadVerdict::Denied;
    }
    // Butun **gruba** gidiyor: is denetimi isleri durdurur, tek tek
    // surecleri degil. Bir boru hattinin bir ucu arka plandan okursa
    // hattin tamami durmali.
    let _ = signal::raise_group(pgid, signal::SIGTTIN);
    READS_STOPPED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    ReadVerdict::Stopped
}

/// Terminal ayari degistirme kararinin sonucu.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChangeVerdict {
    Allowed,
    /// Grup `SIGTTOU` aldi, cagri bolunmeli.
    Stopped,
    /// Yetim grup: durdurulsa kimse devam ettiremez -- `EIO`.
    Orphaned,
}

/// Bir gorev terminalin ayarini degistirebilir mi (`tcsetpgrp`).
///
/// `check_read` ile ayni yapi, tek farkla: sinyal yok sayili ya da
/// engelliyse cevap **izin**. Modul basindaki asimetri.
pub fn check_change(task: usize) -> ChangeVerdict {
    let sid = scheduler::sid_of(task);
    let pgid = scheduler::pgid_of(task);
    if pgid == scheduler::foreground_of(sid) {
        return ChangeVerdict::Allowed;
    }
    if signal::ignored_or_blocked(task, signal::SIGTTOU) {
        return ChangeVerdict::Allowed;
    }
    if scheduler::pgrp_orphaned(pgid) {
        return ChangeVerdict::Orphaned;
    }
    let _ = signal::raise_group(pgid, signal::SIGTTOU);
    CHANGES_STOPPED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    ChangeVerdict::Stopped
}

/// `tcsetpgrp` / `tcgetpgrp` hatalari (POSIX karsiliklariyla).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TtyError {
    /// Tanimlayici gorevin denetleyen terminali degil (`ENOTTY`).
    NotATty,
    /// Arka planda; grup `SIGTTOU` aldi -- cagri bolunmeli.
    Stopped,
    /// Boyle bir grup yok (`ESRCH`).
    NoSuchGroup,
    /// Grup baska bir oturumda (`EPERM`).
    OtherSession,
    /// Arka plandaki yetim bir grup terminale dokundu (`EIO`).
    Orphaned,
}

/// `fd` bu gorevin terminali mi.
///
/// Yonlendirilmemis 0/1/2 terminale bagli; yonlendirilmis olanlar bir
/// dosyaya ya da boruya gidiyor ve onlarda terminal ayari anlamsiz.
fn is_terminal_fd(task: usize, fd: usize) -> bool {
    fd <= 2 && crate::level0a::core::fd::get(fd).is_none() && ctty_of(task).is_some()
}

/// `tcgetpgrp(fd)`.
///
/// Arka plandan sormak serbest; Linux da sinyal gondermez. Sormak
/// terminali degistirmiyor.
pub fn foreground_group(task: usize, fd: usize) -> Result<usize, TtyError> {
    if !is_terminal_fd(task, fd) {
        return Err(TtyError::NotATty);
    }
    Ok(scheduler::foreground_of(scheduler::sid_of(task)))
}

/// `tcsetpgrp(fd, pgid)`.
///
/// Denetimlerin sirasi Linux'unkiyle ayni ve sira gorunur bir fark
/// yaratiyor: once "bu senin terminalin mi", sonra is denetimi, en son
/// grubun kendisi. Grup denetimi ondan once yapilsaydi arka plandaki
/// bir surec var olmayan bir grup numarasiyla `ESRCH` alip **durmadan**
/// gecerdi -- yani is denetimini bir hata koduyla atlatabilirdi.
pub fn set_foreground_group(task: usize, fd: usize, pgid: usize) -> Result<(), TtyError> {
    if !is_terminal_fd(task, fd) {
        return Err(TtyError::NotATty);
    }
    match check_change(task) {
        ChangeVerdict::Allowed => {}
        ChangeVerdict::Stopped => return Err(TtyError::Stopped),
        ChangeVerdict::Orphaned => return Err(TtyError::Orphaned),
    }
    if pgid >= scheduler::MAX_TASKS || !scheduler::group_exists(pgid) {
        return Err(TtyError::NoSuchGroup);
    }
    let sid = scheduler::sid_of(task);
    if !scheduler::session_has_group(sid, pgid) {
        return Err(TtyError::OtherSession);
    }
    scheduler::set_foreground(sid, pgid);
    HANDOFFS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    Ok(())
}

// --- Oturum liderinin cikisi ----------------------------------------------

/// Terminali olan bir oturum lideri cikiyor: on plan grubuna `SIGHUP` ve
/// `SIGCONT`.
///
/// "Hangup" adi seri hatlardan geliyor: modem kapaninca terminalin
/// sahibi gitmis olur ve onda calisan isler bunu ogrenmeli. Kabuk
/// penceresi kapaninca bugun de ayni sey oluyor. Varsayilani surec
/// sonlandirmak; `nohup` tam olarak bu sinyali yok sayan bir sarmalayici.
/// `SIGCONT` ardindan geliyor ki on plandaki durmus bir is sinyali
/// gorebilsin.
///
/// **Pencere kapanmadan once** cagrilmali: terminal liderin penceresi,
/// ve cikis yolu pencereleri zamanlayiciya ulasmadan kapatiyor. Sonra
/// cagrilsaydi "liderin terminali var miydi" sorusu cevaplanamazdi.
pub fn leader_exiting(leader: usize) {
    if scheduler::sid_of(leader) != leader || scheduler::ctty_lost(leader) {
        return;
    }
    if crate::level0a::wm::first_window_of(leader).is_none() {
        return;
    }
    let fg = scheduler::foreground_of(leader);
    if fg >= scheduler::MAX_TASKS || !scheduler::group_exists(fg) {
        return;
    }
    let _ = signal::raise_group(fg, signal::SIGHUP);
    let _ = signal::raise_group(fg, signal::SIGCONT);
    HANGUPS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

// --- Olcum -------------------------------------------------------------

use core::sync::atomic::AtomicUsize;

/// Arka plandan okuyup `SIGTTIN` alan okumalar.
static READS_STOPPED: AtomicUsize = AtomicUsize::new(0);
/// `SIGTTIN` yok sayili oldugu icin `EIO` alan okumalar.
static READS_DENIED: AtomicUsize = AtomicUsize::new(0);
/// Arka plandan terminale dokunup `SIGTTOU` alanlar.
static CHANGES_STOPPED: AtomicUsize = AtomicUsize::new(0);
/// On plan devirleri (`tcsetpgrp` basarili).
static HANDOFFS: AtomicUsize = AtomicUsize::new(0);
/// Lider cikisinda on plana giden `SIGHUP`lar.
static HANGUPS: AtomicUsize = AtomicUsize::new(0);

/// `(lider SIGHUP, yetim grup SIGHUP)` -- kabuk raporu.
pub fn hangup_stats() -> (usize, usize) {
    (
        HANGUPS.load(core::sync::atomic::Ordering::Relaxed),
        scheduler::orphan_hangups(),
    )
}

/// `(SIGTTIN, EIO, SIGTTOU, devir)` -- kabuk raporu.
pub fn stats() -> (usize, usize, usize, usize) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        READS_STOPPED.load(Relaxed),
        READS_DENIED.load(Relaxed),
        CHANGES_STOPPED.load(Relaxed),
        HANDOFFS.load(Relaxed),
    )
}
