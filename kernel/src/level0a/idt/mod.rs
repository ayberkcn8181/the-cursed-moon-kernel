//! Interrupt Descriptor Table -- mimariye ozel, arayuzu ortak (`init`).

#[cfg(target_arch = "x86")]
mod i386;
#[cfg(target_arch = "x86")]
pub use i386::*;

#[cfg(target_arch = "x86_64")]
mod x86_64;
#[cfg(target_arch = "x86_64")]
pub use x86_64::*;
