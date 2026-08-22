//! Windows NT alt sistemi -- PE ikilileri ve NT syscall ABI'si.

// Gomulu DLL tablosu iki mimarida da gereklidir: PE32 (i386) ve PE32+
// (x86_64) yukleyicileri ayni ihracat tablosunu kullanir, yalnizca
// urettikleri thunk'in cagri gelenegi degisir (bkz. dll::emit_thunk).
pub mod dll;
/// Modul tablosu: GetModuleHandleA / GetProcAddress / LoadLibraryA.
pub mod modules;
pub mod nt_syscalls;
/// Windows istisna dagitimi (SEH zinciri + vektorlu isleyiciler).
pub mod seh;
pub mod teb;
