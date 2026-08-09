//! Durum Izleyici: Level-0a'nin heartbeat sayacini takip eder. N donguluk
//! artis gozlenmezse Fallback Interface devreye girer (doc S.11 Hata
//! Tolerans Motoru).

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Idle dongude bu kadar ardisik `tick()` cagrisinda heartbeat degismezse
/// Level-0a "Olu" sayilir. PIT ~100 Hz'de tetiklendigi ve idle dongu her
/// PIT kesmesinde bir `hlt`'ten uyandigi icin bu, kabaca ~5 saniyeye denk
/// gelir.
const DEAD_THRESHOLD: u32 = 500;

/// Doc S.2.2.A: State Monitor'un izledigi uc durum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level0aHealth {
    Healthy,
    Busy,
    Dead,
}

static LAST_HEARTBEAT: AtomicU32 = AtomicU32::new(0);
static STALE_TICKS: AtomicU32 = AtomicU32::new(0);
static FALLBACK_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Ana idle dongude her turda cagrilir.
pub fn tick() {
    let current = crate::level0a::pit::heartbeat();
    let last = LAST_HEARTBEAT.load(Ordering::Relaxed);

    if current != last {
        LAST_HEARTBEAT.store(current, Ordering::Relaxed);
        STALE_TICKS.store(0, Ordering::Relaxed);
        return;
    }

    let stale = STALE_TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    if stale >= DEAD_THRESHOLD && !FALLBACK_ACTIVE.swap(true, Ordering::Relaxed) {
        crate::level0b2::fallback::level0a_dead();
    }
}

/// Nabiz hic durmadiysa Healthy; gecikme baslamis ama esik asilmamissa Busy.
pub fn health() -> Level0aHealth {
    if FALLBACK_ACTIVE.load(Ordering::Relaxed) {
        return Level0aHealth::Dead;
    }
    if STALE_TICKS.load(Ordering::Relaxed) > DEAD_THRESHOLD / 2 {
        return Level0aHealth::Busy;
    }
    Level0aHealth::Healthy
}

pub fn level0a_is_dead() -> bool {
    FALLBACK_ACTIVE.load(Ordering::Relaxed)
}
