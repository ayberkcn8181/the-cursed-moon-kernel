//! Win32 API -- **ithal tablosu uzerinden** (`KERNEL32.dll`, `TCMKGUI.dll`).
//!
//! `nt` modulu sistem cagrilarini elle (`int 0x2E`) yapar; burasi ise
//! gercek bir Windows programinin yaptigini yapar: fonksiyonlari bir
//! DLL'den **ithal eder** ve cagrilar IAT (Import Address Table)
//! uzerinden gider.
//!
//! Bagladigimiz `.lib` dosyalari `llvm-dlltool` ile `win/*.def`'ten
//! uretilir; ortada gercek bir DLL **yoktur**. Cekirdek, ikiliyi
//! yuklerken ithal tablosunu okur, her adi gomulu tablosunda arar ve
//! surecin adres uzayina o servisi cagiran kucuk bir stub yazar
//! (bkz. `nt_subsystem/dll.rs`). Program aradaki farki goremez: IAT'de
//! buldugu sey yine bir kod adresidir.
//!
//! Bu, "Wine'in yapamadigini yapmak" iddiasinin en somut noktasidir --
//! ithal tablosu cozulmeden derleyicinin urettigi hicbir Windows ikilisi
//! calisamaz.

use core::ffi::c_void;

pub type Bool = i32;
pub type Dword = u32;
pub type Handle = u32;
pub type Hwnd = u32;

pub const INVALID_HANDLE_VALUE: Handle = 0xFFFF_FFFF;

pub const STD_OUTPUT_HANDLE: Handle = 1;

// CreateFileA -- dwDesiredAccess
pub const GENERIC_READ: Dword = 0x8000_0000;
pub const GENERIC_WRITE: Dword = 0x4000_0000;
// CreateFileA -- dwCreationDisposition
pub const CREATE_NEW: Dword = 1;
pub const CREATE_ALWAYS: Dword = 2;
pub const OPEN_EXISTING: Dword = 3;

#[link(name = "kernel32")]
extern "system" {
    pub fn ExitProcess(exit_code: Dword) -> !;
    pub fn Sleep(milliseconds: Dword);
    pub fn GetTickCount() -> Dword;
    pub fn CloseHandle(object: Handle) -> Bool;

    pub fn WriteConsoleA(
        console_output: Handle,
        buffer: *const u8,
        chars_to_write: Dword,
        chars_written: *mut Dword,
        reserved: *mut c_void,
    ) -> Bool;

    pub fn CreateFileA(
        file_name: *const u8,
        desired_access: Dword,
        share_mode: Dword,
        security_attributes: *mut c_void,
        creation_disposition: Dword,
        flags_and_attributes: Dword,
        template_file: Handle,
    ) -> Handle;

    pub fn ReadFile(
        file: Handle,
        buffer: *mut u8,
        bytes_to_read: Dword,
        bytes_read: *mut Dword,
        overlapped: *mut c_void,
    ) -> Bool;
    /// `ReadFile`in aynadaki esi. Bu cagri gelene kadar Windows
    /// uygulamalari dosya okuyabiliyor ama **yazamiyordu**; `winpad`
    /// notunu `WriteConsoleA`'ya bir dosya tutamagi vererek
    /// kaydediyordu -- calisiyordu ama Win32 sozlesmesi degildi.
    pub fn WriteFile(
        file: Handle,
        buffer: *const u8,
        bytes_to_write: Dword,
        bytes_written: *mut Dword,
        overlapped: *mut c_void,
    ) -> Bool;
}

#[link(name = "tcmkgui")]
extern "system" {
    pub fn TcmkCreateWindow(title: *const u8, x: Dword, y: Dword, cx: Dword, cy: Dword) -> Hwnd;
    pub fn TcmkGetWindowBits(window: Hwnd) -> *mut u32;
    /// (genislik << 16) | yukseklik
    pub fn TcmkGetClientRect(window: Hwnd) -> Dword;
    /// (x << 16) | y
    pub fn TcmkGetWindowRect(window: Hwnd) -> Dword;
    pub fn TcmkUpdateWindow(window: Hwnd) -> Bool;
    /// Bekleyen tus; yoksa 0.
    pub fn TcmkGetMessage(window: Hwnd) -> Dword;
    pub fn TcmkGetCursorPos() -> Dword;
}

// --- Guvenli sarmalayicilar -------------------------------------------

use crate::canvas::{self, Canvas, Mouse};

/// `WriteConsoleA` uzerinden konsola yazar.
pub struct Console;

impl core::fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut written: Dword = 0;
        unsafe {
            WriteConsoleA(
                STD_OUTPUT_HANDLE,
                s.as_ptr(),
                s.len() as Dword,
                &mut written,
                core::ptr::null_mut(),
            );
        }
        Ok(())
    }
}

/// Pencere tutamaci -- ithal edilen `TCMKGUI.dll` cagrilariyla.
pub struct Window {
    handle: Hwnd,
    canvas: Canvas,
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
    pub fn create(title: &str, x: usize, y: usize, width: usize, height: usize) -> Option<Window> {
        let mut name = [0u8; 64];
        let n = core::cmp::min(title.len(), name.len() - 1);
        name[..n].copy_from_slice(&title.as_bytes()[..n]);

        let handle = unsafe {
            TcmkCreateWindow(
                name.as_ptr(),
                x as Dword,
                y as Dword,
                width as Dword,
                height as Dword,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }

        let bits = unsafe { TcmkGetWindowBits(handle) };
        if bits.is_null() {
            return None;
        }

        let rect = unsafe { TcmkGetClientRect(handle) } as usize;

        Some(Window {
            handle,
            canvas: unsafe { Canvas::new(bits, rect >> 16, rect & 0xFFFF) },
            x,
            y,
        })
    }

    pub fn handle(&self) -> Hwnd {
        self.handle
    }

    /// Pencerenin **guncel** sol ust kosesi (WM surukleyebilir).
    pub fn window_rect(&mut self) -> (usize, usize) {
        let packed = unsafe { TcmkGetWindowRect(self.handle) } as usize;
        if packed != usize::MAX {
            self.x = packed >> 16;
            self.y = packed & 0xFFFF;
        }
        (self.x, self.y)
    }

    /// Bekleyen tus; yoksa 0.
    pub fn get_message(&self) -> u8 {
        unsafe { TcmkGetMessage(self.handle) as u8 }
    }

    /// Kareyi bitirir, CPU'yu birakir.
    pub fn update(&self) {
        unsafe { TcmkUpdateWindow(self.handle) };
    }

    /// Kareyi bitirir ve bir sonraki kareye kadar uyur.
    pub fn frame(&self, ms: usize) {
        self.update();
        unsafe { Sleep(ms as Dword) };
    }

    /// Fareyi pencere ic koordinatlarina cevirir.
    pub fn local_cursor(&mut self, m: Mouse) -> Option<(usize, usize)> {
        let (ox, oy) = self.window_rect();
        canvas::to_local(m.x, m.y, ox, oy, self.width(), self.height())
    }
}

/// `TcmkGetCursorPos` -> fare durumu.
pub fn cursor_pos() -> Mouse {
    canvas::unpack_mouse(unsafe { TcmkGetCursorPos() } as usize)
}
