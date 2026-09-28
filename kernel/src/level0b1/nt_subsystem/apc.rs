//! Windows APC kuyruklari -- `QueueUserAPC` ve **uyarilabilir** bekleme.
//!
//! ## POSIX sinyalinin Windows'taki en yakin akrabasi
//!
//! Ikisi de ayni ise yariyor: bir akisa "su an ne yapiyorsan, su
//! fonksiyonu da calistir" demek. Ama sozlesmeleri bir noktada keskin
//! bicimde ayriliyor ve o nokta **teslim ani**:
//!
//! ```text
//!   POSIX sinyal  ->  her sistem cagrisi donusunde teslim edilir
//!                     (surece sorulmaz)
//!   Windows APC   ->  yalnizca UYARILABILIR bir bekleme noktasinda
//!                     teslim edilir (surec izin vermis olmali)
//! ```
//!
//! Sonucu somut: hicbir zaman `SleepEx(..., TRUE)` ya da
//! `WaitForSingleObjectEx(..., TRUE)` cagirmayan bir Windows akisi,
//! kuyrugunda kac APC olursa olsun **hicbirini calistirmaz**. POSIX'te
//! bunun karsiligi yok -- bir sinyal, isleyicisi kuruluysa, programin
//! izni olmadan akisi keser.
//!
//! Ayrim bir eksiklik degil, iki ayri tasarim karari:
//!
//! ```text
//!   sinyal ->  kesinti.  "su an bolunebilirsin" varsayilan.
//!              Program bolunmek istemiyorsa MASKELER (sigprocmask).
//!   APC    ->  randevu.  "su an bolunebilirsin" ozel olarak SOYLENIR.
//!              Program hicbir sey soylemezse hic bolunmez.
//! ```
//!
//! Birincisi yeniden-girilebilirlik sorununu programa yikiyor: bir
//! sinyal isleyicisi kritik bolgenin ortasinda kosabilir. Ikincisi o
//! sorunu tasarim geregi yok ediyor -- ama bedeli, bir akisin APC'sini
//! **hic** gormeme ihtimali.
//!
//! ## Kuyruk her zaman kuyruk
//!
//! POSIX'te birlesen (standart) ve kuyruklanan (gercek-zamanli) diye iki
//! sinif sinyal var. Windows'ta boyle bir ayrim yok: her APC kuyruga
//! girer ve her APC bir deger tasir (`dwData`). Yani "ucuz ve birlesen"
//! bir bildirim yolu hic olmadi -- bkz. README.
//!
//! ## Teslim nasil oluyor
//!
//! Sinyal tarafiyla ayni ilkel: cekirdek kullanici yiginina bir cagri
//! cercevesi kurup baglami APC yordamina ceviriyor, ve donus adresi
//! olarak TEB'deki **APC tramplenini** koyuyor. Yordam `ret` edince
//! tramplen `NtContinueApc` cagiriyor; cekirdek kuyrukta baska varsa
//! sirakini kuruyor, yoksa saklanan baglami geri koyuyor ve bekleme
//! cagrisina `WAIT_IO_COMPLETION` donduruyor.

use crate::arch::cpu::regs::{SyscallFrame, UserContext};
use crate::level0a::core::{mmu, scheduler};
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use super::{seh, teb};

/// Bir akisin kuyrugunda bekleyebilecek en fazla APC.
///
/// Gercek Windows'ta belgelenmis bir sinir yok; kuyruk cekirdek
/// bellegiyle sinirli. TCMK'de sabit, cunku kuyruk statik bir tabloda
/// duruyor -- ve sinirsiz olsaydi gonderen taraf hedefin adina bellek
/// ayirtabilirdi. Doldugunda `QueueUserAPC` sifir (basarisiz) doner.
pub const MAX_APC: usize = 8;

/// Uyarilabilir bir beklemenin **APC yuzunden** bittigini soyleyen donus.
///
/// Windows'un `WAIT_IO_COMPLETION`i (= `STATUS_USER_APC`). Adi tarihsel:
/// uyarilabilir beklemenin ilk musterisi ortusen (`overlapped`) dosya
/// islemlerinin tamamlanma yordamlariydi. `QueueUserAPC` ayni yolu
/// uygulamalara acti, ama ad kaldi.
pub const WAIT_IO_COMPLETION: usize = 0x0000_00C0;

