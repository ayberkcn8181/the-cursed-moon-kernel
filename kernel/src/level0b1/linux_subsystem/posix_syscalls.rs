//! Linux Ceviri Araci -- POSIX Subsystem (doc S.2.2.B).
//!
//! i386 Linux sistem cagrilarini yakalar ve Level-0a'nin ortak cekirdek
//! API'sine (`level0a::kernel_api`) cevirir. Bu dosya bilerek **hicbir
//! donanima dokunmaz**: gorevi yalnizca ABI cevirisidir.
//!
//! Desteklenen cagrilar (doc S.6, Faz 2):
//!   1 = sys_exit    (EBX = cikis kodu)
//!   4 = sys_write   (EBX = fd, ECX = buf, EDX = count)
//! sys_read/open/close Faz 3'te VFS ile birlikte gelecek.

use crate::arch::i386::regs::SyscallFrame;
use crate::level0a::kernel_api::{self, KernelError};

pub const SYS_EXIT: u32 = 1;
pub const SYS_WRITE: u32 = 4;

// Linux hata kodlari negatif dondurulur (ornegin -EBADF = -9).
const EBADF: i32 = 9;
const EFAULT: i32 = 14;
const ENOSYS: i32 = 38;

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
            Err(KernelError::BadFileDescriptor) => -EBADF,
            Err(KernelError::Fault) => -EFAULT,
        },
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
