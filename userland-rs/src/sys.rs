//! Ham sistem cagrisi katmani -- `int 0x80` (doc S.6 ABI).
//!
//! Sozlesme: `EAX` = numara, `EBX/ECX/EDX` = arg1..3, donus `EAX`'te.
//! Cekirdek tarafinda bu cagrilar once **Level-0b2 Dispatcher**'a duser,
//! oradan **Level-0b1 POSIX cevirmenine** gider ve nihayet Level-0a'nin
//! notr cekirdek API'sine baglanir. Uygulama bu zinciri hic bilmez.

use core::arch::asm;

// --- Linux (i386) numaralari ---
pub const SYS_EXIT: usize = 1;
pub const SYS_FORK: usize = 2;
pub const SYS_WAITPID: usize = 7;
pub const SYS_READ: usize = 3;
pub const SYS_WRITE: usize = 4;
pub const SYS_OPEN: usize = 5;
pub const SYS_CLOSE: usize = 6;
pub const SYS_BRK: usize = 45;
pub const SYS_PIPE: usize = 42;

// --- TCMK'ye ozgu cagrilar (POSIX'te karsiligi yok) ---
pub const SYS_WIN_CREATE: usize = 0x500;
pub const SYS_WIN_BUFFER: usize = 0x501;
pub const SYS_WIN_SIZE: usize = 0x502;
pub const SYS_WIN_FLUSH: usize = 0x503;
pub const SYS_WIN_POLL_KEY: usize = 0x504;
pub const SYS_MOUSE_STATE: usize = 0x505;
pub const SYS_YIELD: usize = 0x506;
pub const SYS_WIN_POS: usize = 0x507;
pub const SYS_SLEEP: usize = 0x508;
pub const SYS_EXECVE: usize = 0x509;

pub const STDIN: usize = 0;
pub const STDOUT: usize = 1;
pub const STDERR: usize = 2;

/// EBX, x86-32'de LLVM tarafindan (PIC taban registeri olarak) rezerve
/// edilebildigi icin inline asm'de dogrudan `in("ebx")` kullanilamaz.
/// Bu yuzden deger genel bir registerdan EBX'e tasinir; eski EBX yigina
/// alinip geri yuklenir. `nostack` bu yuzden **verilemez**.
macro_rules! int80 {
    ($n:expr, $a1:expr, $a2:expr, $a3:expr) => {{
        let ret: usize;
        asm!(
            "push ebx",
            "mov ebx, {a1}",
            "int 0x80",
            "pop ebx",
            a1 = in(reg) $a1,
            inlateout("eax") $n => ret,
            in("ecx") $a2,
            in("edx") $a3,
        );
        ret
    }};
}

/// # Safety
/// Cagiran, verilen numaranin ve argumanlarin cekirdek sozlesmesine
/// uydugunu garanti etmelidir (ornegin isaretciler gecerli olmalidir).
#[inline(always)]
pub unsafe fn syscall3(n: usize, a1: usize, a2: usize, a3: usize) -> usize {
    int80!(n, a1, a2, a3)
}

/// # Safety
/// Bkz. [`syscall3`].
#[inline(always)]
pub unsafe fn syscall2(n: usize, a1: usize, a2: usize) -> usize {
    int80!(n, a1, a2, 0usize)
}

/// # Safety
/// Bkz. [`syscall3`].
#[inline(always)]
pub unsafe fn syscall1(n: usize, a1: usize) -> usize {
    int80!(n, a1, 0usize, 0usize)
}

/// # Safety
/// Bkz. [`syscall3`].
#[inline(always)]
pub unsafe fn syscall0(n: usize) -> usize {
    int80!(n, 0usize, 0usize, 0usize)
}

// --- Guvenli sarmalayicilar -------------------------------------------

/// Sureci sonlandirir. Cekirdek Ring 3 baglamini birakip Ring 0'a doner.
pub fn exit(code: i32) -> ! {
    unsafe { syscall1(SYS_EXIT, code as usize) };
    // sys_exit geri donmez; yine de tip sistemi icin.
    loop {
        yield_now();
    }
}

/// Verilen tanimlayiciya yazar; yazilan bayt sayisini ya da negatif
/// errno dondurur.
pub fn write(fd: usize, buf: &[u8]) -> isize {
    unsafe { syscall3(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len()) as isize }
}

