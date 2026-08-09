//! Level-0a cekirdek modulleri (doc S.9: `kernel/level0a/core/`).

pub mod init;
pub mod kmalloc;
pub mod scheduler;

#[cfg(target_arch = "x86")]
pub mod mmu_i386;

#[cfg(target_arch = "x86")]
pub use mmu_i386 as mmu;
