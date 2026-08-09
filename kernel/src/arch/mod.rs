//! Mimariye ozel kod bu modulun altinda izole edilir (doc S.15 ilke 2).
//! Ortak katmanlar (level0a/level0b1/level0b2) bu modulun disina cikmadan
//! donanimla konusur.
//!
//! Ortak kod **her zaman `arch::cpu`** uzerinden erisir; `arch::i386` gibi
//! somut isimleri kullanmaz. Boylece yeni bir mimari eklemek, ortak
//! katmanlarda tek satir degisiklik gerektirmez.

#[cfg(target_arch = "x86")]
pub mod i386;

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86")]
pub use i386 as cpu;

#[cfg(target_arch = "x86_64")]
pub use x86_64 as cpu;
