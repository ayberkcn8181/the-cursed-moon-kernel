//! Dosya tanimlayici (file descriptor) tablosu -- doc S.7 Faz 5:
//! "sys_open / sys_read / sys_write / sys_close + FD tablosu".
//!
//! Tablo **surec basinadir**. Uzun sure global tutuldu (tek kullanici
//! sureci varken sorun degildi), ama borular bunu surdurulemez hale
//! getirdi: UNIX'in klasik `pipe` + `fork` kalibinda her taraf
//! kullanmadigi ucu kapatir, ve paylasilan bir tabloda cocugun kapattigi
//! tanimlayici ebeveyninkini de yok ediyordu. POSIX semantigi de zaten
//! budur: `fork` tanimlayicilari **kopyalar**, paylastirmaz.
//!
//! Ama kopyalanan **tanimlayicidir**, arkasindaki acik dosya tanimi
//! degil: iki tablo ayni tanima bakar ve konumu paylasir. Tablo uzun sure
//! bu ayrimi yapmiyordu (bkz. "Iki katman").

use core::sync::atomic::{AtomicBool, Ordering};

pub const MAX_FDS: usize = 16;
/// 0,1,2 POSIX geregi stdin/stdout/stderr'e ayrilmistir.
///
/// Bu uc yuva tabloda **bos durur**; bos olmalari "varsayilan" anlamina
/// gelir -- stdin klavye kuyrugu, stdout/stderr konsol. `dup2` ile bir
/// yuvaya bir sey baglanirsa varsayilan yerine o kullanilir; `close` ile
/// varsayilana geri donulur. Yonlendirmenin butun mekanigi budur
/// (bkz. `kernel_api::read` / `kernel_api::write`).
pub const FIRST_FREE_FD: usize = 3;

/// Bir tanimlayicinin ardindaki nesne.
///
/// POSIX'te `read`/`write` tanimlayicinin **neye** bagli oldugunu
/// bilmez; ayrimi cekirdek yapar. Boru eklendiginde bu ayrim gorunur
/// hale geldi: ayni `read` cagrisi ya VFS dugumunden ya halka tampondan
/// okur.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FdKind {
    /// VFS dugumu (RAMFS ya da TCMKFS dosyasi).
    File,
    /// Borunun okuma ucu.
    PipeRead,
    /// Borunun yazma ucu.
    PipeWrite,
    /// Acik bir dizin (`core::dir` havuzundaki yuva).
    ///
    /// POSIX'te dizin de bir tanimlayicidir: `open` ile acilir, `close`
    /// ile kapanir, ama `read` yerine `getdents` ile okunur. `offset`
    /// burada bayt konumu degil **gezinme imleci**dir.
    Dir,
}

/// Bir tanimlayicinin **anlik goruntusu**: yuvasi ve arkasindaki acik
/// dosya tanimi birlikte.
///
/// Disaridan gorunen bicim degismedi; cekirdegin geri kalani `get` ile
/// bunu aliyor. Ama alanlar artik iki ayri katmandan geliyor (asagi bkz.
/// "Iki katman"): `kind`, `node`, `offset`, `flags` acik dosya tanimina,
/// `cloexec` yuvaya ait.
#[derive(Clone, Copy)]
pub struct FileDescriptor {
    pub kind: FdKind,
    /// `File` icin VFS dugumu, boru uclari icin boru indeksi.
    pub node: usize,
    pub offset: usize,
    /// Acik-dosya bayraklari (`O_NONBLOCK`, `O_APPEND`).
    pub flags: u32,
    /// `FD_CLOEXEC` -- **yuvanin** bayragi, paylasilmaz.
    pub cloexec: bool,
}