/// Verilen tanimlayicidan okur.
pub fn read(fd: usize, buf: &mut [u8]) -> isize {
    unsafe { syscall3(SYS_READ, fd, buf.as_mut_ptr() as usize, buf.len()) as isize }
}

/// POSIX `O_CREAT`: dosya yoksa olustur.
pub const O_CREAT: usize = 0o100;

/// VFS'te bir dosya acar. `path` NUL ile sonlanmalidir (bkz. [`crate::io::File`]).
///
/// # Safety
/// `path` NUL sonlandirmali gecerli bir dizi olmalidir.
pub unsafe fn open_raw(path: *const u8, flags: usize) -> isize {
    syscall3(SYS_OPEN, path as usize, flags, 0) as isize
}

/// Tanimlayiciyi kapatir.
///
/// Boru ucu icin ayrica anlam tasir: son yazan uc kapandiginda okuyan
/// taraf icin "dosya sonu" olusur.
pub fn close(fd: usize) -> isize {
    unsafe { syscall1(SYS_CLOSE, fd) as isize }
}

/// Program break'i okur (0) veya tasir. Basarisizlikta eski deger doner.
pub fn brk(new: usize) -> usize {
    unsafe { syscall1(SYS_BRK, new) }
}

/// CPU'yu gonullu olarak birakir.
pub fn yield_now() {
    unsafe { syscall0(SYS_YIELD) };
}

/// Calisan sureci verilen programla **degistirir**.
///
/// Basarili olursa geri donmez: cekirdek eski imaji ve adres uzayini
/// birakir, ayni gorevde yeni programi yukler. Hata halinde negatif
/// errno doner (dosya yok, yol gecersiz).
pub fn execve(path: &str) -> isize {
    let mut buf = [0u8; 64];
    if path.len() >= buf.len() {
        return -22; // -EINVAL
    }
    buf[..path.len()].copy_from_slice(path.as_bytes());
    unsafe { syscall1(SYS_EXECVE, buf.as_ptr() as usize) as isize }
}

/// Sureci en az `ms` milisaniye uyutur.
///
/// `yield`'den farki: uyuyan gorev zamanlayici tarafindan **hic
/// secilmez**, dolayisiyla CPU'yu gercekten birakir. Cozunurluk PIT
/// hizina baglidir (100 Hz -> 10 ms).
pub fn sleep_ms(ms: usize) {
    unsafe { syscall1(SYS_SLEEP, ms) };
}

/// Sureci ikiye ayirir.
///
/// **Tek cagri, iki donus**: ebeveynde cocugun gorev kimligi, cocukta
/// `0`. Cocuk, ebeveynin bellegi kopyalanmis olarak bu satirdan devam
/// eder -- yani `fork()`'tan sonraki kod iki kez calisir.
///
/// Kaynak yoksa negatif deger doner (`-EAGAIN`).
pub fn fork() -> isize {
    unsafe { syscall0(SYS_FORK) as isize }
}

/// `waitpid` secenegi: cocuk bitmemisse bekleme, 0 don.
pub const WNOHANG: usize = 1;

/// Bir cocuk surecin bitmesini bekler.
///
/// Donus: cocugun kimligi, ya da `WNOHANG` verilip cocuk hala
/// calisiyorsa `0`, ya da hata durumunda negatif deger.
///
/// `status`, Linux'un kodlamasini kullanir: normal cikista
/// `(kod & 0xFF) << 8`. `exit_status` bunu cozer.
pub fn waitpid(pid: usize, status: &mut u32, options: usize) -> isize {
    unsafe { syscall3(SYS_WAITPID, pid, status as *mut u32 as usize, options) as isize }
}

/// `WEXITSTATUS`: durum kelimesinden cikis kodunu cikarir.
pub fn exit_status(status: u32) -> u32 {
    (status >> 8) & 0xFF
}

/// Yeni bir boru acar; `(okuma_fd, yazma_fd)` dondurur.
///
/// `fork`'tan **once** cagrilmalidir: cocuk ancak o zaman ayni boruyu
/// gorur. Bu, UNIX'in klasik kalibidir -- once boru, sonra catallanma.
pub fn pipe() -> Option<(usize, usize)> {
    let packed = unsafe { syscall0(SYS_PIPE) };
    if (packed as isize) < 0 {
        return None;
    }
    Some((packed >> 16, packed & 0xFFFF))
}
