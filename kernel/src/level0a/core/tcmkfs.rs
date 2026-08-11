//! TCMKFS -- TCMK'nin kalici, **yazilabilir** dosya sistemi.
//!
//! ## Neden kendi dosya sistemi
//!
//! Doc S.7 Faz 9 ext2 diyordu. ext2'nin okunmasi kolaydir ama **yazilmasi**
//! degildir (blok gruplari, dolayli blok agaclari, dizin karma indeksleri).
//! TCMK'nin bu asamadaki ihtiyaci "Linux diskini okumak" degil, "kendi
//! verisini kalici olarak saklamak" oldugu icin ilk gunden yazma destegi
//! olan kucuk bir dosya sistemi secildi. ext2 okuyucusu ileride VFS'e
//! ikinci bir arka uc olarak eklenebilir -- katman bunun icin var.
//!
//! ## Yerlesim (bolum baslangicina gore, sektor cinsinden)
//!
//! ```text
//!    0        superblock
//!    1..32    inode tablosu   (64 inode x 256 bayt)
//!   33..36    blok bitmap'i   (16384 bit -> 16384 blok -> 64 MiB)
//!   40..4095  ONYUKLEYICI ALANI (2 MiB, dosya sistemi disi)
//!               40..71    2. asama (16 KiB)
//!               72..4095  cekirdek imaji (ham bellek blogu)
//! 4096..      veri bloklari   (blok = 4096 bayt = 8 sektor)
//! ```
//!
//! Onyukleyici alani bilerek dosya sisteminin **icinde** ayrildi: boylece
//! MBR'nin dort bolum yuvasindan biri daha harcanmaz ve "acilabilir TCMK
//! diski" tek bir bolumden ibaret kalir. Alan bitmap'e hic girmedigi icin
//! ayirici oraya asla dosya yazamaz.
//!
//! Dosya basina yalnizca **dogrudan** blok isaretcisi vardir (40 adet);
//! azami dosya boyutu 160 KiB. Dolayli bloklar (indirect) bilincli olarak
//! yok: bu asamada saklanan seyler yapilandirma, gunluk ve kullanici
//! uygulamalari -- hepsi bu sinirin cok altinda.
//!
//! ## Es zamanlilik
//!
//! Inode tablosu ve bitmap bellekte onbelleklenir, degisiklikler diske
//! yazilir (write-through). Disk islemleri sirasinda **hicbir yerde
//! `yield` cagrilmaz** ve hicbir kesme handler'i diske dokunmaz; bu yuzden
//! isbirlikci zamanlamada ayrica kilit gerekmez. Kesme tabanli asenkron
//! G/C geldiginde (Faz 9+) burasi gercek bir kilide ihtiyac duyacak.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::level0a::drivers::block::{self, SECTOR_SIZE};
use crate::level0a::drivers::partition;

/// "TCMK" (little-endian).
pub const MAGIC: u32 = 0x4B4D_4354;
pub const VERSION: u32 = 1;

pub const BLOCK_SIZE: usize = 4096;
const SECTORS_PER_BLOCK: u32 = (BLOCK_SIZE / SECTOR_SIZE) as u32;

pub const MAX_INODES: usize = 64;
const INODE_SIZE: usize = 256;
const INODE_TABLE_SECTOR: u32 = 1;
const INODE_TABLE_SECTORS: u32 = (MAX_INODES * INODE_SIZE / SECTOR_SIZE) as u32; // 32

const BITMAP_SECTOR: u32 = INODE_TABLE_SECTOR + INODE_TABLE_SECTORS; // 33
const BITMAP_SECTORS: u32 = 4;
const MAX_BLOCKS: usize = (BITMAP_SECTORS as usize * SECTOR_SIZE) * 8; // 16384

// Onyukleyici alani sabitleri: yalnizca i386'nin kurulum modulu kullanir
// (boot zinciri su an i386'ya ozgu). Yerlesimin kendisi mimariden
// bagimsiz oldugu icin tanimlar burada durur.
/// Onyukleyicinin 2. asamasi icin ayrilan ilk sektor ve boyu.
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
pub const BOOT_AREA_SECTOR: u32 = 40;
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
pub const STAGE2_SECTORS: u32 = 32;
/// Cekirdek imajinin (ham bellek blogu) basladigi sektor.
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
pub const KERNEL_BLOB_SECTOR: u32 = BOOT_AREA_SECTOR + STAGE2_SECTORS;