// --- Iki katman -------------------------------------------------------
//
// POSIX'te bir tanimlayici iki ayri seydir ve TCMK uzun sure ikisini tek
// bir degerde tutuyordu:
//
//   tanimlayici (fd)           -> surec basina bir numara; tek bayragi
//                                 FD_CLOEXEC
//   acik dosya tanimi (OFD)    -> open() cagrisi basina bir kayit; konum,
//                                 O_NONBLOCK, O_APPEND burada
//
// `dup`, `dup2` ve `fork` **yeni bir tanimlayici** yaratir ama **ayni
// tanima** baglar. Sonucu somut: iki tanimlayici tek bir konumu
// paylasir. Kabugun `(echo a; echo b) > f` kalibi buna dayaniyor -- iki
// cocuk ayni konumdan yazmasa ikincisi birincinin ustune yazardi.
//
// Ayri iki `open` ise iki **ayri** tanim yaratir: ayni dosya, iki konum.
//
// Eskiden tablo tanimlayici basina degerle kopyalaniyordu, yani her
// kopya kendi konumunu tasiyordu. `O_NONBLOCK` da ayni sekilde
// kopyalaniyordu.

/// Surec tablosundaki bir yuva: hangi acik dosya tanimina bakiyor.
#[derive(Clone, Copy)]
struct Slot {
    used: bool,
    /// `FILES` icindeki indeks.
    file: usize,
    /// `FD_CLOEXEC`: `execve`de kapanir. Tanimlayiciya ait; `dup` onu
    /// **kopyalamaz** (POSIX: yeni tanimlayicida bayrak kapali).
    cloexec: bool,
    /// Win32 `HANDLE_FLAG_INHERIT`: `CreateProcess(bInheritHandles=TRUE)`
    /// ile cocuga gecer.
    ///
    /// `FD_CLOEXEC`in **tersi**, ve varsayilanlar da ters: POSIX'te her
    /// tanimlayici `execve`den gecer, kapatmak icin bayrak gerekir;
    /// Windows'ta hicbir tutamac gecmez, gecmesi icin bayrak gerekir. Ayni
    /// yuvada iki ayri alan, cunku iki yuz ayri sorulari soruyor.
    inherit: bool,
}

impl Slot {
    const EMPTY: Self = Slot {
        used: false,
        file: 0,
        cloexec: false,
        inherit: false,
    };
}

/// Acik dosya tanimi.
#[derive(Clone, Copy)]
struct OpenFile {
    /// Kac yuva buna bakiyor (butun sureclerde). 0 = bos kayit.
    refs: usize,
    kind: FdKind,
    node: usize,
    offset: usize,
    flags: u32,
}

impl OpenFile {
    const EMPTY: Self = OpenFile {
        refs: 0,
        kind: FdKind::File,
        node: 0,
        offset: 0,
        flags: 0,
    };
}

/// Ayni anda acik olabilecek tanim sayisi: her yuva ayri bir `open`
/// olsa bile yetecek kadar.
pub const MAX_OPEN: usize = scheduler::MAX_TASKS * MAX_FDS;

use crate::level0a::core::scheduler;

static mut SLOTS: [[Slot; MAX_FDS]; scheduler::MAX_TASKS] =
    [[Slot::EMPTY; MAX_FDS]; scheduler::MAX_TASKS];
static mut FILES: [OpenFile; MAX_OPEN] = [OpenFile::EMPTY; MAX_OPEN];
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Bir gorevin yuva tablosunun basi.
fn table_of(task: usize) -> *mut Slot {
    unsafe { (core::ptr::addr_of_mut!(SLOTS) as *mut Slot).add(task * MAX_FDS) }
}

fn current_table() -> *mut Slot {
    // **Grup** basina: kardes is parcaciklari ayni tabloyu gorur, bir
    // `fork` cocugu gormez. `CLONE_FILES`in anlami tam olarak budur.
    table_of(scheduler::current_group())
}

fn file(index: usize) -> *mut OpenFile {
    unsafe { (core::ptr::addr_of_mut!(FILES) as *mut OpenFile).add(index) }
}

pub fn init() {
    crate::arch::cpu::without_interrupts(|| unsafe {
        for task in 0..scheduler::MAX_TASKS {
            let table = table_of(task);
            for i in 0..MAX_FDS {
                table.add(i).write(Slot::EMPTY);
            }
        }
        for i in 0..MAX_OPEN {
            file(i).write(OpenFile::EMPTY);
        }
    });
    INITIALIZED.store(true, Ordering::Relaxed);
}

