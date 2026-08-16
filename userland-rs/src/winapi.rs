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
    /// Win32'nin `lseek`i. Ayni cekirdek cagrisina iner: Level-0b1'in
    /// butun mesele ettigi sey bu -- iki ABI, tek Level-0a API'si.
    pub fn SetFilePointer(
        file: Handle,
        distance: Dword,
        distance_high: *mut Dword,
        method: Dword,
    ) -> Dword;
    pub fn GetFileSize(file: Handle, size_high: *mut Dword) -> Dword;

    /// Win32'nin dizin gezinmesi. POSIX'in `getdents`i ile **ayni**
    /// cekirdek koduna iner, yalnizca goruntusu farklidir: "ilk"i acmakla
    /// birlestirir ve her cagri bir [`Win32FindData`] doldurur.
    ///
    /// Desen esleme yok: sondaki `\*` atilir ve dizinin tamami listelenir,
    /// yani her desen `*` gibi davranir. Ters bolu isaretleri ve bastaki
    /// surucu harfi (`C:`) cekirdek tarafinda temizlenir.
    ///
    /// Bos dizinde (ya da yol yoksa) `INVALID_HANDLE_VALUE` doner --
    /// Windows'ta da oyledir.
    pub fn FindFirstFileA(file_name: *const u8, find_data: *mut Win32FindData) -> Handle;
    /// Sonraki girdi; dizin bittiginde 0 (FALSE).
    pub fn FindNextFileA(find: Handle, find_data: *mut Win32FindData) -> Bool;
    pub fn FindClose(find: Handle) -> Bool;

    /// Yeni bir dizin olusturur. Ust dizin **onceden var olmali** --
    /// Win32'de de oyledir (`SHCreateDirectoryEx` ara dizinleri kurar,
    /// `CreateDirectoryA` kurmaz).
    ///
    /// `lpSecurityAttributes` yok sayilir: TCMKFS'te izin/sahiplik yok.
    pub fn CreateDirectoryA(path_name: *const u8, security_attributes: *mut c_void) -> Bool;
    /// Bos bir dizini siler.
    pub fn RemoveDirectoryA(path_name: *const u8) -> Bool;
    /// Bir dosyayi siler. POSIX'teki `unlink` ile ayni cekirdek cagrisina
    /// iner; yalnizca diskteki dosyalar silinebilir.
    pub fn DeleteFileA(file_name: *const u8) -> Bool;
    /// Yeniden adlandirir ya da tasir. POSIX `rename` ile ayni cekirdek
    /// cagrisina iner.
    ///
    /// Hedef **zaten varsa basarisiz olur** -- Win32 `MoveFileA` da
    /// oyledir. (POSIX `rename` hedefin uzerine sessizce yazar; TCMK
    /// burada Win32'nin davranisini secti, bkz. README.)
    pub fn MoveFileA(existing_name: *const u8, new_name: *const u8) -> Bool;

    /// Son basarisiz cagrinin sebebi.
    ///
    /// Win32'nin hata bildirimi POSIX'inkinden yapisal olarak farkli:
    /// cagrilar `BOOL` doner (basari/hata), **sebep** ayri bir surec
    /// basina degiskende durur. POSIX ayni bilgiyi negatif errno olarak
    /// dogrudan donus degerinde tasir.
    ///
    /// Windows'ta oldugu gibi yalnizca **basarisiz** cagrilar bu degeri
    /// yazar; basarili bir cagri onceki degeri silmez.
    pub fn GetLastError() -> Dword;
    /// Uygulamanin kendi hatasini bildirmesi icin.
    pub fn SetLastError(error_code: Dword);

    /// Calisma dizinini degistirir. POSIX'teki `chdir` ile ayni cekirdek
    /// cagrisina iner.
    pub fn SetCurrentDirectoryA(path_name: *const u8) -> Bool;
    /// Calisma dizinini `lpBuffer`a yazar.
    ///
    /// Win32 sozlesmesi POSIX'inkinden farkli: yer yeterse **yazilan**
    /// uzunluk (NUL haric), yetmezse **gereken** uzunluk (NUL dahil)
    /// doner. Sifir donmesi gercek bir hatadir.
    pub fn GetCurrentDirectoryA(buffer_length: Dword, buffer: *mut u8) -> Dword;

    /// Surecin komut satiri -- **bolunmemis tek bir dize**.
    ///
    /// POSIX'in `argv`siyle arasindaki fark yapisal: orada cekirdek
    /// argumanlari yiginda bir **dizi** olarak verir, burada tek bir
    /// metin doner ve bolmek cagirana (gercek Windows'ta CRT'ye) kalir.
    /// Ayni sozlesmeyi tek bicime indirmek iki taraftan birinin
    /// beklentisini bozardi.
    pub fn GetCommandLineA() -> *const u8;

    /// Adi verilen ortam degiskeninin degerini `lpBuffer`a yazar.
    ///
    /// POSIX'te ayni bilgi baslangic yigininda bir `environ` **dizisi**
    /// olarak gelir ve aramayi program yapar; burada arama cekirdekte,
    /// program yalnizca adi verir. Donus sozlesmesi
    /// `GetCurrentDirectoryA` ile ayni: yer yeterse yazilan uzunluk
    /// (NUL haric), yetmezse gereken uzunluk (NUL dahil). Sifir
    /// donmesi "degisken yok" demektir ve `GetLastError`
    /// `ERROR_ENVVAR_NOT_FOUND` birakir.
    pub fn GetEnvironmentVariableA(name: *const u8, buffer: *mut u8, size: Dword) -> Dword;

    /// Surecin **kendi** ortam blogundaki bir degiskeni degistirir.
    ///
    /// Windows'ta da boyledir: cagri baska surecleri etkilemez ama
    /// `CreateProcess` ile dogan cocuga gecer. `lpValue` NULL verilirse
    /// degisken silinir -- POSIX'in `unsetenv`i.
    ///
    /// POSIX tarafinda bunun karsiligi bir sistem cagrisi **degildir**:
    /// orada ortam surecin kendi belleginde durur ve `setenv` onu
    /// dogrudan duzenler.
    pub fn SetEnvironmentVariableA(name: *const u8, value: *const u8) -> Bool;
}

