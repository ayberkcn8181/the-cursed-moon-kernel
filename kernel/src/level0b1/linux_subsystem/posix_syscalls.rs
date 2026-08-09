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

use crate::arch::i386::regs::SyscallFrame;
use crate::level0a::core::mmu;
use crate::level0a::kernel_api::{self, KernelError};

pub const SYS_EXIT: u32 = 1;
pub const SYS_READ: u32 = 3;
pub const SYS_WRITE: u32 = 4;
pub const SYS_OPEN: u32 = 5;
pub const SYS_CLOSE: u32 = 6;
pub const SYS_BRK: u32 = 45;

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
            kernel_api::exit_current_task(arg1);
        }

        SYS_WRITE => match unsafe { kernel_api::write(arg1, arg2 as *const u8, arg3 as usize) } {
            Ok(written) => written as i32,
            Err(e) => errno_of(e),
        },

        SYS_READ => match unsafe { kernel_api::read(arg1, arg2 as *mut u8, arg3 as usize) } {
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

        SYS_CLOSE => match kernel_api::close(arg1) {
            Ok(()) => 0,
            Err(e) => errno_of(e),
        },

        SYS_BRK => kernel_api::brk(arg1 as usize) as i32,

        _ => {
            crate::println!(
                "[LEVEL-0b1] POSIX: desteklenmeyen syscall {} (-ENOSYS).",
                number
            );
            -ENOSYS
        }
    };

    frame.set_return(result as u32);
}

/// Kullanici alanindaki NUL sonlandirmali diziyi cekirdek tamponuna kopyalar.
///
/// Isaretci **once dogrulanir**: yalnizca Ring 3'e acilmis sayfalardan
/// okunur. Boylece kotu niyetli bir kullanici programi cekirdek belleginden
/// veri sizdirmak icin `sys_open` kullanamaz.
///
/// # Safety
/// Cagirann `storage`'i gecerli bir tampon olarak vermesi gerekir.
unsafe fn copy_user_cstr(ptr: u32, storage: &mut [u8; PATH_MAX]) -> Option<&str> {
    if ptr == 0 {
        return None;
    }

    let mut len = 0usize;
    while len < PATH_MAX {
        let addr = ptr as usize + len;

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
