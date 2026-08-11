//! PE32 (Windows, i386) yukleyici -- Level-0b1 Binary Loader'in NT tarafi
//! (doc S.7 Faz 7).
//!
//! Cekirdek, PE'yi her zaman `mmu::USER_MEM_START` tabanina yukler. Gercek
//! PE dosyalarinin `ImageBase`'i genellikle 0x00400000 oldugundan taban
//! farki (delta) sifir degildir; bu yuzden **taban yeniden yerlesimi**
//! (base relocation, `.reloc`) uygulanir.
//!
//! **Ithal tablosu** (Faz 7b) destekleniyor: `KERNEL32.dll` gibi adlar
//! gomulu bir tablodan cozulur ve her fonksiyon icin surecin adres
//! uzayina bir thunk yazilir (bkz. `nt_subsystem::dll`). Yani bir program
//! `int 0x2E` yazmak zorunda degildir; siradan bir Windows ikilisi gibi
//! `WriteConsoleA` cagirabilir.
//!
//! **Ordinal ile ithal** (Faz 7c) de destekleniyor: bazi DLL'ler
//! fonksiyonlari adsiz, yalnizca sira numarasiyla ihrac eder. O durumda
//! ikilide ad hic gecmez; arama gomulu tablonun ordinal alanina duser.

use crate::level0a::core::mmu;
use crate::level0b1::nt_subsystem::dll;

const DOS_MAGIC: u16 = 0x5A4D; // "MZ"
const PE_SIGNATURE: u32 = 0x0000_4550; // "PE\0\0"
const MACHINE_I386: u16 = 0x014C;
const PE32_MAGIC: u16 = 0x010B;

const DIR_IMPORT: usize = 1;
const DIR_BASERELOC: usize = 5;
const REL_BASED_ABSOLUTE: u16 = 0;
const REL_BASED_HIGHLOW: u16 = 3;

/// Ithal edilen fonksiyon adlarinin ust siniri (tanilama icin).
const NAME_MAX: usize = 64;

/// Ordinal ile ithal isareti (adi degil numarasi verilmis).
const IMAGE_ORDINAL_FLAG32: u32 = 0x8000_0000;

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
    ImportOutOfBounds,
    /// Ithal edilen bir DLL ya da fonksiyon gomulu tabloda yok. Windows
    /// bu durumda "The procedure entry point X could not be located"
    /// der ve sureci baslatmaz; TCMK de baslatmaz.
    UnresolvedImport,
    /// Thunk'lar icin imajin arkasinda yer kalmadi.
    ThunkAreaFull,
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
    //
    // Ithal tablosundan ONCE yapilir: yeniden yerlesim imajin icindeki
    // mutlak adresleri duzeltir, ithal cozumu ise IAT'ye nihai thunk
    // adreslerini yazar. Ters sirada IAT girdileri delta ile bozulurdu.
    if delta != 0 {
        let reloc_rva = read_u32(image, opt + 96 + DIR_BASERELOC * 8).unwrap_or(0) as usize;
        let reloc_size = read_u32(image, opt + 96 + DIR_BASERELOC * 8 + 4).unwrap_or(0) as usize;
        if reloc_rva != 0 && reloc_size != 0 {
            apply_relocations(load_base, size_of_image, reloc_rva, reloc_size, delta)?;
        }
    }

    // --- Ithal tablosu (Faz 7b) ---
    //
    // Thunk'lar imajin arkasina, sayfa hizali bir alana yazilir. `end`
    // bu alanin sonunu gosterir; kullanici yigini oradan sonra kurulur
    // (bkz. `process::enter_ring3`), yani thunk alani yigin tarafindan
    // ezilmez.
    let mut end = load_base + size_of_image.max(highest);
    let import_rva = read_u32(image, opt + 96 + DIR_IMPORT * 8).unwrap_or(0) as usize;
    let import_size = read_u32(image, opt + 96 + DIR_IMPORT * 8 + 4).unwrap_or(0) as usize;
    if import_rva != 0 && import_size != 0 {
        let thunks_at = (end + 0xFFF) & !0xFFF;
        end = resolve_imports(load_base, size_of_image, import_rva, thunks_at)?;
    }

    Ok(LoadedImage {
        entry: load_base as u32 + entry_rva,
        end: end as u32,
    })
}

