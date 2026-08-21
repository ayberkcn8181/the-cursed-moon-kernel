//! Cagri Dagitici (doc S.2.2.A): gelen cagrinin turunu analiz eder ve ilgili
//! alt sisteme yonlendirir.
//!
//! Doc S.3'teki yonlendirme zinciri:
//!   Level-1 (int 0x80) -> [BURASI] -> Level-0b1 (POSIX) -> Level-0a (VFS/surucu)
//!
//! Dagitim oncesi iki denetim yapilir:
//!
//!   1. **Yuk Dengeleyici** cagriyi kanalina ve gorevine yazar. Gorev
//!      pencere kotasini asmissa cagri islenmeden once CPU birakilir --
//!      isbirlikci zamanlamada donen bir uygulamanin sistemi aclia
//!      surukleme yolu budur ve tek savunma noktasi burasidir.
//!   2. **State Monitor**'a Level-0a'nin sagligi sorulur; Level-0a olu ise
//!      cagri Level-0b1'e hic verilmez, Fallback Interface'in sinirli
//!      emulasyonuna dusulur (doc S.11).

use crate::arch::cpu::regs::SyscallFrame;
use crate::level0a::core::scheduler;
use crate::level0b1::linux_subsystem::posix_syscalls as posix;
use crate::level0b2::load_balancer::{Channel, Verdict};
use crate::level0b2::{fallback, load_balancer, state_monitor};

pub fn print_banner() {
    crate::println!("[LEVEL-0b2] Central Controller: Active");
}

/// Bir POSIX cagrisini olcum kanalina yerlestirir.
///
/// Ayrim, cagrinin sistemin hangi kaynagini zorladigina gore yapilir:
/// dosya cagrilari VFS'i, GUI cagrilari kompozitoru, bellek cagrilari
/// ayiriciyi meshgul eder. Doygunlugun hangi kanalda oldugunu bilmek,
/// darbogazin nerede oldugunu bilmek demektir.
fn classify(number: u32) -> Channel {
    match number {
        posix::SYS_READ | posix::SYS_WRITE | posix::SYS_OPEN | posix::SYS_CLOSE => {
            Channel::PosixFile
        }
        posix::SYS_BRK => Channel::PosixMem,
        0x500..=0x5FF => Channel::Gui,
        _ => Channel::PosixFile,
    }
}

/// Linux uyumlu sistem cagrisi.
///
/// `from_interrupt`: cagri bir **kesme kapisindan** mi geldi (int 0x80)
/// yoksa `syscall` komutundan mi? i386'da tek yol var ve bayrak yok
/// sayilir; x86_64'te ikisi donus bilgisini baska yerde tasir, yani
/// cerceveyi yazacak kod bunu bilmek **zorundadir** (bkz.
/// `SyscallFrame::user_context_via`).
pub fn handle_syscall(frame: &mut SyscallFrame, from_interrupt: bool) {
    let task = scheduler::current_id();

    if load_balancer::note_call(classify(frame.number()), task) == Verdict::Throttle {
        // Kuyruga alma yerine geri baski: cagri kaybolmaz, gorev sirasini
        // bekler. Ring 3'ten gelen cagrilarda bu guvenlidir -- `win_flush`
        // zaten ayni yoldan CPU birakiyor.
        scheduler::yield_now();
    }

    if state_monitor::level0a_is_dead() {
        fallback::emulate_syscall(frame);
        return;
    }

    posix::dispatch(frame, from_interrupt);

    // Cagri islendi, Ring 3'e donulmek uzere: **sinyal teslim noktasi**
    // burasidir. Bekleyen bir sinyal varsa cerceve isleyiciye cevrilir,
    // yani surec cagirdigi yere degil isleyiciye doner.
    //
    // Teslimin kesme isleyicisinde degil de burada yapilmasi bilincli:
    // burada cercevenin Ring 3'ten geldigi kesindir ve `set_user_context`
    // ile guvenle yazilabilir.
    unsafe { crate::level0b1::signal::deliver_pending(frame, from_interrupt) };
}

/// IDT vektor 46'dan (int 0x2E) gelen Windows NT uyumlu sistem cagrisi.
///
/// Dagitici acisindan POSIX ile NT arasindaki tek fark hangi cevirmene
/// verildigidir; durum kontrolu, yuk olcumu ve fallback yolu ortaktir.
/// Doc S.1'in "cift uyumluluk" vaadi tam olarak burada somutlasir.
pub fn handle_nt_syscall(frame: &mut SyscallFrame, from_interrupt: bool) {
    let task = scheduler::current_id();

    if load_balancer::note_call(Channel::Nt, task) == Verdict::Throttle {
        scheduler::yield_now();
    }

    if state_monitor::level0a_is_dead() {
        fallback::emulate_nt_syscall(frame);
        return;
    }

    crate::level0b1::nt_subsystem::nt_syscalls::dispatch(frame, from_interrupt);

    // Sinyaller Windows sureclerinde de gecerlidir: cekirdek acisindan
    // ikisi de ayni gorev tablosundaki ayni turden bir surectir. Bir PE
    // uygulamasi `SIGKILL`/`SIGTERM` ile durur; isleyici kaydetmek icin
    // POSIX cagrisini kullanmasi gerekir, ki bu da mumkun.
    unsafe { crate::level0b1::signal::deliver_pending(frame, from_interrupt) };
}
