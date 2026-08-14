//! Ham sistem cagrisi katmani (doc S.6 ABI).
//!
//! Cagri yolu iki mimaride de aynidir -- once **Level-0b2 Dispatcher**,
//! oradan **Level-0b1 POSIX cevirmeni**, nihayet Level-0a'nin notr
//! cekirdek API'si. Uygulama bu zinciri hic bilmez. Degisen yalnizca
//! cekirdege giris bicimidir:
//!
//! | | i386 | x86_64 |
//! |---|---|---|
//! | giris | `int 0x80` | `syscall` komutu |
//! | numara | EAX | RAX |
//! | arg1..3 | EBX/ECX/EDX | RDI/RSI/RDX |
//! | donus | EAX | RAX |
//!
//! **Numaralar da mimariye gore degisir**: ayni isim, farkli sayi. Bunu
//! tek kumeyle gecistirmek cekirdek tarafinda gercek bir hataya yol
//! acmisti (x86_64 `write`(=1), i386'nin `exit`'i sanilmisti), o yuzden
//! burada da ayri tutulur.

use core::arch::asm;

// --- Linux numaralari (mimariye gore) ---
#[cfg(target_arch = "x86")]
pub use i386_numbers::*;
#[cfg(target_arch = "x86_64")]
pub use x86_64_numbers::*;

#[cfg(target_arch = "x86")]
mod i386_numbers {
    pub const SYS_EXIT: usize = 1;
    pub const SYS_FORK: usize = 2;
    pub const SYS_READ: usize = 3;
    pub const SYS_WRITE: usize = 4;
    pub const SYS_OPEN: usize = 5;
    pub const SYS_CLOSE: usize = 6;
    pub const SYS_DUP: usize = 41;
    pub const SYS_DUP2: usize = 63;
    pub const SYS_POLL: usize = 168;
    pub const SYS_WAITPID: usize = 7;
    pub const SYS_PIPE: usize = 42;
    pub const SYS_BRK: usize = 45;
    pub const SYS_GETPID: usize = 20;
    pub const SYS_KILL: usize = 37;
    pub const SYS_SIGNAL: usize = 48;
    pub const SYS_SIGRETURN: usize = 119;
    pub const SYS_SIGPROCMASK: usize = 126;
    pub const SYS_ALARM: usize = 27;
    pub const SYS_MMAP: usize = 192;
    pub const SYS_MUNMAP: usize = 91;
    pub const SYS_GETPRIORITY: usize = 96;
    pub const SYS_SETPRIORITY: usize = 97;
}

