//! Kullanici sureci calistirma akisi (Faz 3 + Faz 7).
//!
//! Bir ikiliyi Level-0b1'in yukleyicileriyle bellege alir, kullanici
//! bolgesini Ring 3'e acar, TSS.esp0'i ayarlar ve `iret` ile Ring 3'e gecer.
//! Ring 3 -> Ring 0 gecisi (int 0x80 / int 0x2E) TSS sayesinde otomatiktir.
//!
//! **Cift uyumluluk burada baslar:** ayni akis hem ELF (Linux) hem PE
//! (Windows) ikililerini calistirir; format magic baytlarindan secilir ve
//! ikisi de ayni Ring 3 ortamina, ayni kullanici bellek tabanina yuklenir.

use crate::arch::cpu::usermode;
use crate::level0a::core::{kmalloc, mmu, vfs};
use crate::level0a::gdt;
use crate::level0a::kernel_api;
#[cfg(target_arch = "x86")]
use crate::level0b1::binary_loader::{elf32, pe32};
#[cfg(target_arch = "x86_64")]
use crate::level0b1::binary_loader::elf64;

/// Ring 3 yigini icin ayrilan alan (kullanici bolgesinin tepesinde).
const USER_STACK_SIZE: usize = 8 * 1024;
/// Ring 3'ten kesme geldiginde CPU'nun gececegi cekirdek yigini.
const KERNEL_STACK_SIZE: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryFormat {
    #[cfg(target_arch = "x86")]
    Elf32,
    #[cfg(target_arch = "x86")]
    Pe32,
    #[cfg(target_arch = "x86_64")]
    Elf64,
}

