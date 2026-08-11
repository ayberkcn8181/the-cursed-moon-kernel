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
use crate::level0a::core::{kmalloc, mmu, scheduler, vfs};
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
    /// Surece adres uzayi kurulamadi ve paylasimli uzay zaten baska bir
    /// Ring 3 sureci tarafindan kullaniliyor.
    AddressSpaceBusy,
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
    let image = vfs::load(node).ok_or(SpawnError::NotFound)?;
    run_image(path, image)
}

pub unsafe fn run_from_vfs(path: &'static str) -> Result<(), SpawnError> {
    let node = vfs::lookup(path).ok_or(SpawnError::NotFound)?;
    let image = vfs::load(node).ok_or(SpawnError::NotFound)?;
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

    // --- Adres uzayi ---
    // Once bos bir kullanici adres uzayi kurulur ve YUKLEMEDEN ONCE gecilir:
    // yukleyici imaji 0x00C00000'e kopyalarken artik bu surecin kendi
    // cerceveleri eslenmis olur. Onceki modelde tum surecler ayni bolgeyi
    // paylastigi icin her uygulama farkli bir "slot"a linklenmek zorundaydi.
    let space = mmu::create_user_space();

    // Adres uzayi kurulamadiysa (cerceve havuzu tukendi) paylasimli yola
    // duseriz -- ama baska bir Ring 3 sureci kosuyorsa bu, onun kodunun
    // uzerine yazmak demektir: butun imajlar ayni sanal adrese yuklenir.
    // Sessizce bozmaktansa acikca reddetmek dogru olan; belirtisi baska
    // turlu "calisan uygulamanin kendi kodunda page fault almasi" olurdu.
    if space.is_none() && scheduler::user_task_count() > 0 {
        return Err(SpawnError::AddressSpaceBusy);
    }
    if let Some(cr3) = space {
        mmu::switch_to(cr3);
        scheduler::set_current_address_space(cr3);
        if !mmu::map_user_range(cr3, mmu::USER_MEM_START, mmu::USER_MAP_SIZE) {
            release_space(space);
            return Err(SpawnError::OutOfMemory);
        }
    }

    let result = detect_and_load(image).and_then(|prepared| enter_ring3(prepared, space));

    // Surec bitti (ya da yuklenemedi): cerceveleri havuza geri ver.
    release_space(space);
    result
}

/// Surecin adres uzayini birakir ve gorevi cekirdek uzayina dondurur.
unsafe fn release_space(space: Option<usize>) {
    if let Some(cr3) = space {
        scheduler::set_current_address_space(0);
        mmu::switch_to(mmu::kernel_cr3());
        mmu::destroy_user_space(cr3);
    }
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
unsafe fn enter_ring3(prepared: Prepared, space: Option<usize>) -> Result<(), SpawnError> {
    // Kullanici yigini: imajin bittigi yerden sonra, sayfa hizali.
    let stack_bottom = (prepared.end + 0xFFF) & !0xFFF;
    let stack_top = stack_bottom + USER_STACK_SIZE;
    // Sinir, gercekten eslenmis pencere: kendi adres uzayinda 512 KiB,
    // paylasimli modelde (x86_64) tum bolge.
    if stack_top > mmu::USER_MEM_START + mmu::USER_MAP_SIZE {
        return Err(SpawnError::NoRoomForStack);
    }

    match space {
        // Kendi adres uzayi: sayfalar zaten tahsis edildi ve Ring 3'e acik.
        Some(_) => {}
        // Paylasimli model: bolgeyi Ring 3'e ac (eski davranis).
        None => mmu::protect_user_range(mmu::USER_MEM_START, stack_top - mmu::USER_MEM_START),
    }

    // Program break: imajin bittigi yerden yigin tabanina kadar buyuyebilir.
    kernel_api::set_program_break(prepared.end, stack_bottom);

    // Yeni imaj eski sinyal isleyicilerini devralmaz: kayitli adresler
    // artik var olmayan bir programa aittir, calistirilirsa surec kendi
    // kodunun ortasina dallanir. `execve` yolunda bu sart.
    crate::level0b1::signal::reset(scheduler::current_id());

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
