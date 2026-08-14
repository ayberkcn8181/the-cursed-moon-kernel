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

// --- Win32 API tablosu (gomulu DLL thunk'lari) -----------------------
//
// Bu araligin **cagri sozlesmesi digerlerinden farklidir**: argumanlar
// registerlerde degil, `EDX`'in gosterdigi yigin blogundadir (bkz.
// `dll.rs`). Windows'un x86 syscall stub'i da boyle yapar. Kazanci:
// parametre sayisi uc ile sinirli degildir, yani `CreateFileA`'nin yedi
// parametresi ve `WriteConsoleA`'nin cikti parametresi desteklenebilir.
pub const NT_EXIT_PROCESS_W32: u32 = 0x3000;
pub const NT_SLEEP_MS: u32 = 0x3001;
pub const NT_GET_TICK_COUNT: u32 = 0x3002;
pub const NT_WIN32_CLOSE_HANDLE: u32 = 0x3003;
pub const NT_WRITE_CONSOLE_A: u32 = 0x3004;
pub const NT_CREATE_FILE_A: u32 = 0x3005;
pub const NT_READ_FILE_WIN32: u32 = 0x3006;
pub const NT_WRITE_FILE_WIN32: u32 = 0x3007;
pub const NT_SET_FILE_POINTER: u32 = 0x3008;
pub const NT_GET_FILE_SIZE: u32 = 0x3009;

pub const NT_USER_CREATE_WINDOW_W32: u32 = 0x3010;
pub const NT_GDI_GET_BITS_W32: u32 = 0x3011;
pub const NT_USER_CLIENT_RECT_W32: u32 = 0x3012;
pub const NT_USER_WINDOW_RECT_W32: u32 = 0x3013;
pub const NT_USER_FLUSH_WINDOW_W32: u32 = 0x3014;
pub const NT_USER_GET_MESSAGE_W32: u32 = 0x3015;
pub const NT_USER_CURSOR_POS_W32: u32 = 0x3016;

/// win32k araligi burada baslar.
const WIN32K_BASE: u32 = 0x2000;

/// Win32 API araligi (yigin argumanli) burada baslar.
const WIN32_API_BASE: u32 = 0x3000;

/// Win32'nin `BOOL`'u.
const WIN32_TRUE: usize = 1;
const WIN32_FALSE: usize = 0;

