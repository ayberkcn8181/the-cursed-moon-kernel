//! Dosya tanimlayici (file descriptor) tablosu -- doc S.7 Faz 5:
//! "sys_open / sys_read / sys_write / sys_close + FD tablosu".
//!
//! Tablo **surec basinadir**. Uzun sure global tutuldu (tek kullanici
//! sureci varken sorun degildi), ama borular bunu surdurulemez hale
//! getirdi: UNIX'in klasik `pipe` + `fork` kalibinda her taraf
//! kullanmadigi ucu kapatir, ve paylasilan bir tabloda cocugun kapattigi
//! tanimlayici ebeveyninkini de yok ediyordu. POSIX semantigi de zaten
//! budur: `fork` tanimlayicilari **kopyalar**, paylastirmaz.

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
}

#[derive(Clone, Copy)]
pub struct FileDescriptor {
    pub used: bool,
    pub kind: FdKind,
    /// `File` icin VFS dugumu, boru uclari icin boru indeksi.
    pub node: usize,
    pub offset: usize,
}

impl FileDescriptor {
    const fn empty() -> Self {
        FileDescriptor {
            used: false,
            kind: FdKind::File,
            node: 0,
            offset: 0,
        }
    }
}

use crate::level0a::core::scheduler;

static mut TABLES: [[FileDescriptor; MAX_FDS]; scheduler::MAX_TASKS] =
    [[FileDescriptor::empty(); MAX_FDS]; scheduler::MAX_TASKS];
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Calisan gorevin tablosunun basi.
fn table_of(task: usize) -> *mut FileDescriptor {
    unsafe { (core::ptr::addr_of_mut!(TABLES) as *mut FileDescriptor).add(task * MAX_FDS) }
}

fn current_table() -> *mut FileDescriptor {
    table_of(scheduler::current_id())
}

pub fn init() {
    crate::arch::cpu::without_interrupts(|| unsafe {
        for task in 0..scheduler::MAX_TASKS {
            let table = table_of(task);
            for i in 0..MAX_FDS {
                table.add(i).write(FileDescriptor::empty());
            }
        }
    });
    INITIALIZED.store(true, Ordering::Relaxed);
}

/// Ebeveynin tablosunu cocuga kopyalar (`fork`).
///
/// POSIX semantigi: cocuk ayni nesnelere bakan **kendi** tanimlayicilarini
/// alir. Boru uclarinin sayaclari bu yuzden artirilir -- cocuk kapatinca
/// boru olmemeli, iki taraf da kapatinca olmeli.
pub fn clone_into(child: usize) {
    if child >= scheduler::MAX_TASKS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let parent = current_table();
        let target = table_of(child);
        for i in 0..MAX_FDS {
            let entry = parent.add(i).read();
            target.add(i).write(entry);
            if entry.used {
                retain(entry);
            }
        }
    });
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
            let entry = table.add(i).read();
            if !entry.used {
                continue;
            }
            release(entry);
            table.add(i).write(FileDescriptor::empty());
        }
    });
}

/// Bos bir tanimlayici ayirir ve VFS dugumune baglar.
pub fn allocate(node: usize) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = current_table();
        for fd in FIRST_FREE_FD..MAX_FDS {
            if !(*table.add(fd)).used {
                table.add(fd).write(FileDescriptor {
                    used: true,
                    kind: FdKind::File,
                    node,
                    offset: 0,
                });
                return Some(fd);
            }
        }
        None
    })
}

pub fn get(fd: usize) -> Option<FileDescriptor> {
    if fd >= MAX_FDS {
        return None;
    }
    unsafe {
        let table = current_table();
        let entry = *table.add(fd);
        if entry.used {
            Some(entry)
        } else {
            None
        }
    }
}

/// Bir boru ucunu tanimlayiciya baglar.
pub fn allocate_pipe(pipe: usize, kind: FdKind) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = current_table();
        for fd in FIRST_FREE_FD..MAX_FDS {
            if !(*table.add(fd)).used {
                table.add(fd).write(FileDescriptor {
                    used: true,
                    kind,
                    node: pipe,
                    offset: 0,
                });
                return Some(fd);
            }
        }
        None
    })
}

/// Bir tanimlayicinin sahiplendigi nesnenin sayacini artirir.
///
/// Kopyalanan her tanimlayici bir sahiptir: iki kopyadan biri kapaninca
/// boru olmemeli, ikisi de kapaninca olmeli.
fn retain(entry: FileDescriptor) {
    match entry.kind {
        FdKind::PipeRead => crate::level0a::core::pipe::add_ref(entry.node, false),
        FdKind::PipeWrite => crate::level0a::core::pipe::add_ref(entry.node, true),
        FdKind::File => {}
    }
}

fn release(entry: FileDescriptor) {
    match entry.kind {
        FdKind::PipeRead => crate::level0a::core::pipe::close_end(entry.node, false),
        FdKind::PipeWrite => crate::level0a::core::pipe::close_end(entry.node, true),
        FdKind::File => {}
    }
}

/// POSIX `dup`: en kucuk bos tanimlayiciya kopyalar.
///
/// ## Ayrisan yan: konum (offset)
///
/// Gercek POSIX'te `dup` **ayni acik dosya tanimini** paylastirir, yani
/// iki tanimlayici tek bir konumu kullanir: birinden okumak digerinin de
/// konumunu ilerletir. TCMK'de `FileDescriptor` degerle kopyalandigi icin
/// konumlar kopyadan sonra **ayrisir**. Yonlendirme (`dup2` ile stdout'u
/// baska yere baglamak) bundan etkilenmez -- orada onemli olan nesnenin
/// kendisidir, konum degil. Ayrisma yalnizca ayni dosyayi iki
/// tanimlayiciyla okuyup "ikisi ayni yerden devam etsin" beklendiginde
/// gorunur; o kalip burada kullanilmiyor.
pub fn dup(oldfd: usize) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = current_table();
        if oldfd >= MAX_FDS || !(*table.add(oldfd)).used {
            return None;
        }
        let entry = *table.add(oldfd);
        for fd in FIRST_FREE_FD..MAX_FDS {
            if !(*table.add(fd)).used {
                table.add(fd).write(entry);
                retain(entry);
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
        let entry = *table.add(oldfd);
        let previous = *table.add(newfd);
        if previous.used {
            release(previous);
        }
        table.add(newfd).write(entry);
        retain(entry);
        Some(newfd)
    })
}

/// Tanimlayicinin konumunu **mutlak** olarak ayarlar.
///
/// Boru uclarinda anlamsizdir (halka tamponun "konumu" yoktur), o yuzden
/// yalnizca dosyalarda kabul edilir.
pub fn seek(fd: usize, offset: usize) -> bool {
    if fd >= MAX_FDS {
        return false;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = current_table();
        let entry = *table.add(fd);
        if !entry.used || entry.kind != FdKind::File {
            return false;
        }
        (*table.add(fd)).offset = offset;
        true
    })
}

pub fn advance(fd: usize, delta: usize) {
    if fd >= MAX_FDS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = current_table();
        if (*table.add(fd)).used {
            (*table.add(fd)).offset += delta;
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
        let entry = *table.add(fd);
        if !entry.used {
            return false;
        }
        // Boru ucuysa sayaci dusur: son uc de kapaninca boru serbest
        // kalir ve okuyan taraf "yazan kalmadi" bilgisini gorur.
        release(entry);
        table.add(fd).write(FileDescriptor::empty());
        true
    })
}

pub fn open_count() -> usize {
    unsafe {
        let table = current_table();
        (0..MAX_FDS).filter(|&i| (*table.add(i)).used).count()
    }
}
