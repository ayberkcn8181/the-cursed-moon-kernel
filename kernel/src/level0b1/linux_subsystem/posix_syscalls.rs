//! Linux Ceviri Araci -- POSIX Subsystem (doc S.2.2.B).
//!
//! i386 Linux sistem cagrilarini yakalar ve Level-0a'nin ortak cekirdek
//! API'sine (`level0a::kernel_api`) cevirir. Bu dosya bilerek **hicbir
//! donanima dokunmaz**: gorevi yalnizca ABI cevirisidir.
//!
//! Desteklenen cagrilar (doc S.6):
//!    1 = sys_exit    (EBX = cikis kodu)
//!    3 = sys_read    (EBX = fd,   ECX = buf,  EDX = count)
//!    4 = sys_write   (EBX = fd,   ECX = buf,  EDX = count)
//!    5 = sys_open    (EBX = path, ECX = flags, EDX = mode)
//!    6 = sys_close   (EBX = fd)
//!   45 = sys_brk     (EBX = yeni break, 0 ise mevcut break dondurulur)

use crate::arch::cpu::regs::SyscallFrame;
use crate::level0a::core::mmu;
use crate::level0a::kernel_api::{self, KernelError};

// Linux syscall numaralari MIMARIYE GORE DEGISIR -- ayni isim, farkli sayi.
// Bunu tek bir kumeyle gecistirmek Faz 4'te gercek bir hataya yol acti:
// x86_64 userland `write`(=1) cagirdi, cekirdek 1'i i386'nin `exit`'i sandi
// ve programi "cikis kodu 1" ile sonlandirdi.
#[cfg(target_arch = "x86")]
pub use i386_numbers::*;
#[cfg(target_arch = "x86_64")]
pub use x86_64_numbers::*;

#[cfg(target_arch = "x86")]
mod i386_numbers {
    pub const SYS_EXIT: u32 = 1;
    pub const SYS_READ: u32 = 3;
    pub const SYS_WRITE: u32 = 4;
    pub const SYS_OPEN: u32 = 5;
    pub const SYS_CLOSE: u32 = 6;
    pub const SYS_BRK: u32 = 45;
}

#[cfg(target_arch = "x86_64")]
mod x86_64_numbers {
    pub const SYS_READ: u32 = 0;
    pub const SYS_WRITE: u32 = 1;
    pub const SYS_OPEN: u32 = 2;
    pub const SYS_CLOSE: u32 = 3;
    pub const SYS_BRK: u32 = 12;
    pub const SYS_EXIT: u32 = 60;
}

// Linux hata kodlari negatif dondurulur (ornegin -EBADF = -9).
const EBADF: i32 = 9;
const EFAULT: i32 = 14;
const ENOENT: i32 = 2;
const EMFILE: i32 = 24;
const EINVAL: i32 = 22;
const ENOSYS: i32 = 38;

/// Kullanici alanindan gelen yol adinin en fazla uzunlugu.
const PATH_MAX: usize = 128;

fn errno_of(err: KernelError) -> i32 {
    match err {
        KernelError::BadFileDescriptor => -EBADF,
        KernelError::Fault => -EFAULT,
        KernelError::NotFound => -ENOENT,
        KernelError::TooManyOpenFiles => -EMFILE,
        KernelError::NotSupported => -EINVAL,
    }
}

/// Level-0b2 dispatcher'i tarafindan cagrilir. Donus degerini dogrudan
/// frame'in EAX alanina yazar (i386 Linux ABI).
pub fn dispatch(frame: &mut SyscallFrame) {
    let number = frame.number();
    let [arg1, arg2, arg3, _, _] = frame.args();

    let result: i32 = match number {
        SYS_EXIT => {
            // Geri donmez.
            kernel_api::exit_current_task(arg1 as u32);
        }

        SYS_WRITE => match unsafe { kernel_api::write(arg1 as u32, arg2 as *const u8, arg3) } {
            Ok(written) => written as i32,
            Err(e) => errno_of(e),
        },

        SYS_READ => match unsafe { kernel_api::read(arg1 as u32, arg2 as *mut u8, arg3) } {
            Ok(read) => read as i32,
            Err(e) => errno_of(e),
        },

        SYS_OPEN => {
            let mut storage = [0u8; PATH_MAX];
            match unsafe { copy_user_cstr(arg1, &mut storage) } {
                Some(path) => match kernel_api::open(path) {
                    Ok(fd) => fd as i32,
                    Err(e) => errno_of(e),
                },
                None => -EFAULT,
            }
        }

        SYS_CLOSE => match kernel_api::close(arg1 as u32) {
            Ok(()) => 0,
            Err(e) => errno_of(e),
        },

        SYS_BRK => {
            // brk bir ADRES dondurur, hata kodu degil -- isaretli
            // donusum yapilmadan dogrudan yazilir.
            frame.set_return(kernel_api::brk(arg1));
            return;
        }

        _ => {
            crate::println!(
                "[LEVEL-0b1] POSIX: desteklenmeyen syscall {} (-ENOSYS).",
                number
            );
            -ENOSYS
        }
    };

    frame.set_return(result as isize as usize);
}

/// Kullanici alanindaki NUL sonlandirmali diziyi cekirdek tamponuna kopyalar.
///
/// Isaretci **once dogrulanir**: yalnizca Ring 3'e acilmis sayfalardan
/// okunur. Boylece kotu niyetli bir kullanici programi cekirdek belleginden
/// veri sizdirmak icin `sys_open` kullanamaz.
///
/// # Safety
/// Cagirann `storage`'i gecerli bir tampon olarak vermesi gerekir.
unsafe fn copy_user_cstr(ptr: usize, storage: &mut [u8; PATH_MAX]) -> Option<&str> {
    if ptr == 0 {
        return None;
    }

    let mut len = 0usize;
    while len < PATH_MAX {
        let addr = ptr + len;

        // Kullaniciya ait olmayan bir sayfaya gecildiyse dur.
        if !mmu::is_user_accessible(addr) {
            return None;
        }

        let byte = (addr as *const u8).read();
        if byte == 0 {
            break;
        }
        storage[len] = byte;
        len += 1;
    }

    if len == PATH_MAX {
        return None; // NUL bulunamadi
    }

    core::str::from_utf8(&storage[..len]).ok()
}
