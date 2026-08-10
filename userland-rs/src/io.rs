//! Metin cikisi ve dosya erisimi -- POSIX yolunun ince sarmalayicilari.

use core::fmt;

use crate::sys;

/// `core::fmt` ile stdout'a yazan sifir boyutlu yazici.
pub struct Stdout;

impl fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if sys::write(sys::STDOUT, s.as_bytes()) < 0 {
            return Err(fmt::Error);
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    let _ = Stdout.write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::io::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

/// Acik bir VFS dosyasi. `Drop` ile kapanir; boylece uygulamalar
/// tanimlayici sizdirmaz (cekirdek tarafinda `mem` komutu bunu gosterir).
pub struct File {
    fd: usize,
}

impl File {
    /// Yolu acar. Yol cekirdege NUL sonlandirmali gitmek zorunda oldugu
    /// icin once yigin uzerinde bir tampona kopyalanir.
    pub fn open(path: &str) -> Option<File> {
        Self::open_with(path, 0)
    }

    /// Dosyayi yazmak icin acar; yoksa olusturur (POSIX `O_CREAT`).
    ///
    /// Yalnizca kalici dosya sistemi (TCMKFS) yazilabilir; cekirdek
    /// imajina gomulu dosyalar salt okunurdur.
    pub fn create(path: &str) -> Option<File> {
        Self::open_with(path, sys::O_CREAT)
    }

    fn open_with(path: &str, flags: usize) -> Option<File> {
        let mut buf = [0u8; 128];
        if path.len() >= buf.len() {
            return None;
        }
        buf[..path.len()].copy_from_slice(path.as_bytes());
        // buf zaten sifir; buf[path.len()] = NUL.

        let fd = unsafe { sys::open_raw(buf.as_ptr(), flags) };
        if fd < 0 {
            None
        } else {
            Some(File { fd: fd as usize })
        }
    }

    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let n = sys::read(self.fd, buf);
        if n < 0 {
            0
        } else {
            n as usize
        }
    }

    /// Dosyanin mevcut konumuna yazar; yazilan bayt sayisini doner.
    pub fn write(&mut self, data: &[u8]) -> usize {
        let n = sys::write(self.fd, data);
        if n < 0 {
            0
        } else {
            n as usize
        }
    }

    pub fn fd(&self) -> usize {
        self.fd
    }
}

impl Drop for File {
    fn drop(&mut self) {
        sys::close(self.fd);
    }
}
