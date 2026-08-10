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
use core::sync::atomic::{AtomicU32, Ordering};

use crate::arch::cpu::regs::SyscallFrame;

/// Fallback modunda desteklenen minimal syscall kumesi (Co-Service).
///
/// Kume bilincli olarak kucuktur: amac tam bir isletim ortami sunmak degil,
/// **sistemi uyanik ve etkilesime acik tutmaktir** (doc S.11). Konsola
/// yazma, CPU birakma ve kareyi bitirme -- masaustunun donmamasi ve
/// kullanicinin sisteme hala komut verebilmesi icin gereken asgari kume.
const SYS_EXIT: u32 = 1;
const SYS_WRITE: u32 = 4;
const SYS_WIN_SIZE: u32 = 0x502;
const SYS_WIN_FLUSH: u32 = 0x503;
const SYS_WIN_POLL_KEY: u32 = 0x504;
const SYS_MOUSE_STATE: u32 = 0x505;
const SYS_YIELD: u32 = 0x506;
const SYS_WIN_POS: u32 = 0x507;
const ENOSYS: i32 = 38;

/// Minimal modda reddedilen cagri sayaci.
static UNSUPPORTED: AtomicU32 = AtomicU32::new(0);

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
                let bytes = unsafe { core::slice::from_raw_parts(buf, arg3) };
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
        // Co-Service'in "sistemi ayakta tut" ayagi: CPU'yu birakma yolu
        // acik kalmalidir, aksi halde Ring 3'teki uygulamalar minimal
        // modda birbirini aclia surukler.
        SYS_YIELD | SYS_WIN_FLUSH => {
            crate::level0a::core::scheduler::yield_now();
            0
        }
        // Olay/durum sorgulari: Level-0a'ya hic dokunmadan "olay yok"
        // cevabi verilir. Amac uygulamalari canli tutmaktir -- bir GUI
        // dongusu tus sorgusuna cevap alamazsa takilir.
        SYS_WIN_POLL_KEY | SYS_MOUSE_STATE | SYS_WIN_SIZE | SYS_WIN_POS => 0,

        _ => {
            note_unsupported(number);
            -ENOSYS
        }
    };

    frame.set_return(result as isize as usize);
}

/// Desteklenmeyen cagrilar sayilir, her biri ayri ayri yazdirilmaz.
///
/// Minimal modda saniyede binlerce cagri gelebilir; her birini gunluge
/// yazmak konsolu bogar ve asil olayi (Co-Service'e dusus) gorunmez kilar.
fn note_unsupported(number: u32) {
    let count = UNSUPPORTED.fetch_add(1, Ordering::Relaxed) + 1;
    if count == 1 || count % 512 == 0 {
        crate::println!(
            "[LEVEL-0b2][FALLBACK] minimal modda desteklenmeyen syscall {} (-ENOSYS) -- toplam {}",
            number,
            count
        );
    }
}

/// Co-Service modunda reddedilen cagri sayisi (kabuk `health` komutu).
pub fn unsupported_calls() -> u32 {
    UNSUPPORTED.load(Ordering::Relaxed)
}

/// NT tarafinin Co-Service karsiligi: Level-0a olu iken `int 0x2E`
/// cagrilarini sinirli sekilde karsilar. POSIX tarafiyla ayni felsefe --
/// cift uyumluluk fallback modunda da korunur.
pub fn emulate_nt_syscall(frame: &mut SyscallFrame) {
    use crate::level0b1::nt_subsystem::nt_syscalls::{NT_TERMINATE_PROCESS, NT_WRITE_CONSOLE};

    const STATUS_SUCCESS: u32 = 0x0000_0000;
    const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;
    const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;

    let number = frame.number();
    let [_, arg2, arg3, _, _] = frame.args();

    let status = match number {
        NT_WRITE_CONSOLE => {
            let buf = arg2 as *const u8;
            if buf.is_null() {
                STATUS_ACCESS_VIOLATION
            } else {
                let bytes = unsafe { core::slice::from_raw_parts(buf, arg3) };
                crate::print!("[co-service/nt] ");
                for &byte in bytes {
                    crate::print!("{}", byte as char);
                }
                STATUS_SUCCESS
            }
        }
        NT_TERMINATE_PROCESS => {
            emergency(&["NtTerminateProcess fallback modunda yok sayildi."]);
            STATUS_SUCCESS
        }
        _ => {
            emergency(&["Fallback modunda desteklenmeyen NT servisi."]);
            STATUS_NOT_IMPLEMENTED
        }
    };

    frame.set_return(status as usize);
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
    crate::level0b2::ipc::post(
        crate::level0b2::ipc::Kind::HealthChange,
        0,
        2,
        0,
        "Level-0a olu -- Co-Service devraldi",
    );
}

/// Nabiz geri geldiginde State Monitor cagirir (doc S.11: "Level-0a
/// yeniden baslatma / soft reboot").
///
/// TCMK'de Level-0a ayri bir adres uzayinda degildir; "yeniden baslatma"
/// pratikte **denetimi geri devretmektir**: Co-Service modundan cikilir,
/// cagrilar yeniden Level-0b1 cevirmenlerine yonlendirilir ve olcum
/// pencereleri sifirlanir. Gercek bir yeniden yukleme (servislerin
/// yeniden ayaga kaldirilmasi) surec basina adres uzayi geldiginde
/// anlamli olacaktir (doc Faz 8).
pub fn level0a_recovered(stalled_ticks: u32) {
    crate::println!();
    emergency(&[
        "Level-0a nabzi geri geldi -- denetim ana katmana devrediliyor.",
        "Co-Service modu kapandi.",
    ]);
    crate::level0b2::ipc::post(
        crate::level0b2::ipc::Kind::HealthChange,
        0,
        0,
        stalled_ticks as usize,
        "Level-0a toparlandi -- normal moda donuldu",
    );
}

pub fn panic_screen(info: &PanicInfo) -> ! {
    crate::println!();
    crate::println!("[LEVEL-0b2][FALLBACK] KERNEL PANIC: {}", info);
    loop {
        crate::arch::cpu::disable_interrupts();
        crate::arch::cpu::halt();
    }
}
