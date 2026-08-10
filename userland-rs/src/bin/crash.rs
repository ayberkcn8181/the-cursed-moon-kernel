//! `/bin/crash` -- HATA IZOLASYONU TESTI (kasten coken uygulama).
//!
//! Pencere acar, bir sure geri sayim cizer, sonra **cekirdek adresine**
//! yazmaya calisir. Ring 3'ten cekirdek sayfasina yazma page fault (vektor
//! 14) uretir. Beklenen davranis (doc S.2.2.A "Fallback Interface"):
//!
//!   * yalnizca bu surec sonlandirilir,
//!   * masaustu, kabuk ve diger uygulamalar calismaya devam eder,
//!   * `faults` komutu istisnayi ve sonlandirilan surec sayisini gosterir.
//!
//! Slot 3 (taban 0x00CC_0000).

#![no_std]
#![no_main]

use tcmk::gui::Window;

tcmk::entry!(main);

/// Cekirdek kod bolgesi (0x0010_0000) -- Ring 3'e kapali olmasi gerekir.
const KERNEL_ADDR: usize = 0x0010_0000;
/// Kac kare sonra cokecegi.
const COUNTDOWN: usize = 120;

fn main() {
    let mut win = match Window::open("TCMK Crash Test", 380, 300, 300, 120) {
        Some(w) => w,
        None => return,
    };

    let (w, h) = (win.width(), win.height());

    for frame in 0..COUNTDOWN {
        // Geri sayim: kalan sure kadar dolan kirmizi bir cubuk.
        win.clear(0x0020_1418);
        let filled = w * frame / COUNTDOWN;
        win.fill(0, h / 2 - 10, filled, 20, 0x00C0_3030);
        win.fill(filled, h / 2 - 10, w - filled, 20, 0x0040_2020);

        if win.poll_key() == b'q' {
            return; // temiz cikis: cokme testinden vazgecildi
        }
        win.flush();
    }

    // --- Kasitli ihlal ---
    // Ring 3'ten cekirdek sayfasina yazma. Buradan sonrasi calismamalidir.
    unsafe {
        core::ptr::write_volatile(KERNEL_ADDR as *mut u32, 0xDEAD_BEEF);
    }

    // Buraya ulasilirsa izolasyon KIRILMIS demektir.
    win.clear(0x00FF_00FF);
    loop {
        win.flush();
    }
}
