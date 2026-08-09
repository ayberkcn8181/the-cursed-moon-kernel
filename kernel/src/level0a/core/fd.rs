//! Dosya tanimlayici (file descriptor) tablosu -- doc S.7 Faz 5:
//! "sys_open / sys_read / sys_write / sys_close + FD tablosu".
//!
//! Faz 5'te tek bir kullanici sureci oldugu icin tablo global tutulur;
//! Faz 8'de (fork/execve) surec basina tasinacaktir.

use core::sync::atomic::{AtomicBool, Ordering};

pub const MAX_FDS: usize = 16;
/// 0,1,2 POSIX geregi stdin/stdout/stderr'e ayrilmistir.
pub const FIRST_FREE_FD: usize = 3;

#[derive(Clone, Copy)]
pub struct FileDescriptor {
    pub used: bool,
    pub node: usize,
    pub offset: usize,
}

impl FileDescriptor {
    const fn empty() -> Self {
        FileDescriptor {
            used: false,
            node: 0,
            offset: 0,
        }
    }
}

static mut TABLE: [FileDescriptor; MAX_FDS] = [FileDescriptor::empty(); MAX_FDS];
static INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn init() {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = core::ptr::addr_of_mut!(TABLE) as *mut FileDescriptor;
        for i in 0..MAX_FDS {
            table.add(i).write(FileDescriptor::empty());
        }
    });
    INITIALIZED.store(true, Ordering::Relaxed);
}

/// Bos bir tanimlayici ayirir ve VFS dugumune baglar.
pub fn allocate(node: usize) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = core::ptr::addr_of_mut!(TABLE) as *mut FileDescriptor;
        for fd in FIRST_FREE_FD..MAX_FDS {
            if !(*table.add(fd)).used {
                table.add(fd).write(FileDescriptor {
                    used: true,
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
        let table = core::ptr::addr_of!(TABLE) as *const FileDescriptor;
        let entry = *table.add(fd);
        if entry.used {
            Some(entry)
        } else {
            None
        }
    }
}

pub fn advance(fd: usize, delta: usize) {
    if fd >= MAX_FDS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let table = core::ptr::addr_of_mut!(TABLE) as *mut FileDescriptor;
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
        let table = core::ptr::addr_of_mut!(TABLE) as *mut FileDescriptor;
        if !(*table.add(fd)).used {
            return false;
        }
        table.add(fd).write(FileDescriptor::empty());
        true
    })
}

pub fn open_count() -> usize {
    unsafe {
        let table = core::ptr::addr_of!(TABLE) as *const FileDescriptor;
        (0..MAX_FDS).filter(|&i| (*table.add(i)).used).count()
    }
}