// --- `GetLastError` kodlari (Windows ile ayni sayilar) ----------------
pub const ERROR_SUCCESS: Dword = 0;
pub const ERROR_FILE_NOT_FOUND: Dword = 2;
pub const ERROR_TOO_MANY_OPEN_FILES: Dword = 4;
pub const ERROR_ACCESS_DENIED: Dword = 5;
pub const ERROR_INVALID_HANDLE: Dword = 6;
/// `FindNextFileA` dizin bittiginde bunu birakir -- dongunun **normal**
/// sonlanma sebebi budur, gercek bir hata degildir.
pub const ERROR_NO_MORE_FILES: Dword = 18;
pub const ERROR_NOT_SUPPORTED: Dword = 50;
pub const ERROR_DISK_FULL: Dword = 112;
pub const ERROR_DIR_NOT_EMPTY: Dword = 145;
pub const ERROR_ALREADY_EXISTS: Dword = 183;
/// `GetEnvironmentVariableA` adi bulamayinca birakir. Sifir donusu ile
/// birlikte "yok" demektir -- POSIX'te bunun karsiligi yalnizca `NULL`.
pub const ERROR_ENVVAR_NOT_FOUND: Dword = 203;

/// `FILETIME` -- 1601-01-01'den beri gecen 100 nanosaniyelik araliklar.
///
/// Tek bir `u64` **degil** iki `DWORD`: Windows'ta da oyledir ve fark
/// onemli -- `u64` alani 8'e hizalanip yapiya dolgu sokabilir, iki
/// `DWORD` ise 4'e hizali kalir. `WIN32_FIND_DATAA`nin 320 baytlik
/// yerlesimi buna bagli.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FileTime {
    pub low: Dword,
    pub high: Dword,
}

impl FileTime {
    pub const ZERO: FileTime = FileTime { low: 0, high: 0 };

    /// Iki yarimi tek sayida birlestirir.
    pub fn value(&self) -> u64 {
        ((self.high as u64) << 32) | self.low as u64
    }

    /// Unix zaman damgasina cevirir; bilgi yoksa 0.
    pub fn to_unix(&self) -> u32 {
        let raw = self.value();
        if raw == 0 {
            return 0;
        }
        (raw / 10_000_000).saturating_sub(11_644_473_600) as u32
    }
}

/// `WIN32_FIND_DATAA` -- Windows'takiyle **birebir** yerlesim (320 bayt).
///
/// Sadelestirilmis bir kayit ikili uyumu bozardi: bir PE'nin derlendigi
/// basliklar bu ofsetleri varsayar. Cekirdek doldurmadigi alanlari sifir
/// birakir (Windows'ta da bilgi yoksa oyle olur).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Win32FindData {
    pub attributes: Dword,
    pub creation_time: FileTime,
    pub last_access_time: FileTime,
    pub last_write_time: FileTime,
    pub size_high: Dword,
    pub size_low: Dword,
    pub reserved0: Dword,
    pub reserved1: Dword,
    pub file_name: [u8; 260],
    pub alternate_file_name: [u8; 14],
}

impl Win32FindData {
    pub const fn zeroed() -> Self {
        Win32FindData {
            attributes: 0,
            creation_time: FileTime::ZERO,
            last_access_time: FileTime::ZERO,
            last_write_time: FileTime::ZERO,
            size_high: 0,
            size_low: 0,
            reserved0: 0,
            reserved1: 0,
            file_name: [0; 260],
            alternate_file_name: [0; 14],
        }
    }

    pub fn is_directory(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    }

    /// `cFileName`in NUL'a kadarki kismi.
    pub fn name(&self) -> &str {
        let end = self
            .file_name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.file_name.len());
        core::str::from_utf8(&self.file_name[..end]).unwrap_or("?")
    }
}

/// `dwFileAttributes` degerleri.
pub const FILE_ATTRIBUTE_DIRECTORY: Dword = 0x10;
pub const FILE_ATTRIBUTE_NORMAL: Dword = 0x80;

/// `SetFilePointer` -- `dwMoveMethod`. Sayilar POSIX'in `SEEK_*`
/// degerleriyle ayni.
pub const FILE_BEGIN: Dword = 0;
pub const FILE_CURRENT: Dword = 1;
pub const FILE_END: Dword = 2;
/// `SetFilePointer`/`GetFileSize` hata donusu.
pub const INVALID_SET_FILE_POINTER: Dword = 0xFFFF_FFFF;

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
