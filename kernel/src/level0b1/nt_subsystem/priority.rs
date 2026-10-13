//! Windows oncelik siniflari: iki katmanli oncelik.
//!
//! POSIX'te oncelik **tek sayidir**: gorev basina bir `nice`. Windows'ta
//! iki katmandan olusur ve ikisi ayri yerlerde durur:
//!
//! ```text
//!   surec SINIFI   (SetPriorityClass)   -> taban: 4, 6, 8, 10, 13
//!   akis DUZEYI    (SetThreadPriority)  -> taban + (-2 .. +2)
//!                                          ya da uclar: 1 / 15
//! ```
//!
//! Etkin oncelik ikisinin **toplamidir**. Sonucu ince bir sey: akis
//! duzeyi **goreli** bir degerdir. Bir akisi `ABOVE_NORMAL` yapip sonra
//! surecin sinifini degistirirsen akis hala `ABOVE_NORMAL`dir -- yeni
//! sinifin bir ustunde. Duzey akista, sinif surecte saklanmak zorunda;
//! ikisini tek bir `nice`e cevirip saklamak bu bilgiyi kaybederdi.
//!
//! TCMK uzun sure tam olarak oyle yapiyordu: `SetThreadPriority` duzeyi
//! dogrudan `nice`e ceviriyor, `GetThreadPriority` onu `nice`ten geri
//! hesapliyordu. Sinif olmadigi surece fark gorunmuyordu.
//!
//! ## Etkin oncelik -> `nice`
//!
//! Windows'un 1..15 araligi TCMK'nin `nice`ine sira koruyan bir tabloyla
//! iniyor. `NORMAL` sinifinin yedi duzeyi eski eslemenin aynisi -- yani
//! sinif kullanmayan programlar icin hicbir sey degismiyor.
//!
//! ## `REALTIME`
//!
//! Gercek Windows'ta `REALTIME_PRIORITY_CLASS` ayricalik ister
//! (`SeIncreaseBasePriorityPrivilege`); ayricaligi olmayan surec icin
//! cagri **basarili doner** ama sinif sessizce `HIGH` olur. TCMK'de
//! ayricalik modeli yok, yani her surec o ayricaliksiz surectir: ayni
//! davranis.

use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use crate::level0a::core::scheduler;

pub const IDLE_PRIORITY_CLASS: u32 = 0x0000_0040;
pub const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
pub const NORMAL_PRIORITY_CLASS: u32 = 0x0000_0020;
pub const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;
pub const HIGH_PRIORITY_CLASS: u32 = 0x0000_0080;
pub const REALTIME_PRIORITY_CLASS: u32 = 0x0000_0100;

/// `CreateProcess`in `dwCreationFlags`inda sinif bitleri.
const CLASS_MASK: u32 = IDLE_PRIORITY_CLASS
    | BELOW_NORMAL_PRIORITY_CLASS
    | NORMAL_PRIORITY_CLASS
    | ABOVE_NORMAL_PRIORITY_CLASS
    | HIGH_PRIORITY_CLASS
    | REALTIME_PRIORITY_CLASS;

/// Gecerli akis duzeyleri. Windows bunlarin disindakileri
/// `ERROR_INVALID_PARAMETER` ile reddeder.
pub const THREAD_PRIORITY_IDLE: i32 = -15;
pub const THREAD_PRIORITY_TIME_CRITICAL: i32 = 15;

/// Surec basina sinif, **grup** numarasiyla indeksli. 0 = henuz
/// atanmadi = `NORMAL`.
static CLASS: [AtomicU32; scheduler::MAX_TASKS] = [const { AtomicU32::new(0) }; scheduler::MAX_TASKS];
/// Akis basina goreli duzey.
static LEVEL: [AtomicI32; scheduler::MAX_TASKS] = [const { AtomicI32::new(0) }; scheduler::MAX_TASKS];

/// Sinifin taban onceligi.
fn base_of(class: u32) -> i32 {
    match class {
        IDLE_PRIORITY_CLASS => 4,
        BELOW_NORMAL_PRIORITY_CLASS => 6,
        ABOVE_NORMAL_PRIORITY_CLASS => 10,
        HIGH_PRIORITY_CLASS => 13,
        _ => 8,
    }
}

/// Bir sinif sabiti gecerli mi; gecerliyse **saklanacak** degeri doner.
///
/// `REALTIME` -> `HIGH`: modul basina bakiniz.
fn normalize(class: u32) -> Option<u32> {
    match class {
        IDLE_PRIORITY_CLASS
        | BELOW_NORMAL_PRIORITY_CLASS
        | NORMAL_PRIORITY_CLASS
        | ABOVE_NORMAL_PRIORITY_CLASS
        | HIGH_PRIORITY_CLASS => Some(class),
        REALTIME_PRIORITY_CLASS => Some(HIGH_PRIORITY_CLASS),
        _ => None,
    }
}

/// Etkin oncelik (1..15).
///
/// Uclar ozel: `IDLE` duzeyi sinif ne olursa olsun 1, `TIME_CRITICAL`
/// 15'tir (gercek zamanli olmayan siniflarda). Aradakiler tabana eklenip
/// 1..15'e kirpiliyor -- Windows'un "dinamik araliktan cikamazsin"
/// kurali.
fn absolute(class: u32, level: i32) -> i32 {
    match level {
        THREAD_PRIORITY_IDLE => 1,
        THREAD_PRIORITY_TIME_CRITICAL => 15,
        _ => (base_of(class) + level).clamp(1, 15),
    }
}