#[cfg(target_arch = "x86_64")]
mod x86_64_numbers {
    pub const SYS_READ: usize = 0;
    pub const SYS_WRITE: usize = 1;
    pub const SYS_OPEN: usize = 2;
    pub const SYS_CLOSE: usize = 3;
    pub const SYS_DUP: usize = 32;
    pub const SYS_DUP2: usize = 33;
    pub const SYS_POLL: usize = 7;
    pub const SYS_BRK: usize = 12;
    pub const SYS_PIPE: usize = 22;
    pub const SYS_FORK: usize = 57;
    pub const SYS_EXIT: usize = 60;
    /// Linux x86_64'te `waitpid` yoktur; `wait4` onun yerine gecer.
    pub const SYS_WAITPID: usize = 61;
    pub const SYS_GETPID: usize = 39;
    pub const SYS_KILL: usize = 62;
    /// x86_64'te klasik `signal` yoktur; yerini `rt_sigaction` alir.
    pub const SYS_SIGNAL: usize = 13;
    pub const SYS_SIGRETURN: usize = 15;
    pub const SYS_SIGPROCMASK: usize = 14;
    pub const SYS_ALARM: usize = 37;
    pub const SYS_MMAP: usize = 9;
    pub const SYS_MUNMAP: usize = 11;
    pub const SYS_GETPRIORITY: usize = 140;
    pub const SYS_SETPRIORITY: usize = 141;
}

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
#[cfg(target_arch = "x86")]
macro_rules! syscall_asm {
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

/// x86_64'te `syscall` komutu donus adresini **RCX**'e, RFLAGS'i
/// **R11**'e yazar; ikisi de cagri sonrasi bozulmus sayilmalidir, yoksa
/// derleyici onlarda tuttugu degerleri kaybeder.
#[cfg(target_arch = "x86_64")]
macro_rules! syscall_asm {
    ($n:expr, $a1:expr, $a2:expr, $a3:expr) => {{
        let ret: usize;
        asm!(
            "syscall",
            inlateout("rax") $n => ret,
            in("rdi") $a1,
            in("rsi") $a2,
            in("rdx") $a3,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
        ret
    }};
}

/// # Safety
/// Cagiran, verilen numaranin ve argumanlarin cekirdek sozlesmesine
/// uydugunu garanti etmelidir (ornegin isaretciler gecerli olmalidir).
#[inline(always)]
pub unsafe fn syscall3(n: usize, a1: usize, a2: usize, a3: usize) -> usize {
    syscall_asm!(n, a1, a2, a3)
}

/// # Safety
/// Bkz. [`syscall3`].
#[inline(always)]
pub unsafe fn syscall2(n: usize, a1: usize, a2: usize) -> usize {
    syscall_asm!(n, a1, a2, 0usize)
}

/// # Safety
/// Bkz. [`syscall3`].
#[inline(always)]
pub unsafe fn syscall1(n: usize, a1: usize) -> usize {
    syscall_asm!(n, a1, 0usize, 0usize)
}

/// # Safety
/// Bkz. [`syscall3`].
#[inline(always)]
pub unsafe fn syscall0(n: usize) -> usize {
    syscall_asm!(n, 0usize, 0usize, 0usize)
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

// --- `poll(2)` ---

/// Okunacak veri var (ya da dosya sonu).
pub const POLLIN: u16 = 0x001;
/// Yazilabilir: boruda yer var.
pub const POLLOUT: u16 = 0x004;
/// Hata: borunun okuyan ucu kalmadi.
pub const POLLERR: u16 = 0x008;
/// Karsi taraf kapandi: artik veri gelmeyecek.
pub const POLLHUP: u16 = 0x010;
/// Boyle bir tanimlayici yok.
pub const POLLNVAL: u16 = 0x020;

/// `poll`'un izledigi tek tanimlayici.
///
/// Yerlesim POSIX'in `struct pollfd`'siyle bire bir aynidir (4+2+2 = 8
/// bayt) ve iki mimaride de ayni boyuttadir; cekirdek diziyi bu bicimde
/// okur.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PollFd {
    pub fd: i32,
    /// Beklenen olaylar (`POLLIN` / `POLLOUT`).
    pub events: u16,
    /// Cekirdegin **gerceklesen** olaylari yazdigi alan.
    pub revents: u16,
}

impl PollFd {
    pub const fn new(fd: usize, events: u16) -> PollFd {
        PollFd {
            fd: fd as i32,
            events,
            revents: 0,
        }
    }

    /// Bu tanimlayicida beklenen olay gerceklesti mi.
    pub fn ready(&self, mask: u16) -> bool {
        self.revents & mask != 0
    }
}

/// POSIX `poll`: verilen tanimlayicilardan **hangisinin** hazir
/// oldugunu sorar.
///
/// `timeout_ms`: 0 = hemen don, negatif = suresiz bekle. Cozunurluk PIT
/// tikidir (10 ms).
///
/// Hazir tanimlayici sayisini doner; zaman asiminda 0.
pub fn poll(fds: &mut [PollFd], timeout_ms: isize) -> isize {
    unsafe {
        syscall3(
            SYS_POLL,
            fds.as_mut_ptr() as usize,
            fds.len(),
            timeout_ms as usize,
        ) as isize
    }
}

/// Anonim bellek ayirir (`mmap(NULL, len, ...)`); adresi doner.
///
/// `brk`'ten farki: `brk` tek bir siniri iter, `mmap` bagimsiz bloklar
/// verir ve `munmap` **cerceveleri havuza geri dondurur**.
///
/// TCMK yalnizca anonim/ozel eslemeyi destekler: `addr` sifir olmak
/// zorunda, `prot` yok sayilir, dosya destekli esleme yok.
pub fn mmap(len: usize) -> Option<*mut u8> {
    let r = unsafe { syscall3(SYS_MMAP, 0, len, 0) as isize };
    if r < 0 {
        None
    } else {
        Some(r as usize as *mut u8)
    }
}

/// Ayrilan bolgeyi birakir; cerceveler havuza doner.
pub fn munmap(addr: *mut u8, len: usize) -> isize {
    unsafe { syscall2(SYS_MUNMAP, addr as usize, len) as isize }
}

/// POSIX `dup`: tanimlayiciyi en kucuk bos numaraya kopyalar.
pub fn dup(fd: usize) -> isize {
    unsafe { syscall1(SYS_DUP, fd) as isize }
}

/// POSIX `dup2`: kopyayi **istenen** numaraya koyar; varsa oradakini
/// kapatir.
///
/// Yonlendirmenin tamami budur: `dup2(w, STDOUT)` dedikten sonra stdout'a
/// yazan kod -- borudan haberi olmasa bile -- boruya yazar. Kabuktaki
/// `komut > dosya` da tam olarak bu cagridir.
pub fn dup2(oldfd: usize, newfd: usize) -> isize {
    unsafe { syscall2(SYS_DUP2, oldfd, newfd) as isize }
}

/// Program break'i okur (0) veya tasir. Basarisizlikta eski deger doner.
pub fn brk(new: usize) -> usize {
    unsafe { syscall1(SYS_BRK, new) }
}

/// `setpriority`/`getpriority` icin tek desteklenen `which`.
pub const PRIO_PROCESS: usize = 0;

/// Surecin oncelik degerini ayarlar.
///
/// POSIX geleneginde SAYI BUYUDUKCE oncelik DUSER (-20 en yuksek,
/// 19 en dusuk): "baskalarina karsi ne kadar nazik oldugun". `pid = 0`
/// cagiran surec demektir.
pub fn setpriority(pid: usize, nice: i32) -> isize {
    unsafe { syscall3(SYS_SETPRIORITY, PRIO_PROCESS, pid, nice as usize) as isize }
}

/// Surecin oncelik degerini okur.
///
/// Cekirdek Linux sozlesmesine uyar ve `20 - nice` doner (negatif deger
/// hata sayilacagi icin); burada geri cevrilir -- glibc'nin yaptigi da
/// tam olarak budur.
pub fn getpriority(pid: usize) -> i32 {
    let raw = unsafe { syscall2(SYS_GETPRIORITY, PRIO_PROCESS, pid) as isize };
    20 - raw as i32
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

/// `waitpid(WAIT_ANY, ...)`: hangi cocuk once biterse onu topla.
///
/// POSIX'te bu -1'dir; arguman isaretsiz gectigi icin tum bitleri bir
/// olan deger kullanilir.
pub const WAIT_ANY: usize = usize::MAX;

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