/// Veri bloklarinin basladigi sektor. Onyukleyici alaninin hemen ardindan,
/// 2 MiB sinirinda baslar.
const DATA_START_SECTOR: u32 = 4096;

/// Dosya basina dogrudan blok isaretcisi sayisi.
pub const DIRECT_BLOCKS: usize = 40;
pub const MAX_FILE_SIZE: usize = DIRECT_BLOCKS * BLOCK_SIZE;
pub const MAX_NAME: usize = 64;

pub const KIND_FREE: u32 = 0;
pub const KIND_FILE: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    NoDevice,
    NoPartition,
    NotFormatted,
    NotMounted,
    Io,
    Full,
    TooManyFiles,
    NameTooLong,
    FileTooLarge,
    NotFound,
    Exists,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SuperBlock {
    magic: u32,
    version: u32,
    total_blocks: u32,
    free_blocks: u32,
    inode_count: u32,
    file_count: u32,
    block_size: u32,
    data_start: u32,
    label: [u8; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Inode {
    kind: u32,
    size: u32,
    /// Olusturma/degistirme ani -- **Unix zamani** (RTC'den).
    /// RTC yoksa 0 kalir; listelerde "tarih yok" olarak gosterilir.
    mtime: u32,
    flags: u32,
    name: [u8; MAX_NAME],
    blocks: [u32; DIRECT_BLOCKS],
    _pad: [u8; INODE_SIZE - 16 - MAX_NAME - DIRECT_BLOCKS * 4],
}

impl Inode {
    const fn empty() -> Self {
        Inode {
            kind: KIND_FREE,
            size: 0,
            mtime: 0,
            flags: 0,
            name: [0; MAX_NAME],
            blocks: [0; DIRECT_BLOCKS],
            _pad: [0; INODE_SIZE - 16 - MAX_NAME - DIRECT_BLOCKS * 4],
        }
    }
}

// Yerlesimin derleme zamaninda dogrulanmasi: bir alan eklenip pad
// guncellenmezse disk bicimi sessizce bozulurdu.
const _: () = assert!(core::mem::size_of::<Inode>() == INODE_SIZE);
const _: () = assert!(core::mem::size_of::<SuperBlock>() <= SECTOR_SIZE);

// --- Bellek ici onbellek ---
static mut INODES: [Inode; MAX_INODES] = [Inode::empty(); MAX_INODES];
static mut BITMAP: [u8; BITMAP_SECTORS as usize * SECTOR_SIZE] =
    [0; BITMAP_SECTORS as usize * SECTOR_SIZE];
static mut SCRATCH: [u8; BLOCK_SIZE] = [0; BLOCK_SIZE];

static MOUNTED: AtomicBool = AtomicBool::new(false);
static PART_LBA: AtomicU32 = AtomicU32::new(0);
static TOTAL_BLOCKS: AtomicU32 = AtomicU32::new(0);
static FREE_BLOCKS: AtomicU32 = AtomicU32::new(0);
static WRITES: AtomicU32 = AtomicU32::new(0);

fn part_lba() -> u32 {
    PART_LBA.load(Ordering::Relaxed)
}

fn io(result: Result<(), block::BlockError>) -> Result<(), FsError> {
    result.map_err(|_| FsError::Io)
}

/// Bolumun kapasitesine gore kac veri blogu sigdigini hesaplar.
fn capacity_blocks(partition_sectors: u32) -> u32 {
    if partition_sectors <= DATA_START_SECTOR {
        return 0;
    }
    let usable = (partition_sectors - DATA_START_SECTOR) / SECTORS_PER_BLOCK;
    usable.min(MAX_BLOCKS as u32)
}

// --- Ham sektor G/C ---

fn read_super() -> Result<SuperBlock, FsError> {
    let mut sector = [0u8; SECTOR_SIZE];
    io(block::read_sector(part_lba(), &mut sector))?;
    Ok(unsafe { core::ptr::read_unaligned(sector.as_ptr() as *const SuperBlock) })
}

fn write_super(sb: &SuperBlock) -> Result<(), FsError> {
    let mut sector = [0u8; SECTOR_SIZE];
    unsafe { core::ptr::write_unaligned(sector.as_mut_ptr() as *mut SuperBlock, *sb) };
    io(block::write_sector(part_lba(), &sector))
}

/// Inode tablosunu diske yazar (32 sektor, 8'erli gruplar halinde).
fn flush_inodes() -> Result<(), FsError> {
    unsafe {
        let bytes = core::slice::from_raw_parts(
            core::ptr::addr_of!(INODES) as *const u8,
            MAX_INODES * INODE_SIZE,
        );
        for chunk in 0..(INODE_TABLE_SECTORS / 8) {
            let offset = chunk as usize * 8 * SECTOR_SIZE;
            io(block::write(
                part_lba() + INODE_TABLE_SECTOR + chunk * 8,
                8,
                &bytes[offset..offset + 8 * SECTOR_SIZE],
            ))?;
        }
    }
    Ok(())
}

fn load_inodes() -> Result<(), FsError> {
    unsafe {
        let bytes = core::slice::from_raw_parts_mut(
            core::ptr::addr_of_mut!(INODES) as *mut u8,
            MAX_INODES * INODE_SIZE,
        );
        for chunk in 0..(INODE_TABLE_SECTORS / 8) {
            let offset = chunk as usize * 8 * SECTOR_SIZE;
            io(block::read(
                part_lba() + INODE_TABLE_SECTOR + chunk * 8,
                8,
                &mut bytes[offset..offset + 8 * SECTOR_SIZE],
            ))?;
        }
    }
    Ok(())
}

fn flush_bitmap() -> Result<(), FsError> {
    unsafe {
        let bytes = core::slice::from_raw_parts(
            core::ptr::addr_of!(BITMAP) as *const u8,
            BITMAP_SECTORS as usize * SECTOR_SIZE,
        );
        io(block::write(
            part_lba() + BITMAP_SECTOR,
            BITMAP_SECTORS as u8,
            bytes,
        ))
    }
}

fn load_bitmap() -> Result<(), FsError> {
    unsafe {
        let bytes = core::slice::from_raw_parts_mut(
            core::ptr::addr_of_mut!(BITMAP) as *mut u8,
            BITMAP_SECTORS as usize * SECTOR_SIZE,
        );
        io(block::read(
            part_lba() + BITMAP_SECTOR,
            BITMAP_SECTORS as u8,
            bytes,
        ))
    }
}

fn read_block(index: u32, buf: &mut [u8]) -> Result<(), FsError> {
    io(block::read(
        part_lba() + DATA_START_SECTOR + index * SECTORS_PER_BLOCK,
        SECTORS_PER_BLOCK as u8,
        buf,
    ))
}

fn write_block(index: u32, buf: &[u8]) -> Result<(), FsError> {
    WRITES.fetch_add(1, Ordering::Relaxed);
    io(block::write(
        part_lba() + DATA_START_SECTOR + index * SECTORS_PER_BLOCK,
        SECTORS_PER_BLOCK as u8,
        buf,
    ))
}

// --- Bitmap ---

fn bit_get(index: u32) -> bool {
    unsafe {
        let bitmap = core::ptr::addr_of!(BITMAP) as *const u8;
        bitmap.add(index as usize / 8).read() & (1 << (index % 8)) != 0
    }
}

fn bit_set(index: u32, value: bool) {
    unsafe {
        let bitmap = core::ptr::addr_of_mut!(BITMAP) as *mut u8;
        let byte = bitmap.add(index as usize / 8);
        let mask = 1u8 << (index % 8);
        if value {
            byte.write(byte.read() | mask);
        } else {
            byte.write(byte.read() & !mask);
        }
    }
}

fn alloc_block() -> Option<u32> {
    let total = TOTAL_BLOCKS.load(Ordering::Relaxed);
    for i in 0..total {
        if !bit_get(i) {
            bit_set(i, true);
            FREE_BLOCKS.fetch_sub(1, Ordering::Relaxed);
            return Some(i);
        }
    }
    None
}

fn free_block(index: u32) {
    if bit_get(index) {
        bit_set(index, false);
        FREE_BLOCKS.fetch_add(1, Ordering::Relaxed);
    }
}

// --- Bicimlendirme / baglama ---

/// Bolumu TCMKFS olarak bicimlendirir. **Mevcut veriyi yok eder.**
pub fn format(label: &str) -> Result<(), FsError> {
    let (lba, sectors) = partition::tcmkfs().ok_or(FsError::NoPartition)?;
    if !block::available() {
        return Err(FsError::NoDevice);
    }
    PART_LBA.store(lba, Ordering::Relaxed);

    let total = capacity_blocks(sectors);
    TOTAL_BLOCKS.store(total, Ordering::Relaxed);
    FREE_BLOCKS.store(total, Ordering::Relaxed);

    unsafe {
        let inodes = core::ptr::addr_of_mut!(INODES) as *mut Inode;
        for i in 0..MAX_INODES {
            inodes.add(i).write(Inode::empty());
        }
        let bitmap = core::ptr::addr_of_mut!(BITMAP) as *mut u8;
        core::ptr::write_bytes(bitmap, 0, BITMAP_SECTORS as usize * SECTOR_SIZE);
    }

    let mut sb = SuperBlock {
        magic: MAGIC,
        version: VERSION,
        total_blocks: total,
        free_blocks: total,
        inode_count: MAX_INODES as u32,
        file_count: 0,
        block_size: BLOCK_SIZE as u32,
        data_start: DATA_START_SECTOR,
        label: [0; 16],
    };
    for (i, b) in label.bytes().take(15).enumerate() {
        sb.label[i] = b;
    }

    flush_inodes()?;
    flush_bitmap()?;
    write_super(&sb)?;

    MOUNTED.store(true, Ordering::Relaxed);
    Ok(())
}

/// Bolumu baglar; bicimlendirilmemisse `NotFormatted`.
pub fn mount() -> Result<(), FsError> {
    let (lba, sectors) = partition::tcmkfs().ok_or(FsError::NoPartition)?;
    if !block::available() {
        return Err(FsError::NoDevice);
    }
    PART_LBA.store(lba, Ordering::Relaxed);

    let sb = read_super()?;
    if sb.magic != MAGIC {
        return Err(FsError::NotFormatted);
    }
    // Bolum buyudugunde superblock'taki eski kapasiteye takilmamak icin
    // gercek kapasiteyle sinirlanir.
    let total = sb.total_blocks.min(capacity_blocks(sectors));
    TOTAL_BLOCKS.store(total, Ordering::Relaxed);

    load_inodes()?;
    load_bitmap()?;

    // Bos blok sayisi bitmap'ten yeniden hesaplanir: superblock'a
    // guvenmek, temiz kapatilmamis bir sistemde yanlis sayiya yol acardi.
    let mut free = 0;
    for i in 0..total {
        if !bit_get(i) {
            free += 1;
        }
    }
    FREE_BLOCKS.store(free, Ordering::Relaxed);

    MOUNTED.store(true, Ordering::Relaxed);
    Ok(())
}

pub fn mounted() -> bool {
    MOUNTED.load(Ordering::Relaxed)
}

/// Onbellekteki inode tablosunu, bitmap'i ve superblock'u diske yazar.
pub fn sync() -> Result<(), FsError> {
    if !mounted() {
        return Err(FsError::NotMounted);
    }
    flush_inodes()?;
    flush_bitmap()?;

    let mut sb = read_super()?;
    sb.free_blocks = FREE_BLOCKS.load(Ordering::Relaxed);
    sb.file_count = file_count() as u32;
    write_super(&sb)
}

// --- Inode erisimi ---

fn inode_ref(index: usize) -> Option<&'static mut Inode> {
    if index >= MAX_INODES {
        return None;
    }
    unsafe {
        let inodes = core::ptr::addr_of_mut!(INODES) as *mut Inode;
        Some(&mut *inodes.add(index))
    }
}

fn name_of(inode: &Inode) -> &str {
    let end = inode.name.iter().position(|b| *b == 0).unwrap_or(MAX_NAME);
    core::str::from_utf8(&inode.name[..end]).unwrap_or("")
}

/// `index`'teki dosyanin adi (statik omurlu: tablo `static`).
pub fn entry_name(index: usize) -> Option<&'static str> {
    let inode = inode_ref(index)?;
    if inode.kind != KIND_FILE {
        return None;
    }
    Some(name_of(inode))
}

