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

use super::modules;
use super::seh;

/// Gomulu DLL sayisi -- `FreeLibrary` taniticiyi dogrularken kullanir.
fn dll_count() -> usize {
    super::dll::count()
}

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
pub const NT_FIND_FIRST_FILE: u32 = 0x300A;
pub const NT_FIND_NEXT_FILE: u32 = 0x300B;
pub const NT_FIND_CLOSE: u32 = 0x300C;
pub const NT_CREATE_DIRECTORY_A: u32 = 0x300D;
pub const NT_REMOVE_DIRECTORY_A: u32 = 0x300E;
pub const NT_DELETE_FILE_A: u32 = 0x300F;
pub const NT_MOVE_FILE_A: u32 = 0x3017;
pub const NT_GET_LAST_ERROR: u32 = 0x3018;
pub const NT_SET_LAST_ERROR: u32 = 0x3019;
pub const NT_SET_CURRENT_DIRECTORY: u32 = 0x301A;
pub const NT_GET_CURRENT_DIRECTORY: u32 = 0x301B;
pub const NT_GET_COMMAND_LINE_A: u32 = 0x301C;
pub const NT_GET_ENVIRONMENT_VARIABLE_A: u32 = 0x301D;
pub const NT_SET_ENVIRONMENT_VARIABLE_A: u32 = 0x301E;
pub const NT_GET_FILE_ATTRIBUTES_A: u32 = 0x301F;
pub const NT_GET_SYSTEM_TIME_AS_FILE_TIME: u32 = 0x3020;
pub const NT_GET_SYSTEM_TIME: u32 = 0x3021;
pub const NT_FLUSH_FILE_BUFFERS: u32 = 0x3022;
pub const NT_GET_VERSION_EX_A: u32 = 0x3023;
pub const NT_GET_CURRENT_PROCESS_ID: u32 = 0x3024;
pub const NT_GET_CURRENT_THREAD_ID: u32 = 0x3025;
pub const NT_SET_END_OF_FILE: u32 = 0x3026;
pub const NT_GET_MODULE_FILE_NAME_A: u32 = 0x3027;
pub const NT_CREATE_PROCESS_A: u32 = 0x3028;
pub const NT_WAIT_FOR_SINGLE_OBJECT: u32 = 0x3029;
pub const NT_GET_EXIT_CODE_PROCESS: u32 = 0x302A;
pub const NT_ADD_VECTORED_EXCEPTION_HANDLER: u32 = 0x302B;
pub const NT_REMOVE_VECTORED_EXCEPTION_HANDLER: u32 = 0x302C;
pub const NT_RAISE_EXCEPTION: u32 = 0x302D;
/// Istisna dagitim tramplenin donus kapisi.
///
/// `KERNEL32.dll` ihracat listesinde **yoktur** ve olmamalidir: bunu
/// program degil, cekirdegin surecin adres uzayina yazdigi tramplen
/// cagirir (bkz. `teb::emit_trampoline`). Gercek Windows'ta karsiligi
/// `ntdll`nin ic `NtContinue` cagrisidir.
pub const NT_CONTINUE_DISPATCH: u32 = 0x302E;
pub const NT_GET_MODULE_HANDLE_A: u32 = 0x302F;
pub const NT_GET_PROC_ADDRESS: u32 = 0x3030;
pub const NT_LOAD_LIBRARY_A: u32 = 0x3031;
pub const NT_FREE_LIBRARY: u32 = 0x3032;

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
/// Var olan dosyayi acar ve **bosaltir**; yoksa hata. POSIX'te
/// `O_TRUNC` (yalniz, `O_CREAT` olmadan) ile ayni anlam.
const TRUNCATE_EXISTING: u32 = 5;

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
const STATUS_OBJECT_NAME_COLLISION: u32 = 0xC000_0035;
const STATUS_DIRECTORY_NOT_EMPTY: u32 = 0xC000_0101;
const STATUS_DISK_FULL: u32 = 0xC000_007F;
const STATUS_MEDIA_WRITE_PROTECTED: u32 = 0xC000_00A2;

const PATH_MAX: usize = 128;

// --- Win32 hata kodlari (`GetLastError`) ------------------------------
//
// Windows'takiyle **ayni sayilar**: bir PE bunlari `winerror.h`den
// derlenmis sabitlerle karsilastirir, yani TCMK'ye ozgu numaralandirma
// ikili uyumu bozardi.
pub const ERROR_SUCCESS: u32 = 0;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_TOO_MANY_OPEN_FILES: u32 = 4;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_HANDLE: u32 = 6;
/// Istenen DLL yuklu degil (`GetModuleHandleA`/`LoadLibraryA`).
const ERROR_MOD_NOT_FOUND: u32 = 126;
/// DLL var ama fonksiyon yok (`GetProcAddress`).
const ERROR_PROC_NOT_FOUND: u32 = 127;
/// `FindNextFileA` dizin bittiginde bunu birakir -- Windows'ta dongunun
/// **normal** sonlanma sebebi budur, gercek bir hata degildir.
const ERROR_NO_MORE_FILES: u32 = 18;
const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_DISK_FULL: u32 = 112;
const ERROR_DIR_NOT_EMPTY: u32 = 145;
const ERROR_ALREADY_EXISTS: u32 = 183;
// --- `dwFileAttributes` degerleri (Windows ile ayni sayilar) ---------
//
// Iki cagri paylasiyor: `FindFirstFileA` her girdi icin bu bayraklari
// `WIN32_FIND_DATAA`ya yaziyor, `GetFileAttributesA` ayni kumeyi
// **donus degeri** olarak veriyor.
/// Salt okunur -- TCMK'de RAMFS (cekirdek imajinin parcasi).
const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
/// Baska hicbir bayrak yoksa **bu** donmeli; sifir gecerli bir cevap
/// degildir.
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
/// `GetFileAttributesA`nin hata donusu -- `0` degil, tum bitler bir.
const INVALID_FILE_ATTRIBUTES: usize = 0xFFFF_FFFF;

/// Surec tutamaci isareti.
///
/// TCMK'de dosya tutamaclari **tanimlayici numarasidir** (kucuk sayilar);
/// surec tutamaci da bir gorev kimligi, yani ayni araliktan. Ikisini
/// ayirmadan `CloseHandle` bir sureci kapatirken ayni numarali dosyayi
/// kapatirdi. En ust bit isaret olarak kullaniliyor: Windows'ta tutamac
/// degerleri zaten opaktir, cagiran onlari yorumlamaz.
const PROCESS_HANDLE_FLAG: usize = 0x8000_0000;

/// Surec hala calisiyor -- `GetExitCodeProcess` bunu doner.
const STILL_ACTIVE: u32 = 259;

/// Beklenmis cocuklarin cikis kodlari.
///
/// Gerekli, cunku bekleme gorev yuvasini **geri veriyor**: kod
/// zamanlayicida artik yok. Windows'ta cikis kodunu ayakta tutan sey
/// acik tutamactir; TCMK'de o rolu bu kucuk tablo oynuyor.
///
/// `u32::MAX` "bu yuva icin kayit yok" demek -- gecerli bir cikis kodu
/// olamayacak kadar buyuk oldugu icin ayrik bir isaret gerekmiyor.
static REAPED_EXIT: [core::sync::atomic::AtomicU32;
    crate::level0a::core::scheduler::MAX_TASKS] = [const {
    core::sync::atomic::AtomicU32::new(u32::MAX)
}; crate::level0a::core::scheduler::MAX_TASKS];
/// `WaitForSingleObject`: nesne isaretlendi (surec bitti).
const WAIT_OBJECT_0: usize = 0;
/// `WaitForSingleObject`: sure doldu.
const WAIT_TIMEOUT: usize = 0x102;
/// `WaitForSingleObject`: tutamac gecersiz.
const WAIT_FAILED: usize = 0xFFFF_FFFF;

/// Tampon yetmedi -- `GetModuleFileNameA` kirptiginda birakir.
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
/// Cagriya verilen parametre gecersiz (bos ad, okunamayan isaretci).
const ERROR_INVALID_PARAMETER: u32 = 87;
/// Adi verilen ortam degiskeni yok.
const ERROR_ENVVAR_NOT_FOUND: u32 = 203;

/// Surec basina son hata kodu.
///
/// **Level-0b1'e aittir, Level-0a'ya degil**: `GetLastError` bir Win32
/// sozlesmesidir, cekirdek API'sinin kavrami degil. POSIX tarafi ayni
/// bilgiyi negatif errno olarak dogrudan donus degerinde tasir, yani
/// boyle bir yan kanala ihtiyaci yok. Ayrimi burada tutmak, iki ABI'nin
/// birbirinin bicimini odunc almamasini sagliyor.
static LAST_ERROR: [core::sync::atomic::AtomicU32; crate::level0a::core::scheduler::MAX_TASKS] =
    [const { core::sync::atomic::AtomicU32::new(0) };
        crate::level0a::core::scheduler::MAX_TASKS];

