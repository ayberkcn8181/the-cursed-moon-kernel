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
    /// `int 0x2E` ile cagrilacak NT servis numarasi.
    pub service: u32,
    /// stdcall geregi thunk'in yigindan temizleyecegi bayt sayisi
    /// (parametre sayisi x 4).
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
        service: nt::NT_TERMINATE_PROCESS,
        stack_bytes: 4,
    },
    Export {
        name: "Sleep",
        service: nt::NT_SLEEP_MS,
        stack_bytes: 4,
    },
    Export {
        name: "GetTickCount",
        service: nt::NT_GET_TICK_COUNT,
        stack_bytes: 0,
    },
    Export {
        name: "CloseHandle",
        service: nt::NT_WIN32_CLOSE_HANDLE,
        stack_bytes: 4,
    },
    Export {
        name: "WriteConsoleA",
        service: nt::NT_WRITE_CONSOLE_A,
        stack_bytes: 20,
    },
    Export {
        name: "CreateFileA",
        service: nt::NT_CREATE_FILE_A,
        stack_bytes: 28,
    },
    Export {
        name: "ReadFile",
        service: nt::NT_READ_FILE_WIN32,
        stack_bytes: 20,
    },
];

/// `TCMKGUI.dll` -- win32k cagrilarinin kullanici modundaki yuzu.
/// Windows'ta bu rolu USER32/GDI32 oynar; adlar bilerek `Tcmk` onekli,
/// cunku imzalar Win32'nin degil TCMK'nindir.
static TCMKGUI: &[Export] = &[
    Export {
        name: "TcmkCreateWindow",
        service: nt::NT_USER_CREATE_WINDOW_W32,
        stack_bytes: 20,
    },
    Export {
        name: "TcmkGetWindowBits",
        service: nt::NT_GDI_GET_BITS_W32,
        stack_bytes: 4,
    },
    Export {
        name: "TcmkGetClientRect",
        service: nt::NT_USER_CLIENT_RECT_W32,
        stack_bytes: 4,
    },
    Export {
        name: "TcmkGetWindowRect",
        service: nt::NT_USER_WINDOW_RECT_W32,
        stack_bytes: 4,
    },
    Export {
        name: "TcmkUpdateWindow",
        service: nt::NT_USER_FLUSH_WINDOW_W32,
        stack_bytes: 4,
    },
    Export {
        name: "TcmkGetMessage",
        service: nt::NT_USER_GET_MESSAGE_W32,
        stack_bytes: 4,
    },
    Export {
        name: "TcmkGetCursorPos",
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

fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

/// Bir thunk'in kapladigi bayt (asagidaki `emit_thunk` ile ayni olmali).
pub const THUNK_SIZE: usize = 16;

/// Verilen adrese tek bir thunk yazar.
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
