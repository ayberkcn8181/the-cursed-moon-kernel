//! Level-0b1: Uyumluluk ve Ceviri Katmani (doc S.2.2.B).
//!
//! Farkli isletim sistemleri icin yazilmis yazilimlarin, donanima ulasmadan
//! onceki tercume merkezi. Buradaki hicbir modul donanima dogrudan
//! dokunmaz -- yalnizca cagri sozlesmesi cevirisi yapip Level-0a'nin ortak
//! API'sine devreder.
//!
//! Durum:
//!   - `linux_subsystem`  : POSIX cevirisi (Faz 2: sys_write, sys_exit)
//!   - `nt_subsystem`     : Faz 7 (NtWriteFile vb.)
//!   - `binary_loader`    : Faz 3 (ELF), Faz 7 (PE)

pub mod binary_loader;
pub mod linux_subsystem;
pub mod nt_subsystem;
pub mod fork;
pub mod process;
pub mod signal;
