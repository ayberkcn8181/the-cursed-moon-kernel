//! The Cursed Moon Kernel (TCMK) -- Rust portu, Faz 1: Boot & Level-0b2 Temeli.
//!
//! Katman ozeti (bkz. proje dokumantasyonu):
//!   Level-0b2 -> Merkezi Denetleyici (dispatcher/state_monitor/load_balancer/fallback)
//!   Level-0b1 -> Uyumluluk/ceviri katmani (Faz 1'de stub)
//!   Level-0a  -> Ana cekirdek/yurutucu (gdt/idt/pic/pit/keyboard/vga)

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod arch;
mod boot;
mod level0a;
mod level0b1;
mod level0b2;

use core::panic::PanicInfo;

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

    level0a::gdt::init();
    level0a::idt::init();
    level0a::pic::remap();
    level0a::pit::init(100);
    arch::i386::enable_interrupts();

    level0b2::dispatcher::print_banner();

    // Faz 1 dogrulamasi: henuz Ring 3 kullanici uygulamasi yok, bu yuzden
    // int 0x80 hattinin Level-0b2 dispatcher'ina ulastigini cekirdek
    // kendi kendine tetikleyerek kanitlar (doc S.12 Test Plani).
    unsafe {
        arch::i386::int80_selftest();
    }

    loop {
        level0b2::state_monitor::tick();
        arch::i386::halt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    level0b2::fallback::panic_screen(info)
}
