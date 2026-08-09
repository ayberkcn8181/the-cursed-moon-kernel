//! PE32 (Windows, i386) yukleyici -- Level-0b1 Binary Loader'in NT tarafi
//! (doc S.7 Faz 7).
//!
//! Cekirdek, PE'yi her zaman `mmu::USER_MEM_START` tabanina yukler. Gercek
//! PE dosyalarinin `ImageBase`'i genellikle 0x00400000 oldugundan taban
//! farki (delta) sifir degildir; bu yuzden **taban yeniden yerlesimi**
//! (base relocation, `.reloc`) uygulanir.
//!
//! Faz 7 kapsami: statik PE32, import tablosu YOK (Faz 7b), ordinal YOK
//! (Faz 7c). Program NT sistem cagrilarini dogrudan `int 0x2E` ile yapar.

use crate::level0a::core::mmu;

const DOS_MAGIC: u16 = 0x5A4D; // "MZ"
const PE_SIGNATURE: u32 = 0x0000_4550; // "PE\0\0"
const MACHINE_I386: u16 = 0x014C;
const PE32_MAGIC: u16 = 0x010B;

const DIR_BASERELOC: usize = 5;
const REL_BASED_ABSOLUTE: u16 = 0;
const REL_BASED_HIGHLOW: u16 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeError {
    TooSmall,
    BadDosMagic,
    BadPeSignature,
    WrongArchitecture,
    NotPe32,
    /// Doc S.7'de not edilen klasik tuzak: SizeOfOptionalHeader yanlis
    /// paketlenirse bolum tablosu bulunamaz ve .text hic kopyalanmaz.
    BadOptionalHeaderSize,
    SectionOutOfBounds,
    SectionOutsideUserMemory,
    RelocationOutOfBounds,
}

pub struct LoadedImage {
    pub entry: u32,
    pub end: u32,
}

/// Kucuk yardimci: `image` icinden hizalamadan bagimsiz okuma.
fn read_u16(image: &[u8], off: usize) -> Option<u16> {
    let bytes = image.get(off..off + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(image: &[u8], off: usize) -> Option<u32> {
    let bytes = image.get(off..off + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
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

    // --- COFF basligi (PE imzasindan hemen sonra, 20 bayt) ---
    let coff = pe_off + 4;
    let machine = read_u16(image, coff).ok_or(PeError::TooSmall)?;
    if machine != MACHINE_I386 {
        return Err(PeError::WrongArchitecture);
    }
    let num_sections = read_u16(image, coff + 2).ok_or(PeError::TooSmall)? as usize;
    let size_of_optional = read_u16(image, coff + 16).ok_or(PeError::TooSmall)? as usize;
    if size_of_optional < 96 {
        return Err(PeError::BadOptionalHeaderSize);
    }

    // --- Optional header ---
    let opt = coff + 20;
    if read_u16(image, opt).ok_or(PeError::TooSmall)? != PE32_MAGIC {
        return Err(PeError::NotPe32);
    }
    let entry_rva = read_u32(image, opt + 16).ok_or(PeError::TooSmall)?;
    let image_base = read_u32(image, opt + 28).ok_or(PeError::TooSmall)?;
    let size_of_image = read_u32(image, opt + 56).ok_or(PeError::TooSmall)? as usize;

    // Yukleme tabani her zaman kullanici bolgesinin basidir.
    let load_base = mmu::USER_MEM_START;
    if load_base + size_of_image > mmu::USER_MEM_START + mmu::USER_MEM_SIZE {
        return Err(PeError::SectionOutsideUserMemory);
    }

    let delta = (load_base as u32).wrapping_sub(image_base);

    // Imaj bolgesini sifirla: bolumler arasi bosluklar ve .bss temiz olsun.
    unsafe {
        core::ptr::write_bytes(load_base as *mut u8, 0, size_of_image);
    }

    // --- Bolumler ---
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
    if delta != 0 {
        let reloc_rva = read_u32(image, opt + 96 + DIR_BASERELOC * 8).unwrap_or(0) as usize;
        let reloc_size = read_u32(image, opt + 96 + DIR_BASERELOC * 8 + 4).unwrap_or(0) as usize;
        if reloc_rva != 0 && reloc_size != 0 {
            apply_relocations(load_base, size_of_image, reloc_rva, reloc_size, delta)?;
        }
    }

    Ok(LoadedImage {
        entry: load_base as u32 + entry_rva,
        end: (load_base + highest) as u32,
    })
}

/// `.reloc` bolumunu yurutur. Bloklar zaten bellege kopyalanmis durumdadir,
/// bu yuzden dosyadan degil yuklenmis imajdan okunur.
fn apply_relocations(
    load_base: usize,
    image_size: usize,
    reloc_rva: usize,
    reloc_size: usize,
    delta: u32,
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
            let entry =
                unsafe { (block.add(8 + i * 2) as *const u16).read_unaligned() };
            let kind = entry >> 12;
            let patch_rva = page_rva + (entry & 0x0FFF) as usize;

            match kind {
                REL_BASED_ABSOLUTE => {} // dolgu girdisi, atlanir
                REL_BASED_HIGHLOW => {
                    if patch_rva + 4 > image_size {
                        return Err(PeError::RelocationOutOfBounds);
                    }
                    unsafe {
                        let target = (load_base + patch_rva) as *mut u32;
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
