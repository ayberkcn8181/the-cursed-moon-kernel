//! Linux Ceviri Araci -- POSIX Subsystem (doc S.2.2.B).
//!
//! i386 Linux sistem cagrilarini yakalar ve Level-0a'nin ortak cekirdek
//! API'sine (`level0a::kernel_api`) cevirir. Bu dosya bilerek **hicbir
//! donanima dokunmaz**: gorevi yalnizca ABI cevirisidir.
//!
//! Desteklenen cagrilar (doc S.6):
//!    1 = sys_exit    (EBX = cikis kodu)
//!    3 = sys_read    (EBX = fd,   ECX = buf,  EDX = count)
//!    4 = sys_write   (EBX = fd,   ECX = buf,  EDX = count)
//!    5 = sys_open    (EBX = path, ECX = flags, EDX = mode)
//!    6 = sys_close   (EBX = fd)
//!   45 = sys_brk     (EBX = yeni break, 0 ise mevcut break dondurulur)
//!    2 = sys_fork    (argumansiz; ebeveyne cocuk id, cocuga 0 doner)
//!    7 = sys_waitpid (EBX = pid, ECX = *status, EDX = secenekler)
//!
//! x86_64 numaralari farklidir (fork = 57, wait4 = 61); ikisi de ayni
//! cevirmene duser.
//!   42 = sys_pipe    (argumansiz; (okuma << 16) | yazma doner)

use crate::arch::cpu::regs::SyscallFrame;
use crate::level0a::core::{env, fd, mmu, scheduler};
use crate::level0a::gui_api;
use crate::level0a::kernel_api::{self, KernelError};
use crate::level0b1::signal;

// --- TCMK'ye ozgu GUI cagrilari ---
//
// POSIX'te karsiligi olmayan islevler icin 0x500+ araligi ayrildi. Bu
// aralik hem i386 hem x86_64 Linux numaralarinin cok uzerindedir, bu
// yuzden iki mimaride de catisma olmaz. (NT tarafi 0x1000+ kullanir.)
pub const SYS_WIN_CREATE: u32 = 0x500;
pub const SYS_WIN_BUFFER: u32 = 0x501;
pub const SYS_WIN_SIZE: u32 = 0x502;
pub const SYS_WIN_FLUSH: u32 = 0x503;
pub const SYS_WIN_POLL_KEY: u32 = 0x504;
pub const SYS_MOUSE_STATE: u32 = 0x505;
pub const SYS_YIELD: u32 = 0x506;
pub const SYS_WIN_POS: u32 = 0x507;
pub const SYS_SLEEP: u32 = 0x508;
pub const SYS_EXECVE: u32 = 0x509;
/// `setenv`/`unsetenv` -- Linux'ta boyle bir sistem cagrisi **yoktur**.
///
/// Gercek POSIX'te ortam surecin kendi belleginde bir dizidir ve libc
/// onu cekirdege sormadan duzenler. TCMK'de tablo cekirdekte durdugu
/// icin (bkz. `level0a/core/env.rs`) bir cagri gerekiyor. Numaranin
/// Linux araliginda degil TCMK araliginda olmasi da bunu soyluyor: bu
/// cagri Linux uyumlulugunun degil, TCMK'nin kendi tasariminin sonucu.
///
/// Win32 tarafinda ayni islev **gercekten** bir cekirdek cagrisidir
/// (`SetEnvironmentVariableA` sureci temsil eden blogu degistirir), yani
/// iki ABI burada da ayni yerde bulusmuyor.
pub const SYS_SETENV: u32 = 0x50B;

// Linux syscall numaralari MIMARIYE GORE DEGISIR -- ayni isim, farkli sayi.
// Bunu tek bir kumeyle gecistirmek Faz 4'te gercek bir hataya yol acti:
// x86_64 userland `write`(=1) cagirdi, cekirdek 1'i i386'nin `exit`'i sandi
// ve programi "cikis kodu 1" ile sonlandirdi.
#[cfg(target_arch = "x86")]
pub use i386_numbers::*;
#[cfg(target_arch = "x86_64")]
pub use x86_64_numbers::*;

#[cfg(target_arch = "x86")]
mod i386_numbers {
    pub const SYS_EXIT: u32 = 1;
    pub const SYS_FORK: u32 = 2;
    pub const SYS_WAITPID: u32 = 7;
    pub const SYS_PIPE: u32 = 42;
    pub const SYS_READ: u32 = 3;
    pub const SYS_WRITE: u32 = 4;
    pub const SYS_OPEN: u32 = 5;
    pub const SYS_CLOSE: u32 = 6;
    pub const SYS_DUP: u32 = 41;
    pub const SYS_DUP2: u32 = 63;
    pub const SYS_POLL: u32 = 168;
    pub const SYS_LSEEK: u32 = 19;
    /// i386'da `fstat64`. TCMK tam `struct stat` dondurmez (bkz. cagri).
    pub const SYS_FSTAT: u32 = 197;
    /// Linux'ta `getdents64`. Kayit bicimi TCMK'ye ozgu (bkz. cagri).
    pub const SYS_GETDENTS: u32 = 220;
    pub const SYS_MKDIR: u32 = 39;
    pub const SYS_RMDIR: u32 = 40;
    pub const SYS_UNLINK: u32 = 10;
    pub const SYS_RENAME: u32 = 38;
    pub const SYS_CHDIR: u32 = 12;
    pub const SYS_GETCWD: u32 = 183;
    /// i386'da `stat64`. Kayit bicimi TCMK'ye ozgu (bkz. cagri).
    pub const SYS_STAT: u32 = 195;
    pub const SYS_ACCESS: u32 = 33;
    pub const SYS_TIME: u32 = 13;
    pub const SYS_CLOCK_GETTIME: u32 = 265;
    pub const SYS_UNAME: u32 = 122;
    pub const SYS_FSYNC: u32 = 118;
    pub const SYS_GETPPID: u32 = 64;
    pub const SYS_WRITEV: u32 = 146;
    pub const SYS_NANOSLEEP: u32 = 162;
    pub const SYS_SCHED_YIELD: u32 = 158;
    pub const SYS_EXIT_GROUP: u32 = 252;
    pub const SYS_FTRUNCATE: u32 = 93;
    pub const SYS_READV: u32 = 145;
    /// i386'da 199 `getuid32`; eski 24 16-bit kimlik dondururdu.
    pub const SYS_GETUID: u32 = 199;
    pub const SYS_GETGID: u32 = 200;
    pub const SYS_GETEUID: u32 = 201;
    pub const SYS_GETEGID: u32 = 202;
    /// i386'da is-parcacigi tabani bir **GDT tanimlayicisi** ister.
    pub const SYS_SET_THREAD_AREA: u32 = 243;
    pub const SYS_BRK: u32 = 45;
    pub const SYS_GETPID: u32 = 20;
    pub const SYS_KILL: u32 = 37;
    /// i386 Linux'ta klasik `signal(2)`. TCMK ucuncu bir arguman ister
    /// (tramplen); gercek Linux'ta o deger `sigaction.sa_restorer`
    /// alanindan gelir -- yani fikir ayni, tasima yolu farkli.
    pub const SYS_SIGNAL: u32 = 48;
    /// i386'da `rt_sigaction`. Yapi isaretciyle gelir (bkz. cagri).
    pub const SYS_SIGACTION: u32 = 174;
    pub const SYS_SIGRETURN: u32 = 119;
    pub const SYS_SIGPROCMASK: u32 = 126;
    pub const SYS_ALARM: u32 = 27;
    pub const SYS_PAUSE: u32 = 29;
    /// i386'da `sigsuspend`(72) eski `sigset_t`u alir; `rt_sigsuspend`
    /// 179'dur ve TCMK'nin 32-bit maskesine dogrudan oturur.
    pub const SYS_SIGSUSPEND: u32 = 179;
    /// i386'da `mmap2` -- eski `mmap`(90) argumanlari bir yapida alirdi.
    pub const SYS_MMAP: u32 = 192;
    pub const SYS_MUNMAP: u32 = 91;
    pub const SYS_GETPRIORITY: u32 = 96;
    pub const SYS_SETPRIORITY: u32 = 97;
}

