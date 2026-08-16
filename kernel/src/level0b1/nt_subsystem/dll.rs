//! Gomulu DLL ihracatlari ve thunk uretimi (doc S.7 Faz 7b: "import
//! tablosu").
//!
//! ## Sorun
//!
//! Gercek bir Windows programi `int 0x2E` yazmaz. `WriteConsoleA` cagirir;
//! bu cagri `KERNEL32.dll`'in **ithal tablosu** (Import Address Table)
//! uzerinden cozulur ve DLL icindeki bir stub sistem cagrisini yapar.
//! Ithal tablosunu cozmeyen bir yukleyici, derleyicinin urettigi siradan
//! bir Windows ikilisini calistiramaz.
//!
//! ## Cozum: DLL'i sentezle
//!
//! TCMK'de diskte `KERNEL32.dll` diye bir dosya yok -- ve olmasi da
//! gerekmiyor. Yukleyici, ithal edilen her fonksiyon icin **surecin kendi
//! adres uzayina** kucuk bir thunk yazar ve IAT girdisini oraya
//! yonlendirir. Yani DLL, yuklenirken var edilir.
//!
//! Thunk, Windows'un x86'daki syscall stub'iyla ayni bicimdedir:
//!
//! ```text
//!     mov eax, <servis numarasi>
//!     lea edx, [esp+4]          ; cagiranin yigin argumanlarina isaretci
//!     int 0x2E
//!     ret <bayt>                ; stdcall: yigini cagirilan temizler
//! ```
//!
//! `EDX = arguman blogu` sozlesmesi Windows'un kendi seciminin aynisidir
//! (gercek NT stub'i `mov edx, esp; sysenter` yapar) ve onemli bir sey
//! saglar: **parametre sayisi sinirsizdir**. Uc registere sigdirma
//! zorunlulugu olsaydi `CreateFileA`'nin yedi parametresi ya da
//! `WriteConsoleA`'nin cikti parametresi desteklenemezdi.

/// Bir gomulu DLL ihracati.
#[derive(Clone, Copy)]
pub struct Export {
    /// PE ithal tablosunda gorunen ad (suslemesiz).
    pub name: &'static str,
    /// DLL icindeki sira numarasi. Bir ikili fonksiyonu adiyla degil
    /// **ordinaliyle** de ithal edebilir (Windows DLL'lerinde yaygindir);
    /// o zaman ad hic gecmez ve arama bu numaraya duser.
    ///
    /// Numaralar `userland-rs/win/*.def` ile birebir aynidir; iki taraf
    /// oradaki acik `@N` isaretleriyle sabitlenmistir.
    pub ordinal: u16,
    /// `int 0x2E` ile cagrilacak NT servis numarasi.
    pub service: u32,
    /// stdcall geregi thunk'in yigindan temizleyecegi bayt sayisi
    /// (parametre sayisi x 4).
    ///
    /// Yalnizca i386'da anlamlidir: Win64'te yigini her zaman **cagiran**
    /// temizler, yani `ret imm16` diye bir sey yoktur. Alan yine de tek
    /// tabloda tutulur -- iki mimari icin ayri ihracat listeleri tutmak,
    /// ordinallerin birbirinden kaymasi demek olurdu.
    #[cfg_attr(target_arch = "x86_64", allow(dead_code))]
    pub stack_bytes: u16,
}

/// Bir gomulu DLL.
pub struct Dll {
    pub name: &'static str,
    pub exports: &'static [Export],
}

use super::nt_syscalls as nt;

