//! Level-0a'nin disariya actigi **ortak cekirdek API'si**.
//!
//! Doc S.2.2.B: Level-0b1'in POSIX ve NT cevirmenleri, kendi ABI'lerini bu
//! notr API'ye cevirir; Level-0a'nin altindaki suruculere dogrudan
//! dokunmazlar. Boylece ayni `read`/`write` yolunu hem Linux'un
//! `sys_read`'i hem Windows'un `NtReadFile`'i (Faz 7) paylasabilir.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::{cwd, fd, pipe, scheduler, tcmkfs, vfs};

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
    /// Ayni adda bir sey zaten var (POSIX `EEXIST`).
    AlreadyExists,
    /// Silinmek istenen dizin bos degil (POSIX `ENOTEMPTY`).
    NotEmpty,
    /// Kalici depolama yok ya da dolu (POSIX `ENOSPC`).
    NoSpace,
    /// Hedef salt okunur bir arka ucta (RAMFS) -- POSIX `EROFS`.
    ///
    /// "Yok" ile ayni sey **degil**: dosya duruyor, yalnizca cekirdek
    /// imajinin parcasi oldugu icin degistirilemiyor. Ilk olcumde bu
    /// durum `NotFound` olarak bildiriliyordu ve gezgin "bulunamadi"
    /// diyordu -- ekranda duran bir dosya icin yaniltici bir cevap.
    ReadOnly,
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

/// Elle sekiz kez yazilmisti; `MAX_TASKS` degisince derleme kirildi.
/// Sabit tekrar bicimi tablo boyutuna bagli kalmaz.
#[allow(clippy::declare_interior_mutable_const)]
const NEW_BREAK: Break = Break::new();
static BREAKS: [Break; scheduler::MAX_TASKS] = [NEW_BREAK; scheduler::MAX_TASKS];

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
            // Dizine yazmak da oyle: icerigi dosya sistemi belirler.
            fd::FdKind::Dir => Err(KernelError::BadFileDescriptor),
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

// --- Yol cozumu -------------------------------------------------------
//
// Yol alan **butun** cagrilar once buradan gecer. Iki isi birden yapar:
// goreli yolu surecin calisma dizinine gore mutlaklastirir, ve `.`/`..`
// bilesenlerini sadelestirir.
//
// Once yalnizca kabuk yapiyordu; Ring 3 uygulamalari mutlak yol vermek
// zorundaydi. `chdir` geldikten sonra cozumun cekirdekte olmasi sart --
// yoksa "hangi dizindeyim" sorusunun cevabi cagriya gore degisirdi.

/// Cozulen yolun tutuldugu tampon boyu.
const PATH_MAX: usize = cwd::PATH_MAX;

/// `path`i surecin calisma dizinine gore mutlaklastirir.
///
/// Mutlak yollarda da cagrilir: sadelestirme orada da gerekli, cunku
/// RAMFS duz bir isim tablosudur ve `/./bin/x` ile `/bin/x`i ayni
/// saymaz.
fn resolve<'a>(path: &str, buf: &'a mut [u8; PATH_MAX]) -> Option<&'a str> {
    cwd::resolve(path, buf)
}

/// POSIX `chdir`: surecin calisma dizinini degistirir.
///
/// Hedefin gercekten bir dizin oldugu **burada** sinanir; `cwd::set`
/// yalnizca depolamadir. Var olmayan bir dizine gecmek POSIX'te de
/// hatadir (`ENOENT`).
pub fn chdir(path: &str) -> Result<(), KernelError> {
    let mut buf = [0u8; PATH_MAX];
    let target = resolve(path, &mut buf).ok_or(KernelError::NotFound)?;
    if !is_dir_path(target) {
        return Err(KernelError::NotFound);
    }
    if cwd::set(scheduler::current_id(), target) {
        Ok(())
    } else {
        Err(KernelError::NotSupported)
    }
}

