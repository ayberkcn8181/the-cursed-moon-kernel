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
/// Elle kodlanmis en kucuk PE: yukleyicinin dar yolunu sinar.
#[cfg(target_arch = "x86")]
static HELLO_EXE: &[u8] = include_bytes!("../../userland/hello.exe");

/// **Derlenmis** Windows uygulamasi: Rust kaynagindan `rust-lld` ile
/// uretilmis, ImageBase 0x00400000'li, `.reloc` bolumu olan gercek bir
/// PE32 GUI programi. ELF uygulamalariyla ayni pencere yoneticisinde
/// kosar; tek farki cekirdege `int 0x2E`/NT ile girmesidir.
#[cfg(target_arch = "x86")]
static WINCLOCK_EXE: &[u8] = include_bytes!("../../userland/winclock.exe");

/// Ithal tablosu (Faz 7b) gosterimi: bu PE tek bir `int 0x2E` icermez.
/// Butun cagrilari `KERNEL32.dll` ve `TCMKGUI.dll`'den ithal eder; ithal
/// tablosunu cozmeyen bir cekirdek onu calistiramaz.
#[cfg(target_arch = "x86")]
static WINPAD_EXE: &[u8] = include_bytes!("../../userland/winpad.exe");

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
/// Preemption testi: hic syscall yapmayan saf hesap dongusu.
#[cfg(target_arch = "x86")]
static SPIN_ELF: &[u8] = include_bytes!("../../userland/spin.elf");
/// Kalici not defteri: metin cizer, TCMKFS'e yazar, acilista geri okur.
#[cfg(target_arch = "x86")]
static NOTES_ELF: &[u8] = include_bytes!("../../userland/notes.elf");
/// execve gosterimi: secilen uygulama menunun YERINE yuklenir.
#[cfg(target_arch = "x86")]
static MENU_ELF: &[u8] = include_bytes!("../../userland/menu.elf");
/// fork gosterimi: tek program, iki surec, ayrisan sayaclar.
#[cfg(target_arch = "x86")]
static TWINS_ELF: &[u8] = include_bytes!("../../userland/twins.elf");
/// pipe gosterimi: ayri adres uzaylarindaki iki surec konusuyor.
#[cfg(target_arch = "x86")]
static RELAY_ELF: &[u8] = include_bytes!("../../userland/relay.elf");
/// stdin gosterimi: klavyeyi `read(0)` ile, POSIX yolundan okur.
#[cfg(target_arch = "x86")]
static ECHO2_ELF: &[u8] = include_bytes!("../../userland/echo2.elf");
/// Sinyal gosterimi: cekirdek uygulamanin akisini kesip isleyicisini cagirir.
#[cfg(target_arch = "x86")]
static SIGDEMO_ELF: &[u8] = include_bytes!("../../userland/sigdemo.elf");
/// Oncelik gosterimi: iki kopya farkli `nice` ile yarisir.
#[cfg(target_arch = "x86")]
static RACE_ELF: &[u8] = include_bytes!("../../userland/race.elf");
/// waitpid(-1) ve gorev yuvasi geri kazanimi gosterimi.
#[cfg(target_arch = "x86")]
static REAPER_ELF: &[u8] = include_bytes!("../../userland/reaper.elf");
/// dup/dup2 gosterimi: stdout boruya yonlendirilir.
#[cfg(target_arch = "x86")]
static REDIRECT_ELF: &[u8] = include_bytes!("../../userland/redirect.elf");
/// poll gosterimi: boru ile klavye ayni cagriyla beklenir.
#[cfg(target_arch = "x86")]
static MUX_ELF: &[u8] = include_bytes!("../../userland/mux.elf");
/// sigprocmask/alarm gosterimi.
#[cfg(target_arch = "x86")]
static MASKED_ELF: &[u8] = include_bytes!("../../userland/masked.elf");
/// mmap/munmap gosterimi.
#[cfg(target_arch = "x86")]
static ARENA_ELF: &[u8] = include_bytes!("../../userland/arena.elf");
/// lseek/fstat gosterimi.
#[cfg(target_arch = "x86")]
static SEEKER_ELF: &[u8] = include_bytes!("../../userland/seeker.elf");
/// getdents gosterimi (POSIX dizin gezgini).
#[cfg(target_arch = "x86")]
static BROWSE_ELF: &[u8] = include_bytes!("../../userland/browse.elf");
/// pause/sigsuspend gosterimi.
#[cfg(target_arch = "x86")]
static WAITER_ELF: &[u8] = include_bytes!("../../userland/waiter.elf");
/// cwd'nin fork/execve ile devri.
#[cfg(target_arch = "x86")]
static HEIR_ELF: &[u8] = include_bytes!("../../userland/heir.elf");
/// sigaction bayraklari ve ic ice sinyal teslimi.
#[cfg(target_arch = "x86")]
static NESTED_ELF: &[u8] = include_bytes!("../../userland/nested.elf");
/// FindFirstFileA gosterimi -- ayni dizinler, Win32 yuzu.
#[cfg(target_arch = "x86")]
static WINFILES_EXE: &[u8] = include_bytes!("../../userland/winfiles.exe");

