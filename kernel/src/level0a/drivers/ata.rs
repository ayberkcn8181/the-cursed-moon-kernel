//! ATA PIO disk surucusu (doc S.2.2.C "Suruculer: Disk (ATA/AHCI)").
//!
//! Neden ATA PIO ile baslandi: port I/O disinda hicbir sey gerektirmez --
//! PCI taramasi, DMA, kesme yonetimi yok. Hem QEMU'da hem gercek (eski)
//! donanimda calisir. Yavastir (sektor basina 256 `in ax, dx`) ama dosya
//! sistemi ve kurulum icin fazlasiyla yeterlidir.
//!
//! Surucu **yoklamali** (polling) calisir; IRQ14 baglanmaz. Kesmesiz olmasi
//! bilincli: disk erisimi su an yalnizca cekirdek baglamindan, kisa ve
//! senkron sekilde yapiliyor. IRQ tabanli asenkron G/C, blok kuyrugu ve
//! surec engelleme (`TaskState::Blocked`) gerektirir -- Faz 9+ konusu.
//!
//! LBA28 kullanilir: 2^28 sektor = 128 GiB adresleme. LBA48 gerektiginde
//! ayni yapiya eklenebilir.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::arch::cpu::{inb, inw, outb, outw};
use crate::level0a::drivers::block::{BlockError, SECTOR_SIZE};

// Birincil ATA kanali.
const DATA: u16 = 0x1F0;
const FEATURES: u16 = 0x1F1;
const SECTOR_COUNT: u16 = 0x1F2;
const LBA_LOW: u16 = 0x1F3;
const LBA_MID: u16 = 0x1F4;
const LBA_HIGH: u16 = 0x1F5;
const DRIVE: u16 = 0x1F6;
const STATUS: u16 = 0x1F7;
const COMMAND: u16 = 0x1F7;
/// Alternatif durum: okumak kesme bayragini TEMIZLEMEZ; gecikme icin uygun.
const ALT_STATUS: u16 = 0x3F6;

const STATUS_ERR: u8 = 0x01;
const STATUS_DRQ: u8 = 0x08;
const STATUS_DF: u8 = 0x20;
const STATUS_DRDY: u8 = 0x40;
const STATUS_BSY: u8 = 0x80;

const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_FLUSH_CACHE: u8 = 0xE7;
const CMD_IDENTIFY: u8 = 0xEC;

/// Yoklama siniri. Gercek bir diskin en kotu durum yanit suresi uzundur ama
/// burada takilip kalmak yerine hata dondurmek yeglenir: disk arizasi
/// sistemi kilitlememelidir.
const SPIN_LIMIT: u32 = 5_000_000;

static PRESENT: AtomicBool = AtomicBool::new(false);
static TOTAL_SECTORS: AtomicU32 = AtomicU32::new(0);
static READS: AtomicU32 = AtomicU32::new(0);
static WRITES: AtomicU32 = AtomicU32::new(0);
static ERRORS: AtomicU32 = AtomicU32::new(0);

/// Model dizesi (IDENTIFY'dan, 40 karakter).
static mut MODEL: [u8; 40] = [b' '; 40];

/// Surucu/kanal secildikten sonra gereken ~400 ns bekleme: alternatif durum
/// dort kez okunur (her okuma bir ATA saat cevrimi surer).
fn io_delay() {
    for _ in 0..4 {
        unsafe { inb(ALT_STATUS) };
    }
}

fn wait_while_busy() -> Result<u8, BlockError> {
    for _ in 0..SPIN_LIMIT {
        let status = unsafe { inb(STATUS) };
        if status & STATUS_BSY == 0 {
            return Ok(status);
        }
    }
    Err(BlockError::Timeout)
}

/// BSY dususunu ve DRQ yukselisini bekler; ERR/DF gorurse hata doner.
fn wait_for_data() -> Result<(), BlockError> {
    for _ in 0..SPIN_LIMIT {
        let status = unsafe { inb(STATUS) };
        if status & STATUS_BSY != 0 {
            continue;
        }
        if status & (STATUS_ERR | STATUS_DF) != 0 {
            ERRORS.fetch_add(1, Ordering::Relaxed);
            return Err(BlockError::DeviceError);
        }
        if status & STATUS_DRQ != 0 {
            return Ok(());
        }
    }
    Err(BlockError::Timeout)
}

