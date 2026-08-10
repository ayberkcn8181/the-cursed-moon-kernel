//! `/bin/notes` -- kalici not defteri.
//!
//! TCMK'nin parcalarini tek bir uygulamada birlestirir:
//!
//!   * pencere + **kendi cizdigi metin** (`tcmk::gui::Window::text`),
//!   * klavye olaylari (`win_poll_key`),
//!   * kalici dosya sistemine **yazma** (`File::create` + `write`),
//!   * acilista ayni dosyayi geri **okuma**.
//!
//! Yani: yaz, kaydet, makineyi kapat, yeniden ac -- notun yerinde.
//!
//! Tuslar: yazilabilir karakterler, Backspace, Enter, `~`/`` ` `` kaydet.

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::File;

tcmk::entry!(main);

const PATH: &str = "/home/notes.txt";

const COLS: usize = 34;
const ROWS: usize = 11;
const CELL_W: usize = 8;
const CELL_H: usize = 16;

const BG: u32 = 0x0014_1A20;
const FG: u32 = 0x00C8_E0F0;
const ACCENT: u32 = 0x0050_C878;
const WARN: u32 = 0x00F0_C050;

struct Buffer {
    cells: [[u8; COLS]; ROWS],
    row: usize,
    col: usize,
}

impl Buffer {
    fn new() -> Self {
        Buffer {
            cells: [[b' '; COLS]; ROWS],
            row: 0,
            col: 0,
        }
    }

    fn newline(&mut self) {
        self.col = 0;
        if self.row + 1 < ROWS {
            self.row += 1;
        }
    }

    fn put(&mut self, ch: u8) {
        if self.col >= COLS {
            self.newline();
        }
        self.cells[self.row][self.col] = ch;
        self.col += 1;
    }

    fn backspace(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = COLS - 1;
        }
        self.cells[self.row][self.col] = b' ';
    }

    /// Metni satir sonlarindaki bosluklari kirparak diske uygun hale getirir.
    fn flatten(&self, out: &mut [u8]) -> usize {
        let mut n = 0;
        for row in self.cells.iter() {
            let end = row.iter().rposition(|c| *c != b' ').map_or(0, |i| i + 1);
            for &ch in &row[..end] {
                if n < out.len() {
                    out[n] = ch;
                    n += 1;
                }
            }
            if n < out.len() {
                out[n] = b'\n';
                n += 1;
            }
        }
        n
    }

    /// Diskten okunan metni izgaraya dagitir.
    fn load(&mut self, data: &[u8]) {
        self.row = 0;
        self.col = 0;
        for &ch in data {
            match ch {
                b'\n' => {
                    self.newline();
                    if self.row + 1 >= ROWS && self.col == 0 {
                        // Izgara doldu; kalanini atlamak, tasip bozmaktan iyi.
                    }
                }
                0x20..=0x7E => self.put(ch),
                _ => {}
            }
        }
    }
}

fn main() {
    let mut win = match Window::open("TCMK Notes", 60, 420, 300, 220) {
        Some(w) => w,
        None => return,
    };

    let mut buffer = Buffer::new();
    let mut status = "yeni not";

    // Onceki notu geri yukle.
    if let Some(mut file) = File::open(PATH) {
        let mut raw = [0u8; COLS * ROWS + ROWS];
        let n = file.read(&mut raw);
        if n > 0 {
            buffer.load(&raw[..n]);
            status = "diskten yuklendi";
        }
    }

    loop {
        match win.poll_key() {
            0 => {}
            // Ayni fiziksel tus, Shift'siz '`' uretir; ikisini de kabul et.
            b'~' | b'`' => {
                let mut raw = [0u8; COLS * ROWS + ROWS];
                let n = buffer.flatten(&mut raw);
                status = match File::create(PATH) {
                    Some(mut file) => {
                        if file.write(&raw[..n]) == n {
                            "diske kaydedildi"
                        } else {
                            "yazma eksik kaldi"
                        }
                    }
                    None => "dosya acilamadi",
                };
            }
            b'\n' => buffer.newline(),
            0x08 => buffer.backspace(),
            ch @ 0x20..=0x7E => buffer.put(ch),
            _ => {}
        }

        draw(&mut win, &buffer, status);
        win.frame(30);
    }
}

fn draw(win: &mut Window, buffer: &Buffer, status: &str) {
    win.clear(BG);

    // Metin izgarasi.
    for (r, row) in buffer.cells.iter().enumerate() {
        let end = row.iter().rposition(|c| *c != b' ').map_or(0, |i| i + 1);
        for (c, &ch) in row[..end].iter().enumerate() {
            win.glyph(4 + c * CELL_W, 4 + r * CELL_H, ch, FG);
        }
    }

    // Imlec.
    win.fill(
        4 + buffer.col * CELL_W,
        4 + buffer.row * CELL_H + CELL_H - 2,
        CELL_W,
        2,
        ACCENT,
    );

    // Alt bilgi cubugu.
    let h = win.height();
    win.fill(0, h - 34, win.width(), 34, 0x000E_1218);
    win.text(4, h - 32, PATH, ACCENT);
    win.text(4, h - 16, status, WARN);
    let hint_x = win.width().saturating_sub(9 * CELL_W);
    win.text(hint_x, h - 16, "~ kaydet", FG);
}
