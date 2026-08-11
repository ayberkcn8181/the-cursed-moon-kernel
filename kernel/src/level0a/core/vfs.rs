//! Sanal Dosya Sistemi (VFS) -- Level-0a cekirdek modulu
//! (doc S.2.2.C "Cekirdek Modulleri: ... Dosya Sistemi (VFS)").
//!
//! VFS iki arka ucu tek bir isim uzayinda birlestirir:
//!
//!   * **RAMFS** -- cekirdek imajina gomulu, salt okunur dosyalar
//!     (`/bin/paint`, `/boot/msg.txt`). Disk olmadan da acilabilmek icin
//!     sart: TCMK bir ISO'dan, disksiz makinede de calisir.
//!   * **TCMKFS** -- diskteki kalici, yazilabilir bolum
//!     (bkz. `core::tcmkfs`). Yeniden baslatmalar arasinda yasar.
//!
//! Ust katmanlar (POSIX/NT cevirmenleri, ELF yukleyici, kabuk) hangi arka
//! ucun konustugunu **bilmez**; yalnizca yol adiyla dugum ararlar. Yeni bir
//! dosya sistemi (ornegin ext2 okuyucusu) eklemek, buraya bir `Source`
//! degeri eklemekten ibarettir.
//!
//! Isim uzayi duz metindir: yollar tam eslestirilir. Gercek dizin agaci,
//! izinler ve mount noktalari Faz 9-10 konusudur.
//!
//! Katman kurali: VFS Level-0a'ya aittir. Level-0b1'in cevirmenleri buraya
//! yalnizca `kernel_api` uzerinden ulasir, dogrudan degil.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::{kmalloc, tcmkfs};

pub const MAX_NODES: usize = 32;
pub const MAX_PATH: usize = 64;

/// Dugumun hangi arka uctan geldigi.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// Cekirdek imajina gomulu, salt okunur.
    Ram,
    /// Diskteki TCMKFS bolumu -- yazilabilir ve kalici.
    Disk,
}

#[derive(Clone, Copy)]
pub struct Node {
    path: [u8; MAX_PATH],
    path_len: usize,
    pub size: usize,
    pub source: Source,
    /// RAMFS icin icerik isaretcisi.
    data: *const u8,
    /// TCMKFS icin inode numarasi.
    inode: usize,
    used: bool,
}

impl Node {
    const fn empty() -> Self {
        Node {
            path: [0; MAX_PATH],
            path_len: 0,
            size: 0,
            source: Source::Ram,
            data: core::ptr::null(),
            inode: 0,
            used: false,
        }
    }
}

static mut NODES: [Node; MAX_NODES] = [Node::empty(); MAX_NODES];
static NODE_COUNT: AtomicUsize = AtomicUsize::new(0);

fn nodes_mut() -> *mut Node {
    core::ptr::addr_of_mut!(NODES) as *mut Node
}

fn nodes() -> *const Node {
    core::ptr::addr_of!(NODES) as *const Node
}

fn set_path(node: &mut Node, path: &str) {
    let len = path.len().min(MAX_PATH);
    node.path[..len].copy_from_slice(&path.as_bytes()[..len]);
    node.path_len = len;
}

/// Cekirdek imajina gomulu, salt okunur bir dosyayi RAMFS'e baglar.
pub fn mount_static(path: &'static str, data: &'static [u8]) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| {
        let index = NODE_COUNT.load(Ordering::Relaxed);
        if index >= MAX_NODES {
            return None;
        }
        unsafe {
            let node = &mut *nodes_mut().add(index);
            *node = Node::empty();
            set_path(node, path);
            node.size = data.len();
            node.source = Source::Ram;
            node.data = data.as_ptr();
            node.used = true;
        }
        NODE_COUNT.store(index + 1, Ordering::Relaxed);
        Some(index)
    })
}