/// Dosyanin son degisiklik ani (Unix zamani); 0 = bilinmiyor.
pub fn entry_mtime(index: usize) -> Option<u32> {
    let inode = inode_ref(index)?;
    if inode.kind != KIND_FILE {
        return None;
    }
    Some(inode.mtime)
}

pub fn entry_size(index: usize) -> Option<usize> {
    let inode = inode_ref(index)?;
    if inode.kind != KIND_FILE {
        return None;
    }
    Some(inode.size as usize)
}

pub fn lookup(name: &str) -> Option<usize> {
    for i in 0..MAX_INODES {
        if let Some(inode) = inode_ref(i) {
            if inode.kind == KIND_FILE && name_of(inode) == name {
                return Some(i);
            }
        }
    }
    None
}

pub fn file_count() -> usize {
    (0..MAX_INODES)
        .filter(|i| inode_ref(*i).map_or(false, |n| n.kind == KIND_FILE))
        .count()
}

/// Yeni bos dosya olusturur ve inode numarasini doner.
pub fn create(name: &str) -> Result<usize, FsError> {
    if !mounted() {
        return Err(FsError::NotMounted);
    }
    if name.is_empty() || name.len() >= MAX_NAME {
        return Err(FsError::NameTooLong);
    }
    if lookup(name).is_some() {
        return Err(FsError::Exists);
    }

    for i in 0..MAX_INODES {
        let inode = inode_ref(i).ok_or(FsError::TooManyFiles)?;
        if inode.kind != KIND_FREE {
            continue;
        }
        *inode = Inode::empty();
        inode.kind = KIND_FILE;
        inode.mtime = crate::level0a::drivers::rtc::unix_time();
        for (j, b) in name.bytes().enumerate() {
            inode.name[j] = b;
        }
        flush_inodes()?;
        return Ok(i);
    }
    Err(FsError::TooManyFiles)
}

