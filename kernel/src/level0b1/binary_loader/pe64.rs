//! PE32+ (Windows, x86_64) yukleyici -- Binary Loader'in NT tarafinin
//! 64-bit surumu.
//!
//! ## PE32 ile farklar
//!
//! Bicim adi "PE32+"tir, "PE64" degil: dosya duzeni buyuk olcude ayni
//! kalir, degisen yalnizca **isaretci genisligine bagli alanlardir**.
//! Onemli olanlar:
//!
//! | | PE32 | PE32+ |
//! |---|---|---|
//! | optional header magic | `0x010B` | `0x020B` |
//! | `BaseOfData` alani | var (4 bayt) | **yok** |
//! | `ImageBase` | u32 | u64 |
//! | yigin/heap ayirmalari | u32 x4 | u64 x4 |
//! | veri dizinleri nerede | opt+96 | opt+112 |
//! | yeniden yerlesim turu | `HIGHLOW` (3) | `DIR64` (10) |
//! | ordinal isareti | `1 << 31` | `1 << 63` |
//! | IAT yuvasi | 4 bayt | 8 bayt |
//!
//! `BaseOfData`'nin kaybolmasi ile `ImageBase`'in 8 bayta cikmasi
//! birbirini goturur; bu yuzden veri dizinlerine kadar olan fark tam
//! **16 bayttir** (96 -> 112). Bu sayiyi yanlis almak, ithal ve
//! `.reloc` dizinlerini bambaska yerlerde aramak demektir -- ve hata
//! sessizdir: dizin RVA'lari sifir gorunur, program ithalsiz sanilir ve
//! ilk `call [IAT]`'te cakar.
//!
//! ## Cagri gelenegi
//!
//! Ithal edilen fonksiyonlar icin uretilen thunk **Win64** gelenegine
//! gore yazilir (register argumanlari golge alana dokulur); ayrintisi
//! `nt_subsystem::dll::emit_thunk`'te.

use crate::level0a::core::mmu;
use crate::level0b1::nt_subsystem::dll;

const DOS_MAGIC: u16 = 0x5A4D; // "MZ"
const PE_SIGNATURE: u32 = 0x0000_4550; // "PE\0\0"
const MACHINE_AMD64: u16 = 0x8664;
const PE32PLUS_MAGIC: u16 = 0x020B;

/// Veri dizinlerinin optional header icindeki basi. PE32'de 96, PE32+'ta
/// 112 -- bkz. dosya basindaki tablo.
const DIRECTORIES_AT: usize = 112;

const DIR_IMPORT: usize = 1;
const DIR_BASERELOC: usize = 5;
const REL_BASED_ABSOLUTE: u16 = 0;
/// PE32+'in tek anlamli yeniden yerlesim turu: hedefteki **64 bitlik**
/// mutlak adrese delta eklenir.
const REL_BASED_DIR64: u16 = 10;

const NAME_MAX: usize = 64;

/// Ordinal ile ithal isareti -- PE32'deki `1 << 31` degil, `1 << 63`.
const IMAGE_ORDINAL_FLAG64: u64 = 0x8000_0000_0000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeError {
    TooSmall,
    BadDosMagic,
    BadPeSignature,
    WrongArchitecture,
    /// Magic `0x020B` degil: 32-bit PE, x86_64 cekirdeginde calistirilamaz.
    NotPe32Plus,
    BadOptionalHeaderSize,
    SectionOutOfBounds,
    SectionOutsideUserMemory,
    RelocationOutOfBounds,
    ImportOutOfBounds,
    UnresolvedImport,
    ThunkAreaFull,
}

pub struct LoadedImage {
    pub entry: u64,
    pub end: u64,
}

