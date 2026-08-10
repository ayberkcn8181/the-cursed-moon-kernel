//! Yuk Dengeleyici (doc S.2.2.A): "Sistem darbogaza girdiginde cagrilari
//! kuyruga alir veya dagitir."
//!
//! Tum sistem cagrilari ve istisnalar Level-0b2'den gectigi icin bu katman
//! sistemdeki **tek noktadan yuk olcumu** yapabilecek yerdir. Olculen sey
//! kanal basina cagri hizi ve **gorev basina** cagri hizidir.
//!
//! ## Neden gerekli
//!
//! Zamanlama su an isbirlikcidir (doc Faz 2 notu): bir gorev `yield`
//! cagirmadigi surece CPU'yu birakmaz. Sistem cagrisi yapan ama CPU'yu
//! birakmayan bir Ring 3 uygulamasi -- ornegin sikca `win_poll_key`
//! cagiran bir dongu -- masaustunu, kabugu ve diger uygulamalari
//! aclia surukler. Preemption gelene kadar (Faz 8) buna karsi tek
//! savunma cagri yolunun kendisidir.
//!
//! ## Ne yapiyor
//!
//! Her cagri bir **kanala** ve bir **goreve** yazilir. Saniyede bir
//! (PIT tick'leriyle) pencere devrilir ve hizlar hesaplanir. Bir gorev
//! pencere kotasini asarsa `Verdict::Throttle` doner; Dispatcher bu
//! durumda cagriyi **yerine getirmeden once** goreve zorla `yield`
//! ettirir. Cagri kaybolmaz, sadece sirasini bekler -- doc'taki
//! "kuyruga alir veya dagitir" ifadesinin bu cekirdekteki karsiligi budur.
//!
//! Kisitlama olaylari IPC halkasina birakilir; kabuktan `load` ve `ipc`
//! komutlariyla gozlenebilir.

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::level0a::core::scheduler::MAX_TASKS;
use crate::level0b2::ipc;

/// Olculen cagri kanallari.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    PosixFile = 0,
    PosixMem = 1,
    Gui = 2,
    Nt = 3,
    Exception = 4,
}

pub const CHANNELS: usize = 5;

pub const CHANNEL_NAMES: [&str; CHANNELS] = [
    "posix-dosya",
    "posix-bellek",
    "gui",
    "nt",
    "istisna",
];

/// Pencere uzunlugu (PIT 100 Hz -> 1 saniye).
const WINDOW_TICKS: u32 = 100;

/// Bir gorevin bir pencerede yapabilecegi cagri sayisi. Normal bir GUI
/// uygulamasi kare basina birkac cagri yapar (~60-600/sn); bu kota
/// yalnizca gercekten donen bir dongude asilir.
const TASK_QUOTA: u32 = 4000;

/// Bir kanalin "doygun" sayilmasi icin gereken saniyelik hiz.
const CHANNEL_SATURATION: u32 = 8000;

macro_rules! atomic_array {
    ($n:expr) => {
        [const { AtomicU32::new(0) }; $n]
    };
}

static TOTAL: [AtomicU32; CHANNELS] = atomic_array!(CHANNELS);
static WINDOW: [AtomicU32; CHANNELS] = atomic_array!(CHANNELS);
static RATE: [AtomicU32; CHANNELS] = atomic_array!(CHANNELS);
static PEAK: [AtomicU32; CHANNELS] = atomic_array!(CHANNELS);

static TASK_TOTAL: [AtomicU32; MAX_TASKS] = atomic_array!(MAX_TASKS);
static TASK_WINDOW: [AtomicU32; MAX_TASKS] = atomic_array!(MAX_TASKS);
static TASK_RATE: [AtomicU32; MAX_TASKS] = atomic_array!(MAX_TASKS);
static TASK_THROTTLES: [AtomicU32; MAX_TASKS] = atomic_array!(MAX_TASKS);
/// Gorev basina son kisitlama raporunun pencere numarasi -- IPC halkasini
/// saniyede bir olayla bogmamak icin.
static TASK_LAST_REPORT: [AtomicU32; MAX_TASKS] = atomic_array!(MAX_TASKS);

static WINDOW_START: AtomicU32 = AtomicU32::new(0);
static WINDOWS_ROLLED: AtomicU32 = AtomicU32::new(0);
static THROTTLES: AtomicU32 = AtomicU32::new(0);
static SATURATED: AtomicUsize = AtomicUsize::new(0); // kanal bit maskesi
static BUSIEST_TASK: AtomicUsize = AtomicUsize::new(0);

/// Dispatcher'a verilen karar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Cagri dogrudan islensin.
    Pass,
    /// Gorev kotasini asti: once CPU'yu biraksin, sonra islensin.
    Throttle,
}

