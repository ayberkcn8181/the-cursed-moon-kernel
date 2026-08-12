//! Ham NT sistem cagrisi katmani -- `int 0x2E` (doc S.6, Windows yolu).
//!
//! POSIX tarafinin `sys.rs`'i ne ise bu da odur, yalnizca **oteki ABI**:
//! `EAX` = NT servis numarasi, `EBX/ECX/EDX` = arg1..3. Cekirdekte cagri
//! once Level-0b2 Dispatcher'a, oradan Level-0b1'in **NT cevirmenine**
//! gider ve Level-0a'nin notr cekirdek API'sine baglanir.
//!
//! Bir Windows programcisinin tanidigi iki sozlesme burada da gecerlidir:
//!
//!   * `Nt*`     -> `NTSTATUS` (0 = basari, 0xC000_xxxx = hata)
//!   * `NtUser*` -> dogrudan tutamac/deger (HWND, mesaj, ...)
//!
//! Gercek Windows'ta bu ikisi ayri sistem cagrisi tablolarindadir
//! (`ntoskrnl` ve `win32k.sys`); TCMK ayrimi numara araligiyla korur.

use core::arch::asm;

// --- Yurutucu tablosu (NTSTATUS dondururler) -------------------------
pub const NT_TERMINATE_PROCESS: usize = 0x1000;
pub const NT_WRITE_CONSOLE: usize = 0x1001;
pub const NT_CREATE_FILE: usize = 0x1002;
pub const NT_READ_FILE: usize = 0x1003;
pub const NT_CLOSE: usize = 0x1004;
pub const NT_DELAY_EXECUTION: usize = 0x1005;
pub const NT_QUERY_SYSTEM_TIME: usize = 0x1006;
pub const NT_YIELD_EXECUTION: usize = 0x1007;

// --- win32k tablosu (tutamac/deger dondururler) ----------------------
pub const NT_USER_CREATE_WINDOW: usize = 0x2000;
pub const NT_GDI_GET_BITS: usize = 0x2001;
pub const NT_USER_CLIENT_RECT: usize = 0x2002;
pub const NT_USER_WINDOW_RECT: usize = 0x2003;
pub const NT_USER_FLUSH_WINDOW: usize = 0x2004;
pub const NT_USER_GET_MESSAGE: usize = 0x2005;
pub const NT_USER_CURSOR_POS: usize = 0x2006;

pub const STATUS_SUCCESS: u32 = 0x0000_0000;

/// `NtCreateFile`'in CreateDisposition degerleri (Windows ile ayni).
pub const FILE_OPEN: usize = 1;
pub const FILE_CREATE: usize = 2;

/// Windows'un standart tutamaclarinin TCMK karsiligi. Cekirdek altta
/// POSIX tanimlayicilarini kullandigi icin sayilar ayni.
pub const STD_INPUT_HANDLE: usize = 0;
pub const STD_OUTPUT_HANDLE: usize = 1;
pub const STD_ERROR_HANDLE: usize = 2;

/// EBX, x86-32'de LLVM tarafindan rezerve edilebildigi icin inline asm'de
/// dogrudan `in("ebx")` kullanilamaz; deger genel bir registerdan tasinir.
/// `nostack` bu yuzden **verilemez** (bkz. `sys.rs`'teki ayni tuzak).
#[cfg(target_arch = "x86")]
macro_rules! int2e {
    ($n:expr, $a1:expr, $a2:expr, $a3:expr) => {{
        let ret: usize;
        asm!(
            "push ebx",
            "mov ebx, {a1}",
            "int 0x2E",
            "pop ebx",
            a1 = in(reg) $a1,
            inlateout("eax") $n => ret,
            in("ecx") $a2,
            inlateout("edx") $a3 => _,
        );
        ret
    }};
}

