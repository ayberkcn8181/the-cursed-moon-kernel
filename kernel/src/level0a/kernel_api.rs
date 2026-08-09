//! Level-0a'nin disariya actigi **ortak cekirdek API'si**.
//!
//! Doc S.2.2.B: Level-0b1'in POSIX ve NT cevirmenleri, kendi ABI'lerini bu
//! notr API'ye cevirir; Level-0a'nin altindaki suruculere dogrudan
//! dokunmazlar. Boylece ayni `read`/`write` yolunu hem Linux'un
//! `sys_read`'i hem Windows'un `NtReadFile`'i (Faz 7) paylasabilir.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::{fd, vfs};

/// Standart POSIX tanimlayicilari.
pub const FD_STDIN: u32 = 0;
pub const FD_STDOUT: u32 = 1;
pub const FD_STDERR: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelError {
    BadFileDescriptor,
    Fault,
    NotFound,
    TooManyOpenFiles,
    NotSupported,
}

/// Program break (heap sinirı) -- `sys_brk` icin. Kullanici imaji
/// yuklendiginde `set_program_break` ile baslatilir.
static PROGRAM_BREAK: AtomicUsize = AtomicUsize::new(0);
/// Baslangic break'i: heap bunun altina inemez.
static PROGRAM_BREAK_START: AtomicUsize = AtomicUsize::new(0);
static PROGRAM_BREAK_LIMIT: AtomicUsize = AtomicUsize::new(0);

pub fn set_program_break(start: usize, limit: usize) {
    PROGRAM_BREAK.store(start, Ordering::Relaxed);
    PROGRAM_BREAK_START.store(start, Ordering::Relaxed);
    PROGRAM_BREAK_LIMIT.store(limit, Ordering::Relaxed);
}

/// `sys_brk` semantigi: 0 verilirse mevcut break dondurulur; gecerli bir
/// adres verilirse break oraya tasinir. Basarisizlikta break DEGISMEZ ve
/// eski deger dondurulur (Linux davranisi).
pub fn brk(requested: usize) -> usize {
    let current = PROGRAM_BREAK.load(Ordering::Relaxed);
    if requested == 0 {
        return current;
    }

    let floor = PROGRAM_BREAK_START.load(Ordering::Relaxed);
    let limit = PROGRAM_BREAK_LIMIT.load(Ordering::Relaxed);

    if requested < floor || requested > limit {
        return current;
    }

    PROGRAM_BREAK.store(requested, Ordering::Relaxed);
    requested
}

/// `buf`'taki `len` bayti verilen tanimlayiciya yazar.
///
/// # Safety
/// `buf`/`len` cagiran tarafindan gecerli, okunabilir bir bolge olarak
/// garanti edilmelidir.
pub unsafe fn write(fd_num: u32, buf: *const u8, len: usize) -> Result<usize, KernelError> {
    if buf.is_null() {
        return Err(KernelError::Fault);
    }

    match fd_num {
        FD_STDOUT | FD_STDERR => {
            let bytes = core::slice::from_raw_parts(buf, len);
            for &byte in bytes {
                crate::print!("{}", byte as char);
            }
            Ok(len)
        }
        // RAMFS dosyalarina yazma Faz 9-10 (tmpfs) konusudur.
        _ if fd::get(fd_num as usize).is_some() => Err(KernelError::NotSupported),
        _ => Err(KernelError::BadFileDescriptor),
    }
}

/// Yola gore dosya acar ve yeni bir tanimlayici dondurur.
pub fn open(path: &str) -> Result<usize, KernelError> {
    let node = vfs::lookup(path).ok_or(KernelError::NotFound)?;
    fd::allocate(node).ok_or(KernelError::TooManyOpenFiles)
}

/// Acik bir tanimlayicidan okur; okunan bayt sayisini dondurur.
///
/// # Safety
/// `buf`/`len` gecerli, yazilabilir bir bolge olmalidir.
pub unsafe fn read(fd_num: u32, buf: *mut u8, len: usize) -> Result<usize, KernelError> {
    if buf.is_null() {
        return Err(KernelError::Fault);
    }
    // stdin henuz bir cihaza bagli degil (klavye kuyrugu Faz 9+).
    if fd_num == FD_STDIN {
        return Ok(0);
    }

    let entry = fd::get(fd_num as usize).ok_or(KernelError::BadFileDescriptor)?;
    let slice = core::slice::from_raw_parts_mut(buf, len);
    let n = vfs::read(entry.node, entry.offset, slice).ok_or(KernelError::BadFileDescriptor)?;
    fd::advance(fd_num as usize, n);
    Ok(n)
}

pub fn close(fd_num: u32) -> Result<(), KernelError> {
    if fd::close(fd_num as usize) {
        Ok(())
    } else {
        Err(KernelError::BadFileDescriptor)
    }
}

/// Calisan gorevi/sureci sonlandirir (doc S.6: sys_exit).
///
/// Iki farkli baglam vardir:
///   - **Ring 3 sureci**: kullanici kendi yigininda, cekirdek ise TSS.esp0
///     yiginindadir. Gorev degistirme yapilamaz; saklanmis cekirdek baglami
///     geri yuklenerek `run_user_program`'in cagrildigi yere donulur.
///   - **Ring 0 cekirdek gorevi**: normal scheduler sonlandirmasi.
pub fn exit_current_task(code: u32) -> ! {
    if crate::arch::cpu::usermode::in_user_mode() {
        crate::println!("[LEVEL-0a] Ring 3 sureci cikis kodu {} ile sonlandi.", code);
        unsafe { crate::arch::cpu::usermode::leave_user_mode() }
    }

    crate::println!(
        "[LEVEL-0a] gorev '{}' cikis kodu {} ile sonlandi.",
        crate::level0a::core::scheduler::current_name(),
        code
    );
    crate::level0a::core::scheduler::terminate_current()
}
