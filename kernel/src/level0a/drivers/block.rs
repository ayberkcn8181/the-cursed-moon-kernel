//! Blok aygit katmani: dosya sistemi ile disk surucusu arasindaki ince
//! soyutlama.
//!
//! Ust katmanlar (TCMKFS, bolum tablosu ayristirici) yalnizca "LBA'dan
//! sektor oku/yaz" bilir; ATA mi, ileride AHCI/virtio mu oldugu onlari
//! ilgilendirmez. Surucu degistirmek dosya sistemini etkilemez -- doc
//! S.15'in "donanima ozel kod izole" ilkesinin disk ayagi.
//!
//! Kayit su an tek aygitliktir (birincil ATA master). Bolum bazli
//! adresleme `partition` modulunde ele alinir.

use core::sync::atomic::{AtomicUsize, Ordering};

pub const SECTOR_SIZE: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    /// Kayitli bir blok aygiti yok.
    NoDevice,
    /// Istenen LBA aygitin disinda.
    OutOfRange,
    /// Tampon `count * 512` bayttan kucuk.
    BufferTooSmall,
    /// Surucu ERR/DF bildirdi.
    DeviceError,
    /// Yoklama siniri asildi.
    Timeout,
}

/// Kayitli surucunun fonksiyon tablosu.
///
/// `dyn Trait` yerine fonksiyon isaretcileri: `no_std` + ayirmasiz
/// ortamda static bir vtable en ucuz ve en ongorulebilir cozum.
#[derive(Clone, Copy)]
pub struct Driver {
    pub name: &'static str,
    pub read: fn(lba: u32, count: u8, buf: &mut [u8]) -> Result<(), BlockError>,
    pub write: fn(lba: u32, count: u8, buf: &[u8]) -> Result<(), BlockError>,
    pub sectors: fn() -> u32,
}

static mut DRIVER: Option<Driver> = None;
/// Katman uzerinden gecen toplam sektor sayisi (kabuk `disk` komutu).
static SECTORS_READ: AtomicUsize = AtomicUsize::new(0);
static SECTORS_WRITTEN: AtomicUsize = AtomicUsize::new(0);

/// Aygiti tanir ve kaydeder; bulunursa `true`.
///
/// # Safety
/// Acilista, kesmeler kapaliyken bir kez cagrilmalidir.
pub unsafe fn init() -> bool {
    if !crate::level0a::drivers::ata::init() {
        return false;
    }

    DRIVER = Some(Driver {
        name: "ata-pio",
        read: crate::level0a::drivers::ata::read,
        write: crate::level0a::drivers::ata::write,
        sectors: crate::level0a::drivers::ata::total_sectors,
    });
    true
}

fn driver() -> Result<Driver, BlockError> {
    unsafe {
        let slot = core::ptr::addr_of!(DRIVER) as *const Option<Driver>;
        (*slot).ok_or(BlockError::NoDevice)
    }
}

pub fn available() -> bool {
    driver().is_ok()
}

pub fn name() -> &'static str {
    driver().map(|d| d.name).unwrap_or("yok")
}

pub fn sector_count() -> u32 {
    driver().map(|d| (d.sectors)()).unwrap_or(0)
}

/// Aygit kapasitesi (MiB).
pub fn capacity_mib() -> u32 {
    (sector_count() / 2) / 1024
}

pub fn read(lba: u32, count: u8, buf: &mut [u8]) -> Result<(), BlockError> {
    let d = driver()?;
    (d.read)(lba, count, buf)?;
    SECTORS_READ.fetch_add(count as usize, Ordering::Relaxed);
    Ok(())
}

pub fn write(lba: u32, count: u8, buf: &[u8]) -> Result<(), BlockError> {
    let d = driver()?;
    (d.write)(lba, count, buf)?;
    SECTORS_WRITTEN.fetch_add(count as usize, Ordering::Relaxed);
    Ok(())
}

/// Tek sektor okuma kolayligi.
pub fn read_sector(lba: u32, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), BlockError> {
    read(lba, 1, buf)
}

pub fn write_sector(lba: u32, buf: &[u8; SECTOR_SIZE]) -> Result<(), BlockError> {
    write(lba, 1, buf)
}

pub fn stats() -> (usize, usize) {
    (
        SECTORS_READ.load(Ordering::Relaxed),
        SECTORS_WRITTEN.load(Ordering::Relaxed),
    )
}
