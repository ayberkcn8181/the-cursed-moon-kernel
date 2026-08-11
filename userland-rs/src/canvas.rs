//! Piksel tamponu uzerinde cizim -- **ABI'den bagimsiz**.
//!
//! TCMK'nin iddiasi "ayni cekirdek hem Linux hem Windows uygulamasini
//! kosturur". Bu modul o iddianin userland'deki karsiligidir: bir ELF
//! uygulamasi (`gui::Window`, `int 0x80`) ile bir PE uygulamasi
//! (`win32::Hwnd`, `int 0x2E`) **ayni cizim kodunu** kullanir. Iki dunya
//! yalnizca pencereyi acan ve olay okuyan cagrilarda ayrisir; piksele
//! donusen her sey burasidir.
//!
//! Cizim cekirdekten gecmez: `Canvas` pencere tamponunun adresini tutar
//! ve dogrudan oraya yazar. Bir "metin ciz" syscall'i olsaydi her
//! karakter icin Ring 0'a gecilirdi.

use crate::font;

/// WM'nin pencere cercevesi olculeri (`level0a::wm` ile ayni olmalidir).
pub const TITLE_HEIGHT: usize = 20;
pub const BORDER: usize = 1;

/// Fare durumu (ekran koordinatlari).
#[derive(Clone, Copy, Debug)]
pub struct Mouse {
    pub x: usize,
    pub y: usize,
    pub left: bool,
}

/// Cekirdegin paketledigi fare kelimesini acar. POSIX (`SYS_MOUSE_STATE`)
/// ve NT (`NtUserGetCursorPos`) ayni bicimi dondurdugu icin ortaktir.
pub fn unpack_mouse(raw: usize) -> Mouse {
    Mouse {
        x: (raw >> 20) & 0xFFF,
        y: (raw >> 4) & 0xFFF,
        left: raw & 1 != 0,
    }
}

/// Ekran koordinatlarini pencere ic koordinatlarina cevirir; pencere
/// disindaysa `None`. Pencere govdesi baslik cubugunun altinda basladigi
/// icin cevrim kenarlik payini hesaba katar. POSIX ve NT tarafi ayni
/// cerceveyi kullandigi icin hesap ortaktir.
pub fn to_local(
    mouse_x: usize,
    mouse_y: usize,
    origin_x: usize,
    origin_y: usize,
    width: usize,
    height: usize,
) -> Option<(usize, usize)> {
    let x0 = origin_x + BORDER;
    let y0 = origin_y + TITLE_HEIGHT;
    if mouse_x < x0 || mouse_y < y0 {
        return None;
    }
    let (lx, ly) = (mouse_x - x0, mouse_y - y0);
    if lx < width && ly < height {
        Some((lx, ly))
    } else {
        None
    }
}

/// Bir pencerenin icerik alani: satir basi `width` piksel, `height` satir.
pub struct Canvas {
    pixels: *mut u32,
    width: usize,
    height: usize,
}

impl Canvas {
    /// # Safety
    /// `pixels`, en az `width * height` piksellik yazilabilir bir tamponu
    /// gostermelidir. Cekirdek bu adresi pencere olusturulurken verir.
    pub unsafe fn new(pixels: *mut u32, width: usize, height: usize) -> Canvas {
        Canvas {
            pixels,
            width,
            height,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Icerik alaninin tamamina dilim olarak erisim.
    pub fn pixels(&mut self) -> &mut [u32] {
        unsafe { core::slice::from_raw_parts_mut(self.pixels, self.width * self.height) }
    }

    pub fn clear(&mut self, color: u32) {
        let n = self.width * self.height;
        for i in 0..n {
            unsafe { self.pixels.add(i).write(color) };
        }
    }

    /// Tek piksel; pencere disina tasan koordinatlar sessizce yok sayilir.
    #[inline]
    pub fn put(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            unsafe { self.pixels.add(y * self.width + x).write(color) };
        }
    }

    /// Dolu dikdortgen (kirpilmis).
    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        let x1 = core::cmp::min(x + w, self.width);
        let y1 = core::cmp::min(y + h, self.height);
        let mut yy = y;
        while yy < y1 {
            let mut xx = x;
            while xx < x1 {
                unsafe { self.pixels.add(yy * self.width + xx).write(color) };
                xx += 1;
            }
            yy += 1;
        }
    }

