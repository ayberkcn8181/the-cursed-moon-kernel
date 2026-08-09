//! Fallback Interface: Level-0a kilitlenir/cokerse yonetimi devralir
//! (doc S.2.2.A, S.11).
//!
//! Faz 1 yetenekleri: VGA'ya acil durum mesaji, sistemin kilitlenmesini
//! onleme (hlt dongusu).
//! Faz 2 yetenekleri: **basit sys_write emulasyonu** -- Level-0a olu iken
//! gelen yazma cagrilari Level-0b1'e hic ugramadan burada, dogrudan konsola
//! islenir. Bu, doc S.11'deki "Co-Service" fikrinin ilk somut ornegidir:
//! ana katman olse bile sistem konusmaya devam edebilir.

use core::panic::PanicInfo;

use crate::arch::i386::regs::SyscallFrame;

/// Fallback modunda desteklenen minimal syscall kumesi.
const SYS_EXIT: u32 = 1;
const SYS_WRITE: u32 = 4;
const ENOSYS: i32 = 38;

pub fn emergency(lines: &[&str]) {
    for line in lines {
        crate::println!("[LEVEL-0b2][FALLBACK] {}", line);
    }
}

/// Level-0a olu iken syscall'lari sinirli sekilde karsilar (Co-Service).
pub fn emulate_syscall(frame: &mut SyscallFrame) {
    let number = frame.number();
    let [arg1, arg2, arg3, _, _] = frame.args();

    let result: i32 = match number {
        SYS_WRITE => {
            // Level-0a'nin VFS'ine guvenmeden dogrudan konsola yaz.
            let buf = arg2 as *const u8;
            if buf.is_null() {
                -ENOSYS
            } else {
                let bytes = unsafe { core::slice::from_raw_parts(buf, arg3 as usize) };
                crate::print!("[co-service] ");
                for &byte in bytes {
                    crate::print!("{}", byte as char);
                }
                arg3 as i32
            }
        }
        SYS_EXIT => {
            emergency(&["sys_exit fallback modunda yok sayildi (gorev zaten durdu)."]);
            let _ = arg1;
            0
        }
        _ => {
            emergency(&["Fallback modunda desteklenmeyen syscall (-ENOSYS)."]);
            -ENOSYS
        }
    };

    frame.set_return(result as u32);
}

/// State Monitor, heartbeat kayboldugunu tespit ettiginde cagirir.
///
/// Faz 1'de burasi dogrudan `hlt` dongusune giriyordu. Faz 2'den itibaren
/// **geri doner**: boylece sistem uyanik kalir ve gelen syscall'lar
/// `emulate_syscall` ile Co-Service olarak islenmeye devam eder
/// (doc S.11: "Level-0a cokerse Level-0b2 sistemi hayatta tutar").
pub fn level0a_dead() {
    crate::println!();
    emergency(&[
        "Level-0a YANIT VERMIYOR (heartbeat kayboldu).",
        "Fallback Interface devrede - Co-Service'ler ile minimal mod.",
    ]);
}

pub fn panic_screen(info: &PanicInfo) -> ! {
    crate::println!();
    crate::println!("[LEVEL-0b2][FALLBACK] KERNEL PANIC: {}", info);
    loop {
        crate::arch::i386::disable_interrupts();
        crate::arch::i386::halt();
    }
}