#[cfg(target_arch = "x86_64")]
mod x86_64_numbers {
    pub const SYS_READ: u32 = 0;
    pub const SYS_WRITE: u32 = 1;
    pub const SYS_OPEN: u32 = 2;
    pub const SYS_CLOSE: u32 = 3;
    pub const SYS_DUP: u32 = 32;
    pub const SYS_DUP2: u32 = 33;
    pub const SYS_POLL: u32 = 7;
    pub const SYS_LSEEK: u32 = 8;
    pub const SYS_FSTAT: u32 = 5;
    /// `getdents64`.
    pub const SYS_GETDENTS: u32 = 217;
    pub const SYS_MKDIR: u32 = 83;
    pub const SYS_RMDIR: u32 = 84;
    pub const SYS_UNLINK: u32 = 87;
    pub const SYS_RENAME: u32 = 82;
    pub const SYS_CHDIR: u32 = 80;
    pub const SYS_GETCWD: u32 = 79;
    pub const SYS_STAT: u32 = 4;
    pub const SYS_ACCESS: u32 = 21;
    pub const SYS_TIME: u32 = 201;
    pub const SYS_CLOCK_GETTIME: u32 = 228;
    pub const SYS_UNAME: u32 = 63;
    pub const SYS_FSYNC: u32 = 74;
    pub const SYS_GETPPID: u32 = 110;
    pub const SYS_WRITEV: u32 = 20;
    pub const SYS_NANOSLEEP: u32 = 35;
    pub const SYS_SCHED_YIELD: u32 = 24;
    pub const SYS_EXIT_GROUP: u32 = 231;
    pub const SYS_FTRUNCATE: u32 = 77;
    pub const SYS_READV: u32 = 19;
    pub const SYS_GETUID: u32 = 102;
    pub const SYS_GETGID: u32 = 104;
    pub const SYS_GETEUID: u32 = 107;
    pub const SYS_GETEGID: u32 = 108;
    /// x86_64'te tanimlayici yok; taban dogrudan MSR'ye yaziliyor.
    pub const SYS_ARCH_PRCTL: u32 = 158;
    pub const SYS_BRK: u32 = 12;
    pub const SYS_PIPE: u32 = 22;
    pub const SYS_FORK: u32 = 57;
    pub const SYS_EXIT: u32 = 60;
    /// Linux x86_64'te `waitpid` yoktur; `wait4` onun yerine gecer.
    pub const SYS_WAITPID: u32 = 61;
    pub const SYS_GETPID: u32 = 39;
    pub const SYS_KILL: u32 = 62;
    /// x86_64'te 13 **gercekten** `rt_sigaction`dir. Sadelestirilmis
    /// `signal` yuzu icin ayri bir numara ayrildi: kullanici tarafi
    /// zaten libc gibi `signal`i `sigaction` uzerine kuruyor.
    pub const SYS_SIGACTION: u32 = 13;
    pub const SYS_SIGNAL: u32 = 0x50A;
    pub const SYS_SIGRETURN: u32 = 15;
    /// x86_64'te klasik `sigprocmask` yoktur; `rt_sigprocmask` gecer.
    pub const SYS_SIGPROCMASK: u32 = 14;
    pub const SYS_ALARM: u32 = 37;
    pub const SYS_PAUSE: u32 = 34;
    /// x86_64'te `rt_sigsuspend`.
    pub const SYS_SIGSUSPEND: u32 = 130;
    pub const SYS_MMAP: u32 = 9;
    pub const SYS_MUNMAP: u32 = 11;
    pub const SYS_GETPRIORITY: u32 = 140;
    pub const SYS_SETPRIORITY: u32 = 141;
}

// Linux hata kodlari negatif dondurulur (ornegin -EBADF = -9).
const EBADF: i32 = 9;
const EFAULT: i32 = 14;
const ENOENT: i32 = 2;
const EMFILE: i32 = 24;
const EINVAL: i32 = 22;
const ENOSYS: i32 = 38;
/// Kaynak gecici olarak yok -- `fork` icin gorev tablosu ya da cerceve
/// havuzu dolu demektir (Linux `fork` da bu kodu dondurur).
const EAGAIN: i32 = 11;
/// Beklenecek boyle bir cocuk yok.
const ECHILD: i32 = 10;
/// Boyle bir surec yok (`kill` hedefi).
const ESRCH: i32 = 3;
/// Bellek yetmedi (`mmap`).
const ENOMEM: i32 = 12;
const EEXIST: i32 = 17;
const ENOTEMPTY: i32 = 39;
const ENOSPC: i32 = 28;
/// Salt okunur dosya sistemi -- RAMFS cekirdek imajinin parcasidir.
const EROFS: i32 = 30;
/// Cagri bir sinyal tarafindan kesildi. `pause`/`sigsuspend` **yalnizca**
/// bunu dondurur: basarili bir donusleri yoktur.
const EINTR: i32 = 4;
/// Sonuc verilen tampona sigmiyor (`getcwd`).
const ERANGE: i32 = 34;
/// Izin yok -- TCMK'de tek kaynagi RAMFS'in salt okunur olmasi.
const EACCES: i32 = 13;

/// `clock_gettime` saat kimlikleri (Linux ile ayni sayilar).
const CLOCK_MONOTONIC: usize = 1;

/// `setpriority`/`getpriority` icin desteklenen tek `which` degeri.
const PRIO_PROCESS: usize = 0;

/// `waitpid` secenegi: cocuk bitmemisse bloke olma, 0 don.
const WNOHANG: usize = 1;

/// `waitpid(-1, ...)`: herhangi bir cocuk. Arguman isaretsiz geldigi icin
/// -1, tum bitleri bir olan degerdir.
const WAIT_ANY: usize = usize::MAX;

/// Negatif errno'yu cerceveye yazar (isaretli genisletme ile).
fn return_errno(frame: &mut SyscallFrame, errno: i32) {
    frame.set_return(errno as isize as usize);
}

/// `waitpid`'in durum kelimesini kullaniciya yazar.
///
/// Linux kodlamasi: normal cikista `status = (kod & 0xFF) << 8`, boylece
/// `WEXITSTATUS(status)` = `(status >> 8) & 0xFF` calisir. Isaretci NULL
/// olabilir (POSIX'te de oyle); o zaman yazilmaz ve cagri basarilidir.
fn store_status(ptr: usize, code: u32) -> bool {
    if ptr == 0 {
        return true;
    }
    if !mmu::is_user_accessible(ptr) || !mmu::is_user_accessible(ptr + 3) {
        return false;
    }
    unsafe { (ptr as *mut u32).write_unaligned((code & 0xFF) << 8) };
    true
}

/// Kullanici alanindan gelen yol adinin en fazla uzunlugu.
const PATH_MAX: usize = 128;

/// `sigaction` yapisinin kelime sayisi: isleyici, tramplen, bayrak, maske.
const SIGACTION_WORDS: usize = 4;

/// Kullanici alanindan ardisik kelimeler okur.
///
/// Her kelime **ayri ayri** dogrulanir: yapi iki sayfaya yayilmis
/// olabilir ve ikincisi kullaniciya ait olmayabilir.
fn read_user_words(addr: usize, out: &mut [usize]) -> bool {
    let width = core::mem::size_of::<usize>();
    for (i, slot) in out.iter_mut().enumerate() {
        let at = addr + i * width;
        if !mmu::is_user_accessible(at) || !mmu::is_user_accessible(at + width - 1) {
            return false;
        }
        *slot = unsafe { (at as *const usize).read_unaligned() };
    }
    true
}

/// `envp` dizisinden okunan tek bir `AD=deger` satiri.
///
/// Cekirdek yiginda duruyor: isaretciler cagiranin adres uzayina bakar
/// ve o uzay `execve` ile gider, yani satirlarin **once** kopyalanmasi
/// sart.
#[derive(Clone, Copy)]
struct EnvEntry {
    bytes: [u8; env::MAX_ENTRY],
    len: usize,
}

impl EnvEntry {
    const EMPTY: EnvEntry = EnvEntry {
        bytes: [0; env::MAX_ENTRY],
        len: 0,
    };

    fn as_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.bytes[..self.len]).ok()
    }
}

/// NULL ile biten `char *envp[]` dizisini cekirdege kopyalar.
///
/// Donus: kac girdi okundugu. Okunamayan bir isaretci `None` verir --
/// gecersiz bir `envp` ile exec etmek POSIX'te de `EFAULT`tur.
fn read_user_env(addr: usize, out: &mut [EnvEntry; env::MAX_VARS]) -> Option<usize> {
    let width = core::mem::size_of::<usize>();
    let mut count = 0usize;
    let mut index = 0usize;
    loop {
        let at = addr + index * width;
        if !mmu::is_user_accessible(at) || !mmu::is_user_accessible(at + width - 1) {
            return None;
        }
        let pointer = unsafe { (at as *const usize).read_unaligned() };
        if pointer == 0 {
            return Some(count);
        }
        // Tablo dolduysa kalanlar sessizce atilir: sabit boyutlu tablo
        // zaten `MAX_VARS` kadar tasiyabiliyor (bkz. `core::env`).
        if count < out.len() {
            let mut scratch = [0u8; PATH_MAX];
            let text = unsafe { copy_user_cstr(pointer, &mut scratch) }?;
            let taken = text.len().min(env::MAX_ENTRY - 1);
            out[count].bytes[..taken].copy_from_slice(&text.as_bytes()[..taken]);
            out[count].len = taken;
            count += 1;
        }
        index += 1;
        if index > env::MAX_VARS * 4 {
            // Sonlandirilmamis dizi: sinirsiz dolasmak yerine hata.
            return None;
        }
    }
}