/// Yeni bir acik dosya tanimi ayirir (tek sahipli). Kesmeler kapali.
unsafe fn new_file(kind: FdKind, node: usize, flags: u32) -> Option<usize> {
    for i in 0..MAX_OPEN {
        if (*file(i)).refs == 0 {
            file(i).write(OpenFile {
                refs: 1,
                kind,
                node,
                offset: 0,
                flags,
            });
            return Some(i);
        }
    }
    None
}

/// Bir tanima yeni bir sahip ekler (`dup`, `fork`).
unsafe fn hold(index: usize) {
    (*file(index)).refs += 1;
}

/// Bir sahibi birakir; son sahip gidince tanim ve **nesne** serbest.
///
/// Boru ve dizin sayaclari artik tanim basina: iki tanimlayici ayni
/// tanimi paylasiyorsa boru ucu bir kez sayiliyor. "Yazan kalmadi"
/// bilgisi icin onemli olan da budur -- ucun son **tanimi** kapaninca.
unsafe fn drop_ref(index: usize) {
    let f = file(index);
    if (*f).refs == 0 {
        return;
    }
    (*f).refs -= 1;
    if (*f).refs == 0 {
        let entry = *f;
        release(entry.kind, entry.node);
        f.write(OpenFile::EMPTY);
    }
}

/// Ebeveynin tablosunu cocuga kopyalar (`fork`).
///
/// POSIX semantigi: cocuk **kendi** tanimlayicilarini alir ama onlar
/// ebeveyninkilerle **ayni** tanimlara bakar -- konum ve durum bayraklari
/// ortak. `FD_CLOEXEC` yuvayla birlikte kopyalaniyor (POSIX: `fork`
/// korur, `dup` korumaz).
pub fn clone_into(child: usize) {
    if child >= scheduler::MAX_TASKS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let parent = current_table();
        let target = table_of(child);
        for i in 0..MAX_FDS {
            let slot = parent.add(i).read();
            target.add(i).write(slot);
            if slot.used {
                hold(slot.file);
            }
        }
    });
    // Yonlendirilmemis 0/1/2'nin bayraklari da devrediliyor.
    //
    // Uzun sure devredilmiyordu, ve daha kotusu cocugun yuvasinda
    // **onceki sahibinin** bayraklari duruyordu: sifirlama yalnizca
    // `execve`de yapiliyordu, `fork`ta degil. Is denetimi sinavi bunu
    // buldu -- hic `O_NONBLOCK` koymamis bir cocugun "bloke eden"
    // okumasi beklemeden `EAGAIN` dondu, cunku ayni yuvayi daha once
    // kullanan cocuk o bayragi koymustu.
    //
    // Bunlar acik dosya tanimi **degil**: yonlendirilmemis stdin bir
    // dosya degil, cekirdegin numaradan tanidigi klavye. Bayraklari
    // kopyalaniyor, paylasilmiyor -- paylasim bir cocugun ebeveyninin
    // okumasini bloke etmeyen hale getirmesine izin verirdi (Linux'ta
    // bilinen bir tuzak). Yonlendirilmis 0/1/2 ise tanimin kendisi ve
    // paylasiliyor.
    let parent_group = scheduler::current_group();
    if parent_group < scheduler::MAX_TASKS {
        for fd in 0..FIRST_FREE_FD {
            let value = STD_FLAGS[parent_group][fd].load(Ordering::Relaxed);
            STD_FLAGS[child][fd].store(value, Ordering::Relaxed);
        }
    }
}

/// Gorev sonlanirken tum tanimlayicilarini birakir.
///
/// Boru uclari icin sart: kapanmayan bir yazma ucu, okuyan tarafta
/// "dosya sonu"nun hic gorunmemesi demektir.
pub fn close_all(task: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = table_of(task);
        for i in 0..MAX_FDS {
            let slot = table.add(i).read();
            if slot.used {
                drop_ref(slot.file);
                table.add(i).write(Slot::EMPTY);
            }
        }
    });
}