/// Bir cagriyi kaydeder ve ne yapilmasi gerektigini soyler.
pub fn note_call(channel: Channel, task: usize) -> Verdict {
    let index = channel as usize;
    TOTAL[index].fetch_add(1, Ordering::Relaxed);
    WINDOW[index].fetch_add(1, Ordering::Relaxed);

    if task >= MAX_TASKS {
        return Verdict::Pass;
    }

    TASK_TOTAL[task].fetch_add(1, Ordering::Relaxed);
    let in_window = TASK_WINDOW[task].fetch_add(1, Ordering::Relaxed) + 1;

    if in_window <= TASK_QUOTA {
        return Verdict::Pass;
    }

    THROTTLES.fetch_add(1, Ordering::Relaxed);
    let hits = TASK_THROTTLES[task].fetch_add(1, Ordering::Relaxed) + 1;

    // Rapor sikligi: gorev basina en fazla `REPORT_EVERY` pencerede bir.
    // Kota asildigi ilk cagrida bakilir; boylece kisitlama karari sicak
    // yolda ek is yapmaz.
    const REPORT_EVERY: u32 = 5;
    if in_window == TASK_QUOTA + 1 {
        let window = WINDOWS_ROLLED.load(Ordering::Relaxed);
        let last = TASK_LAST_REPORT[task].load(Ordering::Relaxed);
        if last == 0 || window.wrapping_sub(last) >= REPORT_EVERY {
            TASK_LAST_REPORT[task].store(window.max(1), Ordering::Relaxed);
            ipc::post(
                ipc::Kind::Throttled,
                task,
                in_window as usize,
                hits as usize,
                "gorev cagri kotasini asti, CPU birakmaya zorlandi",
            );
        }
    }

    Verdict::Throttle
}

/// PIT kesmesinden her tick'te cagrilir; pencereyi zamani gelince devirir.
///
/// Zamanlayiciya bagli olmasi bilincli: olcum, olculen isin (scheduler
/// dongusu) ilerlemesine bagli olmamalidir. Donen bir gorev idle dongusunu
/// hic calistirmasa bile pencere yine de devrilir.
pub fn on_tick(ticks: u32) {
    let start = WINDOW_START.load(Ordering::Relaxed);
    if ticks.wrapping_sub(start) < WINDOW_TICKS {
        return;
    }
    WINDOW_START.store(ticks, Ordering::Relaxed);
    WINDOWS_ROLLED.fetch_add(1, Ordering::Relaxed);

    let mut mask = 0usize;
    for i in 0..CHANNELS {
        let count = WINDOW[i].swap(0, Ordering::Relaxed);
        RATE[i].store(count, Ordering::Relaxed);
        PEAK[i].fetch_max(count, Ordering::Relaxed);
        if count >= CHANNEL_SATURATION {
            mask |= 1 << i;
        }
    }
    SATURATED.store(mask, Ordering::Relaxed);

    let mut busiest = 0usize;
    let mut busiest_rate = 0u32;
    for t in 0..MAX_TASKS {
        let count = TASK_WINDOW[t].swap(0, Ordering::Relaxed);
        TASK_RATE[t].store(count, Ordering::Relaxed);
        if count > busiest_rate {
            busiest_rate = count;
            busiest = t;
        }
    }
    BUSIEST_TASK.store(busiest, Ordering::Relaxed);
}

// --- Kabuk/rapor arayuzu ------------------------------------------------

pub fn total(channel: usize) -> u32 {
    TOTAL[channel].load(Ordering::Relaxed)
}

pub fn rate(channel: usize) -> u32 {
    RATE[channel].load(Ordering::Relaxed)
}

pub fn peak(channel: usize) -> u32 {
    PEAK[channel].load(Ordering::Relaxed)
}

pub fn task_total(task: usize) -> u32 {
    TASK_TOTAL[task].load(Ordering::Relaxed)
}

pub fn task_rate(task: usize) -> u32 {
    TASK_RATE[task].load(Ordering::Relaxed)
}

pub fn task_throttles(task: usize) -> u32 {
    TASK_THROTTLES[task].load(Ordering::Relaxed)
}

pub fn throttles() -> u32 {
    THROTTLES.load(Ordering::Relaxed)
}

pub fn windows_rolled() -> u32 {
    WINDOWS_ROLLED.load(Ordering::Relaxed)
}

pub fn busiest_task() -> usize {
    BUSIEST_TASK.load(Ordering::Relaxed)
}

pub fn is_saturated(channel: usize) -> bool {
    SATURATED.load(Ordering::Relaxed) & (1 << channel) != 0
}

pub fn any_saturated() -> bool {
    SATURATED.load(Ordering::Relaxed) != 0
}

pub fn quota() -> u32 {
    TASK_QUOTA
}

/// Toplam cagri sayisi -- tum kanallar.
pub fn call_count() -> u32 {
    (0..CHANNELS).map(|i| TOTAL[i].load(Ordering::Relaxed)).sum()
}