    /// Dikdortgen cerceve (dolgusuz).
    pub fn frame_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.fill(x, y, w, 1, color);
        self.fill(x, y + h - 1, w, 1, color);
        self.fill(x, y, 1, h, color);
        self.fill(x + w - 1, y, 1, h, color);
    }

    /// Merkezi (cx, cy) olan dolu daire -- fircalar ve gostergeler icin.
    pub fn disc(&mut self, cx: isize, cy: isize, r: isize, color: u32) {
        let mut dy = -r;
        while dy <= r {
            let mut dx = -r;
            while dx <= r {
                if dx * dx + dy * dy <= r * r {
                    let (px, py) = (cx + dx, cy + dy);
                    if px >= 0 && py >= 0 {
                        self.put(px as usize, py as usize, color);
                    }
                }
                dx += 1;
            }
            dy += 1;
        }
    }

    /// Tek karakter cizer (8x16 bitmap font).
    ///
    /// Font cekirdekle ayni (bkz. `tools/sync_font.py`), yani kabuktaki
    /// yaziyla uygulama yazisi ayni gorunur.
    pub fn glyph(&mut self, x: usize, y: usize, ch: u8, color: u32) {
        if ch < font::FONT_FIRST || ch > font::FONT_LAST {
            return;
        }
        let bitmap = &font::FONT[(ch - font::FONT_FIRST) as usize];
        for (row, bits) in bitmap.iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            for col in 0..font::FONT_WIDTH {
                // MSB soldaki piksel.
                if bits & (0x80 >> col) != 0 {
                    self.put(x + col, y + row, color);
                }
            }
        }
    }

    /// Karakteri `scale` katina buyuterek cizer.
    ///
    /// Font 8x16 bitmap oldugu icin buyutme tam sayi katlariyla yapilir:
    /// her font pikseli `scale x scale`'lik dolu bir kareye acilir. Ara
    /// deger uretilmedigi icin kenarlar keskin kalir -- kucuk bir bitmap
    /// fontu buyuturken istenen de budur.
    pub fn glyph_scaled(&mut self, x: usize, y: usize, ch: u8, scale: usize, color: u32) {
        if ch < font::FONT_FIRST || ch > font::FONT_LAST || scale == 0 {
            return;
        }
        let bitmap = &font::FONT[(ch - font::FONT_FIRST) as usize];
        for (row, bits) in bitmap.iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            for col in 0..font::FONT_WIDTH {
                if bits & (0x80 >> col) != 0 {
                    self.fill(x + col * scale, y + row * scale, scale, scale, color);
                }
            }
        }
    }

    /// Buyutulmus dizi; kapladigi genisligi doner.
    pub fn text_scaled(&mut self, x: usize, y: usize, s: &str, scale: usize, color: u32) -> usize {
        let step = font::FONT_WIDTH * scale;
        let mut cx = x;
        for ch in s.bytes() {
            if cx + step > self.width {
                break;
            }
            self.glyph_scaled(cx, y, ch, scale, color);
            cx += step;
        }
        cx - x
    }

    /// Dizi cizer; pencere kenarina gelince keser.
    pub fn text(&mut self, x: usize, y: usize, s: &str, color: u32) {
        let mut cx = x;
        for ch in s.bytes() {
            if cx + font::FONT_WIDTH > self.width {
                break;
            }
            self.glyph(cx, y, ch, color);
            cx += font::FONT_WIDTH;
        }
    }

    /// Isaretsiz sayiyi cizer ve kapladigi genisligi doner.
    pub fn number(&mut self, x: usize, y: usize, mut value: usize, color: u32) -> usize {
        let mut digits = [0u8; 20];
        let mut n = 0;
        if value == 0 {
            digits[0] = b'0';
            n = 1;
        }
        while value > 0 {
            digits[n] = b'0' + (value % 10) as u8;
            value /= 10;
            n += 1;
        }
        for i in 0..n {
            self.glyph(x + i * font::FONT_WIDTH, y, digits[n - 1 - i], color);
        }
        n * font::FONT_WIDTH
    }

    /// Sayiyi sabit genislikte, basina sifir koyarak cizer (saat icin).
    pub fn number_pad(&mut self, x: usize, y: usize, value: usize, digits: usize, color: u32) {
        let mut scale = 1usize;
        for _ in 1..digits {
            scale *= 10;
        }
        let mut cx = x;
        let mut rest = value;
        while scale > 0 {
            let digit = (rest / scale) % 10;
            self.glyph(cx, y, b'0' + digit as u8, color);
            cx += font::FONT_WIDTH;
            rest %= scale.max(1);
            scale /= 10;
        }
    }
}