/// `CreateFileA`'nin dwCreationDisposition degerleri (Win32 ile ayni).
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_ALWAYS: u32 = 4;

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

    // Gomulu DLL thunk'larindan gelen Win32 API cagrilari: argumanlar
    // registerlerde degil, EDX'in gosterdigi yigin blogunda.
    if number >= WIN32_API_BASE {
        dispatch_win32_api(frame);
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

/// Gomulu DLL'lerin (`KERNEL32.dll`, `TCMKGUI.dll`) arkasindaki Win32 API
/// tablosu.
///
/// Buradaki cagrilar bir thunk'tan gelir (bkz. `dll.rs`) ve argumanlarini
/// **cagiranin yigininda** birakir; `EDX` ilk argumani gosterir. Windows'un
/// x86 syscall stub'i da boyle davranir. Kazanci, `dll.rs`'te anlatildigi
/// gibi, parametre sayisinin uc ile sinirli olmamasidir -- boylece
/// `CreateFileA`'nin yedi parametresi ve `WriteConsoleA`'nin **cikti**
/// parametresi gercek imzalariyla desteklenir.
///
/// Donus degeri Win32 sozlesmesine uyar: `BOOL` icin 1/0, tutamac icin
/// tutamacin kendisi, hata icin `INVALID_HANDLE_VALUE`.
fn dispatch_win32_api(frame: &mut SyscallFrame) {
    let number = frame.number();
    let args = arg_block(frame);

    let value: usize = match number {
        NT_EXIT_PROCESS_W32 => {
            // ExitProcess(UINT uExitCode) -- geri donmez.
            kernel_api::exit_current_task(arg(args, 0).unwrap_or(0));
        }

        NT_SLEEP_MS => {
            // Sleep(DWORD dwMilliseconds)
            let ms = arg(args, 0).unwrap_or(0);
            if ms > 0 {
                crate::level0a::core::scheduler::sleep_ticks((ms / 10).max(1));
            } else {
                crate::level0a::core::scheduler::yield_now();
            }
            WIN32_FALSE
        }

        NT_GET_TICK_COUNT => {
            // GetTickCount() -> acilistan beri gecen milisaniye.
            // PIT 100 Hz oldugu icin cozunurluk 10 ms'dir; Windows'ta da
            // bu cagri kaba cozunurluklu olmakla bilinir.
            crate::level0a::pit::ticks() as usize * 10
        }

        NT_WIN32_CLOSE_HANDLE => {
            // CloseHandle(HANDLE) -> BOOL
            match arg(args, 0) {
                Some(handle) => match kernel_api::close(handle) {
                    Ok(()) => WIN32_TRUE,
                    Err(_) => WIN32_FALSE,
                },
                None => WIN32_FALSE,
            }
        }

        NT_WRITE_CONSOLE_A => {
            // WriteConsoleA(hOutput, lpBuffer, nChars, lpCharsWritten, lpReserved)
            match (arg(args, 0), arg_ptr(args, 1), arg(args, 2)) {
                (Some(handle), Some(buffer), Some(count)) => {
                    match unsafe {
                        kernel_api::write(handle, buffer as *const u8, count as usize)
                    } {
                        Ok(written) => {
                            // Cikti parametresi istege baglidir; NULL degilse
                            // doldurulur (Win32 sozlesmesi).
                            store_out(args, 3, written as u32);
                            WIN32_TRUE
                        }
                        Err(_) => WIN32_FALSE,
                    }
                }
                _ => WIN32_FALSE,
            }
        }

        NT_CREATE_FILE_A => {
            // CreateFileA(lpFileName, dwDesiredAccess, dwShareMode,
            //             lpSecurityAttributes, dwCreationDisposition,
            //             dwFlagsAndAttributes, hTemplateFile)
            //
            // TCMK'de paylasim kipi, guvenlik tanimlayicisi ve oznitelikler
            // yok sayilir; anlam tasiyan iki parametre ad ve dispositiondir.
            let mut storage = [0u8; PATH_MAX];
            let disposition = arg(args, 4).unwrap_or(0);
            let create = matches!(disposition, CREATE_NEW | CREATE_ALWAYS | OPEN_ALWAYS);
            match arg_ptr(args, 0) {
                Some(name) => match unsafe { copy_user_cstr(name, &mut storage) } {
                    Some(path) => match kernel_api::open(path, create) {
                        Ok(handle) => handle,
                        Err(_) => INVALID_HANDLE_VALUE,
                    },
                    None => INVALID_HANDLE_VALUE,
                },
                None => INVALID_HANDLE_VALUE,
            }
        }

        NT_READ_FILE_WIN32 => {
            // ReadFile(hFile, lpBuffer, nBytes, lpBytesRead, lpOverlapped)
            match (arg(args, 0), arg_ptr(args, 1), arg(args, 2)) {
                (Some(handle), Some(buffer), Some(count)) => {
                    match unsafe { kernel_api::read(handle, buffer as *mut u8, count as usize) } {
                        Ok(read) => {
                            store_out(args, 3, read as u32);
                            WIN32_TRUE
                        }
                        Err(_) => WIN32_FALSE,
                    }
                }
                _ => WIN32_FALSE,
            }
        }

        NT_WRITE_FILE_WIN32 => {
            // WriteFile(hFile, lpBuffer, nBytes, lpBytesWritten, lpOverlapped)
            //
            // `ReadFile`in aynadaki esi. Bu cagriya kadar Windows
            // uygulamalari dosya **okuyabiliyor ama yazamiyordu**;
            // `winpad` notunu `WriteConsoleA`'ya bir dosya tutamagi
            // vererek kaydediyordu -- calisiyordu, cunku ikisi de ayni
            // `kernel_api::write`e iniyor, ama Win32 sozlesmesi degildi:
            // `WriteConsoleA`nin dorduncu parametresi "yazilan karakter
            // sayisi"dir ve konsol icin tanimlidir, dosya icin degil.
            match (arg(args, 0), arg_ptr(args, 1), arg(args, 2)) {
                (Some(handle), Some(buffer), Some(count)) => {
                    match unsafe {
                        kernel_api::write(handle, buffer as *const u8, count as usize)
                    } {
                        Ok(written) => {
                            store_out(args, 3, written as u32);
                            WIN32_TRUE
                        }
                        Err(_) => WIN32_FALSE,
                    }
                }
                _ => WIN32_FALSE,
            }
        }

        NT_SET_FILE_POINTER => {
            // SetFilePointer(hFile, lDistance, lpDistanceHigh, dwMoveMethod)
            //
            // Win32'nin `lseek`i. Ayni cekirdek cagrisina iniyor --
            // Level-0b1'in butun mesele ettigi sey bu: iki ABI, tek
            // Level-0a API'si. `lpDistanceHigh` (64-bit uzantisi) yok
            // sayilir; dosyalar 160 KiB ile sinirli.
            //
            // FILE_BEGIN/CURRENT/END sayilari POSIX'in SEEK_* degerleriyle
            // ayni (0/1/2), yani cevirme gerekmiyor.
            match (arg(args, 0), arg(args, 1), arg(args, 3)) {
                (Some(handle), Some(distance), Some(method)) => {
                    match kernel_api::lseek(handle, distance as usize, method as usize) {
                        Ok(position) => position,
                        // Win32 hatada INVALID_SET_FILE_POINTER (0xFFFFFFFF) doner.
                        Err(_) => u32::MAX as usize,
                    }
                }
                _ => u32::MAX as usize,
            }
        }

        NT_GET_FILE_SIZE => {
            // GetFileSize(hFile, lpFileSizeHigh) -> DWORD
            match arg(args, 0) {
                Some(handle) => match kernel_api::file_size(handle) {
                    Ok(size) => size,
                    Err(_) => u32::MAX as usize,
                },
                None => u32::MAX as usize,
            }
        }

        // --- TCMKGUI.dll: pencere cagrilari ---
        NT_USER_CREATE_WINDOW_W32 => {
            // TcmkCreateWindow(lpTitle, x, y, cx, cy) -> HWND
            //
            // win32k tarafindaki karsiligindan farki, olculerin tek
            // kelimeye paketlenmemis olmasi: DLL cagrisinda parametreler
            // yiginda oldugu icin paketlemeye gerek yok.
            let mut storage = [0u8; PATH_MAX];
            let title = match arg_ptr(args, 0) {
                Some(p) => unsafe { copy_user_cstr(p, &mut storage) }.unwrap_or("app"),
                None => "app",
            };
            let x = arg(args, 1).unwrap_or(0) as usize;
            let y = arg(args, 2).unwrap_or(0) as usize;
            let cx = arg(args, 3).unwrap_or(0) as usize;
            let cy = arg(args, 4).unwrap_or(0) as usize;
            match gui_api::create_window(title, x, y, cx, cy) {
                Ok(id) => id,
                Err(_) => INVALID_HANDLE_VALUE,
            }
        }

        NT_GDI_GET_BITS_W32 => match arg(args, 0) {
            Some(h) => gui_api::window_buffer(h as usize).unwrap_or(0),
            None => 0,
        },

        NT_USER_CLIENT_RECT_W32 => match arg(args, 0) {
            Some(h) => gui_api::window_size(h as usize).unwrap_or(0),
            None => 0,
        },

        NT_USER_WINDOW_RECT_W32 => match arg(args, 0) {
            Some(h) => gui_api::window_pos(h as usize).unwrap_or(INVALID_HANDLE_VALUE),
            None => INVALID_HANDLE_VALUE,
        },

        NT_USER_FLUSH_WINDOW_W32 => {
            crate::level0a::core::scheduler::yield_now();
            WIN32_TRUE
        }

        NT_USER_GET_MESSAGE_W32 => match arg(args, 0) {
            Some(h) if (h as usize) < wm::MAX_WINDOWS => gui_api::poll_key(h as usize) as usize,
            _ => 0,
        },

        NT_USER_CURSOR_POS_W32 => gui_api::mouse_state(),

        _ => {
            crate::println!(
                "[LEVEL-0b1] NT: desteklenmeyen Win32 API servisi {:#x}.",
                number
            );
            INVALID_HANDLE_VALUE
        }
    };

    frame.set_return(value);
}

/// Thunk'in biraktigi arguman blogunun adresi (i386'da EDX).
#[cfg(target_arch = "x86")]
fn arg_block(frame: &SyscallFrame) -> usize {
    frame.edx as usize
}

#[cfg(target_arch = "x86_64")]
fn arg_block(frame: &SyscallFrame) -> usize {
    frame.rdx as usize
}

/// Arguman blogunda bir yuvanin genisligi.
///
/// i386 `__stdcall`'da her arguman yigina 4 bayt olarak itilir. Win64'te
/// ise her arguman **8 bayt** yer kaplar -- bir `DWORD` bile. Bunun
/// sebebi, x64 thunk'inin register argumanlarini cagiranin ayirdigi
/// "golge alana" (shadow space) dokmesi ve o alanin 8'er baytlik
/// yuvalardan olusmasidir (bkz. `dll::emit_thunk`).
#[cfg(target_arch = "x86")]
const ARG_SLOT: usize = 4;
#[cfg(target_arch = "x86_64")]
const ARG_SLOT: usize = 8;

/// Arguman blogundan `index`. yuvayi **isaretci genisliginde** okur.
///
/// POSIX/NT tarafindakiyle ayni guvenlik kurali: kullanici alanindan gelen
/// her adres once `mmu::is_user_accessible` ile dogrulanir. Bir uygulama
/// bozuk bir EDX/RDX ile gelirse cagri sessizce basarisiz olur, cekirdek
/// gecersiz bellek okumaz.
fn arg_ptr(block: usize, index: usize) -> Option<usize> {
    if block == 0 {
        return None;
    }
    let addr = block + index * ARG_SLOT;
    // Yuva iki sayfaya yayilamaz varsayimi yapilmaz: her iki uc da ayri
    // ayri dogrulanir.
    if !mmu::is_user_accessible(addr) || !mmu::is_user_accessible(addr + ARG_SLOT - 1) {
        return None;
    }
    Some(unsafe {
        #[cfg(target_arch = "x86")]
        {
            (addr as *const u32).read_unaligned() as usize
        }
        #[cfg(target_arch = "x86_64")]
        {
            (addr as *const u64).read_unaligned() as usize
        }
    })
}

/// Arguman blogundan bir `DWORD` okur.
///
/// Win64'te yuva 8 bayttir ama `DWORD` yalnizca alt 32 biti kullanir --
/// ust yarisi tanimsizdir. Bu yuzden olcu/bayrak gibi degerler bu yoldan,
/// isaretciler ise `arg_ptr` ile okunur.
fn arg(block: usize, index: usize) -> Option<u32> {
    arg_ptr(block, index).map(|v| v as u32)
}

/// Cikti parametresini doldurur (`lpNumberOfBytesWritten` gibi).
///
/// Win32'de bu isaretciler NULL olabilir; NULL ise yazilmaz. Isaretci
/// gecersizse de yazilmaz -- cagri yine de basarili sayilir, cunku asil
/// is (yazma/okuma) yapilmistir.
fn store_out(block: usize, index: usize, value: u32) {
    let Some(addr) = arg_ptr(block, index) else {
        return;
    };
    if addr == 0 || !mmu::is_user_accessible(addr) || !mmu::is_user_accessible(addr + 3) {
        return;
    }
    unsafe { (addr as *mut u32).write_unaligned(value) };
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
