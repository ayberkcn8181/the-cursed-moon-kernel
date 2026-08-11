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
                match entry.kind {
                    FdKind::PipeRead => crate::level0a::core::pipe::add_ref(entry.node, false),
                    FdKind::PipeWrite => crate::level0a::core::pipe::add_ref(entry.node, true),
                    FdKind::File => {}
                }
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
            match entry.kind {
                FdKind::PipeRead => crate::level0a::core::pipe::close_end(entry.node, false),
                FdKind::PipeWrite => crate::level0a::core::pipe::close_end(entry.node, true),
                FdKind::File => {}
            }
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

pub fn close(fd: usize) -> bool {
    if fd < FIRST_FREE_FD || fd >= MAX_FDS {
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
        match entry.kind {
            FdKind::PipeRead => crate::level0a::core::pipe::close_end(entry.node, false),
            FdKind::PipeWrite => crate::level0a::core::pipe::close_end(entry.node, true),
            FdKind::File => {}
        }
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