/// `KERNEL32.dll` -- adlar ve parametre sayilari **gercek Win32
/// imzalariyla ayni**. Argumanlar yigindan okundugu icin cikti
/// parametreleri (ornegin `lpNumberOfBytesWritten`) da doldurulabiliyor.
static KERNEL32: &[Export] = &[
    Export {
        name: "ExitProcess",
        ordinal: 1,
        // 0x3000 araligi -- yigin argumanli. Burada 0x1000 araligindaki
        // `NtTerminateProcess` yazmak, thunk'in kurdugu arguman blogunu
        // hic okumayan ve cikis kodunu ECX'ten bekleyen bir isleyiciye
        // dusmek demektir: surec, o an ECX'te ne varsa onunla (genellikle
        // bir isaretciyle) cikar.
        service: nt::NT_EXIT_PROCESS_W32,
        stack_bytes: 4,
    },
    Export {
        name: "Sleep",
        ordinal: 2,
        service: nt::NT_SLEEP_MS,
        stack_bytes: 4,
    },
    Export {
        name: "GetTickCount",
        ordinal: 3,
        service: nt::NT_GET_TICK_COUNT,
        stack_bytes: 0,
    },
    Export {
        name: "CloseHandle",
        ordinal: 4,
        service: nt::NT_WIN32_CLOSE_HANDLE,
        stack_bytes: 4,
    },
    Export {
        name: "WriteConsoleA",
        ordinal: 5,
        service: nt::NT_WRITE_CONSOLE_A,
        stack_bytes: 20,
    },
    Export {
        name: "CreateFileA",
        ordinal: 6,
        service: nt::NT_CREATE_FILE_A,
        stack_bytes: 28,
    },
    Export {
        name: "ReadFile",
        ordinal: 7,
        service: nt::NT_READ_FILE_WIN32,
        stack_bytes: 20,
    },
    Export {
        name: "WriteFile",
        ordinal: 8,
        service: nt::NT_WRITE_FILE_WIN32,
        // hFile, lpBuffer, nBytes, lpBytesWritten, lpOverlapped
        stack_bytes: 20,
    },
    Export {
        name: "SetFilePointer",
        ordinal: 9,
        service: nt::NT_SET_FILE_POINTER,
        // hFile, lDistanceToMove, lpDistanceToMoveHigh, dwMoveMethod
        stack_bytes: 16,
    },
    Export {
        name: "GetFileSize",
        ordinal: 10,
        service: nt::NT_GET_FILE_SIZE,
        // hFile, lpFileSizeHigh
        stack_bytes: 8,
    },
    Export {
        name: "FindFirstFileA",
        ordinal: 11,
        service: nt::NT_FIND_FIRST_FILE,
        // lpFileName, lpFindFileData
        stack_bytes: 8,
    },
    Export {
        name: "FindNextFileA",
        ordinal: 12,
        service: nt::NT_FIND_NEXT_FILE,
        // hFindFile, lpFindFileData
        stack_bytes: 8,
    },
    Export {
        name: "FindClose",
        ordinal: 13,
        service: nt::NT_FIND_CLOSE,
        // hFindFile
        stack_bytes: 4,
    },
    Export {
        name: "CreateDirectoryA",
        ordinal: 14,
        service: nt::NT_CREATE_DIRECTORY_A,
        // lpPathName, lpSecurityAttributes
        stack_bytes: 8,
    },
    Export {
        name: "RemoveDirectoryA",
        ordinal: 15,
        service: nt::NT_REMOVE_DIRECTORY_A,
        // lpPathName
        stack_bytes: 4,
    },
    Export {
        name: "DeleteFileA",
        ordinal: 16,
        service: nt::NT_DELETE_FILE_A,
        // lpFileName
        stack_bytes: 4,
    },
    Export {
        name: "MoveFileA",
        ordinal: 17,
        service: nt::NT_MOVE_FILE_A,
        // lpExistingFileName, lpNewFileName
        stack_bytes: 8,
    },
    Export {
        name: "GetLastError",
        ordinal: 18,
        service: nt::NT_GET_LAST_ERROR,
        stack_bytes: 0,
    },
    Export {
        name: "SetLastError",
        ordinal: 19,
        service: nt::NT_SET_LAST_ERROR,
        // dwErrCode
        stack_bytes: 4,
    },
    Export {
        name: "SetCurrentDirectoryA",
        ordinal: 20,
        service: nt::NT_SET_CURRENT_DIRECTORY,
        // lpPathName
        stack_bytes: 4,
    },
    Export {
        name: "GetCurrentDirectoryA",
        ordinal: 21,
        service: nt::NT_GET_CURRENT_DIRECTORY,
        // nBufferLength, lpBuffer
        stack_bytes: 8,
    },
    Export {
        name: "GetCommandLineA",
        ordinal: 22,
        service: nt::NT_GET_COMMAND_LINE_A,
        stack_bytes: 0,
    },
    Export {
        name: "GetEnvironmentVariableA",
        ordinal: 23,
        service: nt::NT_GET_ENVIRONMENT_VARIABLE_A,
        // lpName, lpBuffer, nSize
        stack_bytes: 12,
    },
    Export {
        name: "SetEnvironmentVariableA",
        ordinal: 24,
        service: nt::NT_SET_ENVIRONMENT_VARIABLE_A,
        // lpName, lpValue
        stack_bytes: 8,
    },
];