/// Ithal tablosunu cozer: her fonksiyon icin bir thunk uretir ve IAT
/// girdisini oraya yonlendirir. Thunk alaninin sonunu doner.
///
/// Diskte `KERNEL32.dll` diye bir dosya olmadigi icin klasik anlamda bir
/// "DLL yukleme" yoktur; gomulu tablo (bkz. `nt_subsystem::dll`) adi
/// bir NT servis numarasina cevirir ve yukleyici surecin adres uzayina
/// o servisi cagiran kucuk bir stub yazar. Program bunu normal bir DLL
/// girisinden ayirt edemez -- IAT'de gordugu sey yine bir kod adresidir.
fn resolve_imports(
    load_base: usize,
    image_size: usize,
    import_rva: usize,
    thunks_at: usize,
) -> Result<usize, PeError> {
    let mut next_thunk = thunks_at;
    let mut descriptor = import_rva;

    loop {
        // IMAGE_IMPORT_DESCRIPTOR: 20 bayt, sifir girdisiyle biter.
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
        let dll_name =
            image_cstr(load_base, image_size, name_rva, &mut dll_storage).unwrap_or("?");

        // Bazi baglayicilar OriginalFirstThunk'i bos birakir; o zaman ad
        // dizisi IAT'nin kendisidir (henuz baglanmamis oldugu icin ayni
        // degerleri tasir).
        let names_rva = if int_rva != 0 { int_rva } else { iat_rva };
        if names_rva == 0 || iat_rva == 0 {
            return Err(PeError::ImportOutOfBounds);
        }

        let mut index = 0usize;
        let mut ordinals = 0usize;
        loop {
            let entry_at = names_rva + index * 4;
            if entry_at + 4 > image_size {
                return Err(PeError::ImportOutOfBounds);
            }
            let entry = image_u32(load_base, entry_at);
            if entry == 0 {
                break;
            }

            // Ithal iki bicimden biridir (Faz 7b: ad, Faz 7c: ordinal).
            // Ordinal bicimde ikilide ad hic yer almaz; ust bit isaretli
            // olur ve alt 16 bit sira numarasini tasir.
            let export = if entry & IMAGE_ORDINAL_FLAG32 != 0 {
                let ordinal = (entry & 0xFFFF) as u16;
                match dll::resolve_ordinal(dll_name, ordinal) {
                    Some(e) => {
                        ordinals += 1;
                        e
                    }
                    None => {
                        crate::println!(
                            "[LEVEL-0b1] PE ithal: {}#{} bulunamadi -- surec baslatilmiyor.",
                            dll_name,
                            ordinal
                        );
                        return Err(PeError::UnresolvedImport);
                    }
                }
            } else {
                // IMAGE_IMPORT_BY_NAME: u16 hint, ardindan NUL'lu ad.
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
                            "[LEVEL-0b1] PE ithal: {}!{} bulunamadi -- surec baslatilmiyor.",
                            dll_name,
                            function
                        );
                        return Err(PeError::UnresolvedImport);
                    }
                }
            };

            if next_thunk + dll::THUNK_SIZE
                > mmu::USER_MEM_START + mmu::USER_MAP_SIZE
            {
                return Err(PeError::ThunkAreaFull);
            }

            unsafe { dll::emit_thunk(next_thunk, &export) };

            // IAT girdisini thunk'a yonlendir.
            let slot = iat_rva + index * 4;
            if slot + 4 > image_size {
                return Err(PeError::ImportOutOfBounds);
            }
            unsafe {
                ((load_base + slot) as *mut u32).write_unaligned(next_thunk as u32);
            }

            next_thunk += dll::THUNK_SIZE;
            index += 1;
        }

        if ordinals > 0 {
            crate::println!(
                "[LEVEL-0b1] PE ithal: {} -- {} fonksiyon baglandi ({} ordinal ile).",
                dll_name,
                index,
                ordinals
            );
        } else {
            crate::println!(
                "[LEVEL-0b1] PE ithal: {} -- {} fonksiyon baglandi.",
                dll_name,
                index
            );
        }

        descriptor += 20;
    }

    Ok(next_thunk)
}

/// Yuklenmis imajdan hizalamadan bagimsiz u32 okur.
fn image_u32(load_base: usize, rva: usize) -> u32 {
    unsafe { ((load_base + rva) as *const u32).read_unaligned() }
}

/// Yuklenmis imajdan NUL sonlandirmali ad okur.
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
