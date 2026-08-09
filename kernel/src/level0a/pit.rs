//! PIT (8253/8254) 100 Hz zaman dilimi + heartbeat sayaci (doc S.4, S.11).
//! Level-0b2'nin State Monitor'u bu sayaci okuyarak Level-0a'nin
//! "Saglikli/Meşgul/Olu" durumunu izler.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::arch::i386::outb;

const PIT_CHANNEL0_DATA: u16 = 0x40;
const PIT_COMMAND: u16 = 0x43;
const PIT_BASE_FREQUENCY: u32 = 1_193_182;

static HEARTBEAT: AtomicU32 = AtomicU32::new(0);

pub fn init(hz: u32) {
    let divisor = (PIT_BASE_FREQUENCY / hz).clamp(1, 0xFFFF) as u16;
    unsafe {
        outb(PIT_COMMAND, 0x36); // channel 0, lo/hi byte, mode 3 (square wave)
        outb(PIT_CHANNEL0_DATA, (divisor & 0xFF) as u8);
        outb(PIT_CHANNEL0_DATA, ((divisor >> 8) & 0xFF) as u8);
    }
}

/// IRQ0 handler'i tarafindan her tick'te cagrilir (Level-0a'nin "nabzi").
pub fn on_tick() {
    HEARTBEAT.fetch_add(1, Ordering::Relaxed);
}

pub fn heartbeat() -> u32 {
    HEARTBEAT.load(Ordering::Relaxed)
}
