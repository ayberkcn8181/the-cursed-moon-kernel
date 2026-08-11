//! Pencere API'si (POSIX tarafi) -- TCMK'ye ozgu 0x500 araligindaki
//! cagrilarin tipli sarmalayicilari.
//!
//! Bu modul yalnizca **pencere yasam dongusunu ve olaylari** kapsar;
//! cizim `canvas::Canvas`'tadir ve PE tarafiyla (bkz. `win32`) ortaktir.
//! `Window`, `Canvas`'a `Deref` ettigi icin `win.text(...)`, `win.fill(...)`
//! gibi cagrilar dogrudan calisir.
//!
//! Tasarim geregi cizim **cekirdekten gecmez**: uygulama pencerenin piksel
//! tamponunun adresini bir kez alir ve dogrudan oraya yazar. Cekirdek
//! yalnizca kompozisyon (pencereleri ekrana harmanlama) ve olay dagitimi
//! yapar. Bu, "Ring 3 uygulamasi gercekten calisiyor" iddiasinin somut
//! kanitidir: ekrandaki her piksel kullanici kodunun urunudur.

use crate::canvas::{self, Canvas};
use crate::sys;

pub use crate::canvas::{Mouse, BORDER, TITLE_HEIGHT};

/// Fare durumunu okur.
pub fn mouse() -> Mouse {
    canvas::unpack_mouse(unsafe { sys::syscall0(sys::SYS_MOUSE_STATE) })
}

/// Bir pencere tutamaci.
pub struct Window {
    id: usize,
    canvas: Canvas,
    /// Pencerenin olusturuldugu andaki sol ust kosesi. WM pencereyi
    /// tasiyabildigi icin bu deger `origin()` ile tazelenir.
    x: usize,
    y: usize,
}

impl core::ops::Deref for Window {
    type Target = Canvas;
    fn deref(&self) -> &Canvas {
        &self.canvas
    }
}

impl core::ops::DerefMut for Window {
    fn deref_mut(&mut self) -> &mut Canvas {
        &mut self.canvas
    }
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
            canvas: unsafe { Canvas::new(buffer as *mut u32, size >> 16, size & 0xFFFF) },
            x,
            y,
        })
    }

    pub fn id(&self) -> usize {
        self.id
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

    /// Bekleyen tus olayi; yoksa 0.
    pub fn poll_key(&self) -> u8 {
        unsafe { sys::syscall1(sys::SYS_WIN_POLL_KEY, self.id) as u8 }
    }

    /// Kareyi bitirir ve CPU'yu birakir. Kompozitor zaten her karede
    /// cizdigi icin bu cagri bir "sunum" degil, **zamanlama noktasidir**.
    pub fn flush(&self) {
        unsafe { sys::syscall1(sys::SYS_WIN_FLUSH, self.id) };
    }

    /// Kareyi bitirir ve bir sonraki kareye kadar **uyur**.
    ///
    /// `flush`'tan farki: uyuyan gorev zamanlayici tarafindan hic
    /// secilmez. Sadece `flush` kullanan bir uygulama, sirasi geldiginde
    /// hemen bir kare daha cizer -- yani ekranin yenilenme hizindan
    /// bagimsiz olarak CPU'yu doldurur. `frame(30)` ile 30 ms'lik bir
    /// kare butcesi verilir.
    pub fn frame(&self, ms: usize) {
        unsafe { sys::syscall1(sys::SYS_WIN_FLUSH, self.id) };
        sys::sleep_ms(ms);
    }

    /// Fareyi pencere ic koordinatlarina cevirir; pencere disindaysa None.
    ///
    /// Pencere govdesi baslik cubugunun altinda basladigi icin cevrim
    /// guncel `origin` + kenarlik payini kullanir.
    pub fn local_mouse(&mut self, m: Mouse) -> Option<(usize, usize)> {
        let (ox, oy) = self.origin();
        crate::canvas::to_local(m.x, m.y, ox, oy, self.width(), self.height())
    }
}