/// Dosyayi ve bloklarini siler.
pub fn remove(index: usize) -> Result<(), FsError> {
    if !mounted() {
        return Err(FsError::NotMounted);
    }
    let inode = inode_ref(index).ok_or(FsError::NotFound)?;
    if inode.kind != KIND_FILE {
        return Err(FsError::NotFound);
    }

    let used = block_count(inode.size as usize);
    for i in 0..used {
        free_block(inode.blocks[i]);
    }
    *inode = Inode::empty();

    flush_inodes()?;
    flush_bitmap()?;
    Ok(())
}

fn block_count(size: usize) -> usize {
    size.div_ceil(BLOCK_SIZE)
}

/// `offset`'ten itibaren okur; okunan bayt sayisini doner.
pub fn read(index: usize, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
    if !mounted() {
        return Err(FsError::NotMounted);
    }
    let inode = inode_ref(index).ok_or(FsError::NotFound)?;
    if inode.kind != KIND_FILE {
        return Err(FsError::NotFound);
    }

    let size = inode.size as usize;
    if offset >= size {
        return Ok(0);
    }
    let to_read = (size - offset).min(buf.len());

    let mut done = 0;
    while done < to_read {
        let position = offset + done;
        let block_index = position / BLOCK_SIZE;
        let in_block = position % BLOCK_SIZE;
        let chunk = (BLOCK_SIZE - in_block).min(to_read - done);

        unsafe {
            let scratch = core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(SCRATCH) as *mut u8,
                BLOCK_SIZE,
            );
            read_block(inode.blocks[block_index], scratch)?;
            buf[done..done + chunk].copy_from_slice(&scratch[in_block..in_block + chunk]);
        }
        done += chunk;
    }

    Ok(done)
}