/// Linux (ELF64, x86_64) kullanici programi -- `tools/gen_hello_elf64.py`.
/// Elle kodlanmis en kucuk ELF64: yukleyicinin dar yolunu sinar.
#[cfg(target_arch = "x86_64")]
static HELLO_ELF64: &[u8] = include_bytes!("../../userland/hello64.elf");

/// **Rust ile yazilmis** x86_64 uygulamalari. i386'dakilerle ayni kaynak;
/// degisen yalnizca sistem cagrisi bicimi (`syscall` komutu) ve Linux
/// numaralaridir (bkz. `userland-rs/src/sys.rs`).
#[cfg(target_arch = "x86_64")]
static HELLO64_RS: &[u8] = include_bytes!("../../userland/hello.elf64");
#[cfg(target_arch = "x86_64")]
static PLASMA64: &[u8] = include_bytes!("../../userland/plasma.elf64");
#[cfg(target_arch = "x86_64")]
static PAINT64: &[u8] = include_bytes!("../../userland/paint.elf64");
#[cfg(target_arch = "x86_64")]
static NOTES64: &[u8] = include_bytes!("../../userland/notes.elf64");
#[cfg(target_arch = "x86_64")]
static TWINS64: &[u8] = include_bytes!("../../userland/twins.elf64");
#[cfg(target_arch = "x86_64")]
static RELAY64: &[u8] = include_bytes!("../../userland/relay.elf64");
#[cfg(target_arch = "x86_64")]
static MENU64: &[u8] = include_bytes!("../../userland/menu.elf64");
#[cfg(target_arch = "x86_64")]
static CRASH64: &[u8] = include_bytes!("../../userland/crash.elf64");
#[cfg(target_arch = "x86_64")]
static ECHO2_64: &[u8] = include_bytes!("../../userland/echo2.elf64");
#[cfg(target_arch = "x86_64")]
static SIGDEMO64: &[u8] = include_bytes!("../../userland/sigdemo.elf64");
#[cfg(target_arch = "x86_64")]
static RACE64: &[u8] = include_bytes!("../../userland/race.elf64");
#[cfg(target_arch = "x86_64")]
static REAPER64: &[u8] = include_bytes!("../../userland/reaper.elf64");
#[cfg(target_arch = "x86_64")]
static REDIRECT64: &[u8] = include_bytes!("../../userland/redirect.elf64");
#[cfg(target_arch = "x86_64")]
static MUX64: &[u8] = include_bytes!("../../userland/mux.elf64");
#[cfg(target_arch = "x86_64")]
static MASKED64: &[u8] = include_bytes!("../../userland/masked.elf64");
#[cfg(target_arch = "x86_64")]
static ARENA64: &[u8] = include_bytes!("../../userland/arena.elf64");
#[cfg(target_arch = "x86_64")]
static SEEKER64: &[u8] = include_bytes!("../../userland/seeker.elf64");
#[cfg(target_arch = "x86_64")]
static BROWSE64: &[u8] = include_bytes!("../../userland/browse.elf64");
#[cfg(target_arch = "x86_64")]
static WAITER64: &[u8] = include_bytes!("../../userland/waiter.elf64");
#[cfg(target_arch = "x86_64")]
static HEIR64: &[u8] = include_bytes!("../../userland/heir.elf64");
#[cfg(target_arch = "x86_64")]
static NESTED64: &[u8] = include_bytes!("../../userland/nested.elf64");