/// Diskteki TCMKFS dosyalarini isim uzayina ekler.
///
/// Ayni yol hem RAMFS'te hem diskte varsa **RAMFS kazanir**: cekirdegin
/// kendi `/bin` uygulamalarinin diskteki bozuk bir kopya tarafindan
/// golgelenmesi, acilisi kurtarilamaz sekilde bozabilirdi.
pub fn mount_disk_files() -> usize {
    if !tcmkfs::mounted() {
        return 0;
    }

    let mut added = 0;
    for inode in 0..tcmkfs::MAX_INODES {
        let name = match tcmkfs::entry_name(inode) {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };
        if lookup(name).is_some() {
            continue;
        }
        if attach_disk_node(name, inode, tcmkfs::entry_size(inode).unwrap_or(0)).is_some() {
            added += 1;
        }
    }
    added
}

fn attach_disk_node(path: &str, inode: usize, size: usize) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| {
        let index = NODE_COUNT.load(Ordering::Relaxed);
        if index >= MAX_NODES {
            return None;
        }
        unsafe {
            let node = &mut *nodes_mut().add(index);
            *node = Node::empty();
            set_path(node, path);
            node.size = size;
            node.source = Source::Disk;
            node.inode = inode;
            node.used = true;
        }
        NODE_COUNT.store(index + 1, Ordering::Relaxed);
        Some(index)
    })
}

/// Yol adina gore dugum arar.
pub fn lookup(path: &str) -> Option<usize> {
    let count = NODE_COUNT.load(Ordering::Relaxed);
    unsafe {
        for i in 0..count {
            let node = &*nodes().add(i);
            if node.used && &node.path[..node.path_len] == path.as_bytes() {
                return Some(i);
            }
        }
    }
    None
}

fn node_at(index: usize) -> Option<&'static Node> {
    if index >= NODE_COUNT.load(Ordering::Relaxed) {
        return None;
    }
    unsafe {
        let node = &*nodes().add(index);
        if node.used {
            Some(node)
        } else {
            None
        }
    }
}

pub fn size(node: usize) -> Option<usize> {
    node_at(node).map(|n| n.size)
}

/// Dosyanin son degisiklik ani (Unix zamani). RAMFS dugumleri icin `None`:
/// icerikleri cekirdek imajiyla gelir, ayri bir zamanlari yoktur.
pub fn mtime(node: usize) -> Option<u32> {
    let n = node_at(node)?;
    match n.source {
        Source::Ram => None,
        Source::Disk => tcmkfs::entry_mtime(n.inode),
    }
}

pub fn source(node: usize) -> Option<Source> {
    node_at(node).map(|n| n.source)
}

/// `node`'un `offset`'inden itibaren `buf`'a okur; okunan bayt sayisini
/// dondurur (dosya sonunda 0).
pub fn read(node: usize, offset: usize, buf: &mut [u8]) -> Option<usize> {
    let n = node_at(node)?;

    match n.source {
        Source::Ram => {
            if offset >= n.size {
                return Some(0);
            }
            let to_copy = (n.size - offset).min(buf.len());
            unsafe {
                core::ptr::copy_nonoverlapping(n.data.add(offset), buf.as_mut_ptr(), to_copy);
            }
            Some(to_copy)
        }
        Source::Disk => tcmkfs::read(n.inode, offset, buf).ok(),
    }
}

/// Dugumun ham icerigine erisim (ELF/PE yukleyici icin).
///
/// RAMFS dugumleri zaten bellektedir ve dogrudan dilimlenir. Disk
/// dugumleri **heap'e okunur**: yukleyici imajin tamamini bir arada
/// gormek zorundadir. `kmalloc` su an serbest birakma desteklemedigi
/// icin ayni dosyayi tekrar tekrar yuklemek heap'i tuketir -- diskten
/// calistirilan bir uygulamayi yeniden baslatmak, Faz 9'daki gercek
/// ayirici gelene kadar bu bedeli oder.
pub fn load(node: usize) -> Option<&'static [u8]> {
    let n = node_at(node)?;

    match n.source {
        Source::Ram => unsafe { Some(core::slice::from_raw_parts(n.data, n.size)) },
        Source::Disk => {
            if n.size == 0 {
                return None;
            }
            let buffer = kmalloc::kmalloc_aligned(n.size, 16)?;
            let slice = unsafe { core::slice::from_raw_parts_mut(buffer, n.size) };
            match tcmkfs::read(n.inode, 0, slice) {
                Ok(read) if read == n.size => {
                    Some(unsafe { core::slice::from_raw_parts(buffer as *const u8, n.size) })
                }
                _ => None,
            }
        }
    }
}

