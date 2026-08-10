//! `/bin/spin` -- PREEMPTION TESTI (hic sistem cagrisi yapmayan dongu).
//!
//! `hog` acgozluydu ama en azindan syscall yapiyordu; Yuk Dengeleyici onu
//! gorup CPU birakmaya zorlayabiliyordu. Bu uygulama **hicbir sey
//! cagirmaz**: saf hesap. Isbirlikci zamanlamada tek savunma yoktu --
//! masaustu, kabuk ve diger uygulamalar bu dongu boyunca donardi.
//!
//! Beklenen davranis: zamanlayici kesmesi gorevi kendi istegi olmadan
//! birakir; sistem akici kalir. `ps` komutundaki "zorla" sayaci artar.
//!
//! Pencerede donen bir cubuk cizilir: cubuk ilerliyorsa uygulama
//! calisiyor, masaustu de yanit veriyorsa preemption is goruyor.

#![no_std]
#![no_main]

use tcmk::gui::Window;

tcmk::entry!(main);

/// Kare basina yapilan bos donus. Hicbir syscall icermez, dolayisiyla
/// cekirdegin araya girmesinin **tek** yolu zamanlayici kesmesidir.
const SPIN_ROUNDS: usize = 40_000_000;

fn main() {
    let mut win = match Window::open("TCMK Spin (preemption testi)", 380, 300, 300, 120) {
        Some(w) => w,
        None => return,
    };

    let (w, h) = (win.width(), win.height());
    let mut frame = 0usize;

    loop {
        win.clear(0x0018_2818);
        let bar = (frame * 11) % w;
        win.fill(0, h / 2 - 8, bar, 16, 0x0050_D060);
        win.fill(bar, h / 2 - 8, w - bar, 16, 0x0020_5028);

        if win.poll_key() == b'q' {
            break;
        }
        frame += 1;
        win.frame(30);

        // --- Saf hesap: syscall yok, yield yok ---
        let mut acc = frame;
        for i in 0..SPIN_ROUNDS {
            acc = acc.wrapping_mul(1_664_525).wrapping_add(i);
        }
        core::hint::black_box(acc);
    }
}