/// Okunan `envp` satirlarini gorevin tablosuna **yerlestirir**.
///
/// Once temizlenir: yeni ortam eskisinin uzerine eklenmez, yerine gecer.
fn apply_user_env(task: usize, entries: &[EnvEntry]) {
    env::clear(task);
    for entry in entries {
        if let Some(text) = entry.as_str() {
            env::set_entry(task, text);
        }
    }
}

/// Kullanici alanina tek bir kelime yazar.
fn write_user_word(addr: usize, value: usize) -> bool {
    let width = core::mem::size_of::<usize>();
    if !mmu::is_user_accessible(addr) || !mmu::is_user_accessible(addr + width - 1) {
        return false;
    }
    unsafe { (addr as *mut usize).write_unaligned(value) };
    true
}

/// Kullanici isaretcisindeki yolu `stat`e verir.
///
/// Disaridaki `None` "yol okunamadi" (`EFAULT`), icerideki `Err` ise
/// "yol yok" demek -- ikisini ayirmak cagirana dogru errno'yu secme
/// imkani veriyor.
fn stat_user_path(
    ptr: usize,
    storage: &mut [u8; PATH_MAX],
) -> Option<Result<kernel_api::FileInfo, KernelError>> {
    let path = unsafe { copy_user_cstr(ptr, storage) }?;
    Some(kernel_api::stat(path))
}

/// Kullanici isaretcisinden yol adini alip bir `kernel_api` cagrisina verir.
///
/// `mkdir`/`rmdir`/`unlink` uculu ayni sekli paylasiyor: tek bir yol
/// argumani, donusu olmayan bir sonuc. Kaliba isim vermek uc kez
/// tekrarlanan tampon + kopyalama + hata cevirisini tek yerde topluyor.
///
/// Hata durumunda **negatif errno** doner, yani cagiran dogrudan
/// dondurebilir.
fn with_user_path(
    ptr: usize,
    action: fn(&str) -> Result<(), KernelError>,
) -> Result<(), i32> {
    let mut storage = [0u8; PATH_MAX];
    match unsafe { copy_user_cstr(ptr, &mut storage) } {
        Some(path) => action(path).map_err(errno_of),
        None => Err(-EFAULT),
    }
}

/// Tek bir `poll` cagrisinda izlenebilecek tanimlayici sayisi. Bir
/// surecin tablosu zaten bu kadar (`fd::MAX_FDS`).
const MAX_POLL_FDS: usize = fd::MAX_FDS;

/// `struct pollfd { int fd; short events; short revents; }` -- 8 bayt.
/// Boyut iki mimaride de aynidir (int 4, short 2, short 2), o yuzden
/// mimariye gore ayrilmasi gerekmiyor.
const POLLFD_SIZE: usize = 8;

/// Kopyalanan `pollfd` kaydinin cekirdek tarafindaki hali.
#[derive(Clone, Copy)]
struct PollEntry {
    fd: i32,
    events: u16,
    revents: u16,
}

impl PollEntry {
    const EMPTY: PollEntry = PollEntry {
        fd: -1,
        events: 0,
        revents: 0,
    };
}

fn errno_of(err: KernelError) -> i32 {
    match err {
        KernelError::BadFileDescriptor => -EBADF,
        KernelError::Fault => -EFAULT,
        KernelError::NotFound => -ENOENT,
        KernelError::TooManyOpenFiles => -EMFILE,
        KernelError::NotSupported => -EINVAL,
        KernelError::AlreadyExists => -EEXIST,
        KernelError::NotEmpty => -ENOTEMPTY,
        KernelError::NoSpace => -ENOSPC,
        KernelError::ReadOnly => -EROFS,
    }
}

