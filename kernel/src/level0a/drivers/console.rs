//! Metin konsolu: satirlari bir halka tamponda tutar.
//!
//! Iki tuketicisi vardir ve ayni veriyi paylasirlar:
//!   - **Acilista** dogrudan framebuffer'a cizilir (boot logu).
//!   - **GUI basladiktan sonra** pencere yoneticisi ayni tamponu bir
//!     "Sistem Gunlugu" penceresi icinde gosterir.
//!
//! Boylece cekirdek kaydi hicbir zaman kaybolmaz; yalnizca nereye
//! cizildigi degisir.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::gfx;

pub const MAX_LINES: usize = 64;
pub const MAX_COLS: usize = 128;

static mut LINES: [[u8; MAX_COLS]; MAX_LINES] = [[b' '; MAX_COLS]; MAX_LINES];
static mut LENGTHS: [usize; MAX_LINES] = [0; MAX_LINES];

/// Halka tamponun en yeni satirinin indeksi.
static HEAD: AtomicUsize = AtomicUsize::new(0);
/// Su ana kadar yazilmis toplam satir sayisi (tasma tespiti icin).
static TOTAL: AtomicUsize = AtomicUsize::new(1);
static CURSOR_COL: AtomicUsize = AtomicUsize::new(0);

/// GUI devraldi mi? Devraldiysa konsol artik dogrudan ekrana cizmez.
static WM_OWNS_SCREEN: AtomicBool = AtomicBool::new(false);

pub fn wm_take_over() {
    WM_OWNS_SCREEN.store(true, Ordering::Relaxed);
}

pub fn wm_owns_screen() -> bool {
    WM_OWNS_SCREEN.load(Ordering::Relaxed)
}

fn newline() {
    let head = (HEAD.load(Ordering::Relaxed) + 1) % MAX_LINES;
    HEAD.store(head, Ordering::Relaxed);
    TOTAL.fetch_add(1, Ordering::Relaxed);
    CURSOR_COL.store(0, Ordering::Relaxed);

    unsafe {
        let lines = core::ptr::addr_of_mut!(LINES) as *mut [u8; MAX_COLS];
        let lengths = core::ptr::addr_of_mut!(LENGTHS) as *mut usize;
        (*lines.add(head)) = [b' '; MAX_COLS];
        lengths.add(head).write(0);
    }
}

pub fn write_str(s: &str) {
    for byte in s.bytes() {
        match byte {
            b'\n' => newline(),
            0x20..=0x7E => {
                let col = CURSOR_COL.load(Ordering::Relaxed);
                if col >= MAX_COLS {
                    newline();
                }
                let col = CURSOR_COL.load(Ordering::Relaxed);
                let head = HEAD.load(Ordering::Relaxed);
                unsafe {
                    let lines = core::ptr::addr_of_mut!(LINES) as *mut [u8; MAX_COLS];
                    let lengths = core::ptr::addr_of_mut!(LENGTHS) as *mut usize;
                    (*lines.add(head))[col] = byte;
                    if col + 1 > lengths.add(head).read() {
                        lengths.add(head).write(col + 1);
                    }
                }
                CURSOR_COL.store(col + 1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    if !wm_owns_screen() && gfx::available() {
        render_fullscreen();
    }
}

/// Halka tamponun `n` satirlik penceresini, en yeniden geriye dogru verir.
///
/// `visit(satir_indeksi_ekranda, metin)` seklinde cagirir.
pub fn for_each_visible_line<F: FnMut(usize, &[u8])>(max_lines: usize, mut visit: F) {
    let total = TOTAL.load(Ordering::Relaxed).min(MAX_LINES);
    let head = HEAD.load(Ordering::Relaxed);
    let count = max_lines.min(total);

    // En eski gorunur satirdan basla.
    for i in 0..count {
        let offset = count - 1 - i; // head'den geriye
        let idx = (head + MAX_LINES - offset) % MAX_LINES;
        unsafe {
            let lines = core::ptr::addr_of!(LINES) as *const [u8; MAX_COLS];
            let lengths = core::ptr::addr_of!(LENGTHS) as *const usize;
            let len = lengths.add(idx).read();
            // Ham isaretciden dogrudan dilim kur: `&(*ptr)[..]` implicit
            // autoref uretir ve ham isaretci uzerinde bu yasak.
            let row = core::slice::from_raw_parts(lines.add(idx) as *const u8, len);
            visit(i, row);
        }
    }
}

/// Acilis asamasinda tum ekrani konsol olarak kullanir.
fn render_fullscreen() {
    let rows = gfx::height() / gfx::font_height();
    gfx::clear(gfx::BLACK);

    for_each_visible_line(rows, |row, text| {
        let y = row * gfx::font_height();
        let mut x = 0;
        for &byte in text {
            gfx::draw_char(x, y, byte, 0x00C8_C8C8);
            x += gfx::font_width();
        }
    });

    gfx::present();
}