/// Diskteki bir dosyanin tamamini yazar; yoksa olusturur.
///
/// RAMFS dugumleri salt okunurdur -- cekirdek imajinin uzerine yazmak
/// zaten mumkun degildir.
pub fn write_file(path: &str, data: &[u8]) -> Result<usize, tcmkfs::FsError> {
    if let Some(index) = lookup(path) {
        if source(index) == Some(Source::Ram) {
            return Err(tcmkfs::FsError::Exists);
        }
    }

    let written = tcmkfs::write_all(path, data)?;
    refresh_disk_node(path);
    Ok(written)
}

/// Acik bir dugume `offset`'ten itibaren yazar (POSIX `write` yolu).
///
/// RAMFS dugumleri salt okunurdur: icerikleri cekirdek imajinin icinde,
/// `.rodata` bolumundedir -- uzerine yazmak sayfa hatasi verirdi.
pub fn write_at(node: usize, offset: usize, data: &[u8]) -> Result<usize, tcmkfs::FsError> {
    let n = node_at(node).ok_or(tcmkfs::FsError::NotFound)?;
    if n.source != Source::Disk {
        return Err(tcmkfs::FsError::NotFound);
    }
    let written = tcmkfs::write(n.inode, offset, data)?;

    // Dosya buyumus olabilir; dugum boyutunu tazele.
    let size = tcmkfs::entry_size(n.inode).unwrap_or(0);
    crate::arch::cpu::without_interrupts(|| unsafe {
        (*nodes_mut().add(node)).size = size;
    });
    Ok(written)
}

/// Diskte bos bir dosya olusturur ve dugum numarasini doner.
pub fn create_file(path: &str) -> Result<usize, tcmkfs::FsError> {
    if let Some(index) = lookup(path) {
        return Ok(index);
    }
    let inode = tcmkfs::create(path)?;
    attach_disk_node(path, inode, 0).ok_or(tcmkfs::FsError::TooManyFiles)
}

/// Diskteki bir dosyayi siler.
pub fn remove_file(path: &str) -> Result<(), tcmkfs::FsError> {
    let index = lookup(path).ok_or(tcmkfs::FsError::NotFound)?;
    let node = node_at(index).ok_or(tcmkfs::FsError::NotFound)?;
    if node.source != Source::Disk {
        return Err(tcmkfs::FsError::NotFound);
    }

    tcmkfs::remove(node.inode)?;
    crate::arch::cpu::without_interrupts(|| unsafe {
        (*nodes_mut().add(index)).used = false;
    });
    Ok(())
}

/// Yazma sonrasi dugum tablosunu tazeler (boyut/inode degismis olabilir).
fn refresh_disk_node(path: &str) {
    let inode = match tcmkfs::lookup(path) {
        Some(i) => i,
        None => return,
    };
    let size = tcmkfs::entry_size(inode).unwrap_or(0);

    if let Some(index) = lookup(path) {
        crate::arch::cpu::without_interrupts(|| unsafe {
            let node = &mut *nodes_mut().add(index);
            node.inode = inode;
            node.size = size;
            node.source = Source::Disk;
        });
    } else {
        attach_disk_node(path, inode, size);
    }
}

/// Dugumun yol adi (kabuk `ls` icin).
pub fn path_of(node: usize) -> Option<&'static str> {
    let n = node_at(node)?;
    core::str::from_utf8(&n.path[..n.path_len]).ok()
}

/// Tablodaki toplam yuva sayisi (silinmisler dahil) -- gezinme siniri.
pub fn node_count() -> usize {
    NODE_COUNT.load(Ordering::Relaxed)
}

/// Gercekten var olan dosya sayisi.
pub fn file_count() -> usize {
    (0..node_count()).filter(|i| node_at(*i).is_some()).count()
}

/// Kayitli tum dugumleri listeler (acilis raporu icin).
pub fn list() {
    for i in 0..node_count() {
        if let (Some(path), Some(node)) = (path_of(i), node_at(i)) {
            crate::println!(
                "[LEVEL-0a] vfs: {:<16} {:>6} bayt  {}",
                path,
                node.size,
                match node.source {
                    Source::Ram => "ram",
                    Source::Disk => "disk",
                }
            );
        }
    }
}
