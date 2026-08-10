//! Diske kurulum: acilis sektorunu TCMK'nin kendi onyukleyicisiyle
//! degistirir.
//!
//! ## Ne yapiyor, ne yapmiyor
//!
//! `make disk` ile uretilen imaj bir **kurulum ortamidir**: icinde hem
//! GRUB'un actigi ISO bolgesi hem de TCMK'nin kendi onyukleyicisi
//! (2. asama + cekirdek blogu, bolumun 40..4095 sektorleri) hazir durur.
//! Sistem ilk acilista GRUB uzerinden gelir.
//!
//! `install` komutu MBR'nin **ilk 446 baytini** TCMK'nin 1. asamasiyla
//! degistirir ve 2. asamanin LBA'sini oraya yamalar. Bundan sonra disk
//! GRUB'a hic ugramadan acilir; TCMK kendi kendini yukler.
//!
//! Bolum tablosu (446..510) ve imza (510..512) **korunur** -- aksi halde
//! diskteki her sey erisilemez olurdu.
//!
//! ## Neden cekirdegi kurulum yazmiyor
//!
//! Calisan cekirdegin kendi ELF imajina erisimi yoktur (kendi kendini
//! iceremez). Bu yuzden cekirdek blogunu imaj ureticisi (`tools/make_disk.py`)
//! yazar; kurulum yalnizca acilis zincirini devreye alir. Farkli bir
//! diske kurulum (ikinci ATA aygiti) ancak calisan imaji dosya olarak
//! tasidigimizda mumkun olur.

use crate::level0a::core::tcmkfs;
use crate::level0a::drivers::block::{self, SECTOR_SIZE};
use crate::level0a::drivers::partition;

/// 1. asama ikilisi -- `boot/tcmkboot`, `make bootloader` ile uretilir.
static STAGE1: &[u8] = include_bytes!("../../../build/boot/stage1.bin");

/// MBR'de bolum tablosunun basladigi ofset; kod bunu gecemez.
const PARTITION_TABLE_OFFSET: usize = 446;

/// 1. asamadaki parametre bloku (bkz. `boot/tcmkboot/src/bin/stage1.rs`).
const PARAM_MAGIC_OFFSET: usize = 3;
const PARAM_MAGIC: &[u8; 6] = b"TCMKB1";
const PARAM_LBA_OFFSET: usize = 9;
const PARAM_COUNT_OFFSET: usize = 13;

/// 2. asamanin imzasi -- kurulumdan once varligi dogrulanir.
const STAGE2_MAGIC_OFFSET: usize = 3;
const STAGE2_MAGIC: &[u8; 6] = b"TCMKB2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallError {
    NoDevice,
    NoPartition,
    Stage1TooLarge,
    Stage2Missing,
    BadMbr,
    Io,
}

pub fn error_name(error: InstallError) -> &'static str {
    match error {
        InstallError::NoDevice => "disk yok",
        InstallError::NoPartition => "TCMKFS bolumu yok",
        InstallError::Stage1TooLarge => "1. asama 446 bayti asiyor",
        InstallError::Stage2Missing => "2. asama diskte bulunamadi (imaj eksik)",
        InstallError::BadMbr => "MBR imzasi gecersiz",
        InstallError::Io => "disk G/C hatasi",
    }
}

/// Kurulumun ayrintilarini kabuga bildirmek icin.
pub struct Report {
    pub stage1_bytes: usize,
    pub stage2_lba: u32,
    pub stage2_sectors: u32,
    pub blob_lba: u32,
}

/// Cekirdek imajinin diskteki yeri; imaj ureticisi oraya yazar.
pub fn kernel_blob_lba() -> Option<u32> {
    partition::tcmkfs().map(|(lba, _)| lba + tcmkfs::KERNEL_BLOB_SECTOR)
}

/// Onyukleyicinin diskte hazir olup olmadigini dogrular.
pub fn stage2_present() -> bool {
    let (part_lba, _) = match partition::tcmkfs() {
        Some(p) => p,
        None => return false,
    };
    let mut sector = [0u8; SECTOR_SIZE];
    if block::read_sector(part_lba + tcmkfs::BOOT_AREA_SECTOR, &mut sector).is_err() {
        return false;
    }
    &sector[STAGE2_MAGIC_OFFSET..STAGE2_MAGIC_OFFSET + 6] == STAGE2_MAGIC
}

/// Acilis sektorunu TCMK'nin onyukleyicisiyle degistirir.
pub fn install() -> Result<Report, InstallError> {
    if !block::available() {
        return Err(InstallError::NoDevice);
    }
    let (part_lba, _) = partition::tcmkfs().ok_or(InstallError::NoPartition)?;

    if STAGE1.len() > PARTITION_TABLE_OFFSET {
        return Err(InstallError::Stage1TooLarge);
    }
    if !stage2_present() {
        return Err(InstallError::Stage2Missing);
    }

    // Mevcut MBR'yi oku: bolum tablosu ve imza korunacak.
    let mut mbr = [0u8; SECTOR_SIZE];
    block::read_sector(0, &mut mbr).map_err(|_| InstallError::Io)?;
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return Err(InstallError::BadMbr);
    }

    // Kod alanini temizle ve 1. asamayi yerlestir.
    for byte in mbr[..PARTITION_TABLE_OFFSET].iter_mut() {
        *byte = 0;
    }
    mbr[..STAGE1.len()].copy_from_slice(STAGE1);

    // Imza dogrulamasi: yanlis bir ikili yazmak diski acilamaz yapar.
    if &mbr[PARAM_MAGIC_OFFSET..PARAM_MAGIC_OFFSET + 6] != PARAM_MAGIC {
        return Err(InstallError::Stage2Missing);
    }

    // 2. asamanin yerini yamala (mutlak LBA).
    let stage2_lba = part_lba + tcmkfs::BOOT_AREA_SECTOR;
    mbr[PARAM_LBA_OFFSET..PARAM_LBA_OFFSET + 4].copy_from_slice(&stage2_lba.to_le_bytes());
    mbr[PARAM_COUNT_OFFSET..PARAM_COUNT_OFFSET + 2]
        .copy_from_slice(&(tcmkfs::STAGE2_SECTORS as u16).to_le_bytes());

    block::write_sector(0, &mbr).map_err(|_| InstallError::Io)?;

    Ok(Report {
        stage1_bytes: STAGE1.len(),
        stage2_lba,
        stage2_sectors: tcmkfs::STAGE2_SECTORS,
        blob_lba: part_lba + tcmkfs::KERNEL_BLOB_SECTOR,
    })
}
