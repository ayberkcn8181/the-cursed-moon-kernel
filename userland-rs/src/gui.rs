//! Pencere/cizim API'si -- TCMK'ye ozgu 0x500 araligindaki cagrilarin
//! tipli sarmalayicilari.
//!
//! Tasarim geregi cizim **cekirdekten gecmez**: uygulama pencerenin piksel
//! tamponunun adresini bir kez alir ve dogrudan oraya yazar. Cekirdek
//! yalnizca kompozisyon (pencereleri ekrana harmanlama) ve olay dagitimi
//! yapar. Bu, "Ring 3 uygulamasi gercekten calisiyor" iddiasinin somut
//! kanitidir: ekrandaki her piksel kullanici kodunun urunudur.

use crate::sys;

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

/// Fare durumunu okur.
pub fn mouse() -> Mouse {
    let raw = unsafe { sys::syscall0(sys::SYS_MOUSE_STATE) };
    Mouse {
        x: (raw >> 20) & 0xFFF,
        y: (raw >> 4) & 0xFFF,
        left: raw & 1 != 0,
    }
}

/// Bir pencere tutamaci.
pub struct Window {
    id: usize,
    width: usize,
    height: usize,
    pixels: *mut u32,
    /// Pencerenin olusturuldugu andaki sol ust kosesi. WM pencereyi
    /// tasiyabildigi icin bu deger `origin()` ile tazelenir.
    x: usize,
    y: usize,
}

impl Window {
    /// Yeni bir pencere acar.
    ///
    /// Baslik cekirdege NUL sonlandirmali gider; kisa basliklar icin
    /// yigin uzerinde 64 baytlik bir tampon yeterlidir.
    pub fn open(title: &str, x: usize, y: usize, width: usize, height: usize) -> Option<Window> {
        let mut name = [0u8; 64];
        let n = core::cmp::min(title.len(), name.len() - 1);
        name[..n].copy_from_slice(&title.as_bytes()[..n]);

        let id = unsafe {
            sys::syscall3(
                sys::SYS_WIN_CREATE,
                name.as_ptr() as usize,
                (x << 16) | (y & 0xFFFF),
                (width << 16) | (height & 0xFFFF),
            )
        };
        if id == usize::MAX {
            return None;
        }

        let buffer = unsafe { sys::syscall1(sys::SYS_WIN_BUFFER, id) };
        if buffer == 0 {
            return None;
        }

        let size = unsafe { sys::syscall1(sys::SYS_WIN_SIZE, id) };

        Some(Window {
            id,
            width: size >> 16,
            height: size & 0xFFFF,
            pixels: buffer as *mut u32,
            x,
            y,
        })
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Pencerenin ekrandaki **guncel** sol ust kosesi.
    ///
    /// WM pencereyi baslik cubugundan surukleyebildigi icin bu deger
    /// olusturma anindakinden farkli olabilir; her cagrida cekirdekten
    /// tazelenir.
    pub fn origin(&mut self) -> (usize, usize) {
        let packed = unsafe { sys::syscall1(sys::SYS_WIN_POS, self.id) };
        if packed != usize::MAX {
            self.x = packed >> 16;
            self.y = packed & 0xFFFF;
        }
        (self.x, self.y)
    }

    /// Icerik alaninin piksel tamponu (satir basi `width` piksel).
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

    /// Bekleyen tus olayi; yoksa 0.
    pub fn poll_key(&self) -> u8 {
        unsafe { sys::syscall1(sys::SYS_WIN_POLL_KEY, self.id) as u8 }
    }

    /// Kareyi bitirir ve CPU'yu birakir. Kompozitor zaten her karede
    /// cizdigi icin bu cagri bir "sunum" degil, **zamanlama noktasidir**:
    /// isbirlikci scheduler'da diger uygulamalarin kosmasini saglar.
    pub fn flush(&self) {
        unsafe { sys::syscall1(sys::SYS_WIN_FLUSH, self.id) };
    }

    /// Fareyi pencere ic koordinatlarina cevirir; pencere disindaysa None.
    ///
    /// Pencere govdesi baslik cubugunun altinda basladigi icin cevrim
    /// guncel `origin` + kenarlik payini kullanir.
    pub fn local_mouse(&mut self, m: Mouse) -> Option<(usize, usize)> {
        let (ox, oy) = self.origin();
        let x0 = ox + BORDER;
        let y0 = oy + TITLE_HEIGHT;
        if m.x < x0 || m.y < y0 {
            return None;
        }
        let (lx, ly) = (m.x - x0, m.y - y0);
        if lx < self.width && ly < self.height {
            Some((lx, ly))
        } else {
            None
        }
    }
}