fn read_u16(image: &[u8], off: usize) -> Option<u16> {
    let bytes = image.get(off..off + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(image: &[u8], off: usize) -> Option<u32> {
    let bytes = image.get(off..off + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(image: &[u8], off: usize) -> Option<u64> {
    let bytes = image.get(off..off + 8)?;
    let mut buf = [0u8; 8];
    buf.copy_from_slice(bytes);
    Some(u64::from_le_bytes(buf))
}

pub fn load(image: &[u8]) -> Result<LoadedImage, PeError> {
    // --- DOS basligi ---
    if read_u16(image, 0).ok_or(PeError::TooSmall)? != DOS_MAGIC {
        return Err(PeError::BadDosMagic);
    }
    let pe_off = read_u32(image, 0x3C).ok_or(PeError::TooSmall)? as usize;

    if read_u32(image, pe_off).ok_or(PeError::TooSmall)? != PE_SIGNATURE {
        return Err(PeError::BadPeSignature);
    }

    // --- COFF basligi (20 bayt) ---
    let coff = pe_off + 4;
    let machine = read_u16(image, coff).ok_or(PeError::TooSmall)?;
    if machine != MACHINE_AMD64 {
        return Err(PeError::WrongArchitecture);
    }
    let num_sections = read_u16(image, coff + 2).ok_or(PeError::TooSmall)? as usize;
    let size_of_optional = read_u16(image, coff + 16).ok_or(PeError::TooSmall)? as usize;
    // PE32+'ta veri dizinleri 112. bayttan baslar; en az bir dizin
    // okunabilmesi icin baslik bu kadar uzun olmalidir.
    if size_of_optional < DIRECTORIES_AT {
        return Err(PeError::BadOptionalHeaderSize);
    }

    // --- Optional header ---
    let opt = coff + 20;
    if read_u16(image, opt).ok_or(PeError::TooSmall)? != PE32PLUS_MAGIC {
        return Err(PeError::NotPe32Plus);
    }
    let entry_rva = read_u32(image, opt + 16).ok_or(PeError::TooSmall)? as usize;
    // PE32'de burada BaseOfData (opt+24) ve ImageBase (opt+28) vardi;
    // PE32+'ta BaseOfData yoktur ve ImageBase 8 bayt olarak opt+24'tedir.
    let image_base = read_u64(image, opt + 24).ok_or(PeError::TooSmall)?;
    let size_of_image = read_u32(image, opt + 56).ok_or(PeError::TooSmall)? as usize;

    let load_base = mmu::USER_MEM_START;
    if load_base + size_of_image > mmu::USER_MEM_START + mmu::USER_MEM_SIZE {
        return Err(PeError::SectionOutsideUserMemory);
    }

    let delta = (load_base as u64).wrapping_sub(image_base);

    unsafe {
        core::ptr::write_bytes(load_base as *mut u8, 0, size_of_image);
    }

    // --- Bolumler (bolum basligi PE32 ile birebir ayni: 40 bayt) ---
    let sections = opt + size_of_optional;
    let mut highest = 0usize;

    for i in 0..num_sections {
        let sh = sections + i * 40;
        let virtual_size = read_u32(image, sh + 8).ok_or(PeError::TooSmall)? as usize;
        let virtual_addr = read_u32(image, sh + 12).ok_or(PeError::TooSmall)? as usize;
        let raw_size = read_u32(image, sh + 16).ok_or(PeError::TooSmall)? as usize;
        let raw_ptr = read_u32(image, sh + 20).ok_or(PeError::TooSmall)? as usize;

        let span = virtual_size.max(raw_size);
        if virtual_addr + span > size_of_image {
            return Err(PeError::SectionOutOfBounds);
        }

        if raw_size > 0 {
            let src = image
                .get(raw_ptr..raw_ptr + raw_size)
                .ok_or(PeError::SectionOutOfBounds)?;
            unsafe {
                core::ptr::copy_nonoverlapping(
                    src.as_ptr(),
                    (load_base + virtual_addr) as *mut u8,
                    raw_size,
                );
            }
        }

        highest = highest.max(virtual_addr + span);
    }

    // --- Taban yeniden yerlesimi ---
    //
    // PE32'deki gibi ithal cozumunden ONCE: yerlesim imajin icindeki
    // mutlak adresleri duzeltir, ithal cozumu ise IAT'ye nihai thunk
    // adreslerini yazar. Ters sirada IAT girdileri delta ile bozulurdu.
    if delta != 0 {
        let reloc_rva = read_u32(image, opt + DIRECTORIES_AT + DIR_BASERELOC * 8).unwrap_or(0)
            as usize;
        let reloc_size = read_u32(image, opt + DIRECTORIES_AT + DIR_BASERELOC * 8 + 4)
            .unwrap_or(0) as usize;
        if reloc_rva != 0 && reloc_size != 0 {
            apply_relocations(load_base, size_of_image, reloc_rva, reloc_size, delta)?;
        }
    }

    // --- Ithal tablosu ---
    let mut end = load_base + size_of_image.max(highest);
    let import_rva = read_u32(image, opt + DIRECTORIES_AT + DIR_IMPORT * 8).unwrap_or(0) as usize;
    let import_size =
        read_u32(image, opt + DIRECTORIES_AT + DIR_IMPORT * 8 + 4).unwrap_or(0) as usize;
    if import_rva != 0 && import_size != 0 {
        let thunks_at = (end + 0xFFF) & !0xFFF;
        end = resolve_imports(load_base, size_of_image, import_rva, thunks_at)?;
    }

    Ok(LoadedImage {
        entry: (load_base + entry_rva) as u64,
        end: end as u64,
    })
}

/// Ithal tablosunu cozer. Yapisi PE32 ile aynidir
/// (`IMAGE_IMPORT_DESCRIPTOR` yine 20 bayt, RVA alanlari yine 32 bit);
/// degisen yalnizca **thunk dizilerinin kelime genisligidir**: hem ad
/// tablosu (INT) hem adres tablosu (IAT) 8'er baytlik girdiler tasir.
fn resolve_imports(
    load_base: usize,
    image_size: usize,
    import_rva: usize,
    thunks_at: usize,
) -> Result<usize, PeError> {
    let mut next_thunk = thunks_at;
    let mut descriptor = import_rva;

    loop {
        if descriptor + 20 > image_size {
            return Err(PeError::ImportOutOfBounds);
        }
        let int_rva = image_u32(load_base, descriptor) as usize;
        let name_rva = image_u32(load_base, descriptor + 12) as usize;
        let iat_rva = image_u32(load_base, descriptor + 16) as usize;

        if int_rva == 0 && name_rva == 0 && iat_rva == 0 {
            break;
        }

        let mut dll_storage = [0u8; NAME_MAX];
        let dll_name = image_cstr(load_base, image_size, name_rva, &mut dll_storage).unwrap_or("?");

        let names_rva = if int_rva != 0 { int_rva } else { iat_rva };
        if names_rva == 0 || iat_rva == 0 {
            return Err(PeError::ImportOutOfBounds);
        }

        let mut index = 0usize;
        let mut ordinals = 0usize;
        loop {
            let entry_at = names_rva + index * 8;
            if entry_at + 8 > image_size {
                return Err(PeError::ImportOutOfBounds);
            }
            let entry = image_u64(load_base, entry_at);
            if entry == 0 {
                break;
            }

            let export = if entry & IMAGE_ORDINAL_FLAG64 != 0 {
                let ordinal = (entry & 0xFFFF) as u16;
                match dll::resolve_ordinal(dll_name, ordinal) {
                    Some(e) => {
                        ordinals += 1;
                        e
                    }
                    None => {
                        crate::println!(
                            "[LEVEL-0b1] PE32+ ithal: {}#{} bulunamadi -- surec baslatilmiyor.",
                            dll_name,
                            ordinal
                        );
                        return Err(PeError::UnresolvedImport);
                    }
                }
            } else {
                // IMAGE_IMPORT_BY_NAME yapisi 64-bit'te de aynidir: u16
                // hint + NUL'lu ad. Degisen, ona isaret eden girdinin
                // genisligi.
                let mut fn_storage = [0u8; NAME_MAX];
                let function =
                    match image_cstr(load_base, image_size, entry as usize + 2, &mut fn_storage) {
                        Some(name) => name,
                        None => return Err(PeError::ImportOutOfBounds),
                    };

                match dll::resolve(dll_name, function) {
                    Some(e) => e,
                    None => {
                        crate::println!(
                            "[LEVEL-0b1] PE32+ ithal: {}!{} bulunamadi -- surec baslatilmiyor.",
                            dll_name,
                            function
                        );
                        return Err(PeError::UnresolvedImport);
                    }
                }
            };

            if next_thunk + dll::THUNK_SIZE > mmu::USER_MEM_START + mmu::USER_MAP_SIZE {
                return Err(PeError::ThunkAreaFull);
            }

            unsafe { dll::emit_thunk(next_thunk, &export) };

            let slot = iat_rva + index * 8;
            if slot + 8 > image_size {
                return Err(PeError::ImportOutOfBounds);
            }
            unsafe {
                ((load_base + slot) as *mut u64).write_unaligned(next_thunk as u64);
            }

            next_thunk += dll::THUNK_SIZE;
            index += 1;
        }

        if ordinals > 0 {
            crate::println!(
                "[LEVEL-0b1] PE32+ ithal: {} -- {} fonksiyon baglandi ({} ordinal ile).",
                dll_name,
                index,
                ordinals
            );
        } else {
            crate::println!(
                "[LEVEL-0b1] PE32+ ithal: {} -- {} fonksiyon baglandi.",
                dll_name,
                index
            );
        }

        descriptor += 20;
    }

    Ok(next_thunk)
}

fn image_u32(load_base: usize, rva: usize) -> u32 {
    unsafe { ((load_base + rva) as *const u32).read_unaligned() }
}

fn image_u64(load_base: usize, rva: usize) -> u64 {
    unsafe { ((load_base + rva) as *const u64).read_unaligned() }
}

fn image_cstr<'a>(
    load_base: usize,
    image_size: usize,
    rva: usize,
    storage: &'a mut [u8; NAME_MAX],
) -> Option<&'a str> {
    if rva >= image_size {
        return None;
    }
    let mut len = 0usize;
    while len < NAME_MAX && rva + len < image_size {
        let byte = unsafe { ((load_base + rva + len) as *const u8).read() };
        if byte == 0 {
            return core::str::from_utf8(&storage[..len]).ok();
        }
        storage[len] = byte;
        len += 1;
    }
    None
}

/// `.reloc` bolumunu yurutur.
///
/// Blok yapisi PE32 ile birebir aynidir (sayfa RVA'si + blok boyu +
/// 12 bit ofset / 4 bit tur girdileri); tek fark tur numarasidir:
/// `DIR64` (10), 4 bayt degil **8 bayt** yamalar.
fn apply_relocations(
    load_base: usize,
    image_size: usize,
    reloc_rva: usize,
    reloc_size: usize,
    delta: u64,
) -> Result<(), PeError> {
    if reloc_rva + reloc_size > image_size {
        return Err(PeError::RelocationOutOfBounds);
    }

    let mut offset = 0usize;
    while offset + 8 <= reloc_size {
        let block = (load_base + reloc_rva + offset) as *const u8;
        let page_rva = unsafe { (block as *const u32).read_unaligned() } as usize;
        let block_size = unsafe { (block.add(4) as *const u32).read_unaligned() } as usize;

        if block_size < 8 || offset + block_size > reloc_size {
            return Err(PeError::RelocationOutOfBounds);
        }

        let entries = (block_size - 8) / 2;
        for i in 0..entries {
            let entry = unsafe { (block.add(8 + i * 2) as *const u16).read_unaligned() };
            let kind = entry >> 12;
            let patch_rva = page_rva + (entry & 0x0FFF) as usize;

            match kind {
                REL_BASED_ABSOLUTE => {} // dolgu girdisi
                REL_BASED_DIR64 => {
                    if patch_rva + 8 > image_size {
                        return Err(PeError::RelocationOutOfBounds);
                    }
                    unsafe {
                        let target = (load_base + patch_rva) as *mut u64;
                        target.write_unaligned(target.read_unaligned().wrapping_add(delta));
                    }
                }
                _ => return Err(PeError::RelocationOutOfBounds),
            }
        }

        offset += block_size;
    }

    Ok(())
}

/// Imajin PE olup olmadigini ucuz sekilde sinar (VFS magic onceligi icin).
pub fn is_pe(image: &[u8]) -> bool {
    read_u16(image, 0) == Some(DOS_MAGIC)
        && read_u32(image, 0x3C)
            .map(|off| read_u32(image, off as usize) == Some(PE_SIGNATURE))
            .unwrap_or(false)
}