/// `TCMKGUI.dll` -- win32k cagrilarinin kullanici modundaki yuzu.
/// Windows'ta bu rolu USER32/GDI32 oynar; adlar bilerek `Tcmk` onekli,
/// cunku imzalar Win32'nin degil TCMK'nindir.
static TCMKGUI: &[Export] = &[
    Export {
        name: "TcmkCreateWindow",
        ordinal: 1,
        service: nt::NT_USER_CREATE_WINDOW_W32,
        stack_bytes: 20,
    },
    Export {
        name: "TcmkGetWindowBits",
        ordinal: 2,
        service: nt::NT_GDI_GET_BITS_W32,
        stack_bytes: 4,
    },
    Export {
        name: "TcmkGetClientRect",
        ordinal: 3,
        service: nt::NT_USER_CLIENT_RECT_W32,
        stack_bytes: 4,
    },
    Export {
        name: "TcmkGetWindowRect",
        ordinal: 4,
        service: nt::NT_USER_WINDOW_RECT_W32,
        stack_bytes: 4,
    },
    Export {
        name: "TcmkUpdateWindow",
        ordinal: 5,
        service: nt::NT_USER_FLUSH_WINDOW_W32,
        stack_bytes: 4,
    },
    Export {
        name: "TcmkGetMessage",
        ordinal: 6,
        service: nt::NT_USER_GET_MESSAGE_W32,
        stack_bytes: 4,
    },
    Export {
        name: "TcmkGetCursorPos",
        ordinal: 7,
        service: nt::NT_USER_CURSOR_POS_W32,
        stack_bytes: 0,
    },
];

static DLLS: &[Dll] = &[
    Dll {
        name: "KERNEL32.dll",
        exports: KERNEL32,
    },
    Dll {
        name: "TCMKGUI.dll",
        exports: TCMKGUI,
    },
];

/// Bir DLL adi + fonksiyon adi ciftini cozer.
///
/// DLL adi buyuk/kucuk harf duyarsiz karsilastirilir (Windows boyle
/// yapar; derleyiciler `KERNEL32.dll`, `kernel32.dll` gibi farkli
/// yazimlar uretebilir). Fonksiyon adlari ise **duyarlidir** -- gercek
/// PE ithal tablosunda da oyledir.
pub fn resolve(dll: &str, function: &str) -> Option<Export> {
    let module = DLLS.iter().find(|d| eq_ignore_case(d.name, dll))?;
    module
        .exports
        .iter()
        .find(|e| e.name == function)
        .copied()
}

/// Bir DLL adi + **ordinal** ciftini cozer.
///
/// Adla ithal edilen fonksiyonlar `resolve` ile bulunur; ordinal ile
/// ithal edilenlerde ikilide ad hic yer almaz, yalnizca numara vardir.
pub fn resolve_ordinal(dll: &str, ordinal: u16) -> Option<Export> {
    let module = DLLS.iter().find(|d| eq_ignore_case(d.name, dll))?;
    module
        .exports
        .iter()
        .find(|e| e.ordinal == ordinal)
        .copied()
}

fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

/// Bir thunk'in kapladigi bayt (asagidaki `emit_thunk` ile ayni olmali).
/// Bir thunk'in kapladigi bayt (hizalama dolgusu dahil).
#[cfg(target_arch = "x86")]
pub const THUNK_SIZE: usize = 16;
#[cfg(target_arch = "x86_64")]
pub const THUNK_SIZE: usize = 48;

