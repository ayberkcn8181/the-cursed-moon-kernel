//! Cagri Dagitici: gelen syscall/interrupt turunu analiz edip ilgili alt
//! sisteme yonlendirir. Faz 1'de Level-0b1 (POSIX/NT subsystem) henuz yok,
//! bu yuzden int 0x80 sadece burada gozlemlenip loglanir.

pub fn print_banner() {
    crate::println!("[LEVEL-0b2] Central Controller: Active");
}

pub fn handle_syscall(vector: u8) {
    crate::println!(
        "[LEVEL-0b2] Dispatcher: syscall vektoru {:#x} alindi (Level-0b1 henuz bagli degil, Faz 3).",
        vector
    );
}