/// **Windows (PE32+) uygulamalari** -- i386'dakilerle ayni kaynak, ayni
/// ithal kutuphaneleri; degisen yalnizca hedef. Taban 0x140000000
/// (64-bit Windows gelenegi) oldugu icin yukleyici DIR64 yeniden
/// yerlesimini uygulamak zorundadir.
#[cfg(target_arch = "x86_64")]
static WINCLOCK_EXE64: &[u8] = include_bytes!("../../userland/winclock.exe64");
#[cfg(target_arch = "x86_64")]
static WINPAD_EXE64: &[u8] = include_bytes!("../../userland/winpad.exe64");
#[cfg(target_arch = "x86_64")]
static WINFILES_EXE64: &[u8] = include_bytes!("../../userland/winfiles.exe64");

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
    ("/bin/spin", SPIN_ELF),
    ("/bin/notes", NOTES_ELF),
    ("/bin/menu", MENU_ELF),
    ("/bin/twins", TWINS_ELF),
    ("/bin/relay", RELAY_ELF),
    ("/bin/echo2", ECHO2_ELF),
    ("/bin/sigdemo", SIGDEMO_ELF),
    ("/bin/race", RACE_ELF),
    ("/bin/reaper", REAPER_ELF),
    ("/bin/redirect", REDIRECT_ELF),
    ("/bin/mux", MUX_ELF),
    ("/bin/masked", MASKED_ELF),
    ("/bin/arena", ARENA_ELF),
    ("/bin/seeker", SEEKER_ELF),
    ("/bin/browse", BROWSE_ELF),
    ("/bin/waiter", WAITER_ELF),
    ("/bin/heir", HEIR_ELF),
    ("/bin/nested", NESTED_ELF),
    ("/bin/winclock.exe", WINCLOCK_EXE),
    ("/bin/winpad.exe", WINPAD_EXE),
    ("/bin/winfiles.exe", WINFILES_EXE),
    ("/boot/msg.txt", BOOT_MSG),
];

#[cfg(target_arch = "x86_64")]
static RAMFS_FILES: &[(&str, &[u8])] = &[
    ("/bin/hello", HELLO_ELF64),
    ("/bin/hello64", HELLO64_RS),
    ("/bin/plasma", PLASMA64),
    ("/bin/paint", PAINT64),
    ("/bin/notes", NOTES64),
    ("/bin/twins", TWINS64),
    ("/bin/relay", RELAY64),
    ("/bin/menu", MENU64),
    ("/bin/crash", CRASH64),
    ("/bin/echo2", ECHO2_64),
    ("/bin/sigdemo", SIGDEMO64),
    ("/bin/race", RACE64),
    ("/bin/reaper", REAPER64),
    ("/bin/redirect", REDIRECT64),
    ("/bin/mux", MUX64),
    ("/bin/masked", MASKED64),
    ("/bin/arena", ARENA64),
    ("/bin/seeker", SEEKER64),
    ("/bin/browse", BROWSE64),
    ("/bin/waiter", WAITER64),
    ("/bin/heir", HEIR64),
    ("/bin/nested", NESTED64),
    ("/bin/winclock.exe", WINCLOCK_EXE64),
    ("/bin/winpad.exe", WINPAD_EXE64),
    ("/bin/winfiles.exe", WINFILES_EXE64),
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

    // --- Idle gorevi (task 0) ---
    //
    // Bu gorev bir donem ayni zamanda **masaustu dongusuydu**: girdiyi
    // isliyor, pencereleri birlestiriyor, State Monitor'u besliyordu.
    // Olcum bunun bedelini gosterdi -- zamanlayici tiklerinin ~%95'i
    // buraya yaziliyordu ve Ring 3 uygulamalari kalanini paylasiyordu.
    //
    // Artik masaustu kendi gorevindedir (`desktop_task`) ve 0 numara
    // **gercek bir idle**: `pick_next` onu yalnizca baska hicbir gorev
    // calistirilabilir degilken secer, o da `hlt` ile bir sonraki
    // kesmeye kadar CPU'yu tamamen serbest birakir.
    //
    // `hlt` sonrasi `yield_now`: kesme yeni bir gorevi hazir yapmis
    // olabilir (uyku suresi doldu, disk kesmesi geldi). Baska hazir gorev
    // yoksa `yield_now` baglam degistirmeden doner, yani bu dongu bos
    // dondugunde bile ucuzdur.
    loop {
        arch::cpu::halt();
        scheduler::yield_now();
    }
}

