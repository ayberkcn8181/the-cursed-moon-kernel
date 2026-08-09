//! Framebuffer grafik motoru (doc S.7 Faz 13: "Framebuffer/KMS").
//!
//! Cift tamponlu: tum cizim `BACK_BUFFER`'a yapilir, `present()` bunu tek
//! seferde donanim framebuffer'ina kopyalar. Boylece pencere yoneticisi
//! kompozisyonu sirasinda ekranda titreme (tearing) olmaz.
//!
//! Piksel formati 32-bit `0x00RRGGBB` (BGRA bellek duzeni, little-endian).

use super::font_data::{FONT, FONT_FIRST, FONT_HEIGHT, FONT_LAST, FONT_WIDTH};
use crate::boot::multiboot::FramebufferInfo;
use core::sync::atomic::{AtomicBool, Ordering};

pub const MAX_WIDTH: usize = 1024;
pub const MAX_HEIGHT: usize = 768;

/// Arka tampon cekirdek imajinda statik olarak yer alir (3 MiB).
/// kmalloc heap'i 1 MiB oldugu icin buraya sigmaz.
static mut BACK_BUFFER: [u32; MAX_WIDTH * MAX_HEIGHT] = [0; MAX_WIDTH * MAX_HEIGHT];

static mut FB: Option<FramebufferInfo> = None;
static AVAILABLE: AtomicBool = AtomicBool::new(false);

pub type Color = u32;

pub const BLACK: Color = 0x0000_0000;
pub const WHITE: Color = 0x00FF_FFFF;
pub const DESKTOP_BG: Color = 0x0020_2A3A;
pub const WINDOW_BG: Color = 0x00F0_F0F0;
pub const TITLE_ACTIVE: Color = 0x0030_60C0;
pub const TITLE_INACTIVE: Color = 0x0060_6870;
pub const ACCENT: Color = 0x00E0_9020;

/// Framebuffer'i kaydeder. `None` ise sistem metin moduna duser.
///
/// # Safety
/// `info` bootloader'dan gelen gecerli bir tanim olmalidir.
pub unsafe fn init(info: Option<FramebufferInfo>) {
    match info {
        Some(fb) if fb.width <= MAX_WIDTH && fb.height <= MAX_HEIGHT => {
            FB = Some(fb);
            AVAILABLE.store(true, Ordering::Relaxed);
            clear(DESKTOP_BG);
            present();
        }
        _ => {
            AVAILABLE.store(false, Ordering::Relaxed);
        }
    }
}

/// Framebuffer'in fiziksel adres araligi -- MMU eslemesi icin.
pub fn physical_range() -> Option<(usize, usize)> {
    unsafe {
        core::ptr::addr_of!(FB)
            .read()
            .map(|f| (f.addr, f.pitch * f.height))
    }
}

pub fn available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

pub fn width() -> usize {
    unsafe { core::ptr::addr_of!(FB).read().map_or(0, |f| f.width) }
}

pub fn height() -> usize {
    unsafe { core::ptr::addr_of!(FB).read().map_or(0, |f| f.height) }
}

#[inline]
fn back() -> *mut u32 {
    core::ptr::addr_of_mut!(BACK_BUFFER) as *mut u32
}

#[inline]
pub fn put_pixel(x: usize, y: usize, color: Color) {
    if !available() || x >= width() || y >= height() {
        return;
    }
    unsafe {
        back().add(y * width() + x).write(color);
    }
}

pub fn fill_rect(x: usize, y: usize, w: usize, h: usize, color: Color) {
    if !available() {
        return;
    }
    let (sw, sh) = (width(), height());
    let x_end = (x + w).min(sw);
    let y_end = (y + h).min(sh);
    if x >= sw || y >= sh {
        return;
    }

    unsafe {
        let buf = back();
        for row in y..y_end {
            let line = buf.add(row * sw);
            for col in x..x_end {
                line.add(col).write(color);
            }
        }
    }
}

/// Icersiz dikdortgen (pencere kenarliklari icin).
pub fn stroke_rect(x: usize, y: usize, w: usize, h: usize, color: Color) {
    if w == 0 || h == 0 {
        return;
    }
    fill_rect(x, y, w, 1, color);
    fill_rect(x, y + h - 1, w, 1, color);
    fill_rect(x, y, 1, h, color);
    fill_rect(x + w - 1, y, 1, h, color);
}

pub fn clear(color: Color) {
    fill_rect(0, 0, width(), height(), color);
}

/// Kullanici penceresinin piksel tamponunu ekrana kopyalar.
pub fn blit(dst_x: usize, dst_y: usize, w: usize, h: usize, src: &[u32]) {
    if !available() {
        return;
    }
    let (sw, sh) = (width(), height());

    unsafe {
        let buf = back();
        for row in 0..h {
            let dy = dst_y + row;
            if dy >= sh {
                break;
            }
            for col in 0..w {
                let dx = dst_x + col;
                if dx >= sw {
                    break;
                }
                let idx = row * w + col;
                if idx < src.len() {
                    buf.add(dy * sw + dx).write(src[idx]);
                }
            }
        }
    }
}

/// Tek karakter cizer; bilinmeyen karakterler icin '?' kullanilir.
pub fn draw_char(x: usize, y: usize, ch: u8, fg: Color) {
    let index = if (FONT_FIRST..=FONT_LAST).contains(&ch) {
        (ch - FONT_FIRST) as usize
    } else {
        (b'?' - FONT_FIRST) as usize
    };

    let glyph = &FONT[index];
    for (row, bits) in glyph.iter().enumerate() {
        if *bits == 0 {
            continue;
        }
        for col in 0..FONT_WIDTH {
            if bits & (1 << (FONT_WIDTH - 1 - col)) != 0 {
                put_pixel(x + col, y + row, fg);
            }
        }
    }
}

pub fn draw_text(x: usize, y: usize, text: &str, fg: Color) {
    let mut cx = x;
    for byte in text.bytes() {
        if byte == b'\n' {
            break;
        }
        draw_char(cx, y, byte, fg);
        cx += FONT_WIDTH;
    }
}

/// Arka tamponu donanim framebuffer'ina aktarir.
pub fn present() {
    if !available() {
        return;
    }
    unsafe {
        let fb = match core::ptr::addr_of!(FB).read() {
            Some(f) => f,
            None => return,
        };
        let src = back();
        let dst = fb.addr as *mut u8;

        for row in 0..fb.height {
            let src_line = src.add(row * fb.width);
            let dst_line = dst.add(row * fb.pitch) as *mut u32;
            core::ptr::copy_nonoverlapping(src_line, dst_line, fb.width);
        }
    }
}

pub fn font_width() -> usize {
    FONT_WIDTH
}

pub fn font_height() -> usize {
    FONT_HEIGHT
}
