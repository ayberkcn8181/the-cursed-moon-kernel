//! `/bin/menu` -- GUI uygulama baslatici.
//!
//! `execve`'nin somut kullanimi: secilen uygulama **bu surecin yerine**
//! yuklenir. Yeni bir gorev acilmaz; menu kaybolur, uygulama ayni gorev
//! numarasiyla, yeni bir adres uzayinda gelir. `ps` ile izlenebilir.
//!
//! Tuslar: 1-4 sec, q cik.

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0016_1E28;
const FG: u32 = 0x00D0_E4F0;
const SEL: u32 = 0x0028_4A66;
const ACCENT: u32 = 0x0060_D0A0;

const CELL_H: usize = 22;

/// (tus, yol, aciklama)
const ENTRIES: [(u8, &str, &str); 4] = [
    (b'1', "/bin/paint", "Paint  -- fareyle cizim"),
    (b'2', "/bin/plasma", "Plasma -- animasyonlu desen"),
    (b'3', "/bin/notes", "Notes  -- kalici not defteri"),
    (b'4', "/bin/crash", "Crash  -- hata izolasyonu testi"),
];

fn main() {
    let mut win = match Window::open("TCMK Menu", 380, 60, 300, 140) {
        Some(w) => w,
        None => return,
    };

    let mut selected = 0usize;

    loop {
        match win.poll_key() {
            b'q' => return,
            ch @ b'1'..=b'4' => {
                selected = (ch - b'1') as usize;
                // Bu cagri basarili olursa GERI DONMEZ: cekirdek bu surecin
                // imajini ve adres uzayini birakip secilen programi ayni
                // gorevde yukler.
                sys::execve(ENTRIES[selected].1);
                // Buraya dusuldüyse yukleme basarisiz oldu.
            }
            _ => {}
        }

        draw(&mut win, selected);
        win.frame(30);
    }
}

fn draw(win: &mut Window, selected: usize) {
    win.clear(BG);
    win.text(6, 4, "calistirmak icin numaraya bas:", ACCENT);

    let width = win.width();
    for (i, (key, _, label)) in ENTRIES.iter().enumerate() {
        let y = 26 + i * CELL_H;
        if i == selected {
            win.fill(2, y - 3, width - 4, CELL_H, SEL);
        }
        win.glyph(8, y, *key, ACCENT);
        win.glyph(16, y, b')', ACCENT);
        win.text(30, y, label, FG);
    }

    let h = win.height();
    win.text(6, h - 16, "execve: menu yerine uygulama yuklenir", ACCENT);
}