/// Masaustu dongusu: girdi, kompozitor, State Monitor ve durum raporu.
///
/// Ayri bir gorev olmasi bilincli. Idle ile birlesikken zamanlayici
/// muhasebesinin disinda kaliyordu: her turda secilip CPU'yu tuketiyor,
/// ama oncelik butcesine tabi olmadigi icin kimse onu sinirlayamiyordu.
/// Normal bir gorev olarak artik `nice` degeri, donem hakki ve preemption
/// ona da uygulanir -- yani masaustu, sistemin geri kalaniyla ayni
/// kurallara tabidir.
extern "C" fn desktop_task() -> ! {
    /// Kompozitor tavani: en fazla iki tikta bir tam ekran birlestirme.
    ///
    /// 1024x768x32 bir kareyi saniyede yuzlerce kez uretmenin karsiligi
    /// yok; ekran zaten o hizda degismiyor. Iki tik (=20 ms, ~50 kare/sn)
    /// gozle fark edilmeyen ama uygulamalara CPU birakan bir sinirdir.
    const COMPOSE_INTERVAL_TICKS: u32 = 2;

    let mut last_report_tick = 0u32;
    let mut last_compose_tick = 0u32;

    loop {
        level0b2::state_monitor::tick();

        // Level-0b2 -> Level-0a mesaj kuyrugu (doc S.10 Faz 4+).
        level0a::messages::drain();

        // Girdi her turda islenir (gecikmesi fark edilir), kompozitor
        // sinirlanir.
        if level0a::wm::active() {
            level0a::wm::handle_input();
            let now = level0a::pit::ticks();
            if now.wrapping_sub(last_compose_tick) >= COMPOSE_INTERVAL_TICKS {
                last_compose_tick = now;
                level0a::wm::compose();
            }
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

        // Sirayi birak. Kosulsuz birakmak burada guvenlidir cunku bu gorev
        // artik donem hakkina tabidir: hakki bitince secilmez, yani eski
        // "her turda yield" denemesindeki baglam degisimi firtinasi
        // olusamaz.
        scheduler::yield_now();
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
        //
        // Surec basina adres uzayindan sonra kullanici bolgesi de `false`
        // dondurur: bolge yalnizca bir surecin adres uzayinda VARDIR,
        // cekirdek uzayinda hic eslenmez.
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

    // Masaustu artik kendi gorevinde kosar; bu satirdan once cagrilamaz,
    // cunku gorev ilk turunda `wm`'in hazir oldugunu varsayar.
    // Masaustu gorevi UYUTULAMAZ olarak isaretlenir: ekrani cizen,
    // girdiyi dagitan ve kabugu kosturan gorev odur (bkz.
    // `scheduler::mark_no_block`).
    match scheduler::spawn("desktop", desktop_task) {
        Some(id) => {
            scheduler::mark_no_block(id);
            crate::println!("[LEVEL-0a] masaustu gorevi olusturuldu (id={}).", id);
        }
        None => level0b2::fallback::emergency(&["masaustu gorevi olusturulamadi."]),
    }

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
