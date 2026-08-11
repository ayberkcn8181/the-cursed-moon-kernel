//! Windows NT alt sistemi -- PE ikilileri ve NT syscall ABI'si.

// Gomulu DLL tablosu yalnizca PE yukleyicisi tarafindan kullanilir,
// o da yalnizca i386'da derlenir (bkz. binary_loader/mod.rs).
#[cfg(target_arch = "x86")]
pub mod dll;
pub mod nt_syscalls;
