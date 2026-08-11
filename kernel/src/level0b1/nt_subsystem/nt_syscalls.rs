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
//!
//! ## Iki tablo: `Nt*` ve `NtUser*`/`NtGdi*`
//!
//! Windows'ta sistem cagrilari **tek** bir tabloda degildir: cekirdek
//! yurutucusunun cagrilari (`ntoskrnl`, `Nt*`) ile pencere/cizim
//! cagrilari (`win32k.sys`, `NtUser*`/`NtGdi*`) ayri tablolardadir ve
//! ikincisinin indeksleri ayri bir aralikta tutulur. Iki grubun **donus
//! sozlesmesi de farklidir**:
//!
//!   * `Nt*`      -> NTSTATUS (0 = basari, 0xC... = hata)
//!   * `NtUser*`  -> dogrudan tutamac/BOOL (HWND, mesaj degeri, ...)
//!
//! TCMK bu ayrimi korur: 0x1000 araligi yurutucu, 0x2000 araligi win32k.
//! Boylece bir Windows programcisinin bekledigi anlam degismez -- pencere
//! acan cagri NTSTATUS degil HWND dondurur.

use crate::arch::cpu::regs::SyscallFrame;
use crate::level0a::core::mmu;
use crate::level0a::kernel_api::{self, KernelError};
use crate::level0a::{gui_api, wm};

// --- Yurutucu (ntoskrnl) tablosu: NTSTATUS dondururler --------------
pub const NT_TERMINATE_PROCESS: u32 = 0x1000;
pub const NT_WRITE_CONSOLE: u32 = 0x1001;
pub const NT_CREATE_FILE: u32 = 0x1002;
pub const NT_READ_FILE: u32 = 0x1003;
pub const NT_CLOSE: u32 = 0x1004;
pub const NT_DELAY_EXECUTION: u32 = 0x1005;
pub const NT_QUERY_SYSTEM_TIME: u32 = 0x1006;
pub const NT_YIELD_EXECUTION: u32 = 0x1007;

// --- win32k tablosu: tutamac/deger dondururler ------------------------
pub const NT_USER_CREATE_WINDOW: u32 = 0x2000;
pub const NT_GDI_GET_BITS: u32 = 0x2001;
pub const NT_USER_CLIENT_RECT: u32 = 0x2002;
pub const NT_USER_WINDOW_RECT: u32 = 0x2003;
pub const NT_USER_FLUSH_WINDOW: u32 = 0x2004;
pub const NT_USER_GET_MESSAGE: u32 = 0x2005;
pub const NT_USER_CURSOR_POS: u32 = 0x2006;

/// win32k araligi burada baslar.
const WIN32K_BASE: u32 = 0x2000;

/// `NtUser*` cagrilarinin "gecersiz tutamac" karsiligi (Windows'ta NULL
/// HWND'ye denk gelir; 0 gecerli bir pencere kimligi oldugu icin burada
/// -1 kullanilir).
const INVALID_HANDLE_VALUE: usize = usize::MAX;

// NTSTATUS degerleri (Windows ile ayni sayisal karsiliklar).
const STATUS_SUCCESS: u32 = 0x0000_0000;
const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
const STATUS_TOO_MANY_OPENED_FILES: u32 = 0xC000_011F;
const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;

const PATH_MAX: usize = 128;

