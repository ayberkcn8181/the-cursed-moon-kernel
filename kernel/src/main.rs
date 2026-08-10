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

/// Mimarinin makine kelimesi -- syscall ABI'si bu genislikte calisir.
#[cfg(target_arch = "x86")]
type Word = u32;
#[cfg(target_arch = "x86_64")]
type Word = u64;

/// Bootloader'in verdigi imza: i386 Multiboot1, x86_64 Multiboot2.
#[cfg(target_arch = "x86")]
const BOOT_MAGIC: u32 = 0x2BAD_B002;
#[cfg(target_arch = "x86_64")]
const BOOT_MAGIC: u32 = 0x36D7_6289;

/// Linux (ELF32, i386) kullanici programi -- `tools/gen_hello_elf.py`.
#[cfg(target_arch = "x86")]
static HELLO_ELF: &[u8] = include_bytes!("../../userland/hello.elf");

/// Windows (PE32, i386) kullanici programi -- `tools/gen_pe_hello.py`.
#[cfg(target_arch = "x86")]
static HELLO_EXE: &[u8] = include_bytes!("../../userland/hello.exe");

/// Ring 3 GUI uygulamalari -- `tools/gen_gui_app.py`.
#[cfg(target_arch = "x86")]
static PAINT_ELF: &[u8] = include_bytes!("../../userland/paint.elf");
#[cfg(target_arch = "x86")]
static PLASMA_ELF: &[u8] = include_bytes!("../../userland/plasma.elf");
#[cfg(target_arch = "x86")]
static CRASH_ELF: &[u8] = include_bytes!("../../userland/crash.elf");
/// Yuk dengeleyici testi: CPU birakmadan syscall yagmuru yapar.
#[cfg(target_arch = "x86")]
static HOG_ELF: &[u8] = include_bytes!("../../userland/hog.elf");

/// Linux (ELF64, x86_64) kullanici programi -- `tools/gen_hello_elf64.py`.
#[cfg(target_arch = "x86_64")]
static HELLO_ELF64: &[u8] = include_bytes!("../../userland/hello64.elf");

/// Kullanici programlarinin VFS uzerinden okudugu test dosyasi.
static BOOT_MSG: &[u8] = b"/boot/msg.txt: VFS uzerinden okundu (RAMFS).\n";

/// Acilista RAMFS'e baglanan gomulu dosyalar.
#[cfg(target_arch = "x86")]
static RAMFS_FILES: &[(&str, &[u8])] = &[
    ("/bin/hello", HELLO_ELF),
    ("/bin/hello.exe", HELLO_EXE),
    ("/bin/paint", PAINT_ELF),
    ("/bin/plasma", PLASMA_ELF),
    ("/bin/crash", CRASH_ELF),
    ("/bin/hog", HOG_ELF),
    ("/boot/msg.txt", BOOT_MSG),
];

#[cfg(target_arch = "x86_64")]
static RAMFS_FILES: &[(&str, &[u8])] = &[
    ("/bin/hello", HELLO_ELF64),
    ("/boot/msg.txt", BOOT_MSG),
];

/// Ring 3'te denenecek ikililer (mimariye gore).
#[cfg(target_arch = "x86")]
const USER_BINARIES: &[&str] = &["/bin/hello", "/bin/hello.exe"];
#[cfg(target_arch = "x86_64")]
const USER_BINARIES: &[&str] = &["/bin/hello"];

