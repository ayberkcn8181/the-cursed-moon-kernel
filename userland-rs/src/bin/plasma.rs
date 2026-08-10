//! `/bin/plasma` -- animasyonlu desen; kooperatif zamanlamanin canli kaniti.
//!
//! Pencere her karede bastan cizilir. Uygulama `flush()` ile CPU'yu
//! birakir; ayni anda kosan diger Ring 3 uygulamalari ve cekirdek gorevleri
//! bu sayede ilerler. Plasma'nin akmaya devam etmesi, baska bir uygulama
//! cokse bile sistemin ayakta kaldigini gosterir (hata izolasyonu testi).
//!
//! Slot 2 (taban 0x00C8_0000).

#![no_std]
#![no_main]

use tcmk::gui::Window;

tcmk::entry!(main);

/// Tamsayi sinus benzeri dalga: 0..255 arasi ucgen dalga, yumusatilmis.
#[inline(always)]
fn wave(t: usize) -> usize {
    let t = t & 0xFF;
    let tri = if t < 128 { t } else { 255 - t }; // 0..127
    (tri * tri) >> 6 // 0..253, uclarda yumusak
}

fn main() {
    let mut win = match Window::open("TCMK Plasma", 700, 300, 300, 200) {
        Some(w) => w,
        None => return,
    };

    let (w, h) = (win.width(), win.height());
    let mut frame = 0usize;

    loop {
        {
            let pixels = win.pixels();
            for y in 0..h {
                let row = y * w;
                let vy = wave(y * 2 + frame);
                for x in 0..w {
                    let vx = wave(x + frame * 2);
                    let vd = wave(x + y + frame);
                    let r = (vx + vy) >> 1;
                    let g = (vy + vd) >> 1;
                    let b = (vd + vx) >> 1;
                    pixels[row + x] =
                        (((r & 0xFF) << 16) | ((g & 0xFF) << 8) | (b & 0xFF)) as u32;
                }
            }
        }

        if win.poll_key() == b'q' {
            break;
        }

        frame = frame.wrapping_add(3);
        win.frame(30);
    }
}