/// x86_64'te ayni cagri, ayni kesme vektoru (`int 0x2E`) -- degisen
/// yalnizca register genisligi ve arguman registerleri.
///
/// Arguman siralamasi POSIX tarafiyla ayni tutulur (RDI/RSI/RDX), cunku
/// cekirdekteki `SyscallFrame::args()` iki ABI icin de ayni yeri okur.
/// Windows'un kendi x64 syscall stub'i RCX/R10 kullanir; TCMK'de o
/// gelenek yalnizca **gomulu DLL thunk'larinda** gecerlidir (bkz.
/// `nt_subsystem::dll::emit_thunk`), ki orada da argumanlar zaten
/// registerde degil golge alandadir.
#[cfg(target_arch = "x86_64")]
macro_rules! int2e {
    ($n:expr, $a1:expr, $a2:expr, $a3:expr) => {{
        let ret: usize;
        asm!(
            "int 0x2E",
            inlateout("rax") $n => ret,
            in("rdi") $a1,
            in("rsi") $a2,
            inlateout("rdx") $a3 => _,
        );
        ret
    }};
}

/// `int 0x2E`'yi cagirir; donus EAX'tir.
///
/// # Safety
/// Cagiran, numaranin ve argumanlarin NT sozlesmesine uydugunu garanti
/// etmelidir (isaretciler gecerli olmalidir).
#[inline(always)]
pub unsafe fn syscall3(n: usize, a1: usize, a2: usize, a3: usize) -> usize {
    int2e!(n, a1, a2, a3)
}

/// Bkz. [`syscall3`]. Cikti degerini de (EDX) dondurur.
///
/// `NtCreateFile` ve `NtReadFile` sonuclarini (tutamac, okunan bayt)
/// EDX'te birakir; EAX yalnizca NTSTATUS tasir. Ikisini birden almak
/// gerektiginde bu bicim kullanilir.
///
/// # Safety
/// Bkz. [`syscall3`].
#[cfg(target_arch = "x86")]
#[inline(always)]
pub unsafe fn syscall3_out(n: usize, a1: usize, a2: usize, a3: usize) -> (usize, usize) {
    let status: usize;
    let out: usize;
    asm!(
        "push ebx",
        "mov ebx, {a1}",
        "int 0x2E",
        "pop ebx",
        a1 = in(reg) a1,
        inlateout("eax") n => status,
        in("ecx") a2,
        inlateout("edx") a3 => out,
    );
    (status, out)
}

/// Bkz. i386 surumu; cikti degeri x86_64'te RDX'tir.
///
/// # Safety
/// Bkz. [`syscall3`].
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub unsafe fn syscall3_out(n: usize, a1: usize, a2: usize, a3: usize) -> (usize, usize) {
    let status: usize;
    let out: usize;
    asm!(
        "int 0x2E",
        inlateout("rax") n => status,
        in("rdi") a1,
        in("rsi") a2,
        inlateout("rdx") a3 => out,
    );
    (status, out)
}

/// # Safety
/// Bkz. [`syscall3`].
#[inline(always)]
pub unsafe fn syscall1(n: usize, a1: usize) -> usize {
    int2e!(n, a1, 0usize, 0usize)
}

/// # Safety
/// Bkz. [`syscall3`].
#[inline(always)]
pub unsafe fn syscall0(n: usize) -> usize {
    int2e!(n, 0usize, 0usize, 0usize)
}

// --- Tipli sarmalayicilar ---------------------------------------------

/// `NtTerminateProcess(ProcessHandle, ExitStatus)`. Geri donmez.
pub fn terminate_process(exit_status: u32) -> ! {
    unsafe { syscall3(NT_TERMINATE_PROCESS, 0, exit_status as usize, 0) };
    // Cekirdek geri dondurmez; tip sistemi icin.
    loop {
        unsafe { syscall0(NT_YIELD_EXECUTION) };
    }
}

/// `NtWriteConsole(Handle, Buffer, Length)` -> NTSTATUS.
pub fn write_console(handle: usize, bytes: &[u8]) -> u32 {
    unsafe { syscall3(NT_WRITE_CONSOLE, handle, bytes.as_ptr() as usize, bytes.len()) as u32 }
}

/// `NtQuerySystemTime()` -> Unix zamani (saniye). Saat yoksa 0.
pub fn query_system_time() -> u32 {
    let (_, time) = unsafe { syscall3_out(NT_QUERY_SYSTEM_TIME, 0, 0, 0) };
    time as u32
}

/// `NtDelayExecution(Alertable, Milliseconds)`.
pub fn delay_execution(ms: usize) {
    unsafe { syscall3(NT_DELAY_EXECUTION, 0, ms, 0) };
}

/// `NtYieldExecution()`.
pub fn yield_execution() {
    unsafe { syscall0(NT_YIELD_EXECUTION) };
}