#[no_mangle]
pub extern "C" fn kernel_main(multiboot_magic: u32, multiboot_info_addr: usize) -> ! {
    // Multiboot speknine gore IF=0 girilmesi beklenir; buna guvenmek yerine
    // kesmeleri kendimiz ve kosulsuzca kapatiyoruz (savunmaci varsayim --
    // IDT/PIC hazir olmadan hicbir kesme kabul edilmemeli).
    arch::cpu::disable_interrupts();

    level0a::drivers::serial::init();
    level0a::drivers::vga::init();

    // Framebuffer'i mumkun oldugunca erken ac: boot logu da grafik
    // konsolda goruntulensin (doc S.7 Faz 13).
    unsafe {
        let fb = boot::multiboot::framebuffer(multiboot_info_addr);
        level0a::drivers::gfx::init(fb);
    }

    if multiboot_magic != BOOT_MAGIC {
        level0b2::fallback::emergency(&["Multiboot magic gecersiz -- boot guvenilir degil."]);
    }

    // --- Faz 1: donanim izolasyonu ve kesme altyapisi ---
    level0a::gdt::init();
    level0a::idt::init();
    level0a::pic::remap();
    level0a::pit::init(100);
    unsafe { level0a::input::init_mouse() };

    level0b2::dispatcher::print_banner();

    // --- Faz 2/5: Level-0a'yi ayaga kaldir (paging -> heap -> scheduler -> vfs) ---
    unsafe {
        level0a::core::init::bring_up(RAMFS_FILES);
    }

    arch::cpu::enable_interrupts();

    // Faz 2 gosterimi: bir worker gorevi olustur. Idle ile arasinda
    // round-robin gidip gelecek ve syscall'larini tam katman zinciri
    // uzerinden yapacak.
    match scheduler::spawn("worker", worker_task) {
        Some(id) => crate::println!("[LEVEL-0a] worker gorevi olusturuldu (id={}).", id),
        None => level0b2::fallback::emergency(&["worker gorevi olusturulamadi."]),
    }

    // Idle gorevi (task 0) ayni zamanda **masaustu dongusudur**:
    // girdiyi isler, pencereleri birlestirir ve State Monitor'u besler.
    let mut last_report_tick = 0u32;

    loop {
        level0b2::state_monitor::tick();

        // Level-0b2 -> Level-0a mesaj kuyrugu (doc S.10 Faz 4+).
        level0a::messages::drain();

        if level0a::wm::active() {
            level0a::wm::handle_input();
            level0a::wm::compose();
        }

        if scheduler::needs_resched() {
            scheduler::yield_now();
        }

        // ~10 saniyede bir sistem durumu raporu (100 Hz PIT).
        let ticks = level0a::pit::ticks();
        if ticks >= last_report_tick + 1000 {
            last_report_tick = ticks;
            crate::println!(
                "[LEVEL-0b2] durum: Level-0a={:?} tick={} gorev={} pencere={}",
                level0b2::state_monitor::health(),
                ticks,
                scheduler::task_count(),
                level0a::wm::window_count()
            );
        }

        if !level0a::wm::active() {
            arch::cpu::halt();
        }
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
            arch::cpu::syscall3(
                SYS_WRITE as Word,
                level0a::kernel_api::FD_STDOUT as Word,
                MESSAGE.as_ptr() as Word,
                MESSAGE.len() as Word,
            )
        };
        crate::println!("[worker] sys_write dondu: {} bayt", written as i32);

        // Gecersiz bir fd ile hata yolunu da dogrula (-EBADF = -9 beklenir).
        let bad =
            unsafe { arch::cpu::syscall3(SYS_WRITE as Word, 99, MESSAGE.as_ptr() as Word, 4) };
        crate::println!("[worker] gecersiz fd sonucu: {}", bad as i32);

        scheduler::yield_now();
    }

    // --- Faz 3: gercek Ring 3 userland ---
    // TSS'i kur, gomulu ELF'i yukle ve iret ile Ring 3'e gec.
    unsafe {
        let kstack = level0a::core::kmalloc::kmalloc_aligned(16 * 1024, 16)
            .expect("TSS icin cekirdek yigini ayrilamadi");
        let kstack_top = kstack.add(16 * 1024) as usize;
        level0a::gdt::install_tss(kstack_top);

        // x86_64'te Linux'un asil yolu `syscall` komutudur (doc S.15).
        #[cfg(target_arch = "x86_64")]
        level0a::syscall_msr::init(kstack_top);

        // --- Faz 3/5/7: CIFT UYUMLULUK GOSTERIMI ---
        // Ayni cekirdek, ayni Ring 3 ortami, iki farkli isletim sistemi
        // ikilisi. Fark yalnizca Level-0b1'deki cevirmendedir (doc S.1).
        for &path in USER_BINARIES {
            crate::println!();
            let result = match level0b1::process::run_from_vfs(path) {
                Err(level0b1::process::SpawnError::NotFound) => {
                    crate::println!("[worker] {} VFS'te bulunamadi.", path);
                    Err(level0b1::process::SpawnError::NotFound)
                }
                other => other,
            };
            match result {
                Ok(()) => crate::println!("[worker] {} -> Ring 3 testi basarili.", path),
                Err(e) => crate::println!("[worker] {} -> Ring 3 testi BASARISIZ: {:?}", path, e),
            }
        }
        crate::println!();

        // Izolasyon dogrulamasi: cekirdek sayfalari Ring 3'e kapali kalmali.
        // Adresler sabit yazilmaz: bellek haritasi degistiginde test de
        // kendiliginden dogru yeri kontrol etsin.
        crate::println!(
            "[worker] izolasyon: user@{:#x}={} kernel@{:#x}={} heap@{:#x}={}",
            level0a::core::mmu::USER_MEM_START,
            level0a::core::mmu::is_user_accessible(level0a::core::mmu::USER_MEM_START),
            0x0010_0000,
            level0a::core::mmu::is_user_accessible(0x0010_0000),
            level0a::core::kmalloc::HEAP_START,
            level0a::core::mmu::is_user_accessible(level0a::core::kmalloc::HEAP_START),
        );

        // Guvenlik regresyon testi: sys_open'a CEKIRDEK isaretcisi verilirse
        // reddedilmeli (-EFAULT = -14). Aksi halde Ring 3 bir kullanici
        // programi cekirdek belleginden veri sizdirabilirdi.
        let kernel_ptr = RAMFS_FILES.as_ptr() as Word;
        let leak = arch::cpu::syscall3(
            level0b1::linux_subsystem::posix_syscalls::SYS_OPEN as Word,
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

    // --- GUI'yi baslat (grafiksel alfa) ---
    start_desktop();

    crate::println!("[worker] isim bitti, sys_exit cagriliyor.");
    unsafe {
        arch::cpu::syscall3(SYS_EXIT as Word, 0, 0, 0);
    }

    // sys_exit geri donmez; yine de tip sistemi icin sonsuz dongu.
    loop {
        arch::cpu::halt();
    }
}

/// Masaustunu ayaga kaldirir: WM devralir, sistem pencereleri acilir ve
/// iki Ring 3 GUI uygulamasi baslatilir.
fn start_desktop() {
    if !level0a::drivers::gfx::available() {
        crate::println!("[worker] framebuffer yok; GUI atlaniyor.");
        return;
    }

    level0a::wm::start();

    // Sistem gunlugu penceresi: cekirdek kaydini canli gosterir.
    level0a::wm::create("Sistem Gunlugu", 380, 60, 610, 200, true);

    // Etkilesimli kabuk.
    level0a::shell::start(30, 300);

    // Ring 3 GUI uygulamalari (her biri kendi gorevinde).
    #[cfg(target_arch = "x86")]
    for app in ["paint", "plasma"] {
        match level0a::launcher::spawn_user_app(app) {
            Ok(()) => crate::println!("[worker] '{}' baslatildi.", app),
            Err(e) => crate::println!("[worker] '{}' baslatilamadi: {}", app, e),
        }
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    level0b2::fallback::panic_screen(info)
}