/// POSIX `getcwd`: surecin calisma dizini.
pub fn getcwd() -> &'static str {
    cwd::current()
}

/// Yola gore dosya acar ve yeni bir tanimlayici dondurur.
///
/// `create` verilirse (POSIX `O_CREAT`) dosya yoksa kalici dosya
/// sisteminde olusturulur. RAMFS'te olusturma yoktur: icerigi cekirdek
/// imajinin icindedir.
///
/// Yol bir **dizinse** dizin tanimlayicisi doner (bkz. `open_dir`).
/// POSIX'te de boyledir: `open` dizinlerde calisir, ayrim `read` ile
/// `getdents` arasindadir.
pub fn open(path: &str, create: bool) -> Result<usize, KernelError> {
    let mut buf = [0u8; PATH_MAX];
    let path = resolve(path, &mut buf).ok_or(KernelError::NotFound)?;

    // Once dizin sinanir: dosya aramasi bir dizin yolunda zaten
    // basarisiz olur ve `create` verilmisse ayni adda bir **dosya**
    // olusturmaya kalkardi.
    if vfs::lookup(path).is_none() && is_dir_path(path) {
        return open_dir(path);
    }
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
            // POSIX EISDIR: bir dizin `read` ile okunmaz, `getdents` ile
            // okunur. Ham bayt dondurmek, dizin bicimini ABI'ye kacak
            // yoldan sizdirmak olurdu.
            fd::FdKind::Dir => Err(KernelError::NotSupported),
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

// --- `poll(2)` olay bitleri (Linux ile ayni sayilar) ---
/// Okunacak veri var (ya da dosya sonu).
pub const POLLIN: u16 = 0x001;
/// Yazilabilir: boruda yer var.
pub const POLLOUT: u16 = 0x004;
/// Hata: borunun okuyan ucu kalmadi (POSIX'te SIGPIPE'in sessiz hali).
pub const POLLERR: u16 = 0x008;
/// Karsi taraf kapandi: yazan uc kalmadi, artik veri gelmeyecek.
pub const POLLHUP: u16 = 0x010;
/// Boyle bir tanimlayici yok.
pub const POLLNVAL: u16 = 0x020;

/// Bir tanimlayicinin **su anki** hazirlik durumu.
///
/// `poll`'un tek gercek isi budur; gerisi (dongu, zaman asimi, kullanici
/// bellegine yazma) cevre isidir. Kural her tur icin ayri:
///
/// | tanimlayici | hazir sayilma kosulu |
/// |---|---|
/// | dosya | her zaman (yerel dosya okumasi beklemez) |
/// | boru okuma ucu | bekleyen bayt varsa `POLLIN`; yazan uc bittiyse `POLLHUP` |
/// | boru yazma ucu | tamponda yer varsa `POLLOUT`; okuyan kalmadiysa `POLLERR` |
/// | yonlendirilmemis stdin | pencerede bekleyen tus varsa `POLLIN` |
/// | yonlendirilmemis stdout/stderr | her zaman `POLLOUT` (konsol) |
///
/// Tus kuyruguna **bakilir, tuketilmez**: `poll` veriyi yeseydi
/// arkasindan gelen `read` bos donerdi.
pub fn readiness(fd_num: u32) -> u16 {
    match fd::get(fd_num as usize) {
        Some(entry) => match entry.kind {
            fd::FdKind::File => POLLIN | POLLOUT,
            // Dizin her zaman "okunabilir"dir: `getdents` bloke etmez,
            // bitmisse sifir doner.
            fd::FdKind::Dir => POLLIN,
            fd::FdKind::PipeRead => match pipe::info(entry.node) {
                Some((pending, writers, _)) => {
                    let mut mask = 0;
                    if pending > 0 {
                        mask |= POLLIN;
                    }
                    // Yazan uc kalmadi: bu "dosya sonu"dur. POLLHUP ile
                    // bildirilir ve tamponda kalan veri yine POLLIN ile
                    // okunabilir -- ikisi ayni anda gorunebilir.
                    if writers == 0 {
                        mask |= POLLHUP;
                    }
                    mask
                }
                None => POLLNVAL,
            },
            fd::FdKind::PipeWrite => match pipe::info(entry.node) {
                Some((pending, _, readers)) => {
                    if readers == 0 {
                        POLLERR
                    } else if pending < pipe::PIPE_CAPACITY {
                        POLLOUT
                    } else {
                        0
                    }
                }
                None => POLLNVAL,
            },
        },
        None if fd_num == FD_STDIN => {
            let owner = scheduler::current_id();
            match crate::level0a::wm::first_window_of(owner) {
                Some(window) if crate::level0a::gui_api::has_key(window) => POLLIN,
                _ => 0,
            }
        }
        None if fd_num == FD_STDOUT || fd_num == FD_STDERR => POLLOUT,
        None => POLLNVAL,
    }
}

/// `lseek` bicimleri (Linux ile ayni sayilar).
pub const SEEK_SET: usize = 0;
pub const SEEK_CUR: usize = 1;
pub const SEEK_END: usize = 2;

/// POSIX `lseek`: dosya konumunu tasir, **yeni konumu** doner.
///
/// Bu cagriya kadar dosyalar yalnizca **bastan sona** okunabiliyordu:
/// konum yalnizca `read`/`write` tarafindan ilerletiliyor, geri
/// alinamiyordu. Bir dosyanin ortasindan okumak, basa sarip yeniden
/// okumak ya da sonuna eklemek mumkun degildi.
///
/// Bilerek: negatif goreli kaydirma desteklenmiyor. Cagri arayuzu
/// isaretsiz kelime tasidigi icin `SEEK_CUR`/`SEEK_END` yalnizca **ileri**
/// ya da sifir olabilir; `SEEK_END` ile sifir vermek dosya sonuna gitmek
/// demektir ki "ekleme" kalibinin ihtiyaci da budur.
pub fn lseek(fd_num: u32, offset: usize, whence: usize) -> Result<usize, KernelError> {
    let entry = fd::get(fd_num as usize).ok_or(KernelError::BadFileDescriptor)?;
    if entry.kind != fd::FdKind::File {
        // Boru bir akistir; konumu yoktur (POSIX de ESPIPE doner).
        return Err(KernelError::NotSupported);
    }
    let base = match whence {
        SEEK_SET => 0,
        SEEK_CUR => entry.offset,
        SEEK_END => vfs::size(entry.node).ok_or(KernelError::BadFileDescriptor)?,
        _ => return Err(KernelError::NotSupported),
    };
    let target = base.saturating_add(offset);
    if !fd::seek(fd_num as usize, target) {
        return Err(KernelError::BadFileDescriptor);
    }
    Ok(target)
}

/// Acik bir tanimlayicinin dosya boyutu.
///
/// `fstat`in TCMK'deki karsiligi. Gercek `struct stat` yerine tek bir
/// sayi donuyor: izin, sahiplik, aygit numarasi gibi alanlarin hicbiri
/// bu dosya sisteminde yok, yani yapiyi kopyalamak sifir dolu bir kayit
/// tasimak olurdu.
pub fn file_size(fd_num: u32) -> Result<usize, KernelError> {
    let entry = fd::get(fd_num as usize).ok_or(KernelError::BadFileDescriptor)?;
    if entry.kind != fd::FdKind::File {
        return Err(KernelError::NotSupported);
    }
    vfs::size(entry.node).ok_or(KernelError::BadFileDescriptor)
}

// --- Dizin gezinmesi ---------------------------------------------------
//
// Bu cagriya kadar bir uygulama dosya sistemini **goremiyordu**: adini
// onceden bildigi bir dosyayi acabiliyor, ama "burada ne var?" diye
// soramiyordu. Dizin listesini yalnizca kabugun `ls` komutu biliyordu ve
// o da listeyi kendi icinde, iki ayri dongude uretiyordu.
//
// ## Tek kaynak, iki ABI
//
// Gezinme burada **bir kez** yazilir; POSIX tarafi `getdents`, Win32
// tarafi `FindFirstFileA`/`FindNextFileA` olarak ayni koda baglanir.
// Ikisinin de gordugu liste bu yuzden ayni -- bir ELF ve bir PE ayni
// dizini listeleyip farkli sonuc alamaz.
//
// ## Neden imlec (cursor) uzayi?
//
// TCMK'de "dizin icerigi" tek bir tabloda durmaz, uc kaynaktan toplanir:
//
//   1. TCMKFS'in alt dizinleri (diskteki inode agaci),
//   2. RAMFS yollarinin **ima ettigi** dizinler (`/bin` gibi: karsiligi
//      olan bir inode yoktur, yalnizca `/bin/paint` yolunun icinde gecer),
//   3. VFS dosyalari (hem RAMFS hem TCMKFS dosyalari burada birlesir).
//
// Uc kaynak tek bir sayi uzayina dizilir; imlec hangi bolgede oldugumuzu
// da tasir. Boylece cagri **durumsuzdur**: cekirdek gezinmenin yarisini
// bir yerde tutmaz, imleci cagiran (tanimlayicinin `offset` alaninda)
// tasir.

/// Girdi turu: dosya.
pub const DIR_KIND_FILE: u8 = 1;
/// Girdi turu: dizin.
pub const DIR_KIND_DIR: u8 = 2;

/// Bir girdi adinin en fazla uzunlugu (TCMKFS ile ayni sinir).
pub const MAX_DIR_NAME: usize = tcmkfs::MAX_NAME;

/// Imlec uzayinin ikinci bolgesi: RAMFS'in ima ettigi dizinler.
const CURSOR_IMPLIED: usize = tcmkfs::MAX_INODES;
/// Ucuncu bolge: VFS dosyalari.
const CURSOR_FILES: usize = CURSOR_IMPLIED + vfs::MAX_NODES;

/// Tek bir dizin girdisi -- kaynagi ne olursa olsun ayni bicimde.
#[derive(Clone, Copy)]
pub struct DirEntry {
    pub name: [u8; MAX_DIR_NAME],
    pub name_len: usize,
    pub size: usize,
    /// Son degistirilme zamani (Unix epoch); bilinmiyorsa 0.
    ///
    /// RAMFS dosyalarinda **her zaman** 0'dir: icerikleri cekirdek
    /// imajiyla gelir, dosya sisteminde bir zamanlari yoktur.
    pub mtime: u32,
    pub kind: u8,
}

impl DirEntry {
    fn new(name: &str, size: usize, mtime: u32, kind: u8) -> Self {
        let mut entry = DirEntry {
            name: [0; MAX_DIR_NAME],
            name_len: name.len().min(MAX_DIR_NAME),
            size,
            mtime,
            kind,
        };
        entry.name[..entry.name_len].copy_from_slice(&name.as_bytes()[..entry.name_len]);
        entry
    }

    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }
}

