//! The Cursed Moon Kernel (TCMK) -- Rust portu.
//! Faz 1: Boot & Level-0b2 Temeli. Faz 2: Level-0a Cekirdek Temeli.
//! Faz 3: Level-0b1 ELF/POSIX + Ring 3 userland.
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

/// Faz 3/5 kullanici programi. `tools/gen_hello_elf.py` ile uretilir ve
/// RAMFS'e `/bin/hello` olarak baglanir.
static HELLO_ELF: &[u8] = include_bytes!("../../userland/hello.elf");

/// Kullanici programinin VFS uzerinden okuyacagi test dosyasi.
static BOOT_MSG: &[u8] = b"/boot/msg.txt: VFS uzerinden okundu (RAMFS).\n";

/// Acilista RAMFS'e baglanan gomulu dosyalar.
static RAMFS_FILES: &[(&str, &[u8])] = &[("/bin/hello", HELLO_ELF), ("/boot/msg.txt", BOOT_MSG)];

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

    // --- Faz 2/5: Level-0a'yi ayaga kaldir (paging -> heap -> scheduler -> vfs) ---
    unsafe {
        level0a::core::init::bring_up(RAMFS_FILES);
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

    // --- Faz 3: gercek Ring 3 userland ---
    // TSS'i kur, gomulu ELF'i yukle ve iret ile Ring 3'e gec.
    unsafe {
        let kstack = level0a::core::kmalloc::kmalloc_aligned(16 * 1024, 16)
            .expect("TSS icin cekirdek yigini ayrilamadi");
        level0a::gdt::install_tss(kstack.add(16 * 1024) as u32);

        // Faz 5: ikili artik VFS'ten okunur; gomulu imaj yedek yoldur.
        let result = match level0b1::process::run_elf_from_vfs("/bin/hello") {
            Err(level0b1::process::SpawnError::NotFound) => {
                crate::println!("[worker] /bin/hello VFS'te yok, gomulu imaja donuluyor.");
                level0b1::process::run_elf("hello.elf", HELLO_ELF)
            }
            other => other,
        };
        match result {
            Ok(()) => crate::println!("[worker] Ring 3 testi basarili."),
            Err(e) => crate::println!("[worker] Ring 3 testi BASARISIZ: {:?}", e),
        }

        // Izolasyon dogrulamasi: cekirdek sayfalari Ring 3'e kapali kalmali.
        crate::println!(
            "[worker] izolasyon: user@0x300000={} kernel@0x100000={} heap@0x200000={}",
            level0a::core::mmu::is_user_accessible(0x0030_0000),
            level0a::core::mmu::is_user_accessible(0x0010_0000),
            level0a::core::mmu::is_user_accessible(0x0020_0000),
        );

        // Guvenlik regresyon testi: sys_open'a CEKIRDEK isaretcisi verilirse
        // reddedilmeli (-EFAULT = -14). Aksi halde Ring 3 bir kullanici
        // programi cekirdek belleginden veri sizdirabilirdi.
        let kernel_ptr = RAMFS_FILES.as_ptr() as u32;
        let leak = arch::i386::syscall3(
            level0b1::linux_subsystem::posix_syscalls::SYS_OPEN,
            kernel_ptr,
            0,
            0,
        );
        crate::println!(
            "[worker] guvenlik: sys_open(cekirdek isaretcisi) -> {} ({})",
            leak as i32,
            if leak as i32 == -14 { "reddedildi, dogru" } else { "SIZINTI!" }
        );

        // FD sizintisi kontrolu: kullanici programi acti gi dosyayi kapatti mi?
        let msg_size = level0a::core::vfs::lookup("/boot/msg.txt")
            .and_then(level0a::core::vfs::size)
            .unwrap_or(0);
        crate::println!(
            "[worker] vfs: /boot/msg.txt {} bayt | sys_exit sonrasi acik fd: {}",
            msg_size,
            level0a::core::fd::open_count()
        );
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