/// `offset`'ten itibaren yazar; gerekiyorsa dosyayi buyutur ve blok tahsis
/// eder. Yazilan bayt sayisini doner.
pub fn write(index: usize, offset: usize, data: &[u8]) -> Result<usize, FsError> {
    if !mounted() {
        return Err(FsError::NotMounted);
    }
    let inode = inode_ref(index).ok_or(FsError::NotFound)?;
    if inode.kind != KIND_FILE {
        return Err(FsError::NotFound);
    }

    let end = offset + data.len();
    if end > MAX_FILE_SIZE {
        return Err(FsError::FileTooLarge);
    }

    // Gerekli bloklari once tahsis et: yarim yazip yerde birakmamak icin.
    let needed = block_count(end);
    let have = block_count(inode.size as usize);
    for i in have..needed {
        match alloc_block() {
            Some(block) => inode.blocks[i] = block,
            None => {
                // Geri al: bu cagride tahsis edilenleri birak.
                for j in have..i {
                    free_block(inode.blocks[j]);
                    inode.blocks[j] = 0;
                }
                return Err(FsError::Full);
            }
        }
    }

    let mut done = 0;
    while done < data.len() {
        let position = offset + done;
        let block_index = position / BLOCK_SIZE;
        let in_block = position % BLOCK_SIZE;
        let chunk = (BLOCK_SIZE - in_block).min(data.len() - done);

        unsafe {
            let scratch = core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(SCRATCH) as *mut u8,
                BLOCK_SIZE,
            );
            // Kismi blok yazimi: once oku, sonra uzerine yaz
            // (read-modify-write) -- yoksa blogun geri kalani cope doner.
            if chunk < BLOCK_SIZE {
                if block_index < have {
                    read_block(inode.blocks[block_index], scratch)?;
                } else {
                    scratch.fill(0);
                }
            }
            scratch[in_block..in_block + chunk].copy_from_slice(&data[done..done + chunk]);
            write_block(inode.blocks[block_index], scratch)?;
        }
        done += chunk;
    }

    if end > inode.size as usize {
        inode.size = end as u32;
    }
    inode.mtime = crate::level0a::drivers::rtc::unix_time();

    flush_inodes()?;
    flush_bitmap()?;
    Ok(done)
}