/// Diski tanir; bulunursa sektor sayisini ve modelini kaydeder.
///
/// # Safety
/// Yalnizca acilista, kesmeler kapaliyken cagrilmalidir.
pub unsafe fn init() -> bool {
    // Birincil master'i sec.
    outb(DRIVE, 0xA0);
    io_delay();
    outb(SECTOR_COUNT, 0);
    outb(LBA_LOW, 0);
    outb(LBA_MID, 0);
    outb(LBA_HIGH, 0);
    outb(COMMAND, CMD_IDENTIFY);
    io_delay();

    // Durum 0 ise bu kanalda surucu yok.
    if inb(STATUS) == 0 {
        return false;
    }

    if wait_while_busy().is_err() {
        return false;
    }

    // LBA_MID/HIGH sifir degilse cihaz ATA degil (ATAPI/SATA imzasi).
    if inb(LBA_MID) != 0 || inb(LBA_HIGH) != 0 {
        return false;
    }

    if wait_for_data().is_err() {
        return false;
    }

    let mut identify = [0u16; 256];
    for word in identify.iter_mut() {
        *word = inw(DATA);
    }

    // Kelime 60-61: LBA28 toplam sektor sayisi.
    let sectors = (identify[60] as u32) | ((identify[61] as u32) << 16);
    TOTAL_SECTORS.store(sectors, Ordering::Relaxed);

    // Kelime 27-46: model dizesi, her kelimede iki karakter TERS sirada.
    let model = core::ptr::addr_of_mut!(MODEL) as *mut u8;
    for i in 0..20 {
        let word = identify[27 + i];
        model.add(i * 2).write((word >> 8) as u8);
        model.add(i * 2 + 1).write((word & 0xFF) as u8);
    }

    PRESENT.store(true, Ordering::Relaxed);
    true
}

fn setup_lba(lba: u32, count: u8) -> Result<(), BlockError> {
    if !PRESENT.load(Ordering::Relaxed) {
        return Err(BlockError::NoDevice);
    }
    let total = TOTAL_SECTORS.load(Ordering::Relaxed);
    if lba.checked_add(count as u32).map_or(true, |end| end > total) {
        return Err(BlockError::OutOfRange);
    }
    if lba >= 1 << 28 {
        return Err(BlockError::OutOfRange);
    }

    wait_while_busy()?;
    unsafe {
        // 0xE0 = LBA modu, master; alt dort bit LBA'nin 27-24. bitleri.
        outb(DRIVE, 0xE0 | ((lba >> 24) & 0x0F) as u8);
        io_delay();
        outb(FEATURES, 0);
        outb(SECTOR_COUNT, count);
        outb(LBA_LOW, (lba & 0xFF) as u8);
        outb(LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(LBA_HIGH, ((lba >> 16) & 0xFF) as u8);
    }
    Ok(())
}

/// `count` sektoru `lba`'dan okur. `buf` en az `count * 512` bayt olmalidir.
pub fn read(lba: u32, count: u8, buf: &mut [u8]) -> Result<(), BlockError> {
    if count == 0 {
        return Ok(());
    }
    if buf.len() < count as usize * SECTOR_SIZE {
        return Err(BlockError::BufferTooSmall);
    }

    setup_lba(lba, count)?;
    unsafe { outb(COMMAND, CMD_READ_SECTORS) };

    for sector in 0..count as usize {
        wait_for_data()?;
        let base = sector * SECTOR_SIZE;
        for word in 0..SECTOR_SIZE / 2 {
            let value = unsafe { inw(DATA) };
            buf[base + word * 2] = (value & 0xFF) as u8;
            buf[base + word * 2 + 1] = (value >> 8) as u8;
        }
    }

    READS.fetch_add(count as u32, Ordering::Relaxed);
    Ok(())
}

/// `count` sektoru `lba`'ya yazar ve disk onbellegini bosaltir.
pub fn write(lba: u32, count: u8, buf: &[u8]) -> Result<(), BlockError> {
    if count == 0 {
        return Ok(());
    }
    if buf.len() < count as usize * SECTOR_SIZE {
        return Err(BlockError::BufferTooSmall);
    }

    setup_lba(lba, count)?;
    unsafe { outb(COMMAND, CMD_WRITE_SECTORS) };

    for sector in 0..count as usize {
        wait_for_data()?;
        let base = sector * SECTOR_SIZE;
        for word in 0..SECTOR_SIZE / 2 {
            let value = (buf[base + word * 2] as u16) | ((buf[base + word * 2 + 1] as u16) << 8);
            unsafe { outw(DATA, value) };
        }
    }

    // Onbellek bosaltma olmadan yazilanlar guc kesintisinde kaybolabilir.
    wait_while_busy()?;
    unsafe { outb(COMMAND, CMD_FLUSH_CACHE) };
    let status = wait_while_busy()?;
    if status & (STATUS_ERR | STATUS_DF) != 0 {
        ERRORS.fetch_add(1, Ordering::Relaxed);
        return Err(BlockError::DeviceError);
    }

    WRITES.fetch_add(count as u32, Ordering::Relaxed);
    Ok(())
}

pub fn total_sectors() -> u32 {
    TOTAL_SECTORS.load(Ordering::Relaxed)
}

pub fn model() -> &'static str {
    unsafe {
        let model = core::ptr::addr_of!(MODEL) as *const u8;
        let bytes = core::slice::from_raw_parts(model, 40);
        let end = bytes
            .iter()
            .rposition(|b| *b != b' ' && *b != 0)
            .map_or(0, |i| i + 1);
        core::str::from_utf8(&bytes[..end]).unwrap_or("?")
    }
}

pub fn stats() -> (u32, u32, u32) {
    (
        READS.load(Ordering::Relaxed),
        WRITES.load(Ordering::Relaxed),
        ERRORS.load(Ordering::Relaxed),
    )
}

/// Surucunun hazir olup olmadigini kontrol eder (kabuk `disk` komutu).
pub fn drive_ready() -> bool {
    PRESENT.load(Ordering::Relaxed) && unsafe { inb(STATUS) } & STATUS_DRDY != 0
}