/// `execve`: `FD_CLOEXEC` tasiyan tanimlayicilari kapatir.
///
/// Kalanlar yeni programa **ayni tanimlarla** gecer -- konum dahil.
/// Bayrak olmadan bir kabuk, cocuga gecmesini istemedigi her tanimlayiciyi
/// `fork` ile `execve` arasinda tek tek kapatmak zorunda kalirdi; is
/// parcacikli bir programda o aralikta baska bir akisin actigi
/// tanimlayici sizardi. `O_CLOEXEC` bu yarisi kapatmak icin var.
pub fn close_on_exec(group: usize) -> usize {
    if group >= scheduler::MAX_TASKS {
        return 0;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = table_of(group);
        let mut closed = 0;
        for i in 0..MAX_FDS {
            let slot = table.add(i).read();
            if slot.used && slot.cloexec {
                drop_ref(slot.file);
                table.add(i).write(Slot::EMPTY);
                closed += 1;
            }
        }
        CLOSED_ON_EXEC.fetch_add(closed, Ordering::Relaxed);
        closed
    })
}

/// Yeni bir tanim ve onu gosteren en kucuk bos yuva. Kesmeler kapali.
unsafe fn install(kind: FdKind, node: usize, flags: u32, cloexec: bool) -> Option<usize> {
    let table = current_table();
    let fd = (FIRST_FREE_FD..MAX_FDS).find(|&fd| !(*table.add(fd)).used)?;
    let index = new_file(kind, node, flags)?;
    table.add(fd).write(Slot {
        used: true,
        file: index,
        cloexec,
        inherit: false,
    });
    Some(fd)
}

/// `open` bayraklariyla: `O_APPEND`/`O_NONBLOCK` tanima, `O_CLOEXEC`
/// yuvaya.
pub fn allocate_with(node: usize, flags: u32, cloexec: bool) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        install(FdKind::File, node, flags & STATUS_FLAGS, cloexec)
    })
}

pub fn get(fd: usize) -> Option<FileDescriptor> {
    if fd >= MAX_FDS {
        return None;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let slot = *current_table().add(fd);
        if !slot.used {
            return None;
        }
        let f = *file(slot.file);
        Some(FileDescriptor {
            kind: f.kind,
            node: f.node,
            offset: f.offset,
            flags: f.flags,
            cloexec: slot.cloexec,
        })
    })
}

/// Yonlendirilmemis `stdin`/`stdout`/`stderr` bayraklari.
///
/// 0, 1 ve 2 yonlendirilmemisken tabloda **yer tutmuyor**: cekirdek
/// onlari numaradan taniyip konsola/pencereye bagliyor. Ama
/// `fcntl(0, F_SETFL, O_NONBLOCK)` yine de calismali -- POSIX'te
/// bloke olmamak stdin'in en cok istenen ozelligi.
///
/// Tablo gibi **grup** basina: kardes is parcaciklari ayni stdin'i
/// paylasiyor.
static STD_FLAGS: [[core::sync::atomic::AtomicU32; FIRST_FREE_FD]; scheduler::MAX_TASKS] =
    [const {
        [const { core::sync::atomic::AtomicU32::new(0) }; FIRST_FREE_FD]
    }; scheduler::MAX_TASKS];

/// Bir tanimlayicinin acik-dosya bayraklari.
pub fn flags(fd: usize) -> u32 {
    if let Some(entry) = get(fd) {
        return entry.flags;
    }
    // Yonlendirilmemis standart tanimlayici.
    if fd < FIRST_FREE_FD {
        return STD_FLAGS[scheduler::current_group()][fd].load(Ordering::Relaxed);
    }
    0
}

