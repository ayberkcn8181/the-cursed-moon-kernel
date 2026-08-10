//! Level-0a: Ana Cekirdek ve Yurutucu Katman.
//! Donanimla dogrudan konusan sistem araclari, cekirdek modulleri ve
//! suruculer buraya toplanir (doc S.2.2.C).

pub mod core;
pub mod drivers;
pub mod exceptions;
pub mod gdt;
pub mod idt;
pub mod gui_api;
pub mod input;
// Onyukleyici zinciri (ve dolayisiyla kurulum) su an yalnizca i386'da.
#[cfg(target_arch = "x86")]
pub mod installer;
pub mod launcher;
pub mod kernel_api;
pub mod keyboard;
pub mod messages;
pub mod pic;
pub mod pit;
pub mod shell;
pub mod wm;

#[cfg(target_arch = "x86_64")]
pub mod syscall_msr;
