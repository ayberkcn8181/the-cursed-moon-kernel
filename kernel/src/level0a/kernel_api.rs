//! Level-0a'nin disariya actigi **ortak cekirdek API'si**.
//!
//! Doc S.2.2.B: Level-0b1'in POSIX ve NT cevirmenleri, kendi ABI'lerini bu
//! notr API'ye cevirir; Level-0a'nin altindaki suruculere dogrudan
//! dokunmazlar. Boylece ayni `write` yolunu hem Linux'un `sys_write`'i hem
//! Windows'un `NtWriteFile`'i (Faz 7) paylasabilir.

/// Cekirdek ici soyut dosya tanimlayicilari. Faz 5'te gercek VFS/FD
/// tablosuyla degistirilecek.
pub const FD_STDOUT: u32 = 1;
pub const FD_STDERR: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelError {
    BadFileDescriptor,
    Fault,
}

/// `buf`'taki `len` bayti verilen tanimlayiciya yazar; yazilan bayt sayisini
/// dondurur.
///
/// # Safety
/// `buf`/`len` cagiran tarafindan gecerli, okunabilir bir bolge olarak
/// garanti edilmelidir. Faz 2'de henuz Ring 3 olmadigi icin kullanici/cekirdek
/// sinir kontrolu yapilmaz; bu kontrol Faz 3'te (Ring 3 + sayfa koruma)
/// eklenecektir.
pub unsafe fn write(fd: u32, buf: *const u8, len: usize) -> Result<usize, KernelError> {
    if fd != FD_STDOUT && fd != FD_STDERR {
        return Err(KernelError::BadFileDescriptor);
    }
    if buf.is_null() {
        return Err(KernelError::Fault);
    }

    let bytes = core::slice::from_raw_parts(buf, len);
    for &byte in bytes {
        crate::print!("{}", byte as char);
    }
    Ok(len)
}

/// Calisan gorevi sonlandirir (doc S.6: sys_exit).
pub fn exit_current_task(code: u32) -> ! {
    crate::println!(
        "[LEVEL-0a] gorev '{}' cikis kodu {} ile sonlandi.",
        crate::level0a::core::scheduler::current_name(),
        code
    );
    crate::level0a::core::scheduler::terminate_current()
}
