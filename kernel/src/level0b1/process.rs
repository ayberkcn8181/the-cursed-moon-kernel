//! Kullanici sureci calistirma akisi (Faz 3).
//!
//! Bir ELF ikilisini Level-0b1'in yukleyicisiyle bellege alir, kullanici
//! bolgesini Ring 3'e acar, TSS.esp0'i ayarlar ve `iret` ile Ring 3'e gecer.
//! Ring 3 -> Ring 0 gecisi (int 0x80) TSS sayesinde otomatiktir.

use crate::arch::i386::usermode;
use crate::level0a::core::{kmalloc, mmu, vfs};
use crate::level0a::kernel_api;
use crate::level0a::gdt;
use crate::level0b1::binary_loader::elf32;

/// Ring 3 yigini icin ayrilan alan (kullanici bolgesinin tepesinde).
const USER_STACK_SIZE: usize = 8 * 1024;
/// int 0x80 Ring 3'ten geldiginde CPU'nun gececegi cekirdek yigini.
const KERNEL_STACK_SIZE: usize = 16 * 1024;

#[derive(Debug)]
pub enum SpawnError {
    // Alan yalnizca turetilmis Debug uzerinden okunur; dead-code analizi
    // bunu gormedigi icin acikca izin veriliyor.
    Elf(#[allow(dead_code)] elf32::ElfError),
    OutOfMemory,
    NoRoomForStack,
    NotFound,
}

/// VFS'teki bir yoldan ELF calistirir (doc S.7 Faz 5:
/// `elf_load_vfs("/bin/hello")`).
///
/// # Safety
/// `run_elf` ile ayni onkosullar.
pub unsafe fn run_elf_from_vfs(path: &'static str) -> Result<(), SpawnError> {
    let node = vfs::lookup(path).ok_or(SpawnError::NotFound)?;
    let image = vfs::as_slice(node).ok_or(SpawnError::NotFound)?;
    crate::println!("[LEVEL-0b1] VFS'ten yukleniyor: {}", path);
    run_elf(path, image)
}

/// Gomulu bir ELF imajini Ring 3'te calistirir; program `sys_exit`
/// cagirdiginda geri doner.
///
/// # Safety
/// Sayfalama acik ve TSS kurulmus olmalidir.
pub unsafe fn run_elf(name: &str, image: &[u8]) -> Result<(), SpawnError> {
    crate::println!("[LEVEL-0b1] Binary Loader: '{}' ({} bayt) yukleniyor.", name, image.len());

    let loaded = elf32::load(image).map_err(SpawnError::Elf)?;

    // Kullanici yigini: imajin bittigi yerden sonra, sayfa hizali.
    let stack_bottom = (loaded.end as usize + 0xFFF) & !0xFFF;
    let stack_top = stack_bottom + USER_STACK_SIZE;
    if stack_top > mmu::USER_MEM_START + mmu::USER_MEM_SIZE {
        return Err(SpawnError::NoRoomForStack);
    }

    // Tum kullanici bolgesini (kod + veri + yigin) Ring 3'e ac.
    mmu::protect_user_range(mmu::USER_MEM_START, stack_top - mmu::USER_MEM_START);

    // Program break: imajin bittigi yerden yigin tabanina kadar buyuyebilir.
    kernel_api::set_program_break(loaded.end as usize, stack_bottom);

    // Ring 3 -> Ring 0 gecisleri icin ayri bir cekirdek yigini.
    let kstack = kmalloc::kmalloc_aligned(KERNEL_STACK_SIZE, 16).ok_or(SpawnError::OutOfMemory)?;
    let kstack_top = kstack.add(KERNEL_STACK_SIZE) as u32;
    gdt::set_kernel_stack(kstack_top);

    crate::println!(
        "[LEVEL-0b1] entry=0x{:08x} user_stack=0x{:08x} kernel_esp0=0x{:08x}",
        loaded.entry,
        stack_top,
        kstack_top
    );
    crate::println!("[LEVEL-0b1] Ring 3'e geciliyor (iret)...");

    // ESP en ustte degil, 16 bayt asagida baslatilir (hizalama payi).
    usermode::run_user_program(loaded.entry, (stack_top - 16) as u32);

    crate::println!("[LEVEL-0b1] Ring 3 programi sonlandi, Ring 0'a donuldu.");
    Ok(())
}