fn error_slot() -> &'static core::sync::atomic::AtomicU32 {
    let id = crate::level0a::core::scheduler::current_id();
    &LAST_ERROR[id % crate::level0a::core::scheduler::MAX_TASKS]
}

/// Son hatayi yazar. Windows'ta **yalnizca basarisiz** cagrilar yazar;
/// basarili bir cagri onceki degeri silmez, o yuzden burada da oyle.
fn set_last_error(code: u32) {
    error_slot().store(code, core::sync::atomic::Ordering::Relaxed);
    // TEB'e de yaz: gercek Windows'ta `GetLastError` **yalnizca** oradan
    // okur, cekirdege hic gitmez. Iki yerin ayrilmasi, TEB'i dogrudan
    // okuyan derlenmis bir kodun yanlis deger gormesi demek olurdu.
    super::teb::store_last_error(crate::level0a::core::scheduler::current_id(), code);
}

/// Yeni bir imaj yuklenirken son hatayi sifirlar.
///
/// Gorev yuvalari geri kazanildigi icin sart: temizlenmeseydi yeni bir
/// surec, ayni yuvada calismis onceki surecin hatasini gorurdu.
pub fn clear_last_error(task: usize) {
    if task < crate::level0a::core::scheduler::MAX_TASKS {
        LAST_ERROR[task].store(ERROR_SUCCESS, core::sync::atomic::Ordering::Relaxed);
    }
}

/// Cekirdek hatasini Win32 kodunun karsiligina cevirir.
fn win32_error_of(err: KernelError) -> u32 {
    match err {
        KernelError::NotFound => ERROR_FILE_NOT_FOUND,
        KernelError::BadFileDescriptor => ERROR_INVALID_HANDLE,
        KernelError::Fault => ERROR_ACCESS_DENIED,
        KernelError::TooManyOpenFiles => ERROR_TOO_MANY_OPEN_FILES,
        KernelError::NotSupported => ERROR_NOT_SUPPORTED,
        KernelError::AlreadyExists => ERROR_ALREADY_EXISTS,
        KernelError::NotEmpty => ERROR_DIR_NOT_EMPTY,
        KernelError::NoSpace => ERROR_DISK_FULL,
        // Windows salt okunur bir hedefte de bunu dondurur.
        KernelError::ReadOnly => ERROR_ACCESS_DENIED,
    }
}

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
        KernelError::AlreadyExists => STATUS_OBJECT_NAME_COLLISION,
        KernelError::NotEmpty => STATUS_DIRECTORY_NOT_EMPTY,
        KernelError::NoSpace => STATUS_DISK_FULL,
        KernelError::ReadOnly => STATUS_MEDIA_WRITE_PROTECTED,
    }
}

/// Bir syscall numarasinin NT tarafina ait olup olmadigi.
pub fn is_nt_service(number: u32) -> bool {
    number >= NT_TERMINATE_PROCESS
}

