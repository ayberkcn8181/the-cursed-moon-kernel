//! Pencere API'si (NT tarafi) -- `win32k` cagrilarinin tipli
//! sarmalayicilari.
//!
//! `gui`'nin yaptigini yapar, ama `int 0x80`/POSIX yerine `int 0x2E`/NT
//! kullanir. Iki modulu yan yana okumak TCMK'nin ana iddiasini gosterir:
//!
//! ```text
//!   ELF uygulamasi              PE uygulamasi
//!        |                            |
//!   gui::Window                  win32::Hwnd
//!        |  int 0x80                  |  int 0x2E
//!        v                            v
//!   Level-0b1 POSIX            Level-0b1 NT
//!        \                          /
//!         \                        /
//!          ---> Level-0a gui_api <--        (tek pencere yoneticisi)
//! ```
//!
//! Cizim tarafinda ayrim **hic yoktur**: `Hwnd` de `gui::Window` de ayni
//! `canvas::Canvas`'a `Deref` eder. Yani bir uygulamayi Windows ikilisi
//! olarak derlemek, cizim kodunun tek satirini bile degistirmez.

use crate::canvas::{self, Canvas, Mouse};
use crate::nt;

/// Pencere tutamaci -- Windows'un `HWND`'sinin TCMK karsiligi.
pub struct Hwnd {
    handle: usize,
    canvas: Canvas,
    x: usize,
    y: usize,
}

impl core::ops::Deref for Hwnd {
    type Target = Canvas;
    fn deref(&self) -> &Canvas {
        &self.canvas
    }
}

impl core::ops::DerefMut for Hwnd {
    fn deref_mut(&mut self) -> &mut Canvas {
        &mut self.canvas
    }
}

impl Hwnd {
    /// `NtUserCreateWindowEx` -> HWND. Basarisizsa `None`.
    ///
    /// Windows'ta bu cagri bir pencere sinifi (`WNDCLASS`) ve mesaj
    /// yordami (`WndProc`) ister. TCMK'de pencere sinifi yoktur ve mesaj
    /// dongusu uygulamanindir; bu yuzden imza dogrudan olculere iner.
    pub fn create(title: &str, x: usize, y: usize, width: usize, height: usize) -> Option<Hwnd> {
        let mut name = [0u8; 64];
        let n = core::cmp::min(title.len(), name.len() - 1);
        name[..n].copy_from_slice(&title.as_bytes()[..n]);

        let handle = unsafe {
            nt::syscall3(
                nt::NT_USER_CREATE_WINDOW,
                name.as_ptr() as usize,
                (x << 16) | (y & 0xFFFF),
                (width << 16) | (height & 0xFFFF),
            )
        };
        if handle == usize::MAX {
            return None;
        }

        // `NtGdiGetBits`: piksel tamponunun bu surecin adres uzayindaki
        // adresi. Cizim buradan sonra tamamen Ring 3'te kalir.
        let bits = unsafe { nt::syscall1(nt::NT_GDI_GET_BITS, handle) };
        if bits == 0 {
            return None;
        }

        let rect = unsafe { nt::syscall1(nt::NT_USER_CLIENT_RECT, handle) };

        Some(Hwnd {
            handle,
            canvas: unsafe { Canvas::new(bits as *mut u32, rect >> 16, rect & 0xFFFF) },
            x,
            y,
        })
    }

    pub fn handle(&self) -> usize {
        self.handle
    }

    /// `NtUserGetWindowRect` -> pencerenin **guncel** sol ust kosesi.
    pub fn window_rect(&mut self) -> (usize, usize) {
        let packed = unsafe { nt::syscall1(nt::NT_USER_WINDOW_RECT, self.handle) };
        if packed != usize::MAX {
            self.x = packed >> 16;
            self.y = packed & 0xFFFF;
        }
        (self.x, self.y)
    }

    /// `NtUserGetMessage` -> bekleyen tus; yoksa 0.
    ///
    /// Gercek NT'de bu cagri mesaj gelene kadar bloke olur. TCMK'de
    /// uygulamalar cizim dongusunu kendileri surdugu icin yoklamali
    /// bicim dogru olan: `get_message()` cagirmak kareyi durdurmaz.
    pub fn get_message(&self) -> u8 {
        unsafe { nt::syscall1(nt::NT_USER_GET_MESSAGE, self.handle) as u8 }
    }

    /// `NtUserFlushWindow`: kareyi bitirir, CPU'yu birakir.
    pub fn flush(&self) {
        unsafe { nt::syscall1(nt::NT_USER_FLUSH_WINDOW, self.handle) };
    }

    /// Kareyi bitirir ve bir sonraki kareye kadar **uyur**
    /// (`NtDelayExecution`). Yalnizca `flush` kullanan bir uygulama
    /// sirasi geldiginde hemen bir kare daha cizer, yani CPU'yu doldurur.
    pub fn frame(&self, ms: usize) {
        self.flush();
        nt::delay_execution(ms);
    }

    /// Fareyi pencere ic koordinatlarina cevirir; pencere disindaysa None.
    pub fn local_cursor(&mut self, m: Mouse) -> Option<(usize, usize)> {
        let (ox, oy) = self.window_rect();
        crate::canvas::to_local(m.x, m.y, ox, oy, self.width(), self.height())
    }
}

/// `NtUserGetCursorPos` -> fare durumu (ekran koordinatlari).
pub fn cursor_pos() -> Mouse {
    canvas::unpack_mouse(unsafe { nt::syscall0(nt::NT_USER_CURSOR_POS) })
}

/// Konsola yazar (`NtWriteConsole`). Windows'un `WriteConsoleA`'si gibi.
pub struct Console;

impl core::fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        nt::write_console(nt::STD_OUTPUT_HANDLE, s.as_bytes());
        Ok(())
    }
}

/// `GetSystemTime` benzeri: Unix zamanini takvime cevirir.
///
/// Cekirdek `NtQuerySystemTime`'da ham saniyeyi verir; gun/ay/yil
/// hesabi burada, Ring 3'te yapilir (Hinnant'in civil_from_days
/// algoritmasi -- cekirdektekiyle ayni).
#[derive(Clone, Copy, Default)]
pub struct SystemTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

pub fn system_time() -> SystemTime {
    from_unix(nt::query_system_time())
}

fn from_unix(epoch: u32) -> SystemTime {
    let days = (epoch / 86400) as i32;
    let seconds = epoch % 86400;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;

    SystemTime {
        year: (if month <= 2 { y + 1 } else { y }) as u16,
        month,
        day,
        hour: (seconds / 3600) as u8,
        minute: ((seconds % 3600) / 60) as u8,
        second: (seconds % 60) as u8,
    }
}
