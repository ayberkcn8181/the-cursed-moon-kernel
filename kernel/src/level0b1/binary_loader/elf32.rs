//! ELF32 (i386) yukleyici -- Level-0b1'in "Binary Loader" bileseni
//! (doc S.2.2.B: "PE ve ELF dosyalarini ayristirip bellek haritalamalarini
//! yaparak calismaya hazir hale getirir").
//!
//! Faz 3 kapsami: statik, ET_EXEC, EM_386 ikilileri. Dinamik baglama,
//! interpreter (PT_INTERP) ve reloc destegi kapsam disidir.

use crate::level0a::core::mmu;

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELFCLASS32: u8 = 1;
const ELFDATA2LSB: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_386: u16 = 3;
const PT_LOAD: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf32Ehdr {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u32,
    e_phoff: u32,
    e_shoff: u32,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf32Phdr {
    p_type: u32,
    p_offset: u32,
    p_vaddr: u32,
    p_paddr: u32,
    p_filesz: u32,
    p_memsz: u32,
    p_flags: u32,
    p_align: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    TooSmall,
    BadMagic,
    NotElf32Le,
    NotExecutable,
    WrongArchitecture,
    SegmentOutOfBounds,
    SegmentOutsideUserMemory,
}

pub struct LoadedImage {
    pub entry: u32,
    /// Yuklenen en yuksek adres (yigin yerlesimi icin).
    pub end: u32,
}

/// `image` icindeki ELF32'yi kullanici bellek bolgesine yukler.
///
/// Yalnizca `mmu::USER_MEM_START..+USER_MEM_SIZE` araligina yazmaya izin
/// verilir; disina tasan bir segment `SegmentOutsideUserMemory` ile
/// reddedilir. Bu kontrol kasten yukleyicidedir: bozuk/kotu niyetli bir
/// ELF'in cekirdek belleginin uzerine yazmasini engeller.
pub fn load(image: &[u8]) -> Result<LoadedImage, ElfError> {
    let ehdr = read_ehdr(image)?;

    let mut highest: u32 = 0;

    for i in 0..ehdr.e_phnum as usize {
        let off = ehdr.e_phoff as usize + i * ehdr.e_phentsize as usize;
        if off + core::mem::size_of::<Elf32Phdr>() > image.len() {
            return Err(ElfError::SegmentOutOfBounds);
        }

        let phdr: Elf32Phdr =
            unsafe { core::ptr::read_unaligned(image.as_ptr().add(off) as *const Elf32Phdr) };

        if phdr.p_type != PT_LOAD || phdr.p_memsz == 0 {
            continue;
        }

        let file_start = phdr.p_offset as usize;
        let file_end = file_start
            .checked_add(phdr.p_filesz as usize)
            .ok_or(ElfError::SegmentOutOfBounds)?;
        if file_end > image.len() {
            return Err(ElfError::SegmentOutOfBounds);
        }

        let vaddr = phdr.p_vaddr as usize;
        let vend = vaddr
            .checked_add(phdr.p_memsz as usize)
            .ok_or(ElfError::SegmentOutOfBounds)?;
        if vaddr < mmu::USER_MEM_START || vend > mmu::USER_MEM_START + mmu::USER_MEM_SIZE {
            return Err(ElfError::SegmentOutsideUserMemory);
        }

        unsafe {
            let dst = vaddr as *mut u8;
            core::ptr::copy_nonoverlapping(
                image.as_ptr().add(file_start),
                dst,
                phdr.p_filesz as usize,
            );
            // .bss benzeri bolum: filesz'den memsz'ye kadar sifirla.
            let bss = (phdr.p_memsz - phdr.p_filesz) as usize;
            if bss > 0 {
                core::ptr::write_bytes(dst.add(phdr.p_filesz as usize), 0, bss);
            }
        }

        if vend as u32 > highest {
            highest = vend as u32;
        }
    }

    if highest == 0 {
        return Err(ElfError::SegmentOutOfBounds);
    }

    Ok(LoadedImage {
        entry: ehdr.e_entry,
        end: highest,
    })
}

fn read_ehdr(image: &[u8]) -> Result<Elf32Ehdr, ElfError> {
    if image.len() < core::mem::size_of::<Elf32Ehdr>() {
        return Err(ElfError::TooSmall);
    }

    let ehdr: Elf32Ehdr = unsafe { core::ptr::read_unaligned(image.as_ptr() as *const Elf32Ehdr) };

    if ehdr.e_ident[0..4] != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if ehdr.e_ident[4] != ELFCLASS32 || ehdr.e_ident[5] != ELFDATA2LSB {
        return Err(ElfError::NotElf32Le);
    }
    if ehdr.e_type != ET_EXEC {
        return Err(ElfError::NotExecutable);
    }
    if ehdr.e_machine != EM_386 {
        return Err(ElfError::WrongArchitecture);
    }

    Ok(ehdr)
}
