//! Binary Loader (doc S.2.2.B): ELF ve PE ikililerini ayristirip bellege
//! haritalar. Hangi yukleyicilerin derlendigi hedef mimariye baglidir.

#[cfg(target_arch = "x86")]
pub mod elf32;
#[cfg(target_arch = "x86")]
pub mod pe32;

#[cfg(target_arch = "x86_64")]
pub mod elf64;
#[cfg(target_arch = "x86_64")]
pub mod pe64;