/// Verilen adrese tek bir thunk yazar (i386, `__stdcall`).
///
/// ```text
///   B8 xx xx xx xx     mov eax, service
///   8D 54 24 04        lea edx, [esp+4]
///   CD 2E              int 0x2E
///   C2 xx xx           ret imm16      (stack_bytes > 0)
///   C3                 ret            (stack_bytes == 0)
/// ```
///
/// # Safety
/// `at`, en az `THUNK_SIZE` bayt yazilabilir ve **kullanici tarafindan
/// yurutulebilir** olmalidir; cagiran bunu saglar.
#[cfg(target_arch = "x86")]
pub unsafe fn emit_thunk(at: usize, export: &Export) {
    let code = at as *mut u8;

    code.write(0xB8); // mov eax, imm32
    (code.add(1) as *mut u32).write_unaligned(export.service);

    // lea edx, [esp+4] -- cagiranin ilk yigin argumaninin adresi.
    code.add(5).write(0x8D);
    code.add(6).write(0x54);
    code.add(7).write(0x24);
    code.add(8).write(0x04);

    code.add(9).write(0xCD); // int 0x2E
    code.add(10).write(0x2E);

    if export.stack_bytes == 0 {
        code.add(11).write(0xC3); // ret
        code.add(12).write(0xCC);
        code.add(13).write(0xCC);
        code.add(14).write(0xCC);
    } else {
        code.add(11).write(0xC2); // ret imm16
        (code.add(12) as *mut u16).write_unaligned(export.stack_bytes);
        code.add(14).write(0xCC);
    }
    code.add(15).write(0xCC); // hizalama dolgusu (int3)
}

/// Verilen adrese tek bir thunk yazar (x86_64, Win64 cagri gelenegi).
///
/// ## Neden i386'dakinden bambaska
///
/// Win64'te ilk dort arguman **registerdadir** (RCX, RDX, R8, R9);
/// besinciden itibarasi yigina gider. Cekirdegin "argumanlar tek bir
/// blokta" sozlesmesi bozulur -- ta ki Win64'un kendi kuralini
/// kullanana kadar:
///
/// > Cagiran, register argumanlari icin de yiginda **32 baytlik yer**
/// > (golge alan / shadow space) ayirmak zorundadir.
///
/// Bu alan tam olarak register argumanlarinin doküleceği yerdir ve
/// besinci argumanin hemen oncesindedir. Yani dort registeri oraya
/// dokmek, butun argumanlari **kesintisiz tek bir dizi** haline getirir:
///
/// ```text
///   [rsp+8]  arg1 (RCX'ten)   \
///   [rsp+16] arg2 (RDX'ten)    |  golge alan -- cagiran ayirdi
///   [rsp+24] arg3 (R8'den)     |
///   [rsp+32] arg4 (R9'dan)    /
///   [rsp+40] arg5             \  cagiranin yigina ittikleri
///   [rsp+48] arg6              /
/// ```
///
/// Uretilen kod:
///
/// ```text
///   48 89 4C 24 08     mov [rsp+8], rcx
///   48 89 54 24 10     mov [rsp+16], rdx
///   4C 89 44 24 18     mov [rsp+24], r8
///   4C 89 4C 24 20     mov [rsp+32], r9
///   B8 xx xx xx xx     mov eax, service
///   48 8D 54 24 08     lea rdx, [rsp+8]
///   CD 2E              int 0x2E
///   C3                 ret
/// ```
///
/// `stack_bytes` kullanilmaz: Win64'te yigini **her zaman cagiran**
/// temizler, yani `ret imm16` diye bir sey yoktur.
///
/// # Safety
/// Bkz. i386 surumu.
#[cfg(target_arch = "x86_64")]
pub unsafe fn emit_thunk(at: usize, export: &Export) {
    const CODE: [u8; 33] = [
        0x48, 0x89, 0x4C, 0x24, 0x08, // mov [rsp+8], rcx
        0x48, 0x89, 0x54, 0x24, 0x10, // mov [rsp+16], rdx
        0x4C, 0x89, 0x44, 0x24, 0x18, // mov [rsp+24], r8
        0x4C, 0x89, 0x4C, 0x24, 0x20, // mov [rsp+32], r9
        0xB8, 0x00, 0x00, 0x00, 0x00, // mov eax, service (21..25 doldurulur)
        0x48, 0x8D, 0x54, 0x24, 0x08, // lea rdx, [rsp+8]
        0xCD, 0x2E, // int 0x2E
        0xC3, // ret
    ];

    let code = at as *mut u8;
    core::ptr::copy_nonoverlapping(CODE.as_ptr(), code, CODE.len());
    (code.add(21) as *mut u32).write_unaligned(export.service);
    // Kalani int3 ile doldur: bir hata yuzunden buraya dallanilirsa
    // sessizce ilerlemek yerine yakalanabilir bir istisna olussun.
    core::ptr::write_bytes(code.add(CODE.len()), 0xCC, THUNK_SIZE - CODE.len());
}
