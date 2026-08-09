//! Sanal Dosya Sistemi (VFS) + RAMFS arka ucu -- Level-0a cekirdek modulu
//! (doc S.2.2.C "Cekirdek Modulleri: ... Dosya Sistemi (VFS)").
//!
//! Faz 5 kapsami: duz (hiyerarsik olmayan) bir dugum tablosu. Yollar tam
//! metin olarak eslestirilir; dizin girdileri, izinler ve mount noktalari
//! Faz 9-10'da (ext2/tmpfs) gelecektir.
//!
//! Katman kurali: VFS Level-0a'ya aittir. Level-0b1'in POSIX/NT cevirmenleri
//! buraya yalnizca `kernel_api` uzerinden ulasir, dogrudan degil.

use core::sync::atomic::{AtomicUsize, Ordering};

pub const MAX_NODES: usize = 16;

/// Faz 5'te tum dugumler cekirdek imajina gomulu, salt okunur dosyalardir.
/// Yazilabilir (tmpfs) dugumler Faz 9-10'da eklenecek; o zaman buraya bir
/// `kind` alani gelecek.
#[derive(Clone, Copy)]
pub struct Node {
    pub path: &'static str,
    pub data: *const u8,
    pub size: usize,
}

impl Node {
    const fn empty() -> Self {
        Node {
            path: "",
            data: core::ptr::null(),
            size: 0,
        }
    }
}

static mut NODES: [Node; MAX_NODES] = [Node::empty(); MAX_NODES];
static NODE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Cekirdek imajina gomulu, salt okunur bir dosyayi RAMFS'e baglar.
pub fn mount_static(path: &'static str, data: &'static [u8]) -> Option<usize> {
    crate::arch::i386::without_interrupts(|| {
        let index = NODE_COUNT.load(Ordering::Relaxed);
        if index >= MAX_NODES {
            return None;
        }
        unsafe {
            let nodes = core::ptr::addr_of_mut!(NODES) as *mut Node;
            nodes.add(index).write(Node {
                path,
                data: data.as_ptr(),
                size: data.len(),
            });
        }
        NODE_COUNT.store(index + 1, Ordering::Relaxed);
        Some(index)
    })
}

/// Yol adina gore dugum arar.
pub fn lookup(path: &str) -> Option<usize> {
    let count = NODE_COUNT.load(Ordering::Relaxed);
    unsafe {
        let nodes = core::ptr::addr_of!(NODES) as *const Node;
        for i in 0..count {
            if (*nodes.add(i)).path == path {
                return Some(i);
            }
        }
    }
    None
}

pub fn size(node: usize) -> Option<usize> {
    if node >= NODE_COUNT.load(Ordering::Relaxed) {
        return None;
    }
    unsafe {
        let nodes = core::ptr::addr_of!(NODES) as *const Node;
        Some((*nodes.add(node)).size)
    }
}

/// `node`'un `offset`'inden itibaren `buf`'a okur; okunan bayt sayisini
/// dondurur (dosya sonunda 0).
pub fn read(node: usize, offset: usize, buf: &mut [u8]) -> Option<usize> {
    if node >= NODE_COUNT.load(Ordering::Relaxed) {
        return None;
    }

    unsafe {
        let nodes = core::ptr::addr_of!(NODES) as *const Node;
        let n = *nodes.add(node);

        if offset >= n.size {
            return Some(0);
        }
        let available = n.size - offset;
        let to_copy = available.min(buf.len());
        core::ptr::copy_nonoverlapping(n.data.add(offset), buf.as_mut_ptr(), to_copy);
        Some(to_copy)
    }
}

/// Dugumun ham icerigine dogrudan erisim (ELF yukleyici icin).
pub fn as_slice(node: usize) -> Option<&'static [u8]> {
    if node >= NODE_COUNT.load(Ordering::Relaxed) {
        return None;
    }
    unsafe {
        let nodes = core::ptr::addr_of!(NODES) as *const Node;
        let n = *nodes.add(node);
        Some(core::slice::from_raw_parts(n.data, n.size))
    }
}

pub fn node_count() -> usize {
    NODE_COUNT.load(Ordering::Relaxed)
}

/// Kayitli tum dugumleri listeler (acilis raporu icin).
pub fn list() {
    let count = NODE_COUNT.load(Ordering::Relaxed);
    unsafe {
        let nodes = core::ptr::addr_of!(NODES) as *const Node;
        for i in 0..count {
            let n = *nodes.add(i);
            crate::println!("[LEVEL-0a] vfs: {:<16} {:>6} bayt", n.path, n.size);
        }
    }
}
