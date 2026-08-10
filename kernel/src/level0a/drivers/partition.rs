//! MBR bolum tablosu ayristirici.
//!
//! TCMK'nin disk imaji tek bir dosyadir ama iki bolgeden olusur:
//!
//!   bolum 1 -- GRUB'un actigi ISO9660 bolgesi (cekirdek imaji burada)
//!   bolum 2 -- TCMKFS: kalici, yazilabilir veri bolumu
//!
//! Cekirdek yazilabilir bolumu **sabit bir LBA'ya gomerek degil**, bolum
//! tablosundan bularak acar. Imajin duzeni degistiginde cekirdegi yeniden
//! derlemek gerekmez; bu, "diske kurulum" adiminin onkosuludur.

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::level0a::drivers::block::{self, SECTOR_SIZE};

/// TCMKFS bolumleri icin secilen tur bayti. 0x7F "deneysel / uretici
/// tanimli" araligindadir; standart bir dosya sistemiyle catismaz.
pub const TYPE_TCMKFS: u8 = 0x7F;

pub const MAX_PARTITIONS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    pub index: usize,
    pub kind: u8,
    pub bootable: bool,
    pub start_lba: u32,
    pub sectors: u32,
}

impl Partition {
    const fn empty() -> Self {
        Partition {
            index: 0,
            kind: 0,
            bootable: false,
            start_lba: 0,
            sectors: 0,
        }
    }
}

static mut TABLE: [Partition; MAX_PARTITIONS] = [Partition::empty(); MAX_PARTITIONS];
static COUNT: AtomicUsize = AtomicUsize::new(0);
static TCMKFS_LBA: AtomicU32 = AtomicU32::new(0);
static TCMKFS_SECTORS: AtomicU32 = AtomicU32::new(0);

/// MBR'yi okur ve bolum tablosunu doldurur; bulunan bolum sayisini doner.
pub fn scan() -> usize {
    if !block::available() {
        return 0;
    }

    let mut mbr = [0u8; SECTOR_SIZE];
    if block::read_sector(0, &mut mbr).is_err() {
        return 0;
    }

    // Boot imzasi 0x55AA olmayan bir sektorde bolum tablosu aramak,
    // rastgele baytlari bolum sanmak demektir.
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        crate::println!("[LEVEL-0a] disk: MBR imzasi yok (0x55AA), bolum taranmadi.");
        return 0;
    }

    let mut found = 0;
    for i in 0..MAX_PARTITIONS {
        let base = 446 + i * 16;
        let kind = mbr[base + 4];
        if kind == 0 {
            continue;
        }

        let start = u32::from_le_bytes([
            mbr[base + 8],
            mbr[base + 9],
            mbr[base + 10],
            mbr[base + 11],
        ]);
        let sectors = u32::from_le_bytes([
            mbr[base + 12],
            mbr[base + 13],
            mbr[base + 14],
            mbr[base + 15],
        ]);
        if sectors == 0 {
            continue;
        }

        let partition = Partition {
            index: i,
            kind,
            bootable: mbr[base] == 0x80,
            start_lba: start,
            sectors,
        };

        unsafe {
            let table = core::ptr::addr_of_mut!(TABLE) as *mut Partition;
            table.add(found).write(partition);
        }
        found += 1;

        if kind == TYPE_TCMKFS {
            TCMKFS_LBA.store(start, Ordering::Relaxed);
            TCMKFS_SECTORS.store(sectors, Ordering::Relaxed);
        }
    }

    COUNT.store(found, Ordering::Relaxed);
    found
}

pub fn count() -> usize {
    COUNT.load(Ordering::Relaxed)
}

pub fn get(index: usize) -> Option<Partition> {
    if index >= COUNT.load(Ordering::Relaxed) {
        return None;
    }
    unsafe {
        let table = core::ptr::addr_of!(TABLE) as *const Partition;
        Some(*table.add(index))
    }
}

/// TCMKFS bolumunun (baslangic LBA, sektor sayisi) bilgisi; yoksa `None`.
pub fn tcmkfs() -> Option<(u32, u32)> {
    let lba = TCMKFS_LBA.load(Ordering::Relaxed);
    if lba == 0 {
        None
    } else {
        Some((lba, TCMKFS_SECTORS.load(Ordering::Relaxed)))
    }
}

/// Bolum turunun okunabilir adi (yaygin turler + TCMK'ninki).
pub fn type_name(kind: u8) -> &'static str {
    match kind {
        0x00 => "bos",
        0x0B | 0x0C => "FAT32",
        0x07 => "NTFS/exFAT",
        0x83 => "Linux",
        0x82 => "Linux swap",
        0xEE => "GPT koruyucu",
        0x17 | 0x96 => "ISO9660",
        // grub-mkrescue'nun hibrit imajda kullandigi tur: ayni dosya hem
        // CD hem sabit disk olarak acilabilsin diye.
        0xCD => "ISO hibrit",
        TYPE_TCMKFS => "TCMKFS",
        _ => "bilinmeyen",
    }
}