/// Dosyayi bastan yazar (var olan icerigi atar).
pub fn write_all(name: &str, data: &[u8]) -> Result<usize, FsError> {
    let index = match lookup(name) {
        Some(i) => {
            remove(i)?;
            create(name)?
        }
        None => create(name)?,
    };
    write(index, 0, data)
}

// --- Raporlama ---

pub fn total_blocks() -> u32 {
    TOTAL_BLOCKS.load(Ordering::Relaxed)
}

pub fn free_blocks() -> u32 {
    FREE_BLOCKS.load(Ordering::Relaxed)
}

pub fn used_kib() -> u32 {
    (total_blocks() - free_blocks()) * (BLOCK_SIZE as u32 / 1024)
}

pub fn total_kib() -> u32 {
    total_blocks() * (BLOCK_SIZE as u32 / 1024)
}

pub fn block_writes() -> u32 {
    WRITES.load(Ordering::Relaxed)
}

pub fn error_name(error: FsError) -> &'static str {
    match error {
        FsError::NoDevice => "disk yok",
        FsError::NoPartition => "TCMKFS bolumu yok",
        FsError::NotFormatted => "bicimlendirilmemis ('format onayla' ile bicimlendirin)",
        FsError::NotMounted => "dosya sistemi bagli degil",
        FsError::Io => "disk G/C hatasi",
        FsError::Full => "disk dolu",
        FsError::TooManyFiles => "inode tablosu dolu",
        FsError::NameTooLong => "dosya adi gecersiz veya cok uzun",
        FsError::FileTooLarge => "dosya cok buyuk (azami 160 KiB)",
        FsError::NotFound => "dosya bulunamadi",
        FsError::Exists => "dosya zaten var (salt okunur olabilir)",
    }
}