/// Bayraklari degistirir; tanimlayici yoksa `false`.
///
/// Degisiklik **tanima** yaziliyor: ayni tanimi paylasan her tanimlayici
/// -- `dup` kopyasi, `fork` cocugu -- yeni bayragi gorur.
pub fn set_flags(fd: usize, flags: u32) -> bool {
    if fd >= MAX_FDS {
        return false;
    }
    let flags = flags & STATUS_FLAGS;
    let stored = crate::arch::cpu::without_interrupts(|| unsafe {
        let slot = *current_table().add(fd);
        if !slot.used {
            return false;
        }
        (*file(slot.file)).flags = flags;
        true
    });
    if stored {
        return true;
    }
    if fd < FIRST_FREE_FD {
        STD_FLAGS[scheduler::current_group()][fd].store(flags, Ordering::Relaxed);
        return true;
    }
    false
}

/// `FD_CLOEXEC` -- tanimlayici yoksa `None`.
pub fn cloexec(fd: usize) -> Option<bool> {
    get(fd).map(|entry| entry.cloexec)
}

/// `FD_CLOEXEC`i degistirir; **yalnizca bu yuva**. Tanimlayici yoksa `false`.
pub fn set_cloexec(fd: usize, on: bool) -> bool {
    if fd >= MAX_FDS {
        return false;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = current_table();
        if !(*table.add(fd)).used {
            return false;
        }
        (*table.add(fd)).cloexec = on;
        true
    })
}

/// Win32 `HANDLE_FLAG_INHERIT` -- tanimlayici yoksa `None`.
pub fn inherit(fd: usize) -> Option<bool> {
    if fd >= MAX_FDS {
        return None;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let slot = *current_table().add(fd);
        slot.used.then_some(slot.inherit)
    })
}

/// `HANDLE_FLAG_INHERIT`i degistirir; yalnizca bu yuva.
pub fn set_inherit(fd: usize, on: bool) -> bool {
    if fd >= MAX_FDS {
        return false;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = current_table();
        if !(*table.add(fd)).used {
            return false;
        }
        (*table.add(fd)).inherit = on;
        true
    })
}

/// `CreateProcess`: cagiranin tutamaclarini yeni surece devreder.
///
/// `all` (`bInheritHandles`) dogruysa `HANDLE_FLAG_INHERIT` tasiyan her
/// yuva cocuga **ayni numarayla** ve **ayni acik dosya tanimiyla**
/// geciyor -- Windows'ta miras alinan tutamacin degeri cocukta da
/// aynidir, ve konum paylasilir. Bayrak cocukta da acik kalir (torunlar
/// da devralabilir).
///
/// `std` (`STARTF_USESTDHANDLES`): cocugun 0/1/2'si. Windows bunu ancak
/// `bInheritHandles` ile ve miras alinabilir tutamaclarla kabul ediyor;
/// burada da oyle -- kosulu saglamayan tutamac yok sayiliyor ve o yuva
/// varsayilanda (klavye/konsol) kaliyor.
///
/// Doner: cocuga gecen yuva sayisi. Cocugun tablosu bos olmali (yeni
/// yaratilmis surec).
pub fn inherit_into(child: usize, all: bool, std: [Option<usize>; FIRST_FREE_FD]) -> usize {
    if child >= scheduler::MAX_TASKS || !all {
        return 0;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let parent = current_table();
        let target = table_of(child);
        let mut passed = 0;
        for i in FIRST_FREE_FD..MAX_FDS {
            let slot = parent.add(i).read();
            if slot.used && slot.inherit && !(*target.add(i)).used {
                target.add(i).write(Slot { cloexec: false, ..slot });
                hold(slot.file);
                passed += 1;
            }
        }
        for (n, handle) in std.iter().enumerate() {
            let Some(h) = *handle else { continue };
            if h >= MAX_FDS {
                continue;
            }
            let slot = parent.add(h).read();
            if !slot.used || !slot.inherit {
                continue;
            }
            let previous = target.add(n).read();
            if previous.used {
                drop_ref(previous.file);
            }
            target.add(n).write(Slot { cloexec: false, ..slot });
            hold(slot.file);
            passed += 1;
        }
        INHERITED.fetch_add(passed, Ordering::Relaxed);
        passed
    })
}

