//! Level-0a'nin disariya actigi **ortak cekirdek API'si**.
//!
//! Doc S.2.2.B: Level-0b1'in POSIX ve NT cevirmenleri, kendi ABI'lerini bu
//! notr API'ye cevirir; Level-0a'nin altindaki suruculere dogrudan
//! dokunmazlar. Boylece ayni `read`/`write` yolunu hem Linux'un
//! `sys_read`'i hem Windows'un `NtReadFile`'i (Faz 7) paylasabilir.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::{fd, pipe, scheduler, vfs};

/// Standart POSIX tanimlayicilari.
pub const FD_STDIN: u32 = 0;
pub const FD_STDOUT: u32 = 1;
pub const FD_STDERR: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelError {
    BadFileDescriptor,
    Fault,
    NotFound,
    TooManyOpenFiles,
    NotSupported,
}

/// Program break (heap siniri) -- **surec basina**.
///
/// Uzun sure globaldi; tek kullanici sureci varken sorun degildi. `fork`
/// ve coklu surec geldikten sonra artik dogru degil: her surecin kendi
/// adres uzayi var, yani "heap nereye kadar buyudu" sorusunun cevabi da
/// surece ozeldir. Global birakilsaydi bir uygulamanin `malloc`'u
/// otekinin break'ini kaydirirdi -- fd tablosunda yasanan hatanin tipki
/// aynisi.
struct Break {
    current: AtomicUsize,
    /// Baslangic break'i: heap bunun altina inemez.
    start: AtomicUsize,
    limit: AtomicUsize,
}

impl Break {
    const fn new() -> Self {
        Break {
            current: AtomicUsize::new(0),
            start: AtomicUsize::new(0),
            limit: AtomicUsize::new(0),
        }
    }
}

static BREAKS: [Break; scheduler::MAX_TASKS] = [
    Break::new(), Break::new(), Break::new(), Break::new(),
    Break::new(), Break::new(), Break::new(), Break::new(),
];

fn current_break() -> &'static Break {
    &BREAKS[scheduler::current_id() % scheduler::MAX_TASKS]
}

pub fn set_program_break(start: usize, limit: usize) {
    let b = current_break();
    b.current.store(start, Ordering::Relaxed);
    b.start.store(start, Ordering::Relaxed);
    b.limit.store(limit, Ordering::Relaxed);
}

/// Ebeveynin break'ini cocuga kopyalar (`fork`).
///
/// Adres uzayi kopyalandigi icin degerler oldugu gibi gecerlidir: cocuk
/// ayni sanal adreslerde, ayni yere kadar buyumus bir heap gorur.
pub fn clone_program_break(child: usize) {
    if child >= scheduler::MAX_TASKS {
        return;
    }
    let parent = current_break();
    let target = &BREAKS[child];
    target.current.store(parent.current.load(Ordering::Relaxed), Ordering::Relaxed);
    target.start.store(parent.start.load(Ordering::Relaxed), Ordering::Relaxed);
    target.limit.store(parent.limit.load(Ordering::Relaxed), Ordering::Relaxed);
}

/// `sys_brk` semantigi: 0 verilirse mevcut break dondurulur; gecerli bir
/// adres verilirse break oraya tasinir. Basarisizlikta break DEGISMEZ ve
/// eski deger dondurulur (Linux davranisi).
pub fn brk(requested: usize) -> usize {
    let b = current_break();
    let current = b.current.load(Ordering::Relaxed);
    if requested == 0 {
        return current;
    }

    let floor = b.start.load(Ordering::Relaxed);
    let limit = b.limit.load(Ordering::Relaxed);

    if requested < floor || requested > limit {
        return current;
    }

    b.current.store(requested, Ordering::Relaxed);
    requested
}