/// Level-0b2 dispatcher'i tarafindan cagrilir. Donus degerini dogrudan
/// frame'in EAX alanina yazar (i386 Linux ABI).
pub fn dispatch(frame: &mut SyscallFrame, from_interrupt: bool) {
    let number = frame.number();
    let [arg1, arg2, arg3, _, _] = frame.args();

    let result: i32 = match number {
        // `exit` ve `exit_group` ayni yere iner.
        //
        // Ikisini de tasimak sart, cunku **glibc `exit` cagirmaz**:
        // `_exit` bile `exit_group`a duser (butun is parcaciklarini
        // birlikte sonlandirmak icin). TCMK'de is parcacigi yok, yani
        // ayrim pratikte kayboluyor -- ama numarayi tanimayan bir
        // cekirdek, gercek bir Linux ikilisini **cikamaz** hale
        // getirirdi.
        SYS_EXIT | SYS_EXIT_GROUP => {
            // Geri donmez.
            kernel_api::exit_current_task(arg1 as u32);
        }

        SYS_WRITE => match unsafe { kernel_api::write(arg1 as u32, arg2 as *const u8, arg3) } {
            Ok(written) => written as i32,
            Err(e) => errno_of(e),
        },

        SYS_READ => match unsafe { kernel_api::read(arg1 as u32, arg2 as *mut u8, arg3) } {
            Ok(read) => read as i32,
            Err(e) => errno_of(e),
        },

        SYS_OPEN => {
            let mut storage = [0u8; PATH_MAX];
            match unsafe { copy_user_cstr(arg1, &mut storage) } {
                // arg2 = bayraklar. Linux'ta O_CREAT = 0o100 = 0x40,
                // O_TRUNC = 0o1000 = 0x200.
                //
                // `O_TRUNC` bu cagriya kadar **yok sayiliyordu** ve bu
                // sessiz bir hataydi: uzun bir dosyanin uzerine kisa bir
                // metin yazan program, kuyrukta eski icerigi birakiyordu.
                Some(path) => {
                    let create = arg2 & 0x40 != 0;
                    let opened = if arg2 & 0x200 != 0 {
                        kernel_api::open_truncating(path, create)
                    } else {
                        kernel_api::open(path, create)
                    };
                    match opened {
                        Ok(fd) => fd as i32,
                        Err(e) => errno_of(e),
                    }
                }
                None => -EFAULT,
            }
        }

        SYS_CLOSE => match kernel_api::close(arg1 as u32) {
            Ok(()) => 0,
            Err(e) => errno_of(e),
        },

        // `dup` en kucuk bos numaraya, `dup2` istenen numaraya kopyalar.
        // Ikisi de yeni numarayi doner; hatada -EBADF/-EMFILE.
        SYS_DUP => match fd::dup(arg1) {
            Some(new_fd) => {
                frame.set_return(new_fd);
                return;
            }
            None if arg1 < fd::MAX_FDS => -EMFILE,
            None => -EBADF,
        },

        SYS_DUP2 => {
            if arg2 >= fd::MAX_FDS {
                -EBADF
            } else {
                match fd::dup2(arg1, arg2) {
                    Some(new_fd) => {
                        frame.set_return(new_fd);
                        return;
                    }
                    None => -EBADF,
                }
            }
        }

        SYS_BRK => {
            // brk bir ADRES dondurur, hata kodu degil -- isaretli
            // donusum yapilmadan dogrudan yazilir.
            frame.set_return(kernel_api::brk(arg1));
            return;
        }

        SYS_FORK => {
            // Tek cagri, iki donus: ebeveyn cocugun kimligini alir,
            // cocuk 0 alir (bkz. `level0b1::fork`).
            match unsafe { crate::level0b1::fork::fork(frame, from_interrupt) } {
                Ok(child) => {
                    frame.set_return(child);
                    return;
                }
                Err(crate::level0b1::fork::ForkError::NotUserProcess) => -EINVAL,
                Err(crate::level0b1::fork::ForkError::OutOfResources) => -EAGAIN,
            }
        }

        SYS_WAITPID => {
            // waitpid(pid, *status, options)
            //
            // TCMK'de "pid" gorev indeksidir. Beklerken gorev `Waiting`
            // durumundadir ve zamanlayici tarafindan atlanir -- yani
            // bekleyen bir surec CPU harcamaz.
            let child = arg1;
            let status_ptr = arg2;
            let nohang = arg3 & WNOHANG != 0;

            // pid = -1: "herhangi bir cocuk". POSIX'in en cok kullanilan
            // bicimi budur; bir kabuk cocuklarinin hangisinin once
            // bitecegini bilmez.
            if child == WAIT_ANY {
                let me = crate::level0a::core::scheduler::current_id();
                if !crate::level0a::core::scheduler::has_children(me) {
                    return_errno(frame, -ECHILD);
                    return;
                }
                if nohang {
                    // Bitmis cocuk varsa topla, yoksa 0.
                    match crate::level0a::core::scheduler::reap_finished_child(me) {
                        Some((pid, code)) => {
                            if !store_status(status_ptr, code) {
                                return_errno(frame, -EFAULT);
                            } else {
                                frame.set_return(pid);
                            }
                        }
                        None => frame.set_return(0),
                    }
                    return;
                }
                match crate::level0a::core::scheduler::wait_for_any() {
                    Some((pid, code)) => {
                        if !store_status(status_ptr, code) {
                            return_errno(frame, -EFAULT);
                        } else {
                            frame.set_return(pid);
                        }
                    }
                    None => return_errno(frame, -ECHILD),
                }
                return;
            }

            if child >= crate::level0a::core::scheduler::task_count() {
                -ECHILD
            } else if nohang {
                // Bitmediyse hemen 0 don (POSIX WNOHANG davranisi).
                if crate::level0a::core::scheduler::state_of(child)
                    != crate::level0a::core::scheduler::TaskState::Terminated
                {
                    0
                } else {
                    let code = crate::level0a::core::scheduler::exit_code_of(child);
                    if !store_status(status_ptr, code) {
                        -EFAULT
                    } else {
                        child as i32
                    }
                }
            } else {
                match crate::level0a::core::scheduler::wait_for_task(child) {
                    Some(code) => {
                        if !store_status(status_ptr, code) {
                            -EFAULT
                        } else {
                            child as i32
                        }
                    }
                    None => -ECHILD,
                }
            }
        }

        SYS_PIPE => {
            // Linux'ta `pipe(int fd[2])` iki tanimlayiciyi kullanici
            // bellegine yazar. TCMK ikisini tek kelimede paketleyip
            // dondurur (`okuma << 16 | yazma`): kullanici isaretcisi
            // dogrulamak gerekmez ve cagri yalinlasir.
            match kernel_api::create_pipe() {
                Ok((read_fd, write_fd)) => {
                    frame.set_return((read_fd << 16) | (write_fd & 0xFFFF));
                    return;
                }
                Err(e) => errno_of(e),
            }
        }

        // --- GUI cagrilari: adres/durum dondururler, errno degil ---
        SYS_WIN_CREATE => {
            // arg1 = baslik isaretcisi, arg2 = (x<<16)|y, arg3 = (w<<16)|h
            let mut storage = [0u8; PATH_MAX];
            let title = unsafe { copy_user_cstr(arg1, &mut storage) }.unwrap_or("app");
            let (x, y) = (arg2 >> 16, arg2 & 0xFFFF);
            let (w, h) = (arg3 >> 16, arg3 & 0xFFFF);
            let ret = match gui_api::create_window(title, x, y, w, h) {
                Ok(id) => id,
                Err(_) => usize::MAX,
            };
            frame.set_return(ret);
            return;
        }
        SYS_WIN_BUFFER => {
            frame.set_return(gui_api::window_buffer(arg1).unwrap_or(0));
            return;
        }
        SYS_WIN_SIZE => {
            frame.set_return(gui_api::window_size(arg1).unwrap_or(0));
            return;
        }
        SYS_WIN_POS => {
            frame.set_return(gui_api::window_pos(arg1).unwrap_or(usize::MAX));
            return;
        }
        SYS_WIN_FLUSH => {
            // Kompozitor her karede zaten cizer; flush yalnizca uygulamanin
            // CPU'yu birakma noktasidir.
            crate::level0a::core::scheduler::yield_now();
            frame.set_return(0);
            return;
        }
        SYS_WIN_POLL_KEY => {
            frame.set_return(gui_api::poll_key(arg1) as usize);
            return;
        }
        SYS_MOUSE_STATE => {
            frame.set_return(gui_api::mouse_state());
            return;
        }
        // TCMK'nin kendi numarasi (0x506) **ve** gercek Linux numarasi.
        // Ikincisi olmadan, derleyicinin urettigi bir ikili bu yetenege
        // hic ulasamazdi.
        SYS_YIELD | SYS_SCHED_YIELD => {
            crate::level0a::core::scheduler::yield_now();
            frame.set_return(0);
            return;
        }
        SYS_EXECVE => {
            // arg1 = yol, arg2 = arguman dizesi (NULL olabilir).
            //
            // Imaj YERINDE degistirilemez (surec o anda kendi kodunda
            // kosuyor); istek kaydedilip Ring 3'ten cikilir, launcher
            // yeni imaji yukler.
            //
            // Gercek `execve` bir `char *const argv[]` dizisi alir; TCMK
            // tek bir dize alip bolmeyi cekirdege birakiyor. Sebep,
            // ayni dizenin Win32 tarafinda **oldugu gibi** gerekmesi:
            // `GetCommandLineA` bolunmemis bir komut satiri doner.
            let mut storage = [0u8; PATH_MAX];
            let mut arg_storage = [0u8; PATH_MAX];
            let mut env_storage = [EnvEntry::EMPTY; env::MAX_VARS];
            let path_len = unsafe { copy_user_cstr(arg1, &mut storage) }.map(|p| p.len());
            let args_len = if arg2 == 0 {
                Some(0)
            } else {
                unsafe { copy_user_cstr(arg2, &mut arg_storage) }.map(|a| a.len())
            };
            match (path_len, args_len) {
                (Some(plen), Some(alen)) => {
                    let path = core::str::from_utf8(&storage[..plen]).unwrap_or("");
                    let args = core::str::from_utf8(&arg_storage[..alen]).unwrap_or("");
                    // Yol `PATH`/`PATHEXT` uzerinden cozuluyor: egik
                    // cizgi iceren adlar oldugu gibi, icermeyenler
                    // aranarak. `execvp`nin yaptigi da budur -- fark su
                    // ki orada arama libc'de, burada cekirdekte (tablo
                    // cekirdekte oldugu icin).
                    let mut program = [0u8; PATH_MAX];
                    let resolved = kernel_api::resolve_program(path, &mut program);
                    if resolved.is_none() {
                        -ENOENT
                    } else {
                        let path = resolved.unwrap();
                        let task = crate::level0a::core::scheduler::current_id();
                        // arg3 = `envp`. Sifirsa ortam **korunur** (yuva
                        // ayni kaldigi icin kendiliginden); doluysa
                        // tablonun **yerine gecer** -- gercek `execve`de
                        // de verilen dizi ortamin tamamidir.
                        //
                        // Kopyalama, imaj birakilmadan **once** yapilmak
                        // zorunda: isaretciler cagiranin adres uzayina
                        // bakiyor ve o uzay exec ile gidecek.
                        let environment = if arg3 == 0 {
                            Some(0usize)
                        } else {
                            read_user_env(arg3, &mut env_storage)
                        };
                        match environment {
                            None => -EFAULT,
                            Some(entries) => {
                                if crate::level0a::launcher::request_exec(task, path, args) {
                                    if arg3 != 0 {
                                        apply_user_env(task, &env_storage[..entries]);
                                    }
                                    unsafe { kernel_api::exit_to_exec() }
                                }
                                -EINVAL
                            }
                        }
                    }
                }
                _ => -EFAULT,
            }
        }
        // `poll(pollfd[], nfds, timeout_ms)` -- "hangisi hazir?"
        //
        // Bu cagriya kadar cevap yoklamaktan geciyordu: uygulama her
        // tanimlayiciyi sirayla `read` ediyor, sifir donerse uyuyordu.
        // Iki sorunu vardi -- veri gelse bile uyku suresi kadar
        // gecikiyordu, ve tek bir kaynagi beklerken digerlerini de
        // yoklamak zorundaydi.
        SYS_POLL => {
            let count = arg2;
            if count == 0 || count > MAX_POLL_FDS {
                return_errno(frame, -EINVAL);
                return;
            }
            let timeout = arg3 as isize;

            // Dizi once cekirdege kopyalanir. Kullanici bellegine dogrudan
            // bakip donguye girmek, kullanicinin arada sayfa esleme
            // degistirmesine acik olurdu.
            let mut entries = [PollEntry::EMPTY; MAX_POLL_FDS];
            for i in 0..count {
                let record = arg1 + i * POLLFD_SIZE;
                if !mmu::is_user_accessible(record)
                    || !mmu::is_user_accessible(record + POLLFD_SIZE - 1)
                {
                    return_errno(frame, -EFAULT);
                    return;
                }
                unsafe {
                    entries[i].fd = (record as *const i32).read_unaligned();
                    entries[i].events = ((record + 4) as *const u16).read_unaligned();
                }
            }

            // Zaman asimi tike cevrilir (PIT 100 Hz). Negatif = sonsuz.
            let start = crate::level0a::pit::ticks();
            let limit = if timeout < 0 {
                None
            } else {
                Some(((timeout as u32) / 10).max(if timeout > 0 { 1 } else { 0 }))
            };

            let ready = loop {
                let mut ready = 0usize;
                for entry in entries.iter_mut().take(count) {
                    // Istenmeyen olaylar suzulur; ama POLLERR/POLLHUP/
                    // POLLNVAL POSIX'te **istenmese de** bildirilir --
                    // kapanan bir boruyu kimse "istemez", yine de bilmek
                    // zorundadir.
                    let mask = if entry.fd < 0 {
                        0
                    } else {
                        kernel_api::readiness(entry.fd as u32)
                    };
                    entry.revents = mask
                        & (entry.events
                            | kernel_api::POLLERR
                            | kernel_api::POLLHUP
                            | kernel_api::POLLNVAL);
                    if entry.revents != 0 {
                        ready += 1;
                    }
                }
                if ready > 0 {
                    break ready;
                }
                if let Some(limit) = limit {
                    if crate::level0a::pit::ticks().wrapping_sub(start) >= limit {
                        break 0;
                    }
                }
                // Hicbiri hazir degil: bir tik **uyu**.
                //
                // Ilk hali `yield_now()` idi ve olcum onu eledi: tek bir
                // `poll` cagrisi baglam degisimini saniyede 316'dan
                // 104.000'e cikardi (330 kat). Gorev her turda hemen
                // devredip hemen geri geldigi icin `cpu` sutununda
                // gorunmuyordu bile -- yani "sifir CPU" olcusu
                // yaniltiyordu; is zamanlayicinin kendisinde yaniyordu.
                //
                // Uyumak bu turlarin tamamini siliyor. Bedeli en fazla
                // bir tiklik (10 ms) gecikme, ki `poll`'un zaman asimi
                // cozunurlugu zaten PIT tiki oldugu icin yeni bir
                // sinirlama getirmiyor.
                //
                // Dogrusu bir bekleme kuyrugu olurdu: her nesnenin (boru,
                // tus kuyrugu) uyandirma listesi tutmasi ve veri gelince
                // bekleyeni `Ready` yapmasi. O zaman gecikme de sifira
                // inerdi -- ama her nesnenin bekleyen listesi tasimasi
                // gerekirdi; buradaki tik cozunurluguyle deger etmiyor.
                crate::level0a::core::scheduler::sleep_ticks(1);
            };

            // Sonuclar geri yazilir. Kopyalama sirasinda dogrulanan
            // adresler yeniden dogrulanir: arada `yield` edildi.
            for i in 0..count {
                let field = arg1 + i * POLLFD_SIZE + 6;
                if !mmu::is_user_accessible(field) || !mmu::is_user_accessible(field + 1) {
                    return_errno(frame, -EFAULT);
                    return;
                }
                unsafe { (field as *mut u16).write_unaligned(entries[i].revents) };
            }

            frame.set_return(ready);
            return;
        }

        // `sigprocmask(how, maske)` -- ESKI maskeyi doner.
        //
        // Gercek POSIX iki `sigset_t` isaretcisi alir; 32 sinyal tek bir
        // kelimeye sigdigi icin burada maske deger olarak gecirilir
        // (ayni sadelestirme `pipe`'ta da yapildi).
        SYS_SIGPROCMASK => {
            let task = crate::level0a::core::scheduler::current_id();
            match signal::sigprocmask(task, arg1, arg2 as u32) {
                Some(old) => {
                    frame.set_return(old as usize);
                    return;
                }
                None => -EINVAL,
            }
        }

        // `alarm(saniye)` -- onceki alarmdan KALAN saniyeyi doner.
        SYS_ALARM => {
            let task = crate::level0a::core::scheduler::current_id();
            frame.set_return(signal::alarm(task, arg1 as u32) as usize);
            return;
        }

        // `mmap(addr, len, prot, ...)` -- yalnizca anonim/ozel.
        //
        // POSIX imzasi alti argumanlidir; TCMK'nin cerceve yapisi bes
        // tasir ve dosya destekli esleme zaten yok. Bu yuzden `addr`
        // sifir olmak zorunda (cekirdek yeri secer), `prot` yok sayilir
        // (butun kullanici sayfalari okuma+yazma) ve dosya alanlari hic
        // gelmez. Desteklenmeyen bir cagriyi sessizce baska bir sey gibi
        // davranmak yerine reddetmek dogru olan.
        SYS_MMAP => {
            let space = crate::level0a::core::scheduler::address_space_of(
                crate::level0a::core::scheduler::current_id(),
            );
            if arg1 != 0 || space == 0 {
                -EINVAL
            } else {
                match unsafe { mmu::mmap_user(space, arg2) } {
                    Some(addr) => {
                        frame.set_return(addr);
                        return;
                    }
                    // POSIX `mmap` hatada MAP_FAILED (-1) doner; Linux
                    // ABI'sinde bu negatif errno'dur.
                    None => -ENOMEM,
                }
            }
        }

        SYS_MUNMAP => {
            let space = crate::level0a::core::scheduler::address_space_of(
                crate::level0a::core::scheduler::current_id(),
            );
            if space != 0 && unsafe { mmu::munmap_user(space, arg1, arg2) } {
                0
            } else {
                -EINVAL
            }
        }

        // `lseek(fd, offset, whence)` -- yeni konumu doner.
        //
        // Bu cagriya kadar dosyalar yalnizca bastan sona okunabiliyordu.
        SYS_LSEEK => match kernel_api::lseek(arg1 as u32, arg2, arg3) {
            Ok(position) => {
                frame.set_return(position);
                return;
            }
            Err(e) => errno_of(e),
        },

        // `fstat(fd)` -- TCMK'de yalnizca **boyut** doner.
        //
        // Gercek `fstat` bir `struct stat` doldurur; izin, sahiplik,
        // aygit numarasi, bag sayisi gibi alanlarin hicbiri bu dosya
        // sisteminde yok. Yapiyi kopyalamak sifir dolu bir kayit tasimak
        // olurdu -- boyut ise gercek ve tek basina yeterli.
        SYS_FSTAT => match kernel_api::file_size(arg1 as u32) {
            Ok(size) => {
                frame.set_return(size);
                return;
            }
            Err(e) => errno_of(e),
        },

        // `stat(yol, buf)` -- yola gore bilgi; **acmadan**.
        //
        // Bu cagriya kadar "bu yol var mi?" sorusunun tek cevabi acmakti:
        // `open` deneyip sonuca bakmak. Dizinlerde o bile calismiyordu ve
        // acmanin yan etkisi var -- tanimlayici tuketiyor.
        //
        // Kayit bicimi `getdents`te oldugu gibi **TCMK'ye ozgu**: iki
        // `u32`, yani sekiz bayt, iki mimaride de ayni yerlesim.
        //
        //   [0..4)  boyut
        //   [4..8)  bayraklar: bit0 = dizin, bit1 = salt okunur
        //
        // Gercek `struct stat`i taklit etmek yirmi alanin on yedisini
        // sifirla doldurmak olurdu; sifir, "bilinmiyor" ile "sifir"
        // arasindaki farki silerdi.
        SYS_STAT => {
            let mut storage = [0u8; PATH_MAX];
            match stat_user_path(arg1, &mut storage) {
                None => -EFAULT,
                Some(Err(e)) => errno_of(e),
                Some(Ok(info)) => {
                    if !mmu::is_user_accessible(arg2) || !mmu::is_user_accessible(arg2 + 7) {
                        -EFAULT
                    } else {
                        let flags = u32::from(info.is_dir) | (u32::from(info.read_only) << 1);
                        unsafe {
                            (arg2 as *mut u32).write_unaligned(info.size as u32);
                            ((arg2 + 4) as *mut u32).write_unaligned(flags);
                        }
                        0
                    }
                }
            }
        }

        // `access(yol, mode)` -- yalnizca "var mi?" sorusu.
        //
        // `mode` yok sayilir ve bu bilincli: `R_OK`/`W_OK`/`X_OK` izin
        // bitlerini sorar, TCMKFS'te izin biti yok. Var olmayan bir
        // ayrimi varmis gibi cevaplamaktansa varligi bildiriyoruz --
        // `W_OK` icin dogru cevabi yine de veriyoruz, cunku RAMFS
        // gercekten yazilamaz.
        SYS_ACCESS => {
            let mut storage = [0u8; PATH_MAX];
            match stat_user_path(arg1, &mut storage) {
                None => -EFAULT,
                Some(Err(e)) => errno_of(e),
                // W_OK (2) istendiyse salt okunur bir yol icin EACCES.
                Some(Ok(info)) if arg2 & 2 != 0 && info.read_only => -EACCES,
                Some(Ok(_)) => 0,
            }
        }

        // `time(tloc)` -- 1970'ten beri gecen saniye.
        //
        // Bu cagriya kadar POSIX tarafinda **hicbir saat yoktu**: bir ELF
        // "saat kac?" diye soramiyordu, oysa ayni cekirdekte kosan bir PE
        // `NtQuerySystemTime` ile sorabiliyordu. Asimetri kaynakta degil
        // yalnizca ceviri katmanindaydi -- RTC surucusu bastan beri
        // oradaydi.
        SYS_TIME => {
            let now = crate::level0a::drivers::rtc::unix_time() as usize;
            if arg1 != 0 {
                if !write_user_word(arg1, now) {
                    return_errno(frame, -EFAULT);
                    return;
                }
            }
            frame.set_return(now);
            return;
        }

        // `clock_gettime(clk_id, timespec*)` -- saniye + nanosaniye.
        //
        // Iki saat var ve **farklari gercek**:
        //
        //   CLOCK_REALTIME   RTC'den; duvar saati, geri gidebilir
        //   CLOCK_MONOTONIC  PIT tikinden; acilistan beri, geri gitmez
        //
        // Sure olcen kod ikincisini kullanmali. TCMK'de cozunurluk 10 ms
        // (PIT 100 Hz), yani nanosaniye alani dolduruluyor ama o kadar
        // ince degil -- yalan soylememek icin burada yaziyor.
        //
        // `struct timespec` iki **kelime**: i386'da 8, x86_64'te 16 bayt.
        // Sabit bir boyut varsaymak, iki mimariden birinde yigini
        // tasardi.
        SYS_CLOCK_GETTIME => {
            let width = core::mem::size_of::<usize>();
            let (seconds, nanoseconds) = match arg1 {
                CLOCK_MONOTONIC => {
                    let ticks = crate::level0a::pit::ticks() as usize;
                    (ticks / 100, (ticks % 100) * 10_000_000)
                }
                _ => (crate::level0a::drivers::rtc::unix_time() as usize, 0usize),
            };
            if !write_user_word(arg2, seconds) || !write_user_word(arg2 + width, nanoseconds) {
                -EFAULT
            } else {
                0
            }
        }

        // `fsync(fd)` -- bekleyen yazmalari diske indirir.
        //
        // Kabugun `sync` komutu bunu zaten yapabiliyordu; eksik olan bir
        // **uygulamanin** ayni seyi isteyebilmesiydi. Bir metin
        // duzenleyicinin "kaydettim" demeden once cagirmasi gereken sey
        // budur.
        SYS_FSYNC => match kernel_api::fsync(arg1 as u32) {
            Ok(()) => 0,
            Err(e) => errno_of(e),
        },

        // `readv(fd, iovec[], count)` -- `writev`in esi.
        //
        // Ayni `struct iovec` dizisi, ters yon: her tampon sirayla
        // doldurulur ve **toplam** okunan doner. Kisa okuma normaldir --
        // dosya bitince dongu erken kesilir.
        SYS_READV => {
            let width = core::mem::size_of::<usize>();
            if arg3 > MAX_POLL_FDS {
                return_errno(frame, -EINVAL);
                return;
            }
            let mut total = 0usize;
            let mut failed = None;
            for i in 0..arg3 {
                let record = arg2 + i * 2 * width;
                if !mmu::is_user_accessible(record)
                    || !mmu::is_user_accessible(record + 2 * width - 1)
                {
                    failed = Some(-EFAULT);
                    break;
                }
                let base = unsafe { (record as *const usize).read_unaligned() };
                let len = unsafe { ((record + width) as *const usize).read_unaligned() };
                if len == 0 {
                    continue;
                }
                match unsafe { kernel_api::read(arg1 as u32, base as *mut u8, len) } {
                    Ok(read) => {
                        total += read;
                        // Kisa okuma: kaynak bitti, kalan tamponlari
                        // doldurmaya calismak bosuna.
                        if read < len {
                            break;
                        }
                    }
                    Err(e) => {
                        if total == 0 {
                            failed = Some(errno_of(e));
                        }
                        break;
                    }
                }
            }
            match failed {
                Some(e) => e,
                None => {
                    frame.set_return(total);
                    return;
                }
            }
        }

        // `getuid`/`geteuid`/`getgid`/`getegid` -- hepsi **0**.
        //
        // TCMK'de kullanici ve grup kavrami yok: tek bir ayricalik
        // duzeyi var (Ring 3) ve dosya sisteminde izin biti bulunmuyor.
        // Sifir dondurmek "root olarak kosuyorsun" demek ve bu **dogru**
        // cevap -- uydurma bir kullanici numarasi vermek, ayricalik
        // dususu yapmaya calisan bir programi yaniltirdi.
        //
        // Cagrilarin var olmasi yine de gerekli: gercek programlar
        // erkenden `geteuid` cagirir ve `ENOSYS` gormeyi beklemez.
        SYS_GETUID | SYS_GETEUID | SYS_GETGID | SYS_GETEGID => 0,

        // --- Is-parcacigi yerel deposu -------------------------------
        //
        // Ayni amac, iki mimaride **iki ayri cagri** -- ve isim farki
        // donanimdan geliyor:
        //
        //   i386     set_thread_area(user_desc*)  bir GDT tanimlayicisi
        //            ister; cekirdek bir girdi ayirir ve numarasini geri
        //            yazar, program da onu bir segment registerina yukler.
        //
        //   x86_64   arch_prctl(kod, adres)       long mode segmentasyonu
        //            kaldirdi; taban dogrudan bir MSR. Tanimlayici da yok,
        //            secici de.
        //
        // TCMK ikisini de gercek bicimleriyle destekliyor, cunku
        // derlenmis bir ikili hangi mimaride ise onu cagirir.
        #[cfg(target_arch = "x86")]
        SYS_SET_THREAD_AREA => {
            // `struct user_desc`: entry_number, base_addr, limit, ...
            // Yalnizca ilk iki alan okunuyor; limit ve bayraklar TCMK'de
            // sabit (duz 4 GiB, ring 3 verisi).
            //
            // `entry_number` -1 gelirse "sen bir tane ayir" demektir ve
            // cekirdek ayirdigini **geri yazar** -- program o numaradan
            // seciciyi hesaplar: (numara << 3) | 3.
            if !mmu::is_user_accessible(arg1) || !mmu::is_user_accessible(arg1 + 7) {
                -EFAULT
            } else {
                let base = unsafe { ((arg1 + 4) as *const u32).read_unaligned() } as usize;
                let task = crate::level0a::core::scheduler::current_id();
                // Linux'ta GS is-parcacigi registeridir (Windows'ta FS --
                // ayni mimaride, ters secim; bkz. `core::tls`).
                crate::level0a::core::tls::set_gs(task, base);
                unsafe {
                    (arg1 as *mut u32)
                        .write_unaligned(u32::from(crate::level0a::gdt::TLS_GS_SELECTOR >> 3));
                }
                0
            }
        }

        #[cfg(target_arch = "x86_64")]
        SYS_ARCH_PRCTL => {
            const ARCH_SET_FS: usize = 0x1002;
            const ARCH_GET_FS: usize = 0x1003;
            const ARCH_SET_GS: usize = 0x1001;
            const ARCH_GET_GS: usize = 0x1004;
            let task = crate::level0a::core::scheduler::current_id();
            match arg1 {
                ARCH_SET_FS => {
                    crate::level0a::core::tls::set_fs(task, arg2);
                    0
                }
                ARCH_SET_GS => {
                    crate::level0a::core::tls::set_gs(task, arg2);
                    0
                }
                ARCH_GET_FS | ARCH_GET_GS => {
                    let base = if arg1 == ARCH_GET_FS {
                        crate::level0a::core::tls::fs_of(task)
                    } else {
                        crate::level0a::core::tls::gs_of(task)
                    };
                    if write_user_word(arg2, base) {
                        0
                    } else {
                        -EFAULT
                    }
                }
                _ => -EINVAL,
            }
        }

        // `ftruncate(fd, uzunluk)` -- dosyayi verilen boya getirir.
        //
        // Buyutme yonu de destekleniyor ve POSIX orada buyuyen bolgenin
        // **sifir okunmasini** sart kosar. Yeni tahsis edilen bloklar
        // daha once baska bir dosyaya ait olabilir; sifirlanmadan
        // birakmak bir dosya sistemi hatasindan once bir **gizlilik**
        // hatasi olurdu (bkz. `tcmkfs::truncate`).
        SYS_FTRUNCATE => match kernel_api::truncate(arg1 as u32, arg2) {
            Ok(()) => 0,
            Err(e) => errno_of(e),
        },

        // `uname(buf)` -- sistemin kendini tanitmasi.
        //
        // Yapi **gercek `struct utsname`**: alti alan, her biri 65 bayt,
        // toplam 390. Kisaltmak cazipti ama olmazdi: bu yapi glibc
        // basliklarinda sabittir ve alanlara ofsetle erisilir, yani
        // sadelestirilmis bir kayit ikili uyumu bozardi. (`stat`te tam
        // tersini yaptik -- cunku orada alanlarin **karsiligi** yoktu;
        // burada karsiligi var, yalnizca degerler TCMK'nin.)
        SYS_UNAME => {
            const FIELD: usize = 65;
            const FIELDS: [&str; 6] = [
                "TCMK",
                "tcmk",
                // release: cekirdegin surumu.
                "0.1.0",
                "The Cursed Moon Kernel (Rust)",
                #[cfg(target_arch = "x86_64")]
                "x86_64",
                #[cfg(target_arch = "x86")]
                "i686",
                // domainname: Linux'un GNU genislemesi, bos.
                "(none)",
            ];
            if !mmu::is_user_accessible(arg1)
                || !mmu::is_user_accessible(arg1 + FIELD * FIELDS.len() - 1)
            {
                -EFAULT
            } else {
                for (i, text) in FIELDS.iter().enumerate() {
                    let base = arg1 + i * FIELD;
                    unsafe {
                        core::ptr::write_bytes(base as *mut u8, 0, FIELD);
                        let taken = text.len().min(FIELD - 1);
                        core::ptr::copy_nonoverlapping(
                            text.as_ptr(),
                            base as *mut u8,
                            taken,
                        );
                    }
                }
                0
            }
        }

        // `getdents(fd, buf, count)` -- acik bir dizinden girdi paketleri
        // okur; yazilan bayt sayisini, dizin bittiginde `0` doner.
        //
        // Bu cagriya kadar bir ELF dosya sistemini **goremiyordu**: adini
        // onceden bildigi bir dosyayi acabiliyordu ama "burada ne var?"
        // diye soramiyordu. Kayit bicimi ve gezinme cekirdegi
        // `kernel_api::getdents` icinde tanimli; Win32'nin
        // `FindFirstFileA`i da ayni koda baglanir.
        SYS_GETDENTS => match unsafe {
            kernel_api::getdents(arg1 as u32, arg2 as *mut u8, arg3)
        } {
            Ok(written) => {
                frame.set_return(written);
                return;
            }
            Err(e) => errno_of(e),
        },

        // Dosya sistemi **yazma** islemleri. Gezinme geldikten sonra
        // eksikligi gorunur oldu: bir uygulama artik dizinleri
        // dolasabiliyordu ama hicbir sey yaratamiyordu. Ucu de ayni
        // `kernel_api` girisine iner; Win32 tarafinda karsiliklari
        // `CreateDirectoryA`/`RemoveDirectoryA`/`DeleteFileA`.
        //
        // `mode` (arg2) yok sayilir: TCMKFS'te izin biti yok.
        SYS_MKDIR => match with_user_path(arg1, kernel_api::mkdir) {
            Ok(()) => 0,
            Err(e) => e,
        },

        SYS_RMDIR => match with_user_path(arg1, kernel_api::rmdir) {
            Ok(()) => 0,
            Err(e) => e,
        },

        SYS_UNLINK => match with_user_path(arg1, kernel_api::unlink) {
            Ok(()) => 0,
            Err(e) => e,
        },

        // `rename(eski, yeni)` -- iki yol argumani, tek islem.
        //
        // Veri bloklari tasinmaz: TCMKFS'te ad ve ebeveyn ayni inode
        // alaninda oldugu icin "yeniden adlandir" ile "tasi" ayni sey.
        // Win32 karsiligi `MoveFileA`.
        SYS_RENAME => {
            let mut old_storage = [0u8; PATH_MAX];
            let mut new_storage = [0u8; PATH_MAX];
            // Iki yol da **once** kopyalanir: islem basladiktan sonra
            // kullanici bellegine bir daha donulmez. Uzunluklari almak
            // ayni zamanda oduncu bitirir, yani iki tampon asagida
            // birlikte kullanilabilir.
            let old_len = unsafe { copy_user_cstr(arg1, &mut old_storage) }.map(|p| p.len());
            let new_len = unsafe { copy_user_cstr(arg2, &mut new_storage) }.map(|p| p.len());
            match (old_len, new_len) {
                (Some(o), Some(n)) => match (
                    core::str::from_utf8(&old_storage[..o]),
                    core::str::from_utf8(&new_storage[..n]),
                ) {
                    (Ok(old), Ok(new)) => match kernel_api::rename(old, new) {
                        Ok(()) => 0,
                        Err(e) => errno_of(e),
                    },
                    _ => -EFAULT,
                },
                _ => -EFAULT,
            }
        }

        // `pause()` -- teslim edilebilir bir sinyal gelene kadar uyur.
        //
        // Bu cagriya kadar bir surec sinyali bekleyemiyordu: tek yol
        // bayragi yoklayan bir donguydu, yani sinyal gelene kadar CPU
        // yakmak. POSIX geregi **her zaman** -EINTR doner; basarili bir
        // donusu yoktur, cunku donmesinin tek sebebi kesilmesidir.
        SYS_PAUSE => {
            signal::pause(scheduler::current_id());
            -EINTR
        }

        // `sigsuspend(mask)` -- maskeyi gecici degistirip bekler.
        //
        // `sigprocmask` + `pause` ikilisinden farki bolunmez olmasi:
        // ayri cagrilarda sinyal tam aradaki pencerede gelirse `pause`
        // onu kacirir ve surec sonsuza kadar uyur.
        //
        // Gercek Linux maskeyi **isaretciyle** alir; TCMK 32 bitlik
        // maskeyi dogrudan registerda tasiyor (bkz. `sigprocmask`).
        SYS_SIGSUSPEND => {
            signal::sigsuspend(scheduler::current_id(), arg1 as u32);
            -EINTR
        }

        // `chdir(yol)` -- surecin calisma dizinini degistirir.
        //
        // Bu cagriya kadar calisma dizini yalnizca **kabuga** aitti:
        // kabuk yolu cagirmadan once mutlaklastiriyor, uygulama her
        // zaman mutlak yol goruyordu. Artik her surecin kendi dizini
        // var; `fork` ile devrediliyor, `execve` ile korunuyor.
        SYS_CHDIR => match with_user_path(arg1, kernel_api::chdir) {
            Ok(()) => 0,
            Err(e) => e,
        },

        // `setenv(ad, deger)` -- surecin ortamini degistirir.
        //
        // Deger bos verilirse (ya da NULL) degisken **silinir**; POSIX'in
        // `unsetenv`i ve Win32'nin `SetEnvironmentVariableA(ad, NULL)`
        // cagrisi ayni anlama geliyor, o yuzden tek bir cagri yetiyor.
        //
        // Degisiklik yalnizca cagiran surecin tablosunda; kardesler
        // gormez, ama `fork` ile dogan cocuk ve `execve` ile yuklenen
        // yeni imaj gorur -- ikisi de ayni yuvadan devam ettigi icin.
        SYS_SETENV => {
            let mut name_storage = [0u8; PATH_MAX];
            let mut value_storage = [0u8; PATH_MAX];
            let name = unsafe { copy_user_cstr(arg1, &mut name_storage) }.map(|n| n.len());
            let value = if arg2 == 0 {
                Some(0)
            } else {
                unsafe { copy_user_cstr(arg2, &mut value_storage) }.map(|v| v.len())
            };
            match (name, value) {
                (Some(name_len), Some(value_len)) => {
                    let name = core::str::from_utf8(&name_storage[..name_len]).ok();
                    let value = core::str::from_utf8(&value_storage[..value_len]).ok();
                    match (name, value) {
                        (Some(name), Some(value)) => match kernel_api::setenv(name, value) {
                            Ok(()) => 0,
                            Err(e) => errno_of(e),
                        },
                        _ => -EFAULT,
                    }
                }
                _ => -EFAULT,
            }
        }

        // `getcwd(buf, size)` -- calisma dizinini kullaniciya yazar.
        //
        // Linux **uzunlugu** (sondaki NUL dahil) doner; tampon kucukse
        // `-ERANGE`. TCMK ayni sozlesmeyi kullaniyor.
        SYS_GETCWD => {
            let path = kernel_api::getcwd();
            // NUL icin bir bayt daha gerekiyor.
            let needed = path.len() + 1;
            if arg2 < needed {
                -ERANGE
            } else if !mmu::is_user_accessible(arg1)
                || !mmu::is_user_accessible(arg1 + needed - 1)
            {
                -EFAULT
            } else {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        path.as_ptr(),
                        arg1 as *mut u8,
                        path.len(),
                    );
                    (arg1 as *mut u8).add(path.len()).write(0);
                }
                frame.set_return(needed);
                return;
            }
        }

        SYS_SLEEP => {
            // arg1 = milisaniye. PIT 100 Hz oldugu icin cozunurluk 10 ms;
            // sifirdan buyuk her istek en az bir tik surer.
            //
            // SIFIR ayri ele alinir: "0 ms uyu" demek "bir tik uyu"
            // olmamali. Onceden oyleydi ve olcumu bozuyordu -- CPU'ya
            // bagli olmasi gereken bir dongu her karede uykuya gidiyor,
            // boylece butun gorevler ayni hizda uyanip oncelik farkini
            // gorunmez kiliyordu. NT tarafindaki `Sleep` zaten boyle
            // davraniyordu; iki alt sistem artik ayni.
            if arg1 == 0 {
                crate::level0a::core::scheduler::yield_now();
            } else {
                let ticks = ((arg1 as u32) / 10).max(1);
                crate::level0a::core::scheduler::sleep_ticks(ticks);
            }
            frame.set_return(0);
            return;
        }

        // `nanosleep(istek, kalan)` -- `SYS_SLEEP`in gercek Linux yuzu.
        //
        // Ayni yetenek, iki numara: TCMK'nin kendi cagrisi milisaniye
        // aliyor, Linux'unki bir `struct timespec` isaretcisi. Ikincisi
        // olmadan derlenmis bir Linux ikilisi uyuyamazdi.
        //
        // `kalan` (arg2) doldurulmuyor: yalnizca **sinyalle kesilen** bir
        // uykuda anlamli ve TCMK'nin uykusu kesilmiyor. Doldurmus gibi
        // yapmak, kalan sureyi kullanan bir donguyu yaniltirdi.
        SYS_NANOSLEEP => {
            let width = core::mem::size_of::<usize>();
            if !mmu::is_user_accessible(arg1) || !mmu::is_user_accessible(arg1 + 2 * width - 1) {
                -EFAULT
            } else {
                let seconds = unsafe { (arg1 as *const usize).read_unaligned() };
                let nanoseconds =
                    unsafe { ((arg1 + width) as *const usize).read_unaligned() };
                let ms = seconds * 1000 + nanoseconds / 1_000_000;
                if ms == 0 {
                    crate::level0a::core::scheduler::yield_now();
                } else {
                    crate::level0a::core::scheduler::sleep_ticks(((ms as u32) / 10).max(1));
                }
                0
            }
        }

        // `writev(fd, iovec[], count)` -- tek cagride birden cok tampon.
        //
        // glibc'nin stdio'su ciktisini bazi yollarda boyle bosaltir
        // (baslik + govde tek cagride). Desteklenmezse o yollar `ENOSYS`
        // gorur ve **hicbir sey yazilmaz** -- sessiz bir program.
        //
        // `struct iovec` iki kelime: taban + uzunluk. Atomiklik vaadi
        // yok; parcalar sirayla yaziliyor.
        SYS_WRITEV => {
            let width = core::mem::size_of::<usize>();
            let count = arg3;
            if count > MAX_POLL_FDS {
                return_errno(frame, -EINVAL);
                return;
            }
            let mut total = 0usize;
            let mut failed = None;
            for i in 0..count {
                let record = arg2 + i * 2 * width;
                if !mmu::is_user_accessible(record)
                    || !mmu::is_user_accessible(record + 2 * width - 1)
                {
                    failed = Some(-EFAULT);
                    break;
                }
                let base = unsafe { (record as *const usize).read_unaligned() };
                let len = unsafe { ((record + width) as *const usize).read_unaligned() };
                if len == 0 {
                    continue;
                }
                match unsafe { kernel_api::write(arg1 as u32, base as *const u8, len) } {
                    Ok(written) => total += written,
                    Err(e) => {
                        // Kismi yazma gerceklestiyse onu bildirmek dogru:
                        // POSIX de "yazilan kadarini don" der.
                        if total == 0 {
                            failed = Some(errno_of(e));
                        }
                        break;
                    }
                }
            }
            match failed {
                Some(e) => e,
                None => {
                    frame.set_return(total);
                    return;
                }
            }
        }

        SYS_GETPID => crate::level0a::core::scheduler::current_id() as i32,

        // `getppid()` -- ebeveynin kimligi.
        //
        // `fork`tan sonra cocugun "beni kim dogurdu" sorusunun cevabi.
        // Yuva geri kazanildigi icin ebeveyn artik yasamiyor olabilir;
        // cagri yine de kayitli degeri doner -- POSIX'te de oyle, orada
        // oksuz surecler init'e devredilir.
        SYS_GETPPID => {
            let me = crate::level0a::core::scheduler::current_id();
            crate::level0a::core::scheduler::parent_of(me) as i32
        }

        SYS_SETPRIORITY => {
            // setpriority(which, who, prio). `which` yalnizca PRIO_PROCESS
            // olabilir: surec gruplari ve kullanicilar TCMK'de yok.
            if arg1 != PRIO_PROCESS {
                -EINVAL
            } else {
                let task = if arg2 == 0 {
                    crate::level0a::core::scheduler::current_id()
                } else {
                    arg2
                };
                match crate::level0a::core::scheduler::set_nice(task, arg3 as i8) {
                    Ok(()) => 0,
                    Err(_) => -ESRCH,
                }
            }
        }

        SYS_GETPRIORITY => {
            if arg1 != PRIO_PROCESS {
                -EINVAL
            } else {
                let task = if arg2 == 0 {
                    crate::level0a::core::scheduler::current_id()
                } else {
                    arg2
                };
                // Linux sozlesmesi: cagri, -20..19 yerine 20-nice doner
                // (negatif deger hata sayilacagi icin). glibc bunu geri
                // cevirir; ayni kaliba uyuluyor.
                20 - crate::level0a::core::scheduler::nice_of(task) as i32
            }
        }

        SYS_KILL => {
            // arg1 = hedef gorev, arg2 = sinyal.
            //
            // POSIX'te `kill` "oldur" demek degildir; "sinyal gonder"
            // demektir. Oldurme, sinyalin **varsayilan** davranisidir.
            match signal::raise(arg1, arg2 as u32) {
                Ok(()) => 0,
                Err(signal::SignalError::NoSuchTask) => -ESRCH,
                Err(_) => -EINVAL,
            }
        }

        // `sigaction(signo, act, oldact)` -- isleyiciyi **bayraklariyla**
        // kaydeder.
        //
        // `signal(2)`den farki tam olarak bayraklar ve `sa_mask`: bir
        // isleyici artik kendi sinyali disinda baska sinyalleri de
        // engelleyebilir, ya da `SA_NODEFER` ile kendi sinyalini bile
        // engellemeyebilir.
        //
        // Yapi kullanici alanindan **isaretciyle** gelir, gercek
        // `rt_sigaction` gibi: dort kelime (isleyici, tramplen, bayrak,
        // maske). Registerlere sigdirmak icin sadelestirmek, bayrak
        // eklendikce yeniden bozulacak bir ABI demek olurdu.
        SYS_SIGACTION => {
            let task = crate::level0a::core::scheduler::current_id();
            let mut act = [0usize; SIGACTION_WORDS];
            if arg2 != 0 && !read_user_words(arg2, &mut act) {
                -EFAULT
            } else {
                // Eski yerlestirme once okunur: `oldact` istenmisse
                // degistirmeden onceki deger yazilmali.
                let previous = signal::handler_of(task, arg1 as u32);
                let result = if arg2 == 0 {
                    // `act` NULL: yalnizca sorgu.
                    Ok(previous)
                } else {
                    signal::set_handler(
                        task,
                        arg1 as u32,
                        act[0],
                        act[1],
                        act[2] as u32,
                        act[3] as u32,
                    )
                };
                match result {
                    Ok(old) => {
                        if arg3 != 0 && !write_user_word(arg3, old) {
                            -EFAULT
                        } else {
                            0
                        }
                    }
                    Err(_) => -EINVAL,
                }
            }
        }

        SYS_SIGNAL => {
            // arg1 = sinyal, arg2 = isleyici, arg3 = tramplen.
            //
            // Sadelestirilmis eski yuz: bayrak yok, `sa_mask` yok.
            // Kullanici tarafi artik `sigaction`i kullaniyor; bu yol
            // i386 Linux'un gercek `signal`(48) numarasi oldugu icin
            // duruyor.
            let task = crate::level0a::core::scheduler::current_id();
            match signal::set_handler(task, arg1 as u32, arg2, arg3, 0, 0) {
                Ok(previous) => previous as i32,
                Err(_) => -EINVAL,
            }
        }

        SYS_SIGRETURN => {
            // Baglam geri konur; `set_return` CAGRILMAZ, cunku donus
            // degeri de saklanan baglamin parcasidir. Kullanicinin
            // isleyiciye girmeden once aldigi EAX/RAX aynen geri gelir.
            if unsafe { signal::sigreturn(frame, from_interrupt) } {
                return;
            }
            -EINVAL
        }

        _ => {
            crate::println!(
                "[LEVEL-0b1] POSIX: desteklenmeyen syscall {} (-ENOSYS).",
                number
            );
            -ENOSYS
        }
    };

    frame.set_return(result as isize as usize);
}

/// Kullanici alanindaki NUL sonlandirmali diziyi cekirdek tamponuna kopyalar.
///
/// Isaretci **once dogrulanir**: yalnizca Ring 3'e acilmis sayfalardan
/// okunur. Boylece kotu niyetli bir kullanici programi cekirdek belleginden
/// veri sizdirmak icin `sys_open` kullanamaz.
///
/// # Safety
/// Cagirann `storage`'i gecerli bir tampon olarak vermesi gerekir.
unsafe fn copy_user_cstr(ptr: usize, storage: &mut [u8; PATH_MAX]) -> Option<&str> {
    if ptr == 0 {
        return None;
    }

    let mut len = 0usize;
    while len < PATH_MAX {
        let addr = ptr + len;

        // Kullaniciya ait olmayan bir sayfaya gecildiyse dur.
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
        return None; // NUL bulunamadi
    }

    core::str::from_utf8(&storage[..len]).ok()
}