/// `CreateProcess` ile cocuga gecen tutamaclar (kabuk raporu).
static INHERITED: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

pub fn inherited_total() -> usize {
    INHERITED.load(Ordering::Relaxed)
}

/// Bir grubun standart tanimlayici bayraklarini sifirlar.
///
/// `execve` sonrasi yeni program bayraklari devralmamali: POSIX'te
/// `O_NONBLOCK` acik dosya tanimina ait ve `exec` onu korur, ama TCMK'de
/// yonlendirilmemis stdin bir dosya degil -- her program kendi
/// varsayilanindan baslamali.
pub fn reset_std_flags(group: usize) {
    if group >= scheduler::MAX_TASKS {
        return;
    }
    for fd in 0..FIRST_FREE_FD {
        STD_FLAGS[group][fd].store(0, Ordering::Relaxed);
    }
}

/// `O_NONBLOCK` -- Linux'ta i386 ve x86_64'te ayni sayi (0o4000).
pub const O_NONBLOCK: u32 = 0x800;
/// `O_APPEND` (0o2000): her yazma dosya sonuna.
pub const O_APPEND: u32 = 0x400;
/// `O_CLOEXEC` (0o2000000): `open` aninda `FD_CLOEXEC`.
pub const O_CLOEXEC: u32 = 0x8_0000;
/// Tanimda tutulan, `F_SETFL` ile degisebilen durum bayraklari.
pub const STATUS_FLAGS: u32 = O_NONBLOCK | O_APPEND;

/// Bir boru ucunu tanimlayiciya baglar.
pub fn allocate_pipe(pipe: usize, kind: FdKind) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| unsafe { install(kind, pipe, 0, false) })
}

/// Acik bir dizin yuvasini tanimlayiciya baglar.
///
/// `offset` sifirdan baslar: dizin imleci, ilk girdi.
pub fn allocate_dir(slot: usize) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| unsafe { install(FdKind::Dir, slot, 0, false) })
}

/// Son tanim kapaninca nesnenin sayacini dusurur.
///
/// Boru ve dizin nesneleri tanim basina bir kez sayiliyor; `dup` ve
/// `fork` tanimi paylastirdigi icin nesneye dokunmuyor.
fn release(kind: FdKind, node: usize) {
    match kind {
        FdKind::PipeRead => crate::level0a::core::pipe::close_end(node, false),
        FdKind::PipeWrite => crate::level0a::core::pipe::close_end(node, true),
        FdKind::Dir => crate::level0a::core::dir::release(node),
        FdKind::File => {}
    }
}

/// POSIX `dup`: en kucuk bos tanimlayiciya kopyalar.
///
/// Yeni tanimlayici **ayni tanima** bakar: birinden okumak digerinin de
/// konumunu ilerletir. `FD_CLOEXEC` kopyalanmaz (POSIX).
pub fn dup(oldfd: usize) -> Option<usize> {
    dup_from(oldfd, FIRST_FREE_FD, false)
}

/// `F_DUPFD`/`F_DUPFD_CLOEXEC`: `min` numarasindan baslayan en kucuk bos
/// yuvaya kopyalar.
pub fn dup_from(oldfd: usize, min: usize, cloexec: bool) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = current_table();
        if oldfd >= MAX_FDS || !(*table.add(oldfd)).used {
            return None;
        }
        let index = (*table.add(oldfd)).file;
        for fd in min.max(FIRST_FREE_FD)..MAX_FDS {
            if !(*table.add(fd)).used {
                table.add(fd).write(Slot {
                    used: true,
                    file: index,
                    cloexec,
                    inherit: false,
                });
                hold(index);
                return Some(fd);
            }
        }
        None
    })
}