/// Kuyruktaki bir APC: hangi yordam, hangi degerle.
#[derive(Clone, Copy)]
struct Apc {
    /// Ring 3'teki `PAPCFUNC`.
    proc_addr: usize,
    /// `dwData` -- yordama tek arguman olarak gidiyor.
    param: usize,
}

impl Apc {
    const EMPTY: Self = Apc {
        proc_addr: 0,
        param: 0,
    };
}

/// Akis basina kuyruk -- **varis sirasinda** dolu bir on ek.
///
/// Sikistiran dizi, halka degil: cikarma her zaman bastan oldugu icin
/// halka da olabilirdi, ama sekiz ogede kaydirmanin maliyeti olcume
/// girmiyor ve dizi hali `fork`/cikis temizliginde daha sade.
static mut QUEUE: [[Apc; MAX_APC]; scheduler::MAX_TASKS] =
    [[Apc::EMPTY; MAX_APC]; scheduler::MAX_TASKS];

/// Kuyrukta bekleyen APC sayisi (akis basina).
static QUEUED: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Su an bir APC kosuyor mu (akis basina).
///
/// Ic ice teslimi engelliyor: bir APC yordami kendi icinde uyarilabilir
/// bekleme cagirirsa ikinci APC **calismaz**, kuyrukta kalir. Gercek
/// Windows ic ice teslim yapar; buradaki sinir bilincli ve sebebi tek
/// yuvali saklama (asagi bkz. `SAVED`).
static RUNNING: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// APC'ye girilmeden onceki Ring 3 baglami.
///
/// Akis basina **tek** yuva: ic ice teslim olmadigi icin ikincisine
/// gerek yok. Sinyal tarafinda dort katmanli bir yigin var, cunku orada
/// teslim programin izni olmadan olur ve bir isleyicinin icinde ikinci
/// bir sinyal beklenen bir durumdur. Burada teslim noktasini program
/// kendisi seciyor, yani ic ice kalmak da onun karari.
static mut SAVED: [UserContext; scheduler::MAX_TASKS] =
    [UserContext::ZERO; scheduler::MAX_TASKS];

/// Olcum sayaclari (kabuk `sigs` raporu).
static QUEUED_TOTAL: AtomicU32 = AtomicU32::new(0);
static DELIVERED: AtomicU32 = AtomicU32::new(0);
static REFUSED: AtomicU32 = AtomicU32::new(0);
/// Kuyrugun gordugu en buyuk derinlik.
static PEAK: AtomicU32 = AtomicU32::new(0);

/// `QueueUserAPC`nin reddetme sebepleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApcError {
    /// Tutamac bir akisa cozulmedi ya da akis yasamiyor.
    NoSuchThread,
    /// Yordam adresi Ring 3'ten erisilebilir degil.
    BadProc,
    /// Kuyruk dolu.
    QueueFull,
}

/// Bir akisin kuyruguna APC ekler.
///
/// Hedefin **baska** bir akis olabilmesi cagrinin varlik sebebi:
/// `QueueUserAPC`nin asil isi bir akisa is yaptirmaktir, kendine degil.
pub fn queue(target: usize, proc_addr: usize, param: usize) -> Result<(), ApcError> {
    if target >= scheduler::MAX_TASKS {
        return Err(ApcError::NoSuchThread);
    }
    match scheduler::state_of(target) {
        scheduler::TaskState::Unused | scheduler::TaskState::Terminated => {
            return Err(ApcError::NoSuchThread)
        }
        _ => {}
    }
    // Yordam kullanici alaninda olmali; aksi halde cekirdek, bir
    // programin istegiyle kendi kodunun icine dallanirdi. Sinyal
    // tarafindaki `set_handler` denetimiyle ayni kural.
    if proc_addr == 0 || !mmu::is_user_accessible(proc_addr) {
        return Err(ApcError::BadProc);
    }

    crate::arch::cpu::without_interrupts(|| {
        let used = QUEUED[target].load(Ordering::SeqCst);
        if used >= MAX_APC {
            REFUSED.fetch_add(1, Ordering::Relaxed);
            return Err(ApcError::QueueFull);
        }
        // SAFETY: yuva akis ve indekse ozel, sinir yukarida denetlendi.
        unsafe {
            (core::ptr::addr_of_mut!(QUEUE) as *mut Apc)
                .add(target * MAX_APC + used)
                .write(Apc { proc_addr, param });
        }
        QUEUED[target].store(used + 1, Ordering::SeqCst);
        QUEUED_TOTAL.fetch_add(1, Ordering::Relaxed);
        PEAK.fetch_max(used as u32 + 1, Ordering::Relaxed);
        Ok(())
    })
}

