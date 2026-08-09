//! Fallback Interface: Level-0a kilitlenir/cokerse yonetimi devralir
//! (doc S.2.2.A, S.11). Faz 1 yetenekleri: VGA'ya acil durum mesaji yazma
//! ve sistemin kilitlenmesini onleme (hlt dongusu).

use core::panic::PanicInfo;

pub fn emergency(lines: &[&str]) {
    for line in lines {
        crate::println!("[LEVEL-0b2][FALLBACK] {}", line);
    }
}

/// State Monitor, heartbeat kayboldugunu tespit ettiginde cagirir.
pub fn level0a_dead() -> ! {
    crate::println!();
    emergency(&[
        "Level-0a YANIT VERMIYOR (heartbeat kayboldu).",
        "Fallback Interface devrede - sistem minimal modda tutuluyor.",
    ]);
    halt_forever();
}

pub fn panic_screen(info: &PanicInfo) -> ! {
    crate::println!();
    crate::println!("[LEVEL-0b2][FALLBACK] KERNEL PANIC: {}", info);
    halt_forever();
}

fn halt_forever() -> ! {
    loop {
        crate::arch::i386::disable_interrupts();
        crate::arch::i386::halt();
    }
}