/// `path`in `prefix` dizinine gore kalani (bastaki `/` atilmis olarak).
///
/// `prefix` bir ust dizin degilse `None`. Ayirici sarti onemli:
/// `/notlar` onekinin `/notlar2/x` yolunu yutmasini engeller.
fn under<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() || prefix == "/" {
        return path.strip_prefix('/');
    }
    path.strip_prefix(prefix)?.strip_prefix('/')
}

/// `path` dogrudan `prefix` dizininin **icinde** mi (alt dizinlerde degil)?
///
/// Bir dizini listelemek "altindaki her sey" degil, "icindekiler"
/// demektir; ayrimi yapmayan bir liste `/` icin butun agaci dokerdi.
pub fn is_direct_child(path: &str, prefix: &str) -> bool {
    under(path, prefix).is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
}

/// `path`, `prefix` altindaki bir alt dizinin icindeyse o alt dizinin adi.
///
/// `/bin/paint` ile `prefix = "/"` icin `"bin"` doner. RAMFS'te dizin
/// diye bir kayit yoktur -- dizinler yalnizca yol adlarinin icinde
/// **ima edilir**, ve `ls /` icin gorunmeleri gereken sey de budur.
fn implied_dir<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = under(path, prefix)?;
    let slash = rest.find('/')?;
    if slash == 0 {
        return None;
    }
    Some(&rest[..slash])
}

