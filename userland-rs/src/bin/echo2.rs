//! `echo2` -- klavyeyi **POSIX yoluyla** okuyan uygulama.
//!
//! Diger GUI uygulamalari tuslari `win_poll_key` ile alir; bu, TCMK'ye
//! ozgu 0x500 araligindaki bir cagridir. Bu program ise klavyeyi
//! `read(0, ...)` ile okur -- yani standart girdiden, POSIX'in kendi
//! yolundan. Yazilanlar `write(1, ...)` ile seri konsola da doner,
//! boylece ayni veri iki kanaldan dogrulanabilir.
//!
//! Amac, "TCMK'ye ozgu cagrilar olmadan da uygulama yazilabilir mi"
//! sorusuna cevap vermek. Pencere acmak icin TCMK cagrisi gerekir (POSIX
//! pencere bilmez), ama pencere acildiktan sonra girdi/cikti tamamen
//! standart.
//!
//! Tuslar: yazi,  ESC -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x000C_1620;
const PANEL: u32 = 0x0018_2A38;
const FG: u32 = 0x00E0_E8F0;
const ACCENT: u32 = 0x0060_E0B0;

const CAPACITY: usize = 256;
const CELL_W: usize = 8;
const CELL_H: usize = 16;

fn main() {
    let mut win = match Window::open("echo2 -- read(0)", 340, 240, 340, 200) {
        Some(w) => w,
        None => return,
    };

    let mut buffer = [0u8; CAPACITY];
    let mut len = 0usize;
    let mut total_reads = 0usize;

    loop {
        // Standart girdiden oku. Bloke etmez: tus yoksa 0 doner, yani
        // cizim dongusu akici kalir.
        let mut chunk = [0u8; 16];
        let n = sys::read(sys::STDIN, &mut chunk);
        if n > 0 {
            total_reads += 1;
            for &key in &chunk[..n as usize] {
                if key == 0x1B {
                    // ESC
                    let _ = sys::write(sys::STDOUT, b"\n[echo2] cikiliyor.\n");
                    return;
                }
                if key == 8 {
                    len = len.saturating_sub(1);
                } else if len < CAPACITY && (key == b'\n' || key >= 32) {
                    buffer[len] = key;
                    len += 1;
                }
            }
            // Ayni veriyi standart cikisa da yaz -- seri gunlukte gorunur.
            let _ = sys::write(sys::STDOUT, &chunk[..n as usize]);
        }

        draw(&mut win, &buffer[..len], total_reads);
        win.frame(30);
    }
}

fn draw(win: &mut Window, text: &[u8], reads: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "girdi: read(0)   cikti: write(1)", ACCENT);

    // Basit sarma.
    let columns = w.saturating_sub(12) / CELL_W;
    let (mut cx, mut cy) = (0usize, 0usize);
    for &ch in text {
        if ch == b'\n' {
            cx = 0;
            cy += 1;
            continue;
        }
        if columns > 0 && cx >= columns {
            cx = 0;
            cy += 1;
        }
        win.glyph(6 + cx * CELL_W, 30 + cy * CELL_H, ch, FG);
        cx += 1;
    }
    win.fill(6 + cx * CELL_W, 30 + cy * CELL_H + CELL_H - 2, CELL_W, 2, ACCENT);

    win.fill(0, h - 20, w, 20, PANEL);
    win.text(6, h - 18, "ESC = cik   read cagrisi:", FG);
    win.number(w.saturating_sub(40), h - 18, reads, ACCENT);
}
