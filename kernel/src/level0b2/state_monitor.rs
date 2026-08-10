//! Durum Izleyici: Level-0a'nin heartbeat sayacini takip eder. Belirli bir
//! sure artis gozlenmezse Fallback Interface devreye girer; nabiz geri
//! gelirse denetim ana katmana geri devredilir (doc S.11 Hata Tolerans
//! Motoru).
//!
//! Nabiz **paylasimli bellek bolgesinden** okunur (doc S.10): izlenen
//! katmanin bir fonksiyonunu cagirmak, o katman kilitlendiginde izleyiciyi
//! de kilitlerdi. Izleyicinin izlenene bagimli olmamasi hata tolerans
//! motorunun onkosuludur.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::level0b2::ipc;

/// Nabiz bu kadar PIT tick'i boyunca degismezse Level-0a "Olu" sayilir.
/// PIT 100 Hz'de calistigi icin 300 tick = 3 saniye.
///
/// Olcunun **tick** olmasi onemli: onceki surumde esik idle dongusunun
/// tur sayisiydi ve dolayisiyla masaustunun kare hizina gore degisiyordu.
const DEAD_THRESHOLD_TICKS: u32 = 300;
/// "Mesgul" esigi -- olu esiginin yarisi.
const BUSY_THRESHOLD_TICKS: u32 = DEAD_THRESHOLD_TICKS / 2;

/// Doc S.2.2.A: State Monitor'un izledigi uc durum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level0aHealth {
    Healthy,
    Busy,
    Dead,
}

static LAST_HEARTBEAT: AtomicU32 = AtomicU32::new(0);
/// Nabzin en son degistigi PIT tick'i.
static LAST_BEAT_TICK: AtomicU32 = AtomicU32::new(0);
static FALLBACK_ACTIVE: AtomicBool = AtomicBool::new(false);
static LAST_REPORTED: AtomicU32 = AtomicU32::new(0);
/// Kac kez Co-Service'e dusuldu ve kac kez toparlanildi.
static DEAD_EVENTS: AtomicU32 = AtomicU32::new(0);
static RECOVERIES: AtomicU32 = AtomicU32::new(0);
/// Son duraklamanin tick cinsinden uzunlugu.
static LAST_STALL: AtomicU32 = AtomicU32::new(0);

/// Ana idle dongude her turda cagrilir.
pub fn tick() {
    let heartbeat = ipc::SHARED.heartbeat.load(Ordering::Relaxed);
    let now = ipc::SHARED.ticks.load(Ordering::Relaxed);

    if heartbeat != LAST_HEARTBEAT.load(Ordering::Relaxed) {
        LAST_HEARTBEAT.store(heartbeat, Ordering::Relaxed);
        LAST_BEAT_TICK.store(now, Ordering::Relaxed);
        // Sessizlik olcusu de sifirlanmali; aksi halde toparlanmanin hemen
        // ardindan sistem bir tur daha "Mesgul" gorunur.
        LAST_STALL.store(0, Ordering::Relaxed);

        // --- Toparlanma ---
        // Nabiz geri geldiyse Co-Service modundan cikilir. Doc S.11'in
        // "Level-0a yeniden baslatma" maddesinin bu cekirdekteki karsiligi.
        if FALLBACK_ACTIVE.swap(false, Ordering::Relaxed) {
            RECOVERIES.fetch_add(1, Ordering::Relaxed);
            crate::level0b2::fallback::level0a_recovered(LAST_STALL.load(Ordering::Relaxed));
        }
        report_health();
        return;
    }

    let stalled = now.wrapping_sub(LAST_BEAT_TICK.load(Ordering::Relaxed));
    LAST_STALL.store(stalled, Ordering::Relaxed);

    if stalled >= DEAD_THRESHOLD_TICKS && !FALLBACK_ACTIVE.swap(true, Ordering::Relaxed) {
        DEAD_EVENTS.fetch_add(1, Ordering::Relaxed);
        crate::level0b2::fallback::level0a_dead();
    }
    report_health();
}

/// Saglik durumu degistiyse IPC halkasina bir olay birakir.
fn report_health() {
    let state = match health() {
        Level0aHealth::Healthy => 0u32,
        Level0aHealth::Busy => 1,
        Level0aHealth::Dead => 2,
    };
    if LAST_REPORTED.swap(state, Ordering::Relaxed) == state {
        return;
    }
    // Olu/toparlandi olaylarini fallback zaten bildiriyor; burada yalnizca
    // "mesgul" gecisleri raporlanir.
    if state == 1 {
        ipc::post(
            ipc::Kind::HealthChange,
            0,
            1,
            0,
            "Level-0a mesgul -- nabiz gecikiyor",
        );
    }
}

/// Nabiz hic durmadiysa Healthy; gecikme baslamis ama esik asilmamissa Busy.
pub fn health() -> Level0aHealth {
    if FALLBACK_ACTIVE.load(Ordering::Relaxed) {
        return Level0aHealth::Dead;
    }
    if LAST_STALL.load(Ordering::Relaxed) > BUSY_THRESHOLD_TICKS {
        return Level0aHealth::Busy;
    }
    Level0aHealth::Healthy
}

pub fn level0a_is_dead() -> bool {
    FALLBACK_ACTIVE.load(Ordering::Relaxed)
}

/// Nabzin ne kadar suredir (tick) sessiz oldugu.
pub fn stalled_ticks() -> u32 {
    LAST_STALL.load(Ordering::Relaxed)
}

pub fn dead_events() -> u32 {
    DEAD_EVENTS.load(Ordering::Relaxed)
}

pub fn recoveries() -> u32 {
    RECOVERIES.load(Ordering::Relaxed)
}

pub fn dead_threshold_ticks() -> u32 {
    DEAD_THRESHOLD_TICKS
}
