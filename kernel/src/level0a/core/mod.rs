//! Level-0a cekirdek modulleri (doc S.9: `kernel/level0a/core/`).

pub mod fd;
pub mod frames;
pub mod init;
pub mod kmalloc;
pub mod scheduler;
pub mod tcmkfs;
pub mod vfs;

#[cfg(target_arch = "x86")]
pub mod mmu_i386;
#[cfg(target_arch = "x86")]
pub use mmu_i386 as mmu;

#[cfg(target_arch = "x86_64")]
pub mod mmu_x86_64;
#[cfg(target_arch = "x86_64")]
pub use mmu_x86_64 as mmu;
