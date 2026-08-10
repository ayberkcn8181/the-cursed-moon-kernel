//! PIT (8253/8254) 100 Hz zaman dilimi + heartbeat sayaci (doc S.4, S.11).
//! Level-0b2'nin State Monitor'u bu sayaci okuyarak Level-0a'nin
//! "Saglikli/Mesgul/Olu" durumunu izler.
//!
//! Faz 2'den itibaren heartbeat'i ARTIRAN sey artik timer kesmesinin kendisi
//! degil, scheduler'in gorev dongusudur (`scheduler::yield_now` -> `beat()`).
//! Bunun sebebi doc S.11'deki tanimdir: "Level-0a her scheduler dongusunde
//! sayaci artirir". Boylece scheduler kilitlenirse -- timer hala atsa bile --
//! nabiz durur ve Fallback Interface devreye girer.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::arch::cpu::outb;

const PIT_CHANNEL0_DATA: u16 = 0x40;
const PIT_COMMAND: u16 = 0x43;
const PIT_BASE_FREQUENCY: u32 = 1_193_182;

static HEARTBEAT: AtomicU32 = AtomicU32::new(0);
static TICKS: AtomicU32 = AtomicU32::new(0);

pub fn init(hz: u32) {
    let divisor = (PIT_BASE_FREQUENCY / hz).clamp(1, 0xFFFF) as u16;
    unsafe {
        outb(PIT_COMMAND, 0x36); // channel 0, lo/hi byte, mode 3 (square wave)
        outb(PIT_CHANNEL0_DATA, (divisor & 0xFF) as u8);
        outb(PIT_CHANNEL0_DATA, ((divisor >> 8) & 0xFF) as u8);
    }
}

/// IRQ0 handler'i tarafindan her tick'te cagrilir.
pub fn on_tick() {
    let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    crate::level0a::core::scheduler::on_timer_tick();

    // Paylasimli bolgeyi tazele (doc S.10): Level-0b2 durum bilgisini
    // Level-0a'nin fonksiyonlarini cagirarak degil, buradan okur.
    let shared = &crate::level0b2::ipc::SHARED;
    shared.ticks.store(ticks, Ordering::Relaxed);
    shared.tasks.store(
        crate::level0a::core::scheduler::task_count() as u32,
        Ordering::Relaxed,
    );
    shared.switches.store(
        crate::level0a::core::scheduler::switch_count() as u32,
        Ordering::Relaxed,
    );

    // Yuk penceresi zamanlayiciya baglidir: olculen isin (scheduler
    // dongusu) ilerlemesine bagli olsaydi, donen bir gorev olcumun
    // kendisini de dondururdu.
    crate::level0b2::load_balancer::on_tick(ticks);
}

/// Nabzin bastirilacagi son tick (0 = bastirma yok).
static HEARTBEAT_SUPPRESSED_UNTIL: AtomicU32 = AtomicU32::new(0);

/// Scheduler dongusunun "nabiz" atisi (doc S.11).
pub fn beat() {
    // Hata tolerans motorunun testi (doc S.12): nabiz gecici olarak
    // bastirilabilir. Level-0a normal calismaya devam eder ama disaridan
    // **kilitlenmis gibi gorunur**; State Monitor'un ve Co-Service'in
    // dogru davranip davranmadigi ancak boyle gozlenebilir.
    let until = HEARTBEAT_SUPPRESSED_UNTIL.load(Ordering::Relaxed);
    if until != 0 {
        if TICKS.load(Ordering::Relaxed) < until {
            return;
        }
        HEARTBEAT_SUPPRESSED_UNTIL.store(0, Ordering::Relaxed);
    }

    let beat = HEARTBEAT.fetch_add(1, Ordering::Relaxed) + 1;
    crate::level0b2::ipc::SHARED
        .heartbeat
        .store(beat, Ordering::Relaxed);
}

/// Nabzi `ticks` kadar bastirir (kabuk `stall` komutu).
pub fn suppress_heartbeat(ticks: u32) {
    HEARTBEAT_SUPPRESSED_UNTIL.store(TICKS.load(Ordering::Relaxed) + ticks, Ordering::Relaxed);
}

pub fn heartbeat_suppressed() -> bool {
    HEARTBEAT_SUPPRESSED_UNTIL.load(Ordering::Relaxed) != 0
}

pub fn heartbeat() -> u32 {
    HEARTBEAT.load(Ordering::Relaxed)
}

pub fn ticks() -> u32 {
    TICKS.load(Ordering::Relaxed)
}
