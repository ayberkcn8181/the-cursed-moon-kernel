//! The Cursed Moon Kernel (TCMK) -- Rust portu.
//! Faz 1: Boot & Level-0b2 Temeli. Faz 2: Level-0a Cekirdek Temeli.
//!
//! Katman ozeti (bkz. proje dokumantasyonu):
//!   Level-0b2 -> Merkezi Denetleyici (dispatcher/state_monitor/load_balancer/fallback)
//!   Level-0b1 -> Uyumluluk/ceviri katmani (POSIX subsystem)
//!   Level-0a  -> Ana cekirdek/yurutucu (mmu/kmalloc/scheduler + suruculer)

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod arch;
mod boot;
mod level0a;
mod level0b1;
mod level0b2;

use core::panic::PanicInfo;

use level0a::core::scheduler;
use level0b1::linux_subsystem::posix_syscalls::{SYS_EXIT, SYS_WRITE};

#[no_mangle]
pub extern "C" fn kernel_main(multiboot_magic: u32, _multiboot_info_addr: u32) -> ! {
    // Multiboot speknine gore IF=0 girilmesi beklenir; buna guvenmek yerine
    // kesmeleri kendimiz ve kosulsuzca kapatiyoruz (savunmaci varsayim --
    // IDT/PIC hazir olmadan hicbir kesme kabul edilmemeli).
    arch::i386::disable_interrupts();

    level0a::drivers::serial::init();
    level0a::drivers::vga::init();

    if multiboot_magic != 0x2BADB002 {
        level0b2::fallback::emergency(&["Multiboot magic gecersiz -- boot guvenilir degil."]);
    }

    // --- Faz 1: donanim izolasyonu ve kesme altyapisi ---
    level0a::gdt::init();
    level0a::idt::init();
    level0a::pic::remap();
    level0a::pit::init(100);

    level0b2::dispatcher::print_banner();

    // --- Faz 2: Level-0a'yi ayaga kaldir (paging -> heap -> scheduler) ---
    unsafe {
        level0a::core::init::bring_up();
    }

    arch::i386::enable_interrupts();

    // Faz 2 gosterimi: bir worker gorevi olustur. Idle ile arasinda
    // round-robin gidip gelecek ve syscall'larini tam katman zinciri
    // uzerinden yapacak.
    match scheduler::spawn("worker", worker_task) {
        Some(id) => crate::println!("[LEVEL-0a] worker gorevi olusturuldu (id={}).", id),
        None => level0b2::fallback::emergency(&["worker gorevi olusturulamadi."]),
    }

    // Idle gorevi (task 0): sisteme nabiz attirir ve State Monitor'u besler.
    let mut last_report_tick = 0u32;

    loop {
        level0b2::state_monitor::tick();

        if scheduler::needs_resched() {
            scheduler::yield_now();
        }

        // ~5 saniyede bir sistem durumu raporu (100 Hz PIT -> 500 tick).
        let ticks = level0a::pit::ticks();
        if ticks >= last_report_tick + 500 {
            last_report_tick = ticks;
            crate::println!(
                "[LEVEL-0b2] durum: Level-0a={:?} | tick={} nabiz={} gecis={} gorev={}",
                level0b2::state_monitor::health(),
                ticks,
                level0a::pit::heartbeat(),
                scheduler::switch_count(),
                scheduler::task_count()
            );
        }

        arch::i386::halt();
    }
}

/// Faz 2 dogrulama gorevi: syscall'lari Level-1'in yapacagi gibi
/// `int 0x80` ile yapar -- fark yalnizca henuz Ring 0'da olmasidir
/// (Ring 3'e gecis Faz 3).
extern "C" fn worker_task() -> ! {
    const MESSAGE: &[u8] = b"Merhaba: worker gorevinden sys_write!\n";

    for round in 1..=3u32 {
        crate::println!(
            "[worker] tur {} -- scheduler gecis sayisi: {}",
            round,
            scheduler::switch_count()
        );

        let written = unsafe {
            arch::i386::syscall3(
                SYS_WRITE,
                level0a::kernel_api::FD_STDOUT,
                MESSAGE.as_ptr() as u32,
                MESSAGE.len() as u32,
            )
        };
        crate::println!("[worker] sys_write dondu: {} bayt", written as i32);

        // Gecersiz bir fd ile hata yolunu da dogrula (-EBADF = -9 beklenir).
        let bad = unsafe { arch::i386::syscall3(SYS_WRITE, 99, MESSAGE.as_ptr() as u32, 4) };
        crate::println!("[worker] gecersiz fd sonucu: {}", bad as i32);

        scheduler::yield_now();
    }

    crate::println!("[worker] isim bitti, sys_exit cagriliyor.");
    unsafe {
        arch::i386::syscall3(SYS_EXIT, 0, 0, 0);
    }

    // sys_exit geri donmez; yine de tip sistemi icin sonsuz dongu.
    loop {
        arch::i386::halt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    level0b2::fallback::panic_screen(info)
}