/// Verilen yol bir dizin mi?
///
/// Uc kosuldan biri yeter: kok olmasi, TCMKFS'te dizin inode'u olmasi,
/// ya da altinda en az bir VFS dosyasi bulunmasi (`/bin` boyledir).
pub fn is_dir_path(path: &str) -> bool {
    if path.is_empty() || path == "/" {
        return true;
    }
    if tcmkfs::mounted()
        && tcmkfs::resolve(path).and_then(tcmkfs::entry_kind) == Some(tcmkfs::KIND_DIR)
    {
        return true;
    }
    (0..vfs::node_count())
        .filter_map(vfs::path_of)
        .any(|p| under(p, path).is_some_and(|rest| !rest.is_empty()))
}

/// Imlecten sonraki ilk girdiyi ve **bir sonraki** imleci dondurur.
///
/// Dizin bittiginde `None`. Imlec bolgeler arasinda kendiliginden gecer,
/// yani cagiran yalnizca dondurulen sayiyi geri vermekle yukumludur.
pub fn next_entry(dir: &str, cursor: usize) -> Option<(DirEntry, usize)> {
    let dir = if dir.is_empty() { "/" } else { dir };

    // 1. bolge: diskteki alt dizinler.
    if cursor < CURSOR_IMPLIED && tcmkfs::mounted() {
        if let Some(parent) = tcmkfs::resolve(dir) {
            let mut buf = [0u8; tcmkfs::PATH_MAX];
            for i in cursor..CURSOR_IMPLIED {
                if i == tcmkfs::ROOT_INODE
                    || tcmkfs::entry_kind(i) != Some(tcmkfs::KIND_DIR)
                    || tcmkfs::parent_of(i) != Some(parent)
                {
                    continue;
                }
                if let Some(path) = tcmkfs::path_of(i, &mut buf) {
                    let name = path.rsplit('/').next().unwrap_or(path);
                    let mtime = tcmkfs::entry_mtime(i).unwrap_or(0);
                    return Some((DirEntry::new(name, 0, mtime, DIR_KIND_DIR), i + 1));
                }
            }
        }
    }

    // 2. bolge: RAMFS yollarinin ima ettigi dizinler.
    let disk_parent = if tcmkfs::mounted() {
        tcmkfs::resolve(dir)
    } else {
        None
    };
    let start = cursor.saturating_sub(CURSOR_IMPLIED).min(vfs::MAX_NODES);
    if cursor < CURSOR_FILES {
        for i in start..vfs::node_count() {
            let name = match vfs::path_of(i).and_then(|p| implied_dir(p, dir)) {
                Some(n) => n,
                None => continue,
            };
            // Ayni ad daha once ima edilmisse iki kez listelenmesin.
            let seen = (0..i)
                .filter_map(vfs::path_of)
                .filter_map(|p| implied_dir(p, dir))
                .any(|earlier| earlier == name);
            // Diskte gercek bir dizin olarak varsa 1. bolge onu zaten verdi.
            let on_disk = disk_parent
                .and_then(|parent| tcmkfs::child_of(parent, name))
                .is_some();
            if seen || on_disk {
                continue;
            }
            return Some((
                DirEntry::new(name, 0, 0, DIR_KIND_DIR),
                CURSOR_IMPLIED + i + 1,
            ));
        }
    }

    // 3. bolge: dosyalar.
    let start = cursor.saturating_sub(CURSOR_FILES);
    for i in start..vfs::node_count() {
        let path = match vfs::path_of(i) {
            Some(p) => p,
            None => continue,
        };
        if !is_direct_child(path, dir) {
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let size = vfs::size(i).unwrap_or(0);
        let mtime = vfs::mtime(i).unwrap_or(0);
        return Some((
            DirEntry::new(name, size, mtime, DIR_KIND_FILE),
            CURSOR_FILES + i + 1,
        ));
    }

    None
}

/// TCMKFS hatalarini notr cekirdek hatalarina cevirir.
///
/// Cevrim burada, **tek yerde** yapilir: iki alt sistem de kendi errno /
/// NTSTATUS esleme tablosunu bu turden turetir. Dosya sistemine ozgu
/// hata adlarinin ABI katmanlarina sizmasi, Level-0a ile Level-0b1
/// arasindaki siniri delerdi.
fn fs_error(err: tcmkfs::FsError) -> KernelError {
    match err {
        tcmkfs::FsError::Exists => KernelError::AlreadyExists,
        tcmkfs::FsError::NotEmpty => KernelError::NotEmpty,
        tcmkfs::FsError::NotFound | tcmkfs::FsError::BadPath => KernelError::NotFound,
        tcmkfs::FsError::Full | tcmkfs::FsError::TooManyFiles | tcmkfs::FsError::FileTooLarge => {
            KernelError::NoSpace
        }
        // Disk yoksa ya da bagli degilse yazma islemi hic mumkun degil.
        tcmkfs::FsError::NoDevice
        | tcmkfs::FsError::NoPartition
        | tcmkfs::FsError::NotFormatted
        | tcmkfs::FsError::NotMounted => KernelError::NoSpace,
        _ => KernelError::NotSupported,
    }
}

/// Yeni bir dizin olusturur (POSIX `mkdir`, Win32 `CreateDirectoryA`).
///
/// Ust dizin **onceden var olmali**: `mkdir -p` gibi ara dizinleri
/// kendiliginden olusturmaz. POSIX `mkdir(2)` de boyledir; `-p`
/// kabugun isidir, cekirdegin degil.
pub fn mkdir(path: &str) -> Result<(), KernelError> {
    let mut buf = [0u8; PATH_MAX];
    let path = resolve(path, &mut buf).ok_or(KernelError::NotFound)?;
    // RAMFS'te olusturma yoktur (icerigi cekirdek imajinin icinde), ve
    // ayni adda ima edilen bir dizin varsa cakisma bildirmek gerekir --
    // yoksa cagri "basarili" der ama ortada yeni bir sey olmaz.
    if is_dir_path(path) || vfs::lookup(path).is_some() {
        return Err(KernelError::AlreadyExists);
    }
    vfs::mkdir(path).map_err(fs_error)
}

/// Bos bir dizini siler (POSIX `rmdir`, Win32 `RemoveDirectoryA`).
pub fn rmdir(path: &str) -> Result<(), KernelError> {
    let mut buf = [0u8; PATH_MAX];
    let path = resolve(path, &mut buf).ok_or(KernelError::NotFound)?;
    vfs::rmdir(path).map_err(fs_error)
}

/// Bir dosyayi siler (POSIX `unlink`, Win32 `DeleteFileA`).
///
/// Yalnizca diskteki dosyalar silinebilir: RAMFS dosyalari cekirdek
/// imajinin parcasidir, `/bin/paint`i silmek imajin kendisini
/// degistirmek anlamina gelirdi.
pub fn unlink(path: &str) -> Result<(), KernelError> {
    let mut buf = [0u8; PATH_MAX];
    let path = resolve(path, &mut buf).ok_or(KernelError::NotFound)?;
    if let Some(node) = vfs::lookup(path) {
        if vfs::source(node) == Some(vfs::Source::Ram) {
            return Err(KernelError::ReadOnly);
        }
    }
    vfs::remove_file(path).map_err(fs_error)
}

/// Bir dosyayi/dizini yeniden adlandirir ya da tasir.
///
/// POSIX `rename`, Win32 `MoveFileA`. Veri bloklari tasinmaz: TCMKFS'te
/// ad ve ebeveyn ayni inode alaninda oldugu icin islem tek bir alan
/// degisikligidir.
///
/// ## Ayrisan yan: hedef zaten varsa
///
/// POSIX `rename` hedefin **uzerine sessizce yazar** (ayni turdeyse).
/// TCMK Win32'nin `MoveFileA` davranisini secti: hedef varsa cagri
/// basarisiz olur. Sebep, sessiz veri kaybini bir ABI ayrintisina
/// birakmamak -- "tasidim" diyen bir cagrinin ayni anda "sildim"
/// demesi, iki uygulamanin ayni dizinde calistigi bu sistemde
/// gorulmesi zor bir kayip olurdu. Ustune yazmak isteyen once siler.
pub fn rename(old: &str, new: &str) -> Result<(), KernelError> {
    // Iki yol da ayri tamponlara cozulur; ikisi de goreli olabilir.
    let mut old_buf = [0u8; PATH_MAX];
    let mut new_buf = [0u8; PATH_MAX];
    let old = resolve(old, &mut old_buf).ok_or(KernelError::NotFound)?;
    let new = resolve(new, &mut new_buf).ok_or(KernelError::NotFound)?;

    // RAMFS tasinamaz: dosyalar cekirdek imajinin icindedir.
    if let Some(node) = vfs::lookup(old) {
        if vfs::source(node) == Some(vfs::Source::Ram) {
            return Err(KernelError::ReadOnly);
        }
    }
    if is_dir_path(new) || vfs::lookup(new).is_some() {
        return Err(KernelError::AlreadyExists);
    }
    vfs::rename(old, new).map_err(fs_error)
}

/// Bir dizini acar ve gezinme tanimlayicisi dondurur.
///
/// POSIX'te `open` dizinlerde de calisir; ayrim `read` ile `getdents`
/// arasindadir. TCMK de ayni yolu izler: `open` yol bir dizinse dizin
/// tanimlayicisi verir, `read` onda hata doner, `getdents` okur.
pub fn open_dir(path: &str) -> Result<usize, KernelError> {
    let mut buf = [0u8; PATH_MAX];
    let path = resolve(path, &mut buf).ok_or(KernelError::NotFound)?;
    if !is_dir_path(path) {
        return Err(KernelError::NotFound);
    }
    let slot = crate::level0a::core::dir::open(if path.is_empty() { "/" } else { path })
        .ok_or(KernelError::TooManyOpenFiles)?;
    match fd::allocate_dir(slot) {
        Some(fd_num) => Ok(fd_num),
        None => {
            crate::level0a::core::dir::release(slot);
            Err(KernelError::TooManyOpenFiles)
        }
    }
}

/// Paketlenmis bir dizin kaydinin basligi (bayt cinsinden).
pub const DIRENT_HEADER: usize = 12;

/// Bir kaydin toplam uzunlugu: baslik + ad, 4'e hizalanmis.
fn record_len(name_len: usize) -> usize {
    (DIRENT_HEADER + name_len + 3) & !3
}

/// POSIX `getdents`: acik bir dizinden **birden cok** girdiyi tampona paketler.
///
/// Yazilan bayt sayisini doner; `0` dizinin bittigi anlamina gelir.
///
/// ## Kayit bicimi (TCMK'ye ozgu)
///
/// ```text
///   +0  u16 reclen     kaydin toplam uzunlugu (4'un kati)
///   +2  u8  kind       1 = dosya, 2 = dizin
///   +3  u8  name_len   ad uzunlugu
///   +4  u32 size       dosya boyutu (dizinlerde 0)
///   +8  u32 mtime      son degisiklik (Unix epoch), bilinmiyorsa 0
///  +12  ad             name_len bayt, sonda NUL **yok**
/// ```
///
/// Linux'un `linux_dirent64` kaydi degil: oradaki `d_ino` ve `d_off`
/// 64 bit alanlarin TCMK'de karsiligi yok (inode numarasi iki arka uc
/// arasinda anlamli degil, imleci de tanimlayici tasiyor). Buna karsilik
/// `size` ve `mtime` **var**: bir dosya tarayicisi boyut ve tarih
/// gostermek icin her girdide ayri bir `fstat` cagirmak zorunda
/// kalmasin diye. Win32'nin `WIN32_FIND_DATA`si da ayni iki alani
/// tasir, yani ikinci ABI icin fazladan hicbir sey gerekmiyor.
///
/// # Safety
/// `buf`/`len` gecerli, yazilabilir bir kullanici bolgesi olmalidir.
pub unsafe fn getdents(fd_num: u32, buf: *mut u8, len: usize) -> Result<usize, KernelError> {
    if buf.is_null() {
        return Err(KernelError::Fault);
    }
    let entry = fd::get(fd_num as usize).ok_or(KernelError::BadFileDescriptor)?;
    if entry.kind != fd::FdKind::Dir {
        return Err(KernelError::NotSupported);
    }
    let path = crate::level0a::core::dir::path_of(entry.node)
        .ok_or(KernelError::BadFileDescriptor)?;

    let out = core::slice::from_raw_parts_mut(buf, len);
    let mut written = 0usize;
    let mut cursor = entry.offset;

    while let Some((item, next)) = next_entry(path, cursor) {
        let size = record_len(item.name_len);
        if written + size > out.len() {
            // Tek bir kayit bile sigmiyorsa cagiran sonsuza kadar bos
            // donus alirdi; bunu hata olarak bildirmek gerekir.
            if written == 0 {
                return Err(KernelError::NotSupported);
            }
            break;
        }
        out[written..written + size].fill(0);
        out[written] = size as u8;
        out[written + 1] = (size >> 8) as u8;
        out[written + 2] = item.kind;
        out[written + 3] = item.name_len as u8;
        out[written + 4..written + 8].copy_from_slice(&(item.size as u32).to_le_bytes());
        out[written + 8..written + 12].copy_from_slice(&item.mtime.to_le_bytes());
        out[written + DIRENT_HEADER..written + DIRENT_HEADER + item.name_len]
            .copy_from_slice(&item.name[..item.name_len]);
        written += size;
        cursor = next;
    }

    // Imlec tanimlayicida yasar: bir sonraki cagri kaldigi yerden devam
    // eder, iki ayri acilis birbirini etkilemez.
    fd::advance(fd_num as usize, cursor - entry.offset);
    Ok(written)
}

/// Win32 `FindFirstFileA`/`FindNextFileA` icin **tek** girdi okur.
///
/// Win32 tarafi kayit paketlemez: her cagri bir `WIN32_FIND_DATA` doldurur.
/// Ayni gezinme cekirdeginin ikinci yuzu -- imlec yine tanimlayicida.
pub fn next_dir_entry(fd_num: u32) -> Result<DirEntry, KernelError> {
    let entry = fd::get(fd_num as usize).ok_or(KernelError::BadFileDescriptor)?;
    if entry.kind != fd::FdKind::Dir {
        return Err(KernelError::NotSupported);
    }
    let path = crate::level0a::core::dir::path_of(entry.node)
        .ok_or(KernelError::BadFileDescriptor)?;
    match next_entry(path, entry.offset) {
        Some((item, next)) => {
            fd::advance(fd_num as usize, next - entry.offset);
            Ok(item)
        }
        None => Err(KernelError::NotFound),
    }
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