/// `NtCreateFile`'in tanidigi CreateDisposition degerleri (Windows ile
/// ayni sayisal karsiliklar).
const FILE_CREATE: usize = 2;

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

    // win32k cagrilari NTSTATUS degil dogrudan deger dondurur; bu yuzden
    // asagidaki NTSTATUS govdesine hic girmezler.
    if number >= WIN32K_BASE {
        dispatch_win32k(frame);
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
            // NtCreateFile(ObjectName, CreateDisposition, -) -> handle EDX'te
            //
            // Basitlestirme: ObjectName duz bir C dizisi. Gercek NT'nin
            // OBJECT_ATTRIBUTES yapisi Faz 7b+ konusudur. CreateDisposition
            // Windows'un sayisal degerlerini kullanir; su an yalnizca
            // FILE_CREATE (2) ayirt edilir, digerleri "var olani ac".
            let mut storage = [0u8; PATH_MAX];
            let create = arg2 == FILE_CREATE;
            match unsafe { copy_user_cstr(arg1, &mut storage) } {
                Some(path) => match kernel_api::open(path, create) {
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

        NT_DELAY_EXECUTION => {
            // NtDelayExecution(Alertable, Interval). Gercek NT'de aralik
            // 100 ns birimli ve isaretli bir LARGE_INTEGER'dir; TCMK'nin
            // zamanlayici cozunurlugu 10 ms oldugu icin milisaniye alinir.
            crate::level0a::core::scheduler::sleep_ticks(((arg2 as u32) / 10).max(1));
            STATUS_SUCCESS
        }

        NT_QUERY_SYSTEM_TIME => {
            // NtQuerySystemTime(*SystemTime). Windows 1601'den beri gecen
            // 100 ns'leri verir; TCMK Unix zamanini verir -- cevirmenin
            // isi bicim uydurmak degil, saati **var etmek**. Saat yoksa
            // sifir doner ve cagiran bunu ayirt edebilir.
            set_out(frame, crate::level0a::drivers::rtc::unix_time() as usize);
            STATUS_SUCCESS
        }

        NT_YIELD_EXECUTION => {
            crate::level0a::core::scheduler::yield_now();
            STATUS_SUCCESS
        }

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

/// win32k (`NtUser*` / `NtGdi*`) tablosu.
///
/// Bu cagrilar Level-0a'nin `gui_api`'sine baglanir -- yani bir PE
/// uygulamasi ELF uygulamalariyla **ayni** pencere yoneticisini, ayni
/// kompozitoru ve ayni olay kuyrugunu kullanir. Iki dunyanin ortak
/// noktasi tam olarak burasidir: ceviri katmani ustte ayrisir, altta tek
/// bir cekirdek vardir.
///
/// Cizim yine cekirdekten gecmez: uygulama `NtGdiGetBits` ile piksel
/// tamponunun adresini alir ve dogrudan oraya yazar (POSIX tarafindaki
/// `SYS_WIN_BUFFER` ile ayni model).
fn dispatch_win32k(frame: &mut SyscallFrame) {
    let number = frame.number();
    let [arg1, arg2, arg3, _, _] = frame.args();

    let value: usize = match number {
        NT_USER_CREATE_WINDOW => {
            // NtUserCreateWindowEx(WindowName, (x<<16)|y, (cx<<16)|cy)
            let mut storage = [0u8; PATH_MAX];
            let title = unsafe { copy_user_cstr(arg1, &mut storage) }.unwrap_or("app");
            let (x, y) = (arg2 >> 16, arg2 & 0xFFFF);
            let (w, h) = (arg3 >> 16, arg3 & 0xFFFF);
            match gui_api::create_window(title, x, y, w, h) {
                Ok(id) => id,
                Err(_) => INVALID_HANDLE_VALUE,
            }
        }

        NT_GDI_GET_BITS => gui_api::window_buffer(arg1).unwrap_or(0),

        NT_USER_CLIENT_RECT => gui_api::window_size(arg1).unwrap_or(0),

        NT_USER_WINDOW_RECT => gui_api::window_pos(arg1).unwrap_or(INVALID_HANDLE_VALUE),

        NT_USER_FLUSH_WINDOW => {
            // Kompozitor her karede zaten cizer; bu cagri uygulamanin
            // CPU'yu biraktigi noktadir (POSIX tarafindaki flush ile ayni).
            crate::level0a::core::scheduler::yield_now();
            0
        }

        NT_USER_GET_MESSAGE => {
            // NtUserGetMessage: bekleyen tus yoksa 0. Gercek NT'de bu cagri
            // bloke olur; TCMK'de uygulamalar cizim dongusunu kendileri
            // surdugu icin yoklamali (polling) bicim dogru olan.
            if arg1 >= wm::MAX_WINDOWS {
                0
            } else {
                gui_api::poll_key(arg1) as usize
            }
        }

        NT_USER_CURSOR_POS => gui_api::mouse_state(),

        _ => {
            crate::println!(
                "[LEVEL-0b1] NT: desteklenmeyen win32k servisi {:#x}.",
                number
            );
            INVALID_HANDLE_VALUE
        }
    };

    frame.set_return(value);
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