#[derive(Debug)]
pub enum SpawnError {
    // Alanlar yalnizca turetilmis Debug uzerinden okunur; dead-code analizi
    // bunu gormedigi icin acikca izin veriliyor.
    #[cfg(target_arch = "x86")]
    Elf(#[allow(dead_code)] elf32::ElfError),
    #[cfg(target_arch = "x86")]
    Pe(#[allow(dead_code)] pe32::PeError),
    #[cfg(target_arch = "x86_64")]
    Elf64(#[allow(dead_code)] elf64::Elf64Error),
    OutOfMemory,
    NoRoomForStack,
    NotFound,
}

/// Yuklenmis imajin ortak tanimi -- hangi formattan geldigi onemsizdir.
struct Prepared {
    entry: usize,
    end: usize,
    format: BinaryFormat,
}

/// VFS'teki bir yoldan ikili calistirir; format magic baytlarindan secilir
/// (doc S.7 Faz 7: "VFS magic onceligi, PE basarisizsa ELF fallback").
///
/// # Safety
/// `run_image` ile ayni onkosullar.
pub unsafe fn run_from_vfs_dynamic(path: &str) -> Result<(), SpawnError> {
    let node = vfs::lookup(path).ok_or(SpawnError::NotFound)?;
    let image = vfs::as_slice(node).ok_or(SpawnError::NotFound)?;
    run_image(path, image)
}

pub unsafe fn run_from_vfs(path: &'static str) -> Result<(), SpawnError> {
    let node = vfs::lookup(path).ok_or(SpawnError::NotFound)?;
    let image = vfs::as_slice(node).ok_or(SpawnError::NotFound)?;
    crate::println!("[LEVEL-0b1] VFS'ten yukleniyor: {}", path);
    run_image(path, image)
}

/// Bir ikiliyi formatini tespit ederek Ring 3'te calistirir.
///
/// # Safety
/// Sayfalama acik ve TSS kurulmus olmalidir.
pub unsafe fn run_image(name: &str, image: &[u8]) -> Result<(), SpawnError> {
    crate::println!(
        "[LEVEL-0b1] Binary Loader: '{}' ({} bayt) yukleniyor.",
        name,
        image.len()
    );

    let prepared = detect_and_load(image)?;
    enter_ring3(prepared)
}

/// Format secimi ve yukleme -- mimariye gore hangi yukleyicilerin mevcut
/// oldugu degisir (i386: ELF32 + PE32, x86_64: ELF64).
#[cfg(target_arch = "x86")]
unsafe fn detect_and_load(image: &[u8]) -> Result<Prepared, SpawnError> {
    if pe32::is_pe(image) {
        crate::println!("[LEVEL-0b1] format: PE32 (Windows NT alt sistemi)");
        match pe32::load(image) {
            Ok(img) => {
                return Ok(Prepared {
                    entry: img.entry as usize,
                    end: img.end as usize,
                    format: BinaryFormat::Pe32,
                })
            }
            // Doc S.7: PE basarisiz olursa ELF'e geri dusulur.
            Err(pe_err) => {
                crate::println!("[LEVEL-0b1] PE yuklenemedi ({:?}), ELF deneniyor.", pe_err);
                let img = elf32::load(image).map_err(|_| SpawnError::Pe(pe_err))?;
                return Ok(Prepared {
                    entry: img.entry as usize,
                    end: img.end as usize,
                    format: BinaryFormat::Elf32,
                });
            }
        }
    }

    crate::println!("[LEVEL-0b1] format: ELF32 (Linux POSIX alt sistemi)");
    let img = elf32::load(image).map_err(SpawnError::Elf)?;
    Ok(Prepared {
        entry: img.entry as usize,
        end: img.end as usize,
        format: BinaryFormat::Elf32,
    })
}

#[cfg(target_arch = "x86_64")]
unsafe fn detect_and_load(image: &[u8]) -> Result<Prepared, SpawnError> {
    // PE32+ (x86_64 Windows) yukleyicisi Faz 7'nin 64-bit ayagidir ve
    // henuz yok; su an yalnizca ELF64 desteklenir.
    if !elf64::is_elf64(image) {
        crate::println!("[LEVEL-0b1] ELF64 imzasi yok -- PE32+ yukleyicisi Faz 7'nin");
        crate::println!("[LEVEL-0b1] 64-bit ayagidir ve henuz eklenmedi.");
    }
    crate::println!("[LEVEL-0b1] format: ELF64 (Linux POSIX alt sistemi)");
    let img = elf64::load(image).map_err(SpawnError::Elf64)?;
    Ok(Prepared {
        entry: img.entry,
        end: img.end,
        format: BinaryFormat::Elf64,
    })
}

/// Formattan bagimsiz ortak bolum: yigin yerlesimi, sayfa izinleri,
/// TSS ve Ring 3 gecisi.
unsafe fn enter_ring3(prepared: Prepared) -> Result<(), SpawnError> {
    // Kullanici yigini: imajin bittigi yerden sonra, sayfa hizali.
    let stack_bottom = (prepared.end + 0xFFF) & !0xFFF;
    let stack_top = stack_bottom + USER_STACK_SIZE;
    if stack_top > mmu::USER_MEM_START + mmu::USER_MEM_SIZE {
        return Err(SpawnError::NoRoomForStack);
    }

    // Tum kullanici bolgesini (kod + veri + yigin) Ring 3'e ac.
    mmu::protect_user_range(mmu::USER_MEM_START, stack_top - mmu::USER_MEM_START);

    // Program break: imajin bittigi yerden yigin tabanina kadar buyuyebilir.
    kernel_api::set_program_break(prepared.end, stack_bottom);

    // Ring 3 -> Ring 0 gecisleri icin ayri bir cekirdek yigini.
    let kstack = kmalloc::kmalloc_aligned(KERNEL_STACK_SIZE, 16).ok_or(SpawnError::OutOfMemory)?;
    let kstack_top = kstack.add(KERNEL_STACK_SIZE) as usize;
    gdt::set_kernel_stack(kstack_top);
    // x86_64'te `syscall` komutu TSS kullanmaz; yigini ayrica bildirmeliyiz.
    #[cfg(target_arch = "x86_64")]
    crate::level0a::syscall_msr::set_kernel_stack(kstack_top);

    crate::println!(
        "[LEVEL-0b1] {:?} entry=0x{:08x} user_stack=0x{:08x} kernel_stack=0x{:08x}",
        prepared.format,
        prepared.entry,
        stack_top,
        kstack_top
    );
    crate::println!("[LEVEL-0b1] Ring 3'e geciliyor (iret)...");

    // ESP en ustte degil, 16 bayt asagida baslatilir (hizalama payi).
    usermode::run_user_program(prepared.entry, stack_top - 16);

    crate::println!("[LEVEL-0b1] Ring 3 programi sonlandi, Ring 0'a donuldu.");
    Ok(())
}