/// Level-0b2 dispatcher'i tarafindan cagrilir (int 0x2E).
pub fn dispatch(frame: &mut SyscallFrame, from_interrupt: bool) {
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
        dispatch_win32_api(frame, from_interrupt);
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
fn dispatch_win32_api(frame: &mut SyscallFrame, from_interrupt: bool) {
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
            //
            // Windows'ta tek bir `CloseHandle` her tur nesneyi kapatir --
            // dosya, surec, olay. TCMK'de ayrim tutamacin en ust bitinde
            // (bkz. `PROCESS_HANDLE_FLAG`), cunku iki tur ayni sayi
            // araligini kullaniyor.
            //
            // Surec tutamacini kapatmak **basarili bir no-op**: TCMK'de
            // nesne sayaci yok, gorev yuvasi zaten cikista geri veriliyor.
            // Hata dondurmek, tutamacini duzgunce kapatan bir programi
            // yaniltirdi.
            match arg(args, 0) {
                Some(handle) if handle as usize & PROCESS_HANDLE_FLAG != 0 => WIN32_TRUE,
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
            // `CREATE_ALWAYS` ve `TRUNCATE_EXISTING` dosyayi **bosaltir**.
            // Bu cagriya kadar bosaltma yoktu ve `winpad` gibi bir
            // duzenleyici, kisa bir metni uzun bir dosyanin uzerine
            // yazdiginda kuyrukta eski icerigi birakiyordu -- sessiz bir
            // hata, cunku kaydetme "basarili" donuyordu.
            let truncating = matches!(disposition, CREATE_ALWAYS | TRUNCATE_EXISTING);
            // Yol **normalize edilmeli**: Windows uygulamalari `C:\dizin\ad`
            // yazar. Bu cagri uzun sure ham yolu kullaniyordu ve kimse
            // fark etmemisti, cunku TCMK'nin kendi PE uygulamalari POSIX
            // tarzi yollar veriyordu. Bir olcum `C:\tmp\...` deneyince
            // ortaya cikti: ad goreli sayilip **koke** yaziliyor, ve
            // `GetFileAttributesA` (normalize eden) onu bulamiyordu.
            // Yani dosya yaratiliyor ama "kaybediliyordu".
            match arg_ptr(args, 0) {
                Some(name) => match unsafe { copy_user_cstr(name, &mut storage) }
                    .map(|p| p.len())
                    .map(|length| normalize_win_path(&mut storage, length))
                {
                    Some(path) => match if truncating {
                        kernel_api::open_truncating(path, create)
                    } else {
                        kernel_api::open(path, create)
                    } {
                        Ok(handle) => handle,
                        Err(e) => {
                            set_last_error(win32_error_of(e));
                            INVALID_HANDLE_VALUE
                        }
                    },
                    None => {
                        set_last_error(ERROR_ACCESS_DENIED);
                        INVALID_HANDLE_VALUE
                    }
                },
                None => {
                    set_last_error(ERROR_ACCESS_DENIED);
                    INVALID_HANDLE_VALUE
                }
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

        NT_FIND_FIRST_FILE => {
            // FindFirstFileA(lpFileName, lpFindFileData) -> HANDLE
            //
            // Win32'nin dizin gezinmesi POSIX'inkinden farkli goruntu
            // verir -- "ilk"i acmakla birlestirir, kayit paketlemez, her
            // cagri bir `WIN32_FIND_DATAA` doldurur -- ama altinda ayni
            // cekirdek gezinmesi calisir (`kernel_api::next_dir_entry`).
            // Bir ELF ile bir PE ayni dizini listeleyip farkli sonuc
            // alamaz; Level-0b1'in varlik sebebi de tam olarak bu.
            let mut storage = [0u8; PATH_MAX];
            let opened = arg_ptr(args, 0)
                .and_then(|p| unsafe { copy_user_cstr(p, &mut storage) })
                .map(|pattern| pattern.len())
                .map(|length| normalize_win_path(&mut storage, length))
                .and_then(|pattern| kernel_api::open_dir(strip_wildcard(pattern)).ok());
            match opened {
                Some(handle) => match kernel_api::next_dir_entry(handle as u32) {
                    Ok(entry) if store_find_data(args, 1, &entry) => handle,
                    // Bos dizin (ya da bozuk cikti isaretcisi): Win32'de de
                    // INVALID_HANDLE_VALUE'dur. Tutamac sizmasin diye
                    // burada kapatilir.
                    _ => {
                        let _ = kernel_api::close(handle as u32);
                        set_last_error(ERROR_NO_MORE_FILES);
                        INVALID_HANDLE_VALUE
                    }
                },
                None => {
                    set_last_error(ERROR_FILE_NOT_FOUND);
                    INVALID_HANDLE_VALUE
                }
            }
        }

        NT_FIND_NEXT_FILE => {
            // FindNextFileA(hFindFile, lpFindFileData) -> BOOL
            match arg(args, 0) {
                Some(handle) => match kernel_api::next_dir_entry(handle) {
                    Ok(entry) if store_find_data(args, 1, &entry) => WIN32_TRUE,
                    // Dizinin bitmesi Windows'ta hata degil, dongunun
                    // normal sonu: `ERROR_NO_MORE_FILES` tam olarak bunu
                    // ayirt etmek icin var.
                    Ok(_) => {
                        set_last_error(ERROR_ACCESS_DENIED);
                        WIN32_FALSE
                    }
                    Err(_) => {
                        set_last_error(ERROR_NO_MORE_FILES);
                        WIN32_FALSE
                    }
                },
                None => WIN32_FALSE,
            }
        }

        // FindClose(hFindFile) -> BOOL. Tutamac normal bir tanimlayici
        // oldugu icin kapatmasi da normal `close`.
        NT_FIND_CLOSE => match arg(args, 0) {
            Some(handle) => match kernel_api::close(handle) {
                Ok(()) => WIN32_TRUE,
                Err(_) => WIN32_FALSE,
            },
            None => WIN32_FALSE,
        },

        // Dosya sistemi yazma islemleri -- POSIX'teki
        // `mkdir`/`rmdir`/`unlink` ile ayni `kernel_api` girislerine
        // iner. Ucu de Win32 sozlesmesi geregi BOOL doner (basari 1,
        // hata 0); gercek Windows'ta ayrinti `GetLastError` ile
        // alinir, TCMK'de o kavram yok.
        NT_CREATE_DIRECTORY_A => win32_path_action(args, kernel_api::mkdir),
        NT_REMOVE_DIRECTORY_A => win32_path_action(args, kernel_api::rmdir),
        NT_DELETE_FILE_A => win32_path_action(args, kernel_api::unlink),

        NT_MOVE_FILE_A => {
            // MoveFileA(lpExistingFileName, lpNewFileName) -> BOOL
            //
            // Tek yol argumanli kardeslerinden ayri duruyor cunku **iki**
            // yol tasiyor; ikisi de ayri tamponlara kopyalanir.
            let mut old_storage = [0u8; PATH_MAX];
            let mut new_storage = [0u8; PATH_MAX];
            let old_len = arg_ptr(args, 0)
                .and_then(|p| unsafe { copy_user_cstr(p, &mut old_storage) })
                .map(|p| p.len());
            let new_len = arg_ptr(args, 1)
                .and_then(|p| unsafe { copy_user_cstr(p, &mut new_storage) })
                .map(|p| p.len());
            match (old_len, new_len) {
                // Ayirici cevrimi iki yolda da gerekli. Iki tampon ayri
                // degiskenler oldugu icin oduncleri de ayri: ikisi ayni
                // anda tutulabiliyor.
                (Some(o), Some(n)) => {
                    let old = normalize_win_path(&mut old_storage, o);
                    let new = normalize_win_path(&mut new_storage, n);
                    match kernel_api::rename(old, new) {
                        Ok(()) => WIN32_TRUE,
                        Err(e) => {
                            set_last_error(win32_error_of(e));
                            WIN32_FALSE
                        }
                    }
                }
                _ => WIN32_FALSE,
            }
        }

        // GetLastError() -> DWORD. Argumansiz.
        NT_GET_LAST_ERROR => {
            error_slot().load(core::sync::atomic::Ordering::Relaxed) as usize
        }

        // SetLastError(dwErrCode). Windows'ta uygulamalarin kendi
        // hatalarini bildirmesi icindir; donus degeri yoktur.
        NT_SET_LAST_ERROR => {
            set_last_error(arg(args, 0).unwrap_or(0));
            0
        }

        // SetCurrentDirectoryA(lpPathName) -> BOOL.
        // POSIX'teki `chdir` ile ayni cekirdek cagrisina iner.
        NT_SET_CURRENT_DIRECTORY => win32_path_action(args, kernel_api::chdir),

        // GetCurrentDirectoryA(nBufferLength, lpBuffer) -> DWORD
        //
        // Win32 sozlesmesi POSIX'inkinden farkli ve **ikisi de** burada
        // korunuyor: yeterli yer varsa yazilan uzunluk (NUL haric),
        // yetmiyorsa **gereken** uzunluk (NUL dahil) doner. Cagiran
        // boylece tamponu buyutup yeniden deneyebilir.
        NT_GET_CURRENT_DIRECTORY => {
            let path = kernel_api::getcwd();
            let capacity = arg(args, 0).unwrap_or(0) as usize;
            match arg_ptr(args, 1) {
                Some(buffer) if buffer != 0 => {
                    if capacity < path.len() + 1 {
                        // Yer yetmedi: **gereken** boyut doner, tampona
                        // dokunulmaz. Cagiran buyutup yeniden dener.
                        path.len() + 1
                    } else if !mmu::is_user_accessible(buffer)
                        || !mmu::is_user_accessible(buffer + path.len())
                    {
                        0
                    } else {
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                path.as_ptr(),
                                buffer as *mut u8,
                                path.len(),
                            );
                            (buffer as *mut u8).add(path.len()).write(0);
                        }
                        path.len()
                    }
                }
                _ => 0,
            }
        }

        // GetCommandLineA() -> LPSTR
        //
        // Win32'nin arguman sozlesmesi POSIX'inkinden **yapisal olarak**
        // farkli: burada bolunmemis **tek bir dize** doner, bolmek
        // CRT'nin (`CommandLineToArgvW`) isidir. POSIX tarafi ayni
        // argumanlari yiginda `argc`/`argv` **dizisi** olarak alir.
        //
        // Cekirdek argumanlari bir kez aliyor; ayrilan yalnizca sunum
        // (bkz. `process::build_start_stack`).
        NT_GET_COMMAND_LINE_A => crate::level0b1::process::command_line_ptr(),

        // GetEnvironmentVariableA(lpName, lpBuffer, nSize) -> DWORD
        //
        // Win32'nin ortam erisimi POSIX'inkinden yine yapisal olarak
        // farkli: burada **adla sorulur** ve deger tampona yazilir.
        // POSIX tarafi ayni bilgiyi baslangic yigininda bir **dizi**
        // (`environ`) olarak alir ve aramayi kendi yapar.
        //
        // Donus sozlesmesi `GetCurrentDirectoryA` ile ayni kalipta:
        // yer yeterse yazilan uzunluk (NUL haric), yetmezse gereken
        // uzunluk (NUL dahil), degisken yoksa 0.
        NT_GET_ENVIRONMENT_VARIABLE_A => {
            let mut name_storage = [0u8; PATH_MAX];
            let name_len = arg_ptr(args, 0)
                .and_then(|p| unsafe { copy_user_cstr(p, &mut name_storage) })
                .map(|n| n.len());
            let value = name_len
                .and_then(|len| core::str::from_utf8(&name_storage[..len]).ok())
                .and_then(kernel_api::getenv);

            match value {
                // Degisken yok: Win32'de bu bir **hata**, bos dize
                // degil. POSIX `getenv` yalnizca NULL doner; buradaki
                // `GetLastError` ayrimi cagirana "yoktu" ile "tampon
                // yetmedi"yi ayirt ettirir.
                None => {
                    set_last_error(ERROR_ENVVAR_NOT_FOUND);
                    0
                }
                Some(value) => {
                    let capacity = arg(args, 2).unwrap_or(0) as usize;
                    match arg_ptr(args, 1) {
                        Some(buffer) if buffer != 0 => {
                            if capacity < value.len() + 1 {
                                value.len() + 1
                            } else if !mmu::is_user_accessible(buffer)
                                || !mmu::is_user_accessible(buffer + value.len())
                            {
                                0
                            } else {
                                unsafe {
                                    core::ptr::copy_nonoverlapping(
                                        value.as_ptr(),
                                        buffer as *mut u8,
                                        value.len(),
                                    );
                                    (buffer as *mut u8).add(value.len()).write(0);
                                }
                                value.len()
                            }
                        }
                        // Tampon verilmemis: yalnizca gereken boyut
                        // sorulmus demektir (NUL dahil).
                        _ => value.len() + 1,
                    }
                }
            }
        }

        // SetEnvironmentVariableA(lpName, lpValue) -> BOOL
        //
        // Windows'ta bu cagri surecin **kendi** ortam blogunu degistirir;
        // baska surecleri etkilemez ama `CreateProcess` ile dogan cocuga
        // gecer. TCMK'de de oyle: tablo gorev yuvasina bagli, `fork`
        // kopyalar, `execve` korur.
        //
        // `lpValue` NULL verilirse degisken **silinir** -- Windows'un
        // sozlesmesi bu, ve POSIX tarafindaki `unsetenv`in karsiligi.
        NT_SET_ENVIRONMENT_VARIABLE_A => {
            let mut name_storage = [0u8; PATH_MAX];
            let name_len = arg_ptr(args, 0)
                .and_then(|p| unsafe { copy_user_cstr(p, &mut name_storage) })
                .map(|n| n.len());
            let mut value_storage = [0u8; PATH_MAX];
            // NULL isaretci ile bos dize ayni sonuca ciksa da yollari
            // ayri: NULL hic okunmaz, bos dize okunur ve bos bulunur.
            let value_len = match arg_ptr(args, 1) {
                Some(0) | None => Some(0),
                Some(p) => unsafe { copy_user_cstr(p, &mut value_storage) }.map(|v| v.len()),
            };

            let name = name_len.and_then(|len| core::str::from_utf8(&name_storage[..len]).ok());
            let value = value_len.and_then(|len| core::str::from_utf8(&value_storage[..len]).ok());
            match (name, value) {
                (Some(name), Some(value)) => match kernel_api::setenv(name, value) {
                    Ok(()) => WIN32_TRUE,
                    Err(e) => {
                        set_last_error(win32_error_of(e));
                        WIN32_FALSE
                    }
                },
                _ => {
                    set_last_error(ERROR_INVALID_PARAMETER);
                    WIN32_FALSE
                }
            }
        }

        // GetFileAttributesA(lpFileName) -> DWORD
        //
        // POSIX'in `stat`i ile **ayni** cekirdek cagrisina iner; ayrisan
        // yalnizca cevabin bicimi, ve ayrim ogretici:
        //
        //   POSIX  stat(yol, buf) -> 0 / -errno, bilgi TAMPONA yazilir
        //   Win32  GetFileAttributesA(yol) -> bilgi DONUS DEGERINDE,
        //          bir bayrak kumesi olarak; hata 0 degil 0xFFFFFFFF
        //
        // Sifirin hata olarak kullanilamamasinin sebebi var: sifir
        // "hicbir ozellik yok" demek olurdu ve o gecerli bir durum
        // sayilabilirdi. Windows bu yuzden `INVALID_FILE_ATTRIBUTES`i
        // tum bitler bir olarak secmis; TCMK de oyle.
        NT_GET_FILE_ATTRIBUTES_A => {
            let mut storage = [0u8; PATH_MAX];
            let path = match arg_ptr(args, 0)
                .and_then(|p| unsafe { copy_user_cstr(p, &mut storage) })
                .map(|p| p.len())
            {
                Some(length) => Some(normalize_win_path(&mut storage, length)),
                None => None,
            };
            match path.map(kernel_api::stat) {
                Some(Ok(info)) => {
                    let mut attributes = 0u32;
                    if info.is_dir {
                        attributes |= FILE_ATTRIBUTE_DIRECTORY;
                    }
                    if info.read_only {
                        attributes |= FILE_ATTRIBUTE_READONLY;
                    }
                    if attributes == 0 {
                        attributes = FILE_ATTRIBUTE_NORMAL;
                    }
                    attributes as usize
                }
                Some(Err(e)) => {
                    set_last_error(win32_error_of(e));
                    INVALID_FILE_ATTRIBUTES
                }
                None => {
                    set_last_error(ERROR_INVALID_PARAMETER);
                    INVALID_FILE_ATTRIBUTES
                }
            }
        }

        // GetSystemTimeAsFileTime(lpSystemTimeAsFileTime)
        //
        // Ayni saati POSIX `time`/`clock_gettime` de okuyor; ayrisan
        // **cagin baslangici** ve **birim**:
        //
        //   POSIX  1970-01-01'den beri SANIYE
        //   Win32  1601-01-01'den beri 100 NANOSANIYELIK ARALIK
        //
        // Cevrim `filetime_of` icinde ve tek yerde; iki taraftan birine
        // otekinin cagini dayatmak, o tarafta derlenmis her programin
        // tarih hesabini kaydirirdi.
        NT_GET_SYSTEM_TIME_AS_FILE_TIME => {
            let filetime = filetime_of(crate::level0a::drivers::rtc::unix_time());
            match arg_ptr(args, 0) {
                Some(target) if target != 0 => {
                    if !mmu::is_user_accessible(target) || !mmu::is_user_accessible(target + 7) {
                        WIN32_FALSE
                    } else {
                        // Tek bir `u64` degil iki `DWORD`: `FILETIME`
                        // Windows'ta da oyle ve hizalama farki gercek
                        // (bkz. `Filetime`).
                        unsafe {
                            (target as *mut u32).write_unaligned(filetime as u32);
                            ((target + 4) as *mut u32).write_unaligned((filetime >> 32) as u32);
                        }
                        WIN32_TRUE
                    }
                }
                _ => WIN32_FALSE,
            }
        }

        // GetSystemTime(lpSystemTime) -> SYSTEMTIME
        //
        // Buradaki ayrim daha da ogretici: POSIX **ham sayiyi** verir ve
        // yil/ay/gune bolmeyi cagirana birakir (`localtime` libc'dedir).
        // Win32 bolunmus halini dogrudan verir -- yani takvim bilgisi
        // cekirdegin sozlesmesinin parcasidir.
        //
        // `SYSTEMTIME` sekiz `WORD`: yil, ay, haftagunu, gun, saat,
        // dakika, saniye, milisaniye.
        NT_GET_SYSTEM_TIME => {
            let now = crate::level0a::drivers::rtc::now();
            match (arg_ptr(args, 0), now) {
                (Some(target), Some(time)) if target != 0 => {
                    if !mmu::is_user_accessible(target) || !mmu::is_user_accessible(target + 15) {
                        WIN32_FALSE
                    } else {
                        let fields = [
                            time.year,
                            time.month as u16,
                            // Haftanin gunu RTC'de yok; Windows'ta bu alan
                            // her zaman doludur, o yuzden tarihten
                            // hesaplaniyor (0 = Pazar).
                            day_of_week(time.year, time.month, time.day),
                            time.day as u16,
                            time.hour as u16,
                            time.minute as u16,
                            time.second as u16,
                            0,
                        ];
                        for (i, value) in fields.iter().enumerate() {
                            unsafe { ((target + i * 2) as *mut u16).write_unaligned(*value) };
                        }
                        WIN32_TRUE
                    }
                }
                _ => WIN32_FALSE,
            }
        }

        // FlushFileBuffers(hFile) -> BOOL
        //
        // POSIX `fsync` ile ayni cekirdek cagrisi. Ikisi de tutamac
        // aliyor, ikisi de dosya sistemi genelinde calisiyor -- TCMKFS'te
        // dosya basina tampon yok, tek bir ortak tablo var.
        NT_FLUSH_FILE_BUFFERS => match arg(args, 0) {
            Some(handle) => match kernel_api::fsync(handle) {
                Ok(()) => WIN32_TRUE,
                Err(e) => {
                    set_last_error(win32_error_of(e));
                    WIN32_FALSE
                }
            },
            None => {
                set_last_error(ERROR_INVALID_HANDLE);
                WIN32_FALSE
            }
        },

        // GetVersionExA(lpVersionInformation) -> BOOL
        //
        // POSIX'in `uname`i ile ayni soruyu soruyor -- "sen kimsin?" --
        // ama **cevabin turu** farkli:
        //
        //   POSIX  alti DIZE: sysname, nodename, release, version, machine
        //   Win32  uc SAYI (major/minor/build) + platform kimligi + bir
        //          servis paketi dizesi
        //
        // Yani POSIX'te surum karsilastirmasi metin isi, Win32'de sayi
        // isi. Bir Windows programi `dwMajorVersion >= 5` diye yazar;
        // ayni sey `uname`de dizeyi ayristirmakla yapilir.
        //
        // `OSVERSIONINFOA`: dwOSVersionInfoSize, dwMajorVersion,
        // dwMinorVersion, dwBuildNumber, dwPlatformId, szCSDVersion[128].
        NT_GET_VERSION_EX_A => {
            const CSD_OFFSET: usize = 20;
            const CSD_LEN: usize = 128;
            const TOTAL: usize = CSD_OFFSET + CSD_LEN;
            /// `VER_PLATFORM_WIN32_NT` -- Windows'un NT soyu.
            const VER_PLATFORM_WIN32_NT: u32 = 2;

            match arg_ptr(args, 0) {
                Some(target)
                    if target != 0
                        && mmu::is_user_accessible(target)
                        && mmu::is_user_accessible(target + TOTAL - 1) =>
                {
                    // Cagiran ilk alani doldurup gelir (yapinin boyu);
                    // Windows onu **dogrular**. Sifir gelmesi, cagiranin
                    // yapiyi hic kurmadigi anlamina gelir.
                    let declared = unsafe { (target as *const u32).read_unaligned() };
                    if declared == 0 {
                        set_last_error(ERROR_INVALID_PARAMETER);
                        WIN32_FALSE
                    } else {
                        let csd = b"TCMK\0";
                        unsafe {
                            ((target + 4) as *mut u32).write_unaligned(0);
                            ((target + 8) as *mut u32).write_unaligned(1);
                            ((target + 12) as *mut u32).write_unaligned(0);
                            ((target + 16) as *mut u32).write_unaligned(VER_PLATFORM_WIN32_NT);
                            core::ptr::write_bytes((target + CSD_OFFSET) as *mut u8, 0, CSD_LEN);
                            core::ptr::copy_nonoverlapping(
                                csd.as_ptr(),
                                (target + CSD_OFFSET) as *mut u8,
                                csd.len(),
                            );
                        }
                        WIN32_TRUE
                    }
                }
                _ => {
                    set_last_error(ERROR_INVALID_PARAMETER);
                    WIN32_FALSE
                }
            }
        }

        // GetCurrentProcessId() / GetCurrentThreadId()
        //
        // POSIX'te bu ikisi ayri cagridir (`getpid`/`gettid`) ve **ayri
        // sayilar** dondururler, cunku bir surecte cok is parcacigi olur.
        // TCMK'de is parcacigi yok: bir gorev = bir surec = bir akis.
        //
        // Bu yuzden ikisi de ayni sayiyi donduruyor, ve bu bir eksiklik
        // degil dogru cevap: sistemde gercekten tek bir akis var. Ayri
        // sayilar uydurmak, is parcacigi varmis gibi gorunmek olurdu.
        NT_GET_CURRENT_PROCESS_ID | NT_GET_CURRENT_THREAD_ID => {
            crate::level0a::core::scheduler::current_id()
        }

        // SetEndOfFile(hFile) -> BOOL
        //
        // POSIX `ftruncate` ile ayni cekirdek cagrisina iner, ama
        // **uzunlugu baska yerden alir** ve fark tasarimsal:
        //
        //   ftruncate(fd, uzunluk)   uzunluk PARAMETRE
        //   SetEndOfFile(hFile)      uzunluk DOSYA IMLECI
        //
        // Yani Win32'de once `SetFilePointer` ile konumlanilir, sonra
        // "buraya kadar" denir. Iki cagrili bir kalip, ama imleci zaten
        // tasiyan bir yazma dongusunde daha dogal: yazdiktan sonra
        // dogrudan `SetEndOfFile` demek yeter.
        NT_SET_END_OF_FILE => match arg(args, 0) {
            Some(handle) => match kernel_api::file_offset(handle) {
                Ok(offset) => match kernel_api::truncate(handle, offset) {
                    Ok(()) => WIN32_TRUE,
                    Err(e) => {
                        set_last_error(win32_error_of(e));
                        WIN32_FALSE
                    }
                },
                Err(e) => {
                    set_last_error(win32_error_of(e));
                    WIN32_FALSE
                }
            },
            None => {
                set_last_error(ERROR_INVALID_HANDLE);
                WIN32_FALSE
            }
        },

        // GetModuleFileNameA(hModule, lpFilename, nSize) -> DWORD
        //
        // POSIX'te bunun karsiligi **yok**: orada programin kendi yolu
        // `argv[0]`dir ve o kullanicinin yigininda durur -- surec onu
        // degistirebilir, hatta gercek POSIX'te cagiran ona istedigi
        // seyi koyabilir. Windows'ta cevap cekirdegin bildigi yoldur ve
        // bir program kendi dizinini bununla bulur.
        //
        // TCMK'de de oyle: yol Level-0b1'de saklaniyor (bkz.
        // `process::program_path`), yigindaki `argv[0]`dan bagimsiz.
        //
        // `hModule` yok sayilir: TCMK'de yuklu tek modul surecin
        // kendisidir, DLL'ler sentetik (bkz. `dll.rs`).
        //
        // Donus sozlesmesi `GetCurrentDirectoryA`dan **farkli** ve bu
        // Windows'un kendi tutarsizligi: burada tampon yetmezse dize
        // **kirpilir** ve `nSize` dondurulur, gereken boy degil.
        NT_GET_MODULE_FILE_NAME_A => {
            let path = crate::level0b1::process::program_path();
            let capacity = arg(args, 2).unwrap_or(0) as usize;
            match arg_ptr(args, 1) {
                Some(buffer) if buffer != 0 && capacity > 0 => {
                    if !mmu::is_user_accessible(buffer)
                        || !mmu::is_user_accessible(buffer + capacity - 1)
                    {
                        set_last_error(ERROR_INVALID_PARAMETER);
                        0
                    } else {
                        let taken = path.len().min(capacity - 1);
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                path.as_ptr(),
                                buffer as *mut u8,
                                taken,
                            );
                            (buffer as *mut u8).add(taken).write(0);
                        }
                        if taken < path.len() {
                            // Windows kirpildiginda `nSize` doner ve
                            // `ERROR_INSUFFICIENT_BUFFER` birakir.
                            set_last_error(ERROR_INSUFFICIENT_BUFFER);
                            capacity
                        } else {
                            taken
                        }
                    }
                }
                _ => {
                    set_last_error(ERROR_INVALID_PARAMETER);
                    0
                }
            }
        }

        // CreateProcessA(lpApplicationName, lpCommandLine, ...,
        //                lpProcessInformation) -> BOOL
        //
        // Bu cagriya kadar Win32 tarafi **surec yaratamiyordu**: POSIX'te
        // `fork`/`execve` vardi, Windows tarafinda karsiligi yoktu. En
        // buyuk asimetri buydu.
        //
        // Ve iki dunyanin en keskin ayrildigi yer de burasi:
        //
        //   POSIX  fork() + execve()  IKI cagri. Once surec ikiye ayrilir
        //          (cocuk her seyi devralir), sonra imaj degistirilir.
        //          Aradaki pencerede cocuk hala ebeveynin kodudur -- ve o
        //          pencere kasitlidir: yonlendirme orada kurulur.
        //
        //   Win32  CreateProcess()    TEK cagri. Yaratma ve yukleme
        //          ayrilmaz; "aradaki an" diye bir sey yoktur. Devralma
        //          bu yuzden **parametrelerle** anlatilir
        //          (bInheritHandles, STARTUPINFO).
        //
        // TCMK'de yol `launcher::spawn_user_app_id`e iniyor: yeni gorev
        // yuvasi acilir, ebeveyni cagiran surectir, ve imaj bicimi
        // **magic'ten** secilir. Yani `CreateProcessA` ile bir **ELF**
        // baslatmak da mumkun -- projenin butun iddiasi tek satirda.
        //
        // Desteklenmeyenler bilerek yok sayiliyor: guvenlik
        // tanimlayicilari, oncelik siniflari, `STARTUPINFO` (TCMK'de
        // pencere/konsol devralma kavrami yok).
        NT_CREATE_PROCESS_A => {
            let mut path_storage = [0u8; PATH_MAX];
            let mut line_storage = [0u8; PATH_MAX];

            // Windows iki ad alani kabul eder: `lpApplicationName` (tam
            // yol) ve `lpCommandLine` (ilk kelimesi program adi). Biri
            // NULL olabilir; ikisi de doluysa ilki kazanir.
            let application = arg_ptr(args, 0)
                .filter(|p| *p != 0)
                .and_then(|p| unsafe { copy_user_cstr(p, &mut path_storage) })
                .map(|p| p.len())
                .map(|len| normalize_win_path(&mut path_storage, len));
            let command = arg_ptr(args, 1)
                .filter(|p| *p != 0)
                .and_then(|p| unsafe { copy_user_cstr(p, &mut line_storage) })
                .map(|p| p.len());

            let mut program = [0u8; PATH_MAX];
            let mut program_len = 0usize;
            // Tasiyici blok: `argv[0]` dahil, NUL ayrili (bkz.
            // `level0b1::argv`). Komut satiri **burada** Windows'un
            // alintilama kurallariyla bolunur, boylece bosluklu bir yol
            // ("C:\Program Files\x") tek arguman kalir. Eskiden bolme
            // ilk boslukta yapiliyordu ve o yolu ikiye ayiriyordu.
            let mut block = [0u8; PATH_MAX];
            let mut block_len = 0usize;

            match application {
                Some(path) => {
                    program_len = path.len().min(PATH_MAX);
                    program[..program_len].copy_from_slice(&path.as_bytes()[..program_len]);
                    match command {
                        // Ikisi de doluysa Windows'un kurali: **calisan**
                        // imaj `lpApplicationName`, ama `argv[0]` dahil
                        // butun komut satiri cocuga oldugu gibi gecer.
                        Some(len) => {
                            let line =
                                core::str::from_utf8(&line_storage[..len]).unwrap_or("");
                            block_len = crate::level0b1::argv::split(line, &mut block);
                        }
                        // Komut satiri yoksa `argv[0]` programin kendisi.
                        None => {
                            block[..program_len]
                                .copy_from_slice(&program[..program_len]);
                            block_len = program_len;
                            if block_len < block.len() {
                                block[block_len] = crate::level0b1::argv::SEP;
                                block_len += 1;
                            }
                        }
                    }
                }
                None => match command {
                    Some(len) => {
                        let line = core::str::from_utf8(&line_storage[..len]).unwrap_or("");
                        block_len = crate::level0b1::argv::split(line, &mut block);
                        // Ilk eleman calistirilacak imaj. Yol cevirisi
                        // (`C:\bin\x` -> `/bin/x`) **bolmeden sonra**
                        // yapiliyor: once bolup sonra cevirmek, bosluklu
                        // bir yolun cevrilmeden bolunmesini onluyor.
                        let block_str =
                            core::str::from_utf8(&block[..block_len]).unwrap_or("");
                        if let Some(first) = crate::level0b1::argv::iter(block_str).next() {
                            let mut first_storage = [0u8; PATH_MAX];
                            let first_len = first.len().min(PATH_MAX);
                            first_storage[..first_len]
                                .copy_from_slice(&first.as_bytes()[..first_len]);
                            let normalized =
                                normalize_win_path(&mut first_storage, first_len);
                            program_len = normalized.len().min(PATH_MAX);
                            program[..program_len]
                                .copy_from_slice(&normalized.as_bytes()[..program_len]);
                        }
                    }
                    // Ne uygulama adi ne komut satiri verilmis:
                    // `program_len` sifir kalir ve asagida yakalanir.
                    None => {}
                },
            }

            let program = core::str::from_utf8(&program[..program_len]).unwrap_or("");
            let block = core::str::from_utf8(&block[..block_len]).unwrap_or("");

            if program.is_empty() {
                set_last_error(ERROR_INVALID_PARAMETER);
                return_win32(frame, WIN32_FALSE);
                return;
            }

            // `lpEnvironment` (arg6) ve `lpCurrentDirectory` (arg7).
            //
            // Windows ortami **duz bir blok** olarak gecirir:
            // `AD=deger\0AD=deger\0\0`. POSIX ayni bilgiyi `char *[]`
            // dizisiyle gecirir (bkz. `execve`). Ayni bilgi, iki temsil
            // -- ve ikisi de ayni tabloya yaziliyor.
            //
            // Cocugun tablosu **once** oturumdan/ebeveynden kopyalanir
            // (bkz. `env::reset`), o yuzden ozel ortam gorev
            // yaratildiktan SONRA uygulanmali.
            let environment = arg_ptr(args, 6).unwrap_or(0);
            let directory = arg_ptr(args, 7)
                .filter(|p| *p != 0)
                .and_then(|p| unsafe { copy_user_cstr(p, &mut path_storage) })
                .map(|p| p.len())
                .map(|len| normalize_win_path(&mut path_storage, len));

            match crate::level0a::launcher::spawn_child_app_block(program, block) {
                Ok(id) => {
                    if environment != 0 {
                        apply_environment_block(id, environment);
                    }
                    if let Some(dir) = directory {
                        // Calisma dizini surece aittir (bkz.
                        // `core::cwd`), yani cocuga ayrica verilmesi
                        // gerekiyor. POSIX'te bunun karsiligi yok:
                        // orada cocuk `fork` aninda ebeveynin dizinini
                        // devralir ve degistirmek isteyen `chdir`
                        // cagirir.
                        let _ = crate::level0a::core::cwd::set(id, dir);
                    }
                    // `PROCESS_INFORMATION`: hProcess, hThread,
                    // dwProcessId, dwThreadId. TCMK'de is parcacigi yok,
                    // yani ikinci ikili birinciyle ayni deger -- bir
                    // gorev = bir surec = bir akis.
                    if let Some(info) = arg_ptr(args, 9) {
                        let handle = id | PROCESS_HANDLE_FLAG;
                        let width = core::mem::size_of::<usize>();
                        if info != 0
                            && mmu::is_user_accessible(info)
                            && mmu::is_user_accessible(info + 2 * width + 7)
                        {
                            unsafe {
                                (info as *mut usize).write_unaligned(handle);
                                ((info + width) as *mut usize).write_unaligned(handle);
                                ((info + 2 * width) as *mut u32).write_unaligned(id as u32);
                                ((info + 2 * width + 4) as *mut u32)
                                    .write_unaligned(id as u32);
                            }
                        }
                    }
                    WIN32_TRUE
                }
                Err(_) => {
                    set_last_error(ERROR_FILE_NOT_FOUND);
                    WIN32_FALSE
                }
            }
        }

        // WaitForSingleObject(hHandle, dwMilliseconds) -> DWORD
        //
        // POSIX `waitpid`in karsiligi, ama sozlesmesi farkli: donus
        // **cikis kodu degil**, "ne oldu" bilgisidir (`WAIT_OBJECT_0` /
        // `WAIT_TIMEOUT`). Cikis kodunu ogrenmek icin ayri bir cagri
        // gerekir -- `waitpid`in tek cagride ikisini birden vermesinin
        // tersi.
        //
        // TCMK yalnizca `INFINITE` ve sifir sureyi ayirt ediyor: ara
        // degerler icin zamanlayicida "sureli bekleme" kavrami yok, ve
        // beklenen sureyi uydurmak yaniltirdi.
        NT_WAIT_FOR_SINGLE_OBJECT => {
            let handle = arg(args, 0).unwrap_or(0) as usize;
            let timeout = arg(args, 1).unwrap_or(0);
            match handle.checked_sub(PROCESS_HANDLE_FLAG) {
                None => {
                    set_last_error(ERROR_INVALID_HANDLE);
                    WAIT_FAILED
                }
                Some(task) => {
                    // Bir surec nesnesi, surec **calismiyorken**
                    // isaretlidir. Iki durum da buna girer: bitmis ama
                    // toplanmamis (`Terminated`), ya da yuvasi coktan
                    // geri verilmis (`Unused`). Ikincisini hata saymak
                    // yanlis olurdu -- "cocugum bitti mi?" sorusunun
                    // cevabi ikisinde de evet.
                    let state = crate::level0a::core::scheduler::state_of(task);
                    let signalled = matches!(
                        state,
                        crate::level0a::core::scheduler::TaskState::Terminated
                            | crate::level0a::core::scheduler::TaskState::Unused
                    );
                    if signalled {
                        // Bitmis ama toplanmamissa kodu simdi saklayalim:
                        // sonraki `GetExitCodeProcess` icin tek sans bu.
                        if state == crate::level0a::core::scheduler::TaskState::Terminated {
                            remember_exit(task, crate::level0a::core::scheduler::exit_code_of(task));
                        }
                        WAIT_OBJECT_0
                    } else if timeout == 0 {
                        WAIT_TIMEOUT
                    } else {
                        if let Some(code) =
                            crate::level0a::core::scheduler::wait_for_task(task)
                        {
                            remember_exit(task, code);
                        }
                        WAIT_OBJECT_0
                    }
                }
            }
        }

        // GetExitCodeProcess(hProcess, lpExitCode) -> BOOL
        //
        // Surec hala calisiyorsa `STILL_ACTIVE` (259) yazilir. Windows'un
        // bilinen tuzagi da budur: 259 ile cikan bir surec "hala
        // calisiyor" gibi gorunur. TCMK bu davranisi **koruyor**, cunku o
        // sayiyi ayikliyormus gibi yapmak, gercek Windows'ta calismayan
        // bir varsayimi burada calisir kilardi.
        NT_GET_EXIT_CODE_PROCESS => {
            let handle = arg(args, 0).unwrap_or(0) as usize;
            match (handle.checked_sub(PROCESS_HANDLE_FLAG), arg_ptr(args, 1)) {
                (Some(task), Some(out)) if out != 0 && mmu::is_user_accessible(out) => {
                    // Once beklemede saklanan kod; yoksa zamanlayiciya
                    // bak. Sirasi onemli: bekleme yuvayi geri verdigi
                    // icin zamanlayicida artik deger yok.
                    let code = match remembered_exit(task) {
                        Some(code) => code,
                        None
                            if crate::level0a::core::scheduler::state_of(task)
                                == crate::level0a::core::scheduler::TaskState::Terminated =>
                        {
                            crate::level0a::core::scheduler::exit_code_of(task)
                        }
                        None => STILL_ACTIVE,
                    };
                    unsafe { (out as *mut u32).write_unaligned(code) };
                    WIN32_TRUE
                }
                _ => {
                    set_last_error(ERROR_INVALID_HANDLE);
                    WIN32_FALSE
                }
            }
        }

        // --- Istisna dagitimi (bkz. `seh.rs`) ---

        // AddVectoredExceptionHandler(First, Handler) -> PVOID
        //
        // Windows'ta bu **surec genelinde** bir listedir ve SEH
        // zincirinden ONCE calisir. TCMK'de de oyle. `First` sifirdan
        // farkliysa isleyici listenin basina gecer.
        NT_ADD_VECTORED_EXCEPTION_HANDLER => {
            let first = arg(args, 0).unwrap_or(0) != 0;
            let handler = arg_ptr(args, 1).unwrap_or(0);
            let task = crate::level0a::core::scheduler::current_id();
            let result = seh::add_vectored(task, first, handler);
            if result == 0 {
                set_last_error(ERROR_INVALID_PARAMETER);
            }
            result
        }

        // RemoveVectoredExceptionHandler(Handle) -> ULONG
        NT_REMOVE_VECTORED_EXCEPTION_HANDLER => {
            let handle = arg_ptr(args, 0).unwrap_or(0);
            let task = crate::level0a::core::scheduler::current_id();
            if seh::remove_vectored(task, handle) {
                WIN32_TRUE
            } else {
                set_last_error(ERROR_INVALID_PARAMETER);
                WIN32_FALSE
            }
        }

        // RaiseException(dwCode, dwFlags, nArgs, lpArgs)
        //
        // Donmez: ya bir isleyici yurutmeyi surdurur ya da surec biter.
        // Bu yuzden `return_win32` ile erken cikilir -- normal donus
        // degeri yazilirsa isleyiciye kurulan cerceve ezilirdi.
        NT_RAISE_EXCEPTION => {
            let code = arg(args, 0).unwrap_or(0);
            let flags = arg(args, 1).unwrap_or(0);
            let count = arg(args, 2).unwrap_or(0) as usize;
            let list = arg_ptr(args, 3).unwrap_or(0);

            let mut params = [0usize; 15];
            let count = count.min(15);
            let mut usable = 0usize;
            if list != 0 {
                let width = core::mem::size_of::<usize>();
                for i in 0..count {
                    let at = list + i * width;
                    if !mmu::is_user_accessible(at) {
                        break;
                    }
                    params[i] = unsafe { (at as *const usize).read_unaligned() };
                    usable = i + 1;
                }
            }

            if unsafe { seh::raise(frame, from_interrupt, code, flags, &params[..usable]) } {
                return;
            }
            crate::println!(
                "[LEVEL-0b1] NT: yakalanmayan RaiseException {:#010x}; surec sonlandiriliyor.",
                code
            );
            kernel_api::exit_current_task(code & 0xFF);
        }

        // NtContinueDispatch(disposition)
        //
        // Yalnizca tramplen cagirir. Ya yurutme surer (cerceve
        // isleyicinin duzenledigi CONTEXT'ten yuklenir) ya da siradaki
        // isleyiciye gecilir; iki durumda da cerceve **burada** kurulur,
        // o yuzden normal donus yolu kullanilmaz.
        NT_CONTINUE_DISPATCH => {
            let disposition = arg_ptr(args, 0).unwrap_or(0);
            if unsafe { seh::continue_dispatch(frame, from_interrupt, disposition) } {
                return;
            }
            // Zincirin sonuna gelindi ve kimse sahiplenmedi. Windows'ta
            // bu noktada `UnhandledExceptionFilter` calisir ve surec
            // sonlanir; TCMK dogrudan sonlandirir.
            let task = crate::level0a::core::scheduler::current_id();
            crate::println!(
                "[LEVEL-0b1] NT: yakalanmayan istisna (IP=0x{:08x}); surec sonlandiriliyor.",
                seh::fault_address(task)
            );
            kernel_api::exit_current_task(seh::STATUS_ACCESS_VIOLATION & 0xFF);
        }

        // --- Modul tablosu (bkz. `modules.rs`) ---

        // GetModuleHandleA(lpModuleName) -> HMODULE
        //
        // NULL istenirse surecin **kendi** imaj tabani doner. Bu, gercek
        // Windows'ta da en cok kullanilan bicimdir: kaynak yuklemek ya
        // da kendi yolunu bulmak icin gerekir.
        NT_GET_MODULE_HANDLE_A => {
            let mut storage = [0u8; PATH_MAX];
            let name = arg_ptr(args, 0)
                .filter(|p| *p != 0)
                .and_then(|p| unsafe { copy_user_cstr(p, &mut storage) });
            let task = crate::level0a::core::scheduler::current_id();
            let handle = modules::handle_for(task, name);
            if handle == 0 {
                set_last_error(ERROR_MOD_NOT_FOUND);
            }
            handle
        }

        // GetProcAddress(hModule, lpProcName) -> FARPROC
        //
        // `lpProcName`in ust 16 biti sifirsa Windows onu **ordinal**
        // sayar (`MAKEINTRESOURCE`). Ayrimi burada yapmak sart: gercek
        // DLL'lerde ordinal-only ihracatlar vardir ve adres olarak
        // dereferans edilirse cop okunur.
        NT_GET_PROC_ADDRESS => {
            let handle = arg_ptr(args, 0).unwrap_or(0);
            let raw = arg_ptr(args, 1).unwrap_or(0);
            let task = crate::level0a::core::scheduler::current_id();

            let mut storage = [0u8; PATH_MAX];
            let (name, ordinal) = if raw != 0 && raw < 0x1_0000 {
                (None, Some(raw as u16))
            } else {
                (
                    unsafe { copy_user_cstr(raw, &mut storage) },
                    None,
                )
            };
            let address = unsafe { modules::proc_address(task, handle, name, ordinal) };
            if address == 0 {
                set_last_error(ERROR_PROC_NOT_FOUND);
            }
            address
        }

        // LoadLibraryA(lpLibFileName) -> HMODULE
        //
        // TCMK'de yuklenecek bir dosya yok: gomulu tabloda varsa
        // tanitici doner, yoksa hata. Gercek Windows'ta fark, diskten
        // okuyup eslemesi ve basvuru sayacini artirmasi -- burada ikisi
        // de anlamsiz oldugu icin cagri `GetModuleHandleA` ile ayni yere
        // cikiyor. Bu bilincli bir sadelestirme, gizlenen bir eksiklik
        // degil.
        NT_LOAD_LIBRARY_A => {
            let mut storage = [0u8; PATH_MAX];
            let name = arg_ptr(args, 0)
                .filter(|p| *p != 0)
                .and_then(|p| unsafe { copy_user_cstr(p, &mut storage) });
            let task = crate::level0a::core::scheduler::current_id();
            match name {
                None => {
                    set_last_error(ERROR_INVALID_PARAMETER);
                    0
                }
                Some(name) => {
                    let handle = modules::handle_for(task, Some(name));
                    if handle == 0 {
                        set_last_error(ERROR_MOD_NOT_FOUND);
                    }
                    handle
                }
            }
        }

        // FreeLibrary(hModule) -> BOOL
        //
        // Basvuru sayaci yok, yani serbest birakilacak bir sey de yok.
        // Yine de gecerli bir tanitici icin TRUE donmeli: cagiran bunu
        // temizlik yolunda kontrol eder.
        NT_FREE_LIBRARY => {
            let handle = arg_ptr(args, 0).unwrap_or(0);
            let task = crate::level0a::core::scheduler::current_id();
            let known = handle != 0
                && (handle == modules::image_base(task)
                    || (0..dll_count()).any(|i| modules::synthetic_handle(i) == handle));
            if known {
                WIN32_TRUE
            } else {
                set_last_error(ERROR_INVALID_HANDLE);
                WIN32_FALSE
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

/// `FindFirstFileA`'nin desenini gezilecek **dizin yoluna** indirger.
///
/// Windows'ta cagri bir desen alir (`C:\Windows\*.dll`), yol degil.
/// TCMK'de desen esleme yok: sondaki `\*`, `/*` ya da yalin `*` atilir,
/// geri kalan yol dizin olarak acilir -- yani her desen `*` gibi davranir.
/// Bastaki surucu harfi (`C:`) de atilir; TCMK'nin tek isim uzayi vardir.
///
/// ## Yalin `*` koku degil, **calisma dizinini** gosterir
///
/// Desen atildiktan sonra geriye bos dize kalabilir. Iki ayri anlami
/// var ve ayirmak sart: `\*` (ya da `C:\*`) **mutlak** bir desendir,
/// koku gosterir; yalin `*` ise gorelidir, "bulundugum dizin" demektir.
/// Ikisi de koke cevrildiginde `SetCurrentDirectoryA` gorunuste
/// calisiyor ama listeleme hep koku gosteriyordu -- gezginin yolu
/// degisiyor, icerigi degismiyordu.
///
/// Ters bolu isaretlerini bolu isaretine cevirmek cagirana kalir
/// (yerinde yapilir, bkz. `NT_FIND_FIRST_FILE`), cunku burada yeni bir
/// tampon ayirmadan degistirilemez.
fn strip_wildcard(pattern: &str) -> &str {
    // "C:\dizin\*" -> "\dizin\*"
    let path = match pattern.as_bytes() {
        [drive, b':', ..] if drive.is_ascii_alphabetic() => &pattern[2..],
        _ => pattern,
    };
    let trimmed = match path.strip_suffix('*') {
        Some(head) => head.trim_end_matches(['\\', '/']),
        None => path.trim_end_matches(['\\', '/']),
    };
    if trimmed.is_empty() {
        if path.starts_with('/') {
            "/"
        } else {
            "."
        }
    } else {
        trimmed
    }
}

/// Tek yol argumanli Win32 cagrilarinin ortak govdesi.
///
/// `CreateDirectoryA`/`RemoveDirectoryA`/`DeleteFileA` ayni sekli
/// paylasir: bir yol, bir `BOOL`. Yol Windows tarzinda gelir, yani
/// ayirici cevrimi ve surucu harfi temizligi burada da gerekir --
/// `FindFirstFileA` ile ayni islem (bkz. `normalize_win_path`).
fn win32_path_action(args: usize, action: fn(&str) -> Result<(), KernelError>) -> usize {
    let mut storage = [0u8; PATH_MAX];
    let path = match arg_ptr(args, 0)
        .and_then(|p| unsafe { copy_user_cstr(p, &mut storage) })
        .map(|p| p.len())
    {
        Some(length) => normalize_win_path(&mut storage, length),
        None => return WIN32_FALSE,
    };
    match action(path) {
        Ok(()) => WIN32_TRUE,
        Err(e) => {
            set_last_error(win32_error_of(e));
            WIN32_FALSE
        }
    }
}

/// Windows tarzi yolu TCMK isim uzayina cevirir -- **yerinde**.
///
/// Windows uygulamalari yollari `\` ile yazar ve basa surucu harfi
/// koyar; TCMK'nin tek bir isim uzayi var ve `/` kullaniyor. Cevrim
/// yeni bir tampon ayirmadan, tek gecisde yapilir.
fn normalize_win_path(storage: &mut [u8; PATH_MAX], length: usize) -> &str {
    for byte in &mut storage[..length] {
        if *byte == b'\\' {
            *byte = b'/';
        }
    }
    let text = core::str::from_utf8(&storage[..length]).unwrap_or("");
    // "C:/dizin" -> "/dizin"
    match text.as_bytes() {
        [drive, b':', ..] if drive.is_ascii_alphabetic() => &text[2..],
        _ => text,
    }
}

/// `WIN32_FIND_DATAA` yapisinin boyutu (Windows ile birebir).
const FIND_DATA_SIZE: usize = 320;
/// `cFileName` alaninin yapidaki konumu.
const FIND_DATA_NAME: usize = 44;
/// `nFileSizeLow` alaninin konumu.
const FIND_DATA_SIZE_LOW: usize = 32;
/// `ftLastWriteTime` alaninin konumu.
const FIND_DATA_WRITE_TIME: usize = 20;

/// Unix epoch (1970) ile Windows epoch (1601) arasindaki saniye farki.
const FILETIME_EPOCH_DELTA: u64 = 11_644_473_600;

/// Unix zaman damgasini Windows `FILETIME`'ina cevirir.
///
/// `FILETIME`, 1601-01-01'den beri gecen **100 nanosaniyelik** araliklarin
/// sayisidir. Cevrim gercek olsun diye yapiliyor: bir Win32 uygulamasi bu
/// alani `FileTimeToSystemTime`a verdiginde dogru tarihi gormeli, TCMK'ye
/// ozel bir sayi degil.
fn filetime_of(unix: u32) -> u64 {
    if unix == 0 {
        return 0;
    }
    (unix as u64 + FILETIME_EPOCH_DELTA) * 10_000_000
}
/// Haftanin gunu (0 = Pazar) -- Sakamoto'nun tablosu.
///
/// RTC bu bilgiyi vermiyor (CMOS'un "gun" registeri guvenilmez), ama
/// `SYSTEMTIME`in alani var ve Windows'ta her zaman dolu. Bos birakmak,
/// takvim cizen bir programi sessizce yanlis gune goturur.
fn day_of_week(year: u16, month: u8, day: u8) -> u16 {
    const OFFSETS: [u16; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year;
    if month < 3 {
        y -= 1;
    }
    let index = (month.max(1).min(12) - 1) as usize;
    ((y + y / 4 - y / 100 + y / 400 + OFFSETS[index] + day as u16) % 7) as u16
}

/// Bir dizin girdisini kullanicinin `WIN32_FIND_DATAA` yapisina yazar.
///
/// Yapi Windows'takiyle **ayni yerlesimde** doldurulur (320 bayt,
/// `cFileName` +44'te): bir PE'nin derlendigi basliklar bu ofsetleri
/// varsayar, yani sadelestirilmis bir kayit ikili uyumu bozardi.
/// Doldurulmayan alanlar (zaman damgalari, `cAlternateFileName`) sifir
/// birakilir -- Windows'ta da bilgi yoksa oyle yapilir.
fn store_find_data(block: usize, index: usize, entry: &kernel_api::DirEntry) -> bool {
    let Some(addr) = arg_ptr(block, index) else {
        return false;
    };
    if addr == 0
        || !mmu::is_user_accessible(addr)
        || !mmu::is_user_accessible(addr + FIND_DATA_SIZE - 1)
    {
        return false;
    }
    // Ad, alanin sonundaki NUL icin bir bayt birakmalidir.
    let name_len = entry.name_len.min(FIND_DATA_SIZE - FIND_DATA_NAME - 1);
    unsafe {
        core::ptr::write_bytes(addr as *mut u8, 0, FIND_DATA_SIZE);
        let attributes = if entry.kind == kernel_api::DIR_KIND_DIR {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        };
        (addr as *mut u32).write_unaligned(attributes);
        ((addr + FIND_DATA_SIZE_LOW) as *mut u32).write_unaligned(entry.size as u32);
        // FILETIME hizali olmayabilir ve iki DWORD'dur; 64 bitlik tek bir
        // yazma her iki mimarida da dogru sirayi (kucuk-endian) verir.
        ((addr + FIND_DATA_WRITE_TIME) as *mut u64).write_unaligned(filetime_of(entry.mtime));
        core::ptr::copy_nonoverlapping(
            entry.name.as_ptr(),
            (addr + FIND_DATA_NAME) as *mut u8,
            name_len,
        );
    }
    true
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
/// Beklenmis bir cocugun cikis kodunu saklar.
fn remember_exit(task: usize, code: u32) {
    if task < crate::level0a::core::scheduler::MAX_TASKS {
        REAPED_EXIT[task].store(code, core::sync::atomic::Ordering::Relaxed);
    }
}

/// Saklanmis cikis kodu -- yoksa `None`.
fn remembered_exit(task: usize) -> Option<u32> {
    if task >= crate::level0a::core::scheduler::MAX_TASKS {
        return None;
    }
    match REAPED_EXIT[task].load(core::sync::atomic::Ordering::Relaxed) {
        u32::MAX => None,
        code => Some(code),
    }
}

/// Yeni bir imaj bu yuvaya gelince eski kaydi sil.
///
/// Yuvalar geri kazanildigi icin sart: aksi halde yeni bir surecin
/// tutamaci, ayni numarali onceki surecin cikis kodunu gorurdu.
pub fn clear_reaped_exit(task: usize) {
    if task < crate::level0a::core::scheduler::MAX_TASKS {
        REAPED_EXIT[task].store(u32::MAX, core::sync::atomic::Ordering::Relaxed);
    }
}

/// Windows'un duz ortam blogunu bir gorevin tablosuna yazar.
///
/// Bicim: `AD=deger\0AD=deger\0\0` -- yani girdiler NUL ile ayrilir ve
/// **cift NUL** blogu bitirir. POSIX ayni bilgiyi `char *[]` dizisiyle
/// tasir; ikisi de burada ayni tabloya iniyor.
///
/// Blok verildiginde cocugun devraldigi tablo **tamamen** silinir:
/// `CreateProcessA`in sozlesmesi "bu ortamin yerine gecer", "buna
/// eklenir" degil.
fn apply_environment_block(task: usize, block: usize) {
    let mut at = block;
    let mut entry = [0u8; ENV_ENTRY_MAX];
    let mut wrote_any = false;

    loop {
        let mut len = 0usize;
        loop {
            if !mmu::is_user_accessible(at) {
                return;
            }
            let byte = unsafe { (at as *const u8).read() };
            at += 1;
            if byte == 0 {
                break;
            }
            if len < entry.len() {
                entry[len] = byte;
                len += 1;
            }
        }
        // Bos girdi = cift NUL = blogun sonu.
        if len == 0 {
            return;
        }
        if !wrote_any {
            crate::level0a::core::env::clear(task);
            wrote_any = true;
        }
        if let Ok(text) = core::str::from_utf8(&entry[..len]) {
            let _ = crate::level0a::core::env::set_entry(task, text);
        }
    }
}

/// Bir ortam girdisi icin ayrilan azami uzunluk.
const ENV_ENTRY_MAX: usize = 64;

/// Win32 araligindan **erken** donmek icin.
///
/// Dagitici donus degerini bir `match` ifadesinden kuruyor, yani bir
/// kolun ortasindan cikmak ancak boyle mumkun.
fn return_win32(frame: &mut SyscallFrame, value: usize) {
    frame.set_return(value);
}

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