/// `buf`'taki `len` bayti verilen tanimlayiciya yazar.
///
/// # Safety
/// `buf`/`len` cagiran tarafindan gecerli, okunabilir bir bolge olarak
/// garanti edilmelidir.
pub unsafe fn write(fd_num: u32, buf: *const u8, len: usize) -> Result<usize, KernelError> {
    if buf.is_null() {
        return Err(KernelError::Fault);
    }

    let bytes = core::slice::from_raw_parts(buf, len);

    // Once TABLOYA bakilir, sonra varsayilana dusulur. Sira boyle olmak
    // **zorunda**: `dup2(fd, 1)` stdout yuvasini doldurur ve ondan sonra
    // 1'e yazilanlar konsola degil oraya gitmelidir. Numaraya once bakan
    // eski sira yonlendirmeyi gorunmez kilardi.
    match fd::get(fd_num as usize) {
        Some(entry) => match entry.kind {
            fd::FdKind::PipeWrite => Ok(pipe::write(entry.node, bytes)),
            // Borunun okuma ucuna yazmak POSIX'te de hatadir.
            fd::FdKind::PipeRead => Err(KernelError::BadFileDescriptor),
            fd::FdKind::File => {
                let written = vfs::write_at(entry.node, entry.offset, bytes)
                    .map_err(|_| KernelError::NotSupported)?;
                fd::advance(fd_num as usize, written);
                Ok(written)
            }
        },
        // Yonlendirilmemis stdout/stderr: konsol.
        None if fd_num == FD_STDOUT || fd_num == FD_STDERR => {
            for &byte in bytes {
                crate::print!("{}", byte as char);
            }
            Ok(len)
        }
        None => Err(KernelError::BadFileDescriptor),
    }
}

/// Yola gore dosya acar ve yeni bir tanimlayici dondurur.
///
/// `create` verilirse (POSIX `O_CREAT`) dosya yoksa kalici dosya
/// sisteminde olusturulur. RAMFS'te olusturma yoktur: icerigi cekirdek
/// imajinin icindedir.
pub fn open(path: &str, create: bool) -> Result<usize, KernelError> {
    let node = match vfs::lookup(path) {
        Some(n) => n,
        None if create => vfs::create_file(path).map_err(|_| KernelError::NotFound)?,
        None => return Err(KernelError::NotFound),
    };
    fd::allocate(node).ok_or(KernelError::TooManyOpenFiles)
}

/// Acik bir tanimlayicidan okur; okunan bayt sayisini dondurur.
///
/// # Safety
/// `buf`/`len` gecerli, yazilabilir bir bolge olmalidir.
pub unsafe fn read(fd_num: u32, buf: *mut u8, len: usize) -> Result<usize, KernelError> {
    if buf.is_null() {
        return Err(KernelError::Fault);
    }
    let slice = core::slice::from_raw_parts_mut(buf, len);

    // Yazmada oldugu gibi once tabloya bakilir: `dup2(boru, 0)` diyen bir
    // surec stdin'i borudan okumalidir, klavyeden degil.
    match fd::get(fd_num as usize) {
        Some(entry) => match entry.kind {
            // Boru okumasi bloke ETMEZ: veri yoksa 0 doner. Bloke olan bir
            // surec penceresini de dondururdu (bkz. `core::pipe`).
            fd::FdKind::PipeRead => Ok(pipe::read(entry.node, slice)),
            fd::FdKind::PipeWrite => Err(KernelError::BadFileDescriptor),
            fd::FdKind::File => {
                let n = vfs::read(entry.node, entry.offset, slice)
                    .ok_or(KernelError::BadFileDescriptor)?;
                fd::advance(fd_num as usize, n);
                Ok(n)
            }
        },
        // Yonlendirilmemis stdin = cagiran surecin penceresinin tus kuyrugu.
        //
        // POSIX'te klavye bir dosya tanimlayicisidir, pencere kimligi degil;
        // bu yuzden `read(0, ...)` surecin kendi penceresine baglanir. Bir
        // GUI uygulamasi ayni tuslara `win_poll_key` ile de ulasabilir --
        // ikisi ayni kuyrugu okur, yani hangisi once cagirirsa tusu o alir.
        //
        // Okuma **bloke etmez**: tus yoksa 0 doner.
        None if fd_num == FD_STDIN => {
            let owner = scheduler::current_id();
            let window = match crate::level0a::wm::first_window_of(owner) {
                Some(w) => w,
                None => return Ok(0),
            };
            let mut read = 0usize;
            while read < slice.len() {
                let key = crate::level0a::gui_api::poll_key(window);
                if key == 0 {
                    break;
                }
                slice[read] = key;
                read += 1;
            }
            Ok(read)
        }
        None => Err(KernelError::BadFileDescriptor),
    }
}