/// Kuyrugun basindaki APC'yi cikarir.
fn pop(task: usize) -> Option<Apc> {
    crate::arch::cpu::without_interrupts(|| {
        let used = QUEUED[task].load(Ordering::SeqCst);
        if used == 0 {
            return None;
        }
        // SAFETY: yalnizca 0..used araligi okunuyor.
        unsafe {
            let row = (core::ptr::addr_of_mut!(QUEUE) as *mut Apc).add(task * MAX_APC);
            let first = row.read();
            for i in 0..used - 1 {
                row.add(i).write(row.add(i + 1).read());
            }
            row.add(used - 1).write(Apc::EMPTY);
            QUEUED[task].store(used - 1, Ordering::SeqCst);
            Some(first)
        }
    })
}

/// Akisin kuyrugunda kac APC bekliyor.
pub fn pending(task: usize) -> usize {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    QUEUED[task].load(Ordering::Relaxed)
}

/// Kuyrugu bosaltir (`execve`, akis cikisi, yeni imaj).
pub fn reset(task: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| {
        QUEUED[task].store(0, Ordering::SeqCst);
        RUNNING[task].store(0, Ordering::SeqCst);
        // SAFETY: butun satir kendi yuvasi.
        unsafe {
            let row = (core::ptr::addr_of_mut!(QUEUE) as *mut Apc).add(task * MAX_APC);
            for i in 0..MAX_APC {
                row.add(i).write(Apc::EMPTY);
            }
        }
    });
}

/// `(kuyruga giren, teslim edilen, reddedilen, en derin)` -- kabuk raporu.
pub fn stats() -> (u32, u32, u32, u32) {
    (
        QUEUED_TOTAL.load(Ordering::Relaxed),
        DELIVERED.load(Ordering::Relaxed),
        REFUSED.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed),
    )
}

/// Uyarilabilir bir bekleme noktasi: bekleyen APC varsa ilkini calistirir.
///
/// Doner: `true` ise cerceve APC yordamina cevrildi ve cagiran **hemen
/// donmeli** -- bekleme cagrisinin donus degeri artik burada degil,
/// `resume`da uretilecek.
///
/// `false` donmesinin uc sebebi olabilir ve ucu de "APC kosmadi"
/// demektir: kuyruk bos, zaten bir APC kosuyor, ya da cerceve
/// kurulamadi (yigin yazilamiyor).
///
/// # Safety
/// `frame` Ring 3'ten gelen gecerli bir cerceve olmali ve cagiran
/// gorevin adres uzayi etkin olmalidir.
pub unsafe fn deliver(frame: &mut SyscallFrame, from_interrupt: bool) -> bool {
    let task = scheduler::current_id();
    if task >= scheduler::MAX_TASKS {
        return false;
    }
    // Zaten bir APC kosuyorsa ikincisi beklesin. Kuyrukta kaliyor, yani
    // kaybolmuyor: bir sonraki uyarilabilir beklemede calisacak.
    if RUNNING[task].load(Ordering::SeqCst) != 0 {
        return false;
    }
    if QUEUED[task].load(Ordering::SeqCst) == 0 {
        return false;
    }

    // Baglam **cikarmadan once** saklaniyor: cerceve kurulamazsa APC
    // kuyrukta kalmali, yoksa sessizce kaybolurdu.
    let saved = frame.user_context_via(from_interrupt);
    let Some(next) = build(task, &saved) else {
        return false;
    };

    (core::ptr::addr_of_mut!(SAVED) as *mut UserContext)
        .add(task)
        .write(saved);
    RUNNING[task].store(1, Ordering::SeqCst);
    DELIVERED.fetch_add(1, Ordering::Relaxed);
    frame.set_user_context_via(from_interrupt, &next);
    true
}

