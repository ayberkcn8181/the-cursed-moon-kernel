//! `/bin/paint` -- fareyle cizim yapilan gercek bir Ring 3 uygulamasi.
//!
//! Onceki surumu sabit bir gradyan ciziyordu; bu surum fare olaylarini
//! okuyup kullanicinin cizdigini gosteriyor. Ekrandaki her piksel
//! kullanici kodunun urunudur: cekirdek yalnizca tamponu ekrana
//! harmanlar.
//!
//! Tuslar: 1-6 renk sec, c temizle, q cik.
//! Slot 1 (taban 0x00C4_0000).

#![no_std]
#![no_main]

use tcmk::gui::{self, Window};

tcmk::entry!(main);

const BG: u32 = 0x0018_1C24;
const PALETTE: [u32; 6] = [
    0x00F0_5050, // kirmizi
    0x0050_F080, // yesil
    0x0050_A0F0, // mavi
    0x00F0_D050, // sari
    0x00D0_70F0, // mor
    0x00F0_F0F0, // beyaz
];
/// Palet seridinin yuksekligi (pencerenin ustunde).
const STRIP: usize = 14;

fn main() {
    let mut win = match Window::open("TCMK Paint", 30, 60, 320, 200) {
        Some(w) => w,
        None => return,
    };

    let mut color = 0usize;
    let mut brush: isize = 3;

    win.clear(BG);
    draw_palette(&mut win, color);

    loop {
        match win.poll_key() {
            b'q' => break,
            b'c' => {
                win.clear(BG);
                draw_palette(&mut win, color);
            }
            k @ b'1'..=b'6' => {
                color = (k - b'1') as usize;
                draw_palette(&mut win, color);
            }
            b'+' => {
                if brush < 12 {
                    brush += 1;
                }
            }
            b'-' => {
                if brush > 1 {
                    brush -= 1;
                }
            }
            _ => {}
        }

        let m = gui::mouse();
        if m.left {
            if let Some((lx, ly)) = win.local_mouse(m) {
                if ly >= STRIP {
                    win.disc(lx as isize, ly as isize, brush, PALETTE[color]);
                } else {
                    // Serit uzerine tiklamak rengi secer.
                    let slot = lx * PALETTE.len() / win.width();
                    color = core::cmp::min(slot, PALETTE.len() - 1);
                    draw_palette(&mut win, color);
                }
            }
        }

        win.frame(20);
    }
}

/// Ust palet seridi; secili renk beyaz cerceveyle isaretlenir.
fn draw_palette(win: &mut Window, selected: usize) {
    let w = win.width();
    let cell = w / PALETTE.len();
    for (i, &c) in PALETTE.iter().enumerate() {
        let x = i * cell;
        win.fill(x, 0, cell, STRIP, c);
        if i == selected {
            // Ince cerceve.
            win.fill(x, 0, cell, 2, 0x00FF_FFFF);
            win.fill(x, STRIP - 2, cell, 2, 0x00FF_FFFF);
            win.fill(x, 0, 2, STRIP, 0x00FF_FFFF);
            win.fill(x + cell - 2, 0, 2, STRIP, 0x00FF_FFFF);
        }
    }
    // Seridin altina ayirac.
    win.fill(0, STRIP, w, 1, 0x0040_4850);
}
