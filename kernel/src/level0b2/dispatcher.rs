//! Cagri Dagitici (doc S.2.2.A): gelen cagrinin turunu analiz eder ve ilgili
//! alt sisteme yonlendirir.
//!
//! Doc S.3'teki yonlendirme zinciri:
//!   Level-1 (int 0x80) -> [BURASI] -> Level-0b1 (POSIX) -> Level-0a (VFS/surucu)
//!
//! Dagitim oncesi State Monitor'a Level-0a'nin sagligi sorulur; Level-0a
//! olu ise cagri Level-0b1'e hic verilmez, Fallback Interface'in sinirli
//! emulasyonuna dusulur (doc S.11).

use crate::arch::i386::regs::SyscallFrame;
use crate::level0b2::{fallback, load_balancer, state_monitor};

pub fn print_banner() {
    crate::println!("[LEVEL-0b2] Central Controller: Active");
}

/// IDT vektor 128'den (int 0x80) gelen Linux uyumlu sistem cagrisi.
pub fn handle_syscall(frame: &mut SyscallFrame) {
    load_balancer::note_call(0x80);

    if state_monitor::level0a_is_dead() {
        fallback::emulate_syscall(frame);
        return;
    }

    crate::level0b1::linux_subsystem::posix_syscalls::dispatch(frame);
}