/// APC yordami dondu (`NtContinueApc`): sirakini kur ya da beklemeyi bitir.
///
/// Doner: `false` ise cagiran APC icinde degildi -- yani kullanici
/// tramplen olmadan bu cagriyi yapmis. O durumda cerceve degistirilmez.
///
/// # Safety
/// `deliver` ile ayni kosul.
pub unsafe fn resume(frame: &mut SyscallFrame, from_interrupt: bool) -> bool {
    let task = scheduler::current_id();
    if task >= scheduler::MAX_TASKS || RUNNING[task].load(Ordering::SeqCst) == 0 {
        return false;
    }

    let saved = (core::ptr::addr_of!(SAVED) as *const UserContext)
        .add(task)
        .read();

    // Kuyrukta baska varsa **ayni** beklemenin icinde kosuyor: Windows
    // uyarilabilir bir beklemede bekleyen APC'lerin hepsini bosaltir,
    // sonra tek bir `WAIT_IO_COMPLETION` doner.
    if QUEUED[task].load(Ordering::SeqCst) > 0 {
        if let Some(next) = build(task, &saved) {
            DELIVERED.fetch_add(1, Ordering::Relaxed);
            frame.set_user_context_via(from_interrupt, &next);
            return true;
        }
        // Cerceve kurulamadi: kalanlar kuyrukta kaliyor ve bekleme
        // yine de bitiyor. Burada takilip kalmak, geri donusu olmayan
        // bir dongu olurdu.
    }

    RUNNING[task].store(0, Ordering::SeqCst);
    frame.set_user_context_via(from_interrupt, &saved);
    // Baglam geri konduktan **sonra** donus degeri yaziliyor: saklanan
    // baglamdaki EAX/RAX, yarida kalan bekleme cagrisinin ara degeriydi.
    // Sirayi ters cevirmek `WAIT_IO_COMPLETION`i ezerdi.
    frame.set_return(WAIT_IO_COMPLETION);
    true
}

/// Kuyrugun basindaki APC icin Ring 3 cagri cercevesini kurar.
///
/// `None` ise yigina yazilamiyor; APC kuyrukta kalir.
unsafe fn build(task: usize, context: &UserContext) -> Option<UserContext> {
    let trampoline = teb::apc_trampoline(task);
    if trampoline == 0 {
        // TEB yok, yani bu bir PE degil. Bir ELF surecine APC kuyruga
        // girmis olamaz (cagri yalnizca Win32 yuzunde var), ama
        // cerceveyi tramplensiz kurmak donusu olmayan bir dallanma
        // olurdu.
        return None;
    }
    // Cerceve, kesilen yiginin **altina** kuruluyor. Arada birakilan
    // bosluk Win64'un kirmizi bolgesi degil (Windows'ta o yok);
    // hizalama paylari ve yordamin kendi cercevesi icin.
    let sp = context.stack_pointer();
    if sp < RESERVE {
        return None;
    }
    let base = (sp - RESERVE) & !0xF;
    let apc = pop(task)?;
    match seh::build_call_frame(base, apc.proc_addr, trampoline, &[apc.param]) {
        Some(next) => Some(next),
        None => {
            // Cerceve kurulamadi: APC **geri konuyor**, yoksa kuyruktan
            // cikarilmis ve hic calismamis olurdu.
            let _ = queue(task, apc.proc_addr, apc.param);
            None
        }
    }
}

/// Cerceve icin kesilen yiginin altinda birakilan pay.
const RESERVE: usize = 256;
