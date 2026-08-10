//! `/bin/hog` -- YUK DENGELEYICI TESTI (kasten acgozlu uygulama).
//!
//! Sistem cagrisi yapan ama **CPU'yu birakmayan** bir dongu kurar. Ring 3
//! uygulamalari isbirlikci zamanlandigi icin (doc Faz 2 notu) boyle bir
//! uygulama normalde masaustunu, kabugu ve diger uygulamalari aclia
//! surukler: PIT "zaman dilimi doldu" der ama kimse dinlemez.
//!
//! Beklenen davranis (doc S.2.2.A "Yuk Dengeleyici"):
//!
//!   * Level-0b2 bu gorevin cagri hizini olcer,
//!   * pencere kotasi asilinca cagriyi islemeden **once** CPU birakilir,
//!   * masaustu ve diger uygulamalar calismaya devam eder,
//!   * `load` / `top` / `ipc` komutlari kisitlamayi gosterir.
//!
//! Slot 4 (taban 0x00D0_0000).

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::sys;

tcmk::entry!(main);

/// Bir "kare" basina yapilan cagri sayisi. CPU'yu birakan tek cagri
/// (`win_flush`) dongunun sonundadir; arada yalnizca `mouse_state`
/// cagrilir -- yani uygulama kendiliginden asla sira vermez.
const BURST: usize = 8_000;

fn main() {
    let mut win = match Window::open("TCMK Hog (yuk testi)", 700, 60, 300, 120) {
        Some(w) => w,
        None => return,
    };

    let (w, h) = (win.width(), win.height());
    let mut frame = 0usize;

    loop {
        // Acgozlu bolum: CPU birakmadan sistem cagrisi yagmuru.
        let mut sink = 0usize;
        for _ in 0..BURST {
            sink = sink.wrapping_add(unsafe { sys::syscall0(sys::SYS_MOUSE_STATE) });
        }
        core::hint::black_box(sink);

        // Yasadigini gosteren kucuk bir animasyon.
        win.clear(0x0028_1810);
        let bar = (frame * 7) % w;
        win.fill(0, h / 2 - 8, bar, 16, 0x00E0_9040);
        win.fill(bar, h / 2 - 8, w - bar, 16, 0x0050_3020);

        if win.poll_key() == b'q' {
            break;
        }

        frame += 1;
        win.flush();
    }
}
