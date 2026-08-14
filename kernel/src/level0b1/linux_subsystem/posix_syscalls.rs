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
use crate::level0a::core::{fd, mmu};
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
    pub const SYS_BRK: u32 = 45;
    pub const SYS_GETPID: u32 = 20;
    pub const SYS_KILL: u32 = 37;
    /// i386 Linux'ta klasik `signal(2)`. TCMK ucuncu bir arguman ister
    /// (tramplen); gercek Linux'ta o deger `sigaction.sa_restorer`
    /// alanindan gelir -- yani fikir ayni, tasima yolu farkli.
    pub const SYS_SIGNAL: u32 = 48;
    pub const SYS_SIGRETURN: u32 = 119;
    pub const SYS_SIGPROCMASK: u32 = 126;
    pub const SYS_ALARM: u32 = 27;
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
    pub const SYS_BRK: u32 = 12;
    pub const SYS_PIPE: u32 = 22;
    pub const SYS_FORK: u32 = 57;
    pub const SYS_EXIT: u32 = 60;
    /// Linux x86_64'te `waitpid` yoktur; `wait4` onun yerine gecer.
    pub const SYS_WAITPID: u32 = 61;
    pub const SYS_GETPID: u32 = 39;
    pub const SYS_KILL: u32 = 62;
    /// x86_64'te klasik `signal` numarasi yoktur; yerini `rt_sigaction`
    /// alir. TCMK ayni yuvayi sadelestirilmis uc registerli bicimle
    /// kullanir (isleyici, tramplen) -- `sigaction` yapisi kopyalanmaz.
    pub const SYS_SIGNAL: u32 = 13;
    pub const SYS_SIGRETURN: u32 = 15;
    /// x86_64'te klasik `sigprocmask` yoktur; `rt_sigprocmask` gecer.
    pub const SYS_SIGPROCMASK: u32 = 14;
    pub const SYS_ALARM: u32 = 37;
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
    }
}

/// Level-0b2 dispatcher'i tarafindan cagrilir. Donus degerini dogrudan
/// frame'in EAX alanina yazar (i386 Linux ABI).
pub fn dispatch(frame: &mut SyscallFrame) {
    let number = frame.number();
    let [arg1, arg2, arg3, _, _] = frame.args();

    let result: i32 = match number {
        SYS_EXIT => {
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
                // arg2 = bayraklar; Linux'ta O_CREAT = 0o100 = 0x40.
                Some(path) => match kernel_api::open(path, arg2 & 0x40 != 0) {
                    Ok(fd) => fd as i32,
                    Err(e) => errno_of(e),
                },
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
            match unsafe { crate::level0b1::fork::fork(frame) } {
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
        SYS_YIELD => {
            crate::level0a::core::scheduler::yield_now();
            frame.set_return(0);
            return;
        }
        SYS_EXECVE => {
            // arg1 = yol. Imaj YERINDE degistirilemez (surec o anda kendi
            // kodunda kosuyor); istek kaydedilip Ring 3'ten cikilir,
            // launcher yeni imaji yukler.
            let mut storage = [0u8; PATH_MAX];
            let path = unsafe { copy_user_cstr(arg1, &mut storage) };
            match path {
                Some(p) if crate::level0a::core::vfs::lookup(p).is_some() => {
                    let task = crate::level0a::core::scheduler::current_id();
                    if crate::level0a::launcher::request_exec(task, p) {
                        unsafe { kernel_api::exit_to_exec() }
                    }
                    -EINVAL
                }
                Some(_) => -ENOENT,
                None => -EFAULT,
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

        SYS_GETPID => crate::level0a::core::scheduler::current_id() as i32,

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

        SYS_SIGNAL => {
            // arg1 = sinyal, arg2 = isleyici, arg3 = tramplen.
            let task = crate::level0a::core::scheduler::current_id();
            match signal::set_handler(task, arg1 as u32, arg2, arg3) {
                Ok(previous) => previous as i32,
                Err(_) => -EINVAL,
            }
        }

        SYS_SIGRETURN => {
            // Baglam geri konur; `set_return` CAGRILMAZ, cunku donus
            // degeri de saklanan baglamin parcasidir. Kullanicinin
            // isleyiciye girmeden once aldigi EAX/RAX aynen geri gelir.
            if unsafe { signal::sigreturn(frame) } {
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
