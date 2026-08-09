//! Global Descriptor Table -- mimariye ozel, arayuzu ortak
//! (`init`, `install_tss`, `set_kernel_stack`, `KERNEL_CODE_SELECTOR`).

#[cfg(target_arch = "x86")]
mod i386;
#[cfg(target_arch = "x86")]
pub use i386::*;

#[cfg(target_arch = "x86_64")]
mod x86_64;
#[cfg(target_arch = "x86_64")]
pub use x86_64::*;