/// POSIX `dup2`: kopyayi **istenen** numaraya koyar.
///
/// Yonlendirmenin tamami bu cagridadir: `dup2(boru_yazma_ucu, 1)` dedikten
/// sonra, stdout'a yazan bir kod -- borudan haberi olmasa bile -- boruya
/// yazar. UNIX'in kabuk yonlendirmesi (`komut > dosya`) tam olarak bu
/// mekanizmadir.
///
/// `newfd` 0/1/2 olabilir; onlarin normalde bos duran yuvasi dolar ve
/// varsayilan davranis (klavye/konsol) devre disi kalir.
pub fn dup2(oldfd: usize, newfd: usize) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = current_table();
        if oldfd >= MAX_FDS || newfd >= MAX_FDS || !(*table.add(oldfd)).used {
            return None;
        }
        // POSIX: ayni numaraysa hicbir sey yapilmaz. Ozellikle kapatilmaz --
        // once kapatip sonra kopyalasaydik tek sahipli bir boru olurdu.
        if oldfd == newfd {
            return Some(newfd);
        }
        let index = (*table.add(oldfd)).file;
        // Once yeni sahip, sonra eskisini birakmak: `newfd` zaten ayni
        // tanima bakiyorsa sira tersine donse tanim bir an sahipsiz
        // kalip serbest birakilirdi.
        hold(index);
        let previous = *table.add(newfd);
        if previous.used {
            drop_ref(previous.file);
        }
        table.add(newfd).write(Slot {
            used: true,
            file: index,
            cloexec: false,
            inherit: false,
        });
        Some(newfd)
    })
}

/// Tanimlayicinin konumunu **mutlak** olarak ayarlar.
///
/// Konum tanima ait: ayni tanimi paylasan her tanimlayici yeni konumu
/// gorur. Boru uclarinda anlamsizdir, o yuzden yalnizca dosyalarda.
pub fn seek(fd: usize, offset: usize) -> bool {
    if fd >= MAX_FDS {
        return false;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let slot = *current_table().add(fd);
        if !slot.used || (*file(slot.file)).kind != FdKind::File {
            return false;
        }
        (*file(slot.file)).offset = offset;
        true
    })
}

pub fn advance(fd: usize, delta: usize) {
    if fd >= MAX_FDS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let slot = *current_table().add(fd);
        if slot.used {
            (*file(slot.file)).offset += delta;
        }
    });
}

/// Tanimlayiciyi kapatir.
///
/// 0/1/2 icin bu, **yonlendirmeyi geri almak** demektir: yuva bosalir ve
/// varsayilan davranis (klavye/konsol) geri doner. Yonlendirilmemis bir
/// stdio numarasini kapatmak `false` doner -- kapatilacak bir sey yoktur.
pub fn close(fd: usize) -> bool {
    if fd >= MAX_FDS {
        return false;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = current_table();
        let slot = *table.add(fd);
        if !slot.used {
            return false;
        }
        // Tanimin son sahibiyse nesne de birakiliyor: son boru ucu
        // kapaninca okuyan taraf "yazan kalmadi" bilgisini gorur.
        drop_ref(slot.file);
        table.add(fd).write(Slot::EMPTY);
        true
    })
}

pub fn open_count() -> usize {
    unsafe {
        let table = current_table();
        (0..MAX_FDS).filter(|&i| (*table.add(i)).used).count()
    }
}

/// `execve`de `FD_CLOEXEC` yuzunden kapanan tanimlayicilar.
static CLOSED_ON_EXEC: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// `(acik tanim, paylasilan tanim, exec'te kapanan)` -- kabuk raporu.
///
/// "Paylasilan" birden fazla yuvanin baktigi tanim: `dup` ya da `fork`
/// kopyasi. Eski tabloda bu sayi kavram olarak yoktu.
pub fn stats() -> (usize, usize, usize) {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let mut open = 0;
        let mut shared = 0;
        for i in 0..MAX_OPEN {
            let refs = (*file(i)).refs;
            if refs > 0 {
                open += 1;
            }
            if refs > 1 {
                shared += 1;
            }
        }
        (open, shared, CLOSED_ON_EXEC.load(Ordering::Relaxed))
    })
}