/// Etkin oncelik -> `nice`. Sira koruyan bir tablo; 6..10 (yani
/// `NORMAL` sinifinin `LOWEST`..`HIGHEST` duzeyleri) eski eslemeyle
/// birebir ayni.
fn nice_of_absolute(priority: i32) -> i8 {
    match priority {
        i32::MIN..=1 => 19,
        2 => 17,
        3 => 15,
        4 => 13,
        5 => 11,
        6 => 10,
        7 => 5,
        8 => 0,
        9 => -5,
        10 => -10,
        11 => -12,
        12 => -14,
        13 => -16,
        14 => -18,
        _ => -20,
    }
}

/// Gorevin surecinin sinifi.
pub fn class_of_task(task: usize) -> u32 {
    let group = scheduler::group_of(task);
    if group >= scheduler::MAX_TASKS {
        return NORMAL_PRIORITY_CLASS;
    }
    match CLASS[group].load(Ordering::Relaxed) {
        0 => NORMAL_PRIORITY_CLASS,
        c => c,
    }
}

/// Akisin goreli duzeyi.
pub fn level_of(task: usize) -> i32 {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    LEVEL[task].load(Ordering::Relaxed)
}

/// Sinif ve duzeyi zamanlayiciya uygular.
pub fn apply(task: usize) {
    let nice = nice_of_absolute(absolute(class_of_task(task), level_of(task)));
    let _ = scheduler::set_nice(task, nice);
}

/// `SetThreadPriority`. Gecersiz duzey -> `false`.
pub fn set_level(task: usize, level: i32) -> bool {
    if task >= scheduler::MAX_TASKS {
        return false;
    }
    let valid = matches!(level, -2..=2) || level == THREAD_PRIORITY_IDLE || level == THREAD_PRIORITY_TIME_CRITICAL;
    if !valid {
        return false;
    }
    LEVEL[task].store(level, Ordering::Relaxed);
    apply(task);
    true
}

/// `SetPriorityClass`. Gecersiz sinif -> `false`, hicbir sey degismez.
///
/// Surecin **butun** akislari yeniden hesaplaniyor ve her biri kendi
/// goreli duzeyini koruyor -- iki katmanin ayri tutulmasinin sebebi bu.
pub fn set_class(task: usize, class: u32) -> bool {
    let Some(stored) = normalize(class) else {
        return false;
    };
    let group = scheduler::group_of(task);
    if group >= scheduler::MAX_TASKS {
        return false;
    }
    CLASS[group].store(stored, Ordering::Relaxed);
    for i in 0..scheduler::task_count() {
        match scheduler::state_of(i) {
            scheduler::TaskState::Unused | scheduler::TaskState::Terminated => continue,
            _ => {}
        }
        if scheduler::group_of(i) == group {
            apply(i);
        }
    }
    true
}

/// Yeni bir surecin sinifi (`CreateProcess`).
///
/// Windows'un kurali: bayraklarda bir sinif verildiyse o. Verilmediyse
/// `NORMAL` -- **ancak** olusturan surec `IDLE` ya da `BELOW_NORMAL`
/// ise cocuk o sinifi devralir. Asimetri bilincli: dusuk oncelikli bir
/// arka plan isinin cocuklari da arka planda kalmali, ama yuksek
/// oncelikli bir surec cocuklarina bu ayricaligi kendiliginden
/// gecirmemeli.
///
/// Bayraklar onceden `creation_flags_valid` ile denetlenmis olmali.
pub fn assign_new_process(child: usize, parent: usize, creation_flags: u32) {
    let requested = creation_flags & CLASS_MASK;
    let class = if requested == 0 {
        match class_of_task(parent) {
            c @ (IDLE_PRIORITY_CLASS | BELOW_NORMAL_PRIORITY_CLASS) => c,
            _ => NORMAL_PRIORITY_CLASS,
        }
    } else {
        normalize(requested).unwrap_or(NORMAL_PRIORITY_CLASS)
    };
    set_class(child, class);
}

/// `CreateProcess` bayraklarindaki sinif bitleri gecerli mi.
///
/// Birden fazla sinif biti Windows'ta `ERROR_INVALID_PARAMETER`dir ve
/// surec **hic yaratilmaz**. Denetimin yaratmadan once yapilmasi bu
/// yuzden: sonra yapilsaydi reddedilen bir cagri arkasinda calisan bir
/// surec birakirdi.
pub fn creation_flags_valid(creation_flags: u32) -> bool {
    (creation_flags & CLASS_MASK).count_ones() <= 1
}

/// Yuva yeniden kullaniliyor: onceki sahibin sinifi ve duzeyi kalmamali.
///
/// Is denetimi batisinda stdin bayraklarinda tam olarak bu hata
/// bulunmustu; ayni tuzaga burada bastan girilmiyor.
pub fn reset(task: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    CLASS[task].store(0, Ordering::Relaxed);
    LEVEL[task].store(0, Ordering::Relaxed);
}
