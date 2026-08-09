pub mod multiboot;

#[cfg(target_arch = "x86")]
pub mod i386;

#[cfg(target_arch = "x86_64")]
pub mod x86_64;
