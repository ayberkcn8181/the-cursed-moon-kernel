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
}

impl Disposition {
    const DEFAULT: Self = Disposition {
        handler: SIG_DFL,
        restorer: 0,
    };
}

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

/// Isleyiciye girilmeden onceki Ring 3 baglami. `sigreturn` bunu geri
/// koyar. Tek katman: isleyici icinde yeni sinyal teslim edilmedigi icin
/// yigin gerekmiyor.
static mut SAVED: [UserContext; scheduler::MAX_TASKS] =
    [UserContext::ZERO; scheduler::MAX_TASKS];
static mut IN_HANDLER: [bool; scheduler::MAX_TASKS] = [false; scheduler::MAX_TASKS];

/// Teslim edilen sinyal sayaci (kabuk `sigs` komutu icin).
static DELIVERED: AtomicU32 = AtomicU32::new(0);
static SENT: AtomicU32 = AtomicU32::new(0);

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
        slot.write(Disposition { handler, restorer });
        Ok(old)
    })
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
        (core::ptr::addr_of_mut!(IN_HANDLER) as *mut bool)
            .add(child)
            .write(false);
    });
    PENDING[child].store(0, Ordering::SeqCst);
}

/// `execve` ve gorev cikisi: yeni imaj eski isleyicileri devralmaz --
/// adresleri zaten baska bir programa aittir.
pub fn reset(task: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let base = core::ptr::addr_of_mut!(DISPOSITIONS) as *mut Disposition;
        let width = MAX_SIGNAL as usize + 1;
        for i in 0..width {
            base.add(task * width + i).write(Disposition::DEFAULT);
        }
        (core::ptr::addr_of_mut!(IN_HANDLER) as *mut bool)
            .add(task)
            .write(false);
    });
    PENDING[task].store(0, Ordering::SeqCst);
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
    // Isleyicinin icindeyken yeni sinyal teslim edilmez: tek katmanli
    // `SAVED` yuvasi ezilir ve surec asla eski baglamina donemezdi.
    if (core::ptr::addr_of!(IN_HANDLER) as *const bool).add(task).read() {
        return;
    }

    loop {
        let mask = PENDING[task].load(Ordering::SeqCst);
        if mask == 0 {
            return;
        }
        let signo = mask.trailing_zeros();
        PENDING[task].fetch_and(!(1 << signo), Ordering::SeqCst);

        let width = MAX_SIGNAL as usize + 1;
        let d = (core::ptr::addr_of!(DISPOSITIONS) as *const Disposition)
            .add(task * width + signo as usize)
            .read();

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
                let mut context = frame.user_context();
                (core::ptr::addr_of_mut!(SAVED) as *mut UserContext)
                    .add(task)
                    .write(context);
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
                (core::ptr::addr_of_mut!(IN_HANDLER) as *mut bool)
                    .add(task)
                    .write(true);
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
    let in_handler = (core::ptr::addr_of!(IN_HANDLER) as *const bool).add(task);
    if !in_handler.read() {
        // Isleyici icinde degilken `sigreturn` cagirmak, kullanicinin
        // rastgele bir baglama zipladigi anlamina gelirdi.
        return false;
    }
    let context = (core::ptr::addr_of!(SAVED) as *const UserContext)
        .add(task)
        .read();
    frame.set_user_context(&context);
    (core::ptr::addr_of_mut!(IN_HANDLER) as *mut bool)
        .add(task)
        .write(false);
    true
}
