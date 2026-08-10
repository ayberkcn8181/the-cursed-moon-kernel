//! Windows Ceviri Araci -- NT Subsystem (doc S.2.2.B).
//!
//! NT API cagrilarini (`NtWriteFile`, `NtTerminateProcess` vb.) Level-0a'nin
//! ortak cekirdek API'sine cevirir. POSIX cevirmeni gibi burasi da **hicbir
//! donaniuma dokunmaz**; tek isi ABI ve hata kodu cevirisidir.
//!
//! Cagri yolu (doc S.3, Windows senaryosu):
//!   Level-1 (PE, int 0x2E) -> Level-0b2 dispatcher -> [BURASI] -> Level-0a
//!
//! ABI (i386): EAX = NT servis numarasi, EBX/ECX/EDX = arg1..3, donus EAX.
//! Gercek Windows'ta servis numaralari surume gore degisir; TCMK kendi
//! kararli numaralandirmasini kullanir (0x1000+), boylece bir syscall'in
//! POSIX mi NT mi oldugu numaradan da ayirt edilebilir.

use crate::arch::cpu::regs::SyscallFrame;
use crate::level0a::core::mmu;
use crate::level0a::kernel_api::{self, KernelError};

pub const NT_TERMINATE_PROCESS: u32 = 0x1000;
pub const NT_WRITE_CONSOLE: u32 = 0x1001;
pub const NT_CREATE_FILE: u32 = 0x1002;
pub const NT_READ_FILE: u32 = 0x1003;
pub const NT_CLOSE: u32 = 0x1004;

// NTSTATUS degerleri (Windows ile ayni sayisal karsiliklar).
const STATUS_SUCCESS: u32 = 0x0000_0000;
const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
const STATUS_TOO_MANY_OPENED_FILES: u32 = 0xC000_011F;
const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;

const PATH_MAX: usize = 128;

fn ntstatus_of(err: KernelError) -> u32 {
    match err {
        KernelError::BadFileDescriptor => STATUS_INVALID_HANDLE,
        KernelError::Fault => STATUS_ACCESS_VIOLATION,
        KernelError::NotFound => STATUS_OBJECT_NAME_NOT_FOUND,
        KernelError::TooManyOpenFiles => STATUS_TOO_MANY_OPENED_FILES,
        KernelError::NotSupported => STATUS_NOT_IMPLEMENTED,
    }
}

/// Bir syscall numarasinin NT tarafina ait olup olmadigi.
pub fn is_nt_service(number: u32) -> bool {
    number >= NT_TERMINATE_PROCESS
}

/// Level-0b2 dispatcher'i tarafindan cagrilir (int 0x2E).
pub fn dispatch(frame: &mut SyscallFrame) {
    let number = frame.number();
    let [arg1, arg2, arg3, _, _] = frame.args();

    // int 0x2E yalnizca NT servis araligini kabul eder. POSIX numaralari
    // buradan girmeye calisirsa reddedilir -- iki ABI'nin karismasi
    // (ornegin POSIX sys_exit'in NT sanilmasi) boylece engellenir.
    if !is_nt_service(number) {
        crate::println!(
            "[LEVEL-0b1] NT: {} NT servis araliginda degil (int 0x80 mi olmaliydi?).",
            number
        );
        frame.set_return(STATUS_NOT_IMPLEMENTED as usize);
        return;
    }

    let status: u32 = match number {
        NT_TERMINATE_PROCESS => {
            // NtTerminateProcess(ProcessHandle, ExitStatus). Geri donmez.
            kernel_api::exit_current_task(arg2 as u32);
        }

        NT_WRITE_CONSOLE => {
            // NtWriteConsole(Handle, Buffer, Length)
            match unsafe { kernel_api::write(arg1 as u32, arg2 as *const u8, arg3) } {
                Ok(_) => STATUS_SUCCESS,
                Err(e) => ntstatus_of(e),
            }
        }

        NT_CREATE_FILE => {
            // NtCreateFile(ObjectName, DesiredAccess, OutHandle)
            // Basitlestirme: ObjectName duz bir C dizisi, handle EBX'in
            // gosterdigi yere degil dogrudan EAX'in ustune yazilir --
            // gercek NT'nin OBJECT_ATTRIBUTES yapisi Faz 7b+ konusudur.
            let mut storage = [0u8; PATH_MAX];
            match unsafe { copy_user_cstr(arg1, &mut storage) } {
                Some(path) => match kernel_api::open(path, false) {
                    // Handle'i cagirana EDX uzerinden bildiriyoruz.
                    Ok(handle) => {
                        set_out(frame, handle);
                        STATUS_SUCCESS
                    }
                    Err(e) => ntstatus_of(e),
                },
                None => STATUS_ACCESS_VIOLATION,
            }
        }

        NT_READ_FILE => {
            // NtReadFile(Handle, Buffer, Length) -> okunan bayt EDX'te
            match unsafe { kernel_api::read(arg1 as u32, arg2 as *mut u8, arg3) } {
                Ok(read) => {
                    set_out(frame, read);
                    STATUS_SUCCESS
                }
                Err(e) => ntstatus_of(e),
            }
        }

        NT_CLOSE => match kernel_api::close(arg1 as u32) {
            Ok(()) => STATUS_SUCCESS,
            Err(e) => ntstatus_of(e),
        },

        _ => {
            crate::println!(
                "[LEVEL-0b1] NT: desteklenmeyen servis {:#x} (STATUS_NOT_IMPLEMENTED).",
                number
            );
            STATUS_NOT_IMPLEMENTED
        }
    };

    frame.set_return(status as usize);
}

/// NT cagrilarinin "cikti" degerini (handle, okunan bayt sayisi) cagirana
/// bildirdigi register: i386'da EDX, x86_64'te RDX. Mimariden bagimsiz
/// kalmasi icin tek yerde toplanmistir.
#[cfg(target_arch = "x86")]
fn set_out(frame: &mut SyscallFrame, value: usize) {
    frame.edx = value as u32;
}

#[cfg(target_arch = "x86_64")]
fn set_out(frame: &mut SyscallFrame, value: usize) {
    frame.rdx = value as u64;
}

/// POSIX tarafindakiyle ayni guvenlik kurali: kullanici alanindan gelen
/// isaretci once `mmu::is_user_accessible` ile dogrulanir.
unsafe fn copy_user_cstr(ptr: usize, storage: &mut [u8; PATH_MAX]) -> Option<&str> {
    if ptr == 0 {
        return None;
    }

    let mut len = 0usize;
    while len < PATH_MAX {
        let addr = ptr + len;
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
        return None;
    }

    core::str::from_utf8(&storage[..len]).ok()
}
