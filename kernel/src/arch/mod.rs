//! Mimariye ozel kod bu modulun altinda izole edilir (doc S.15 ilke 2).
//! Ortak katmanlar (level0a/level0b1/level0b2) bu modulun disina cikmadan
//! donanimla konusur.

#[cfg(target_arch = "x86")]
pub mod i386;
