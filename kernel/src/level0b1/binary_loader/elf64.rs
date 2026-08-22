//! ELF64 (x86_64) yukleyici -- ELF32'nin 64-bit karsiligi.
//!
//! Yapisal fark: baslik alanlari genisler ve program header duzeni degisir
//! (`p_flags` `p_offset`'ten hemen sonra gelir, ELF32'de sonda idi).

use crate::level0a::core::mmu;

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
/// Program baslik tablosunun kendisini tarif eden girdi.
const PT_PHDR: u32 = 6;

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Ehdr {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
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
struct Elf64Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elf64Error {
    TooSmall,
    BadMagic,
    NotElf64Le,
    NotExecutable,
    WrongArchitecture,
    SegmentOutOfBounds,
    SegmentOutsideUserMemory,
}

pub struct LoadedImage {
    pub entry: usize,
    pub end: usize,
    /// Program basliklarinin **bellekteki** adresi -- bkz. `elf32`.
    pub phdr: usize,
    pub phentsize: usize,
    pub phnum: usize,
    /// Yuklenen en dusuk adres (imajin tabani).
    pub base: usize,
}

/// Imajin ELF64 olup olmadigini ucuz sekilde sinar.
pub fn is_elf64(image: &[u8]) -> bool {
    image.len() >= 20
        && image[0..4] == ELF_MAGIC
        && image[4] == ELFCLASS64
        && image[5] == ELFDATA2LSB
}

pub fn load(image: &[u8]) -> Result<LoadedImage, Elf64Error> {
    if image.len() < core::mem::size_of::<Elf64Ehdr>() {
        return Err(Elf64Error::TooSmall);
    }

    let ehdr: Elf64Ehdr = unsafe { core::ptr::read_unaligned(image.as_ptr() as *const Elf64Ehdr) };

    if ehdr.e_ident[0..4] != ELF_MAGIC {
        return Err(Elf64Error::BadMagic);
    }
    if ehdr.e_ident[4] != ELFCLASS64 || ehdr.e_ident[5] != ELFDATA2LSB {
        return Err(Elf64Error::NotElf64Le);
    }
    if ehdr.e_type != ET_EXEC {
        return Err(Elf64Error::NotExecutable);
    }
    if ehdr.e_machine != EM_X86_64 {
        return Err(Elf64Error::WrongArchitecture);
    }

    let mut highest = 0usize;
    let mut lowest = 0usize;
    let mut phdr_at = 0usize;

    for i in 0..ehdr.e_phnum as usize {
        let off = ehdr.e_phoff as usize + i * ehdr.e_phentsize as usize;
        if off + core::mem::size_of::<Elf64Phdr>() > image.len() {
            return Err(Elf64Error::SegmentOutOfBounds);
        }

        let phdr: Elf64Phdr =
            unsafe { core::ptr::read_unaligned(image.as_ptr().add(off) as *const Elf64Phdr) };

        // PT_PHDR, program baslik tablosunun **bellekteki** adresini
        // dogrudan soyler ve yetkili kaynak odur. Yoksa asagida
        // segment ofsetinden hesaplanir.
        if phdr.p_type == PT_PHDR {
            phdr_at = phdr.p_vaddr as usize;
        }
        if phdr.p_type != PT_LOAD || phdr.p_memsz == 0 {
            continue;
        }

        let file_start = phdr.p_offset as usize;
        let file_end = file_start
            .checked_add(phdr.p_filesz as usize)
            .ok_or(Elf64Error::SegmentOutOfBounds)?;
        if file_end > image.len() {
            return Err(Elf64Error::SegmentOutOfBounds);
        }

        let vaddr = phdr.p_vaddr as usize;
        let vend = vaddr
            .checked_add(phdr.p_memsz as usize)
            .ok_or(Elf64Error::SegmentOutOfBounds)?;
        if vaddr < mmu::USER_MEM_START || vend > mmu::USER_MEM_START + mmu::USER_MEM_SIZE {
            return Err(Elf64Error::SegmentOutsideUserMemory);
        }

        unsafe {
            let dst = vaddr as *mut u8;
            core::ptr::copy_nonoverlapping(
                image.as_ptr().add(file_start),
                dst,
                phdr.p_filesz as usize,
            );
            let bss = (phdr.p_memsz - phdr.p_filesz) as usize;
            if bss > 0 {
                core::ptr::write_bytes(dst.add(phdr.p_filesz as usize), 0, bss);
            }
        }

        if vend > highest {
            highest = vend;
        }
        if lowest == 0 || vaddr < lowest {
            lowest = vaddr;
        }

        // Program baslik tablosunun bellekteki karsiligi (bkz. `elf32`).
        let table_end =
            ehdr.e_phoff as usize + ehdr.e_phnum as usize * ehdr.e_phentsize as usize;
        if phdr_at == 0 && (ehdr.e_phoff as usize) >= file_start && table_end <= file_end {
            phdr_at = vaddr + (ehdr.e_phoff as usize - file_start);
        }
    }

    if highest == 0 {
        return Err(Elf64Error::SegmentOutOfBounds);
    }

    Ok(LoadedImage {
        entry: ehdr.e_entry as usize,
        end: highest,
        phdr: phdr_at,
        phentsize: ehdr.e_phentsize as usize,
        phnum: ehdr.e_phnum as usize,
        base: lowest,
    })
}