/// Yeni bir boru acar; `(okuma_fd, yazma_fd)` dondurur.
///
/// POSIX `pipe(int fd[2])` ile ayni anlam, farkli tasima: iki
/// tanimlayici tek bir kelimede paketlenir (`okuma << 16 | yazma`),
/// boylece cagri kullanici bellegine yazmak zorunda kalmaz.
pub fn create_pipe() -> Result<(usize, usize), KernelError> {
    let index = pipe::create().ok_or(KernelError::TooManyOpenFiles)?;

    let read_fd = match fd::allocate_pipe(index, fd::FdKind::PipeRead) {
        Some(f) => f,
        None => {
            pipe::close_end(index, false);
            pipe::close_end(index, true);
            return Err(KernelError::TooManyOpenFiles);
        }
    };
    let write_fd = match fd::allocate_pipe(index, fd::FdKind::PipeWrite) {
        Some(f) => f,
        None => {
            let _ = fd::close(read_fd);
            pipe::close_end(index, true);
            return Err(KernelError::TooManyOpenFiles);
        }
    };

    Ok((read_fd, write_fd))
}

pub fn close(fd_num: u32) -> Result<(), KernelError> {
    if fd::close(fd_num as usize) {
        Ok(())
    } else {
        Err(KernelError::BadFileDescriptor)
    }
}

/// `execve` icin Ring 3'ten cikar: gorev **sonlanmaz**.
///
/// Cikis yolu `sys_exit` ile aynidir (saklanmis cekirdek baglami geri
/// yuklenir); fark, `launcher`'in donguye devam edip yeni imaji
/// yuklemesidir. Surecin eski adres uzayi bu sirada birakilir, yenisi
/// sifirdan kurulur -- execve'nin zaten istedigi sey.
///
/// # Safety
/// Yalnizca Ring 3 baglamindan, `launcher::request_exec` basarili
/// olduktan sonra cagrilmalidir.
pub unsafe fn exit_to_exec() -> ! {
    crate::arch::cpu::usermode::leave_user_mode()
}

/// Calisan gorevi/sureci sonlandirir (doc S.6: sys_exit).
///
/// Iki farkli baglam vardir:
///   - **Ring 3 sureci**: kullanici kendi yigininda, cekirdek ise TSS.esp0
///     yiginindadir. Gorev degistirme yapilamaz; saklanmis cekirdek baglami
///     geri yuklenerek `run_user_program`'in cagrildigi yere donulur.
///   - **Ring 0 cekirdek gorevi**: normal scheduler sonlandirmasi.
pub fn exit_current_task(code: u32) -> ! {
    // Kodu ONCE sakla: `waitpid` ile bekleyen ebeveyn bunu okuyacak ve
    // gorev `Terminated` olur olmaz uyanabilir.
    crate::level0a::core::scheduler::set_current_exit_code(code);

    crate::level0b2::ipc::post(
        crate::level0b2::ipc::Kind::AppExit,
        crate::level0a::core::scheduler::current_id(),
        code as usize,
        0,
        crate::level0a::core::scheduler::current_name(),
    );

    if crate::arch::cpu::usermode::in_user_mode() {
        crate::println!("[LEVEL-0a] Ring 3 sureci cikis kodu {} ile sonlandi.", code);
        unsafe { crate::arch::cpu::usermode::leave_user_mode() }
    }

    crate::println!(
        "[LEVEL-0a] gorev '{}' cikis kodu {} ile sonlandi.",
        crate::level0a::core::scheduler::current_name(),
        code
    );
    crate::level0a::core::scheduler::terminate_current()
}
