//! `tcmk` -- TCMK Ring 3 uygulamalari icin kucuk bir standart kutuphane.
//!
//! Bu kutuphane, projenin **Level-1** ayagidir (doc S.2.3: "Kullanici
//! Katmani ... libc / win32_api"). Simdiye kadarki Ring 3 uygulamalari
//! `tools/gen_*.py` icinde elle kodlanmis makine diliyle uretiliyordu;
//! bu yontem "Ring 3 gercekten calisiyor" iddiasini kanitlamak icin
//! yeterliydi ama uygulama yazmayi pratikte imkansiz kiliyordu.
//!
//! Buradaki kutuphaneyle bir TCMK uygulamasi yazmak siradan Rust yazmaya
//! doner:
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! tcmk::entry!(main);
//!
//! fn main() {
//!     let mut win = tcmk::gui::Window::open("Ornek", 30, 60, 320, 200).unwrap();
//!     loop {
//!         win.clear(0x0010_2030);
//!         if win.poll_key() == b'q' { break; }
//!         win.flush();
//!     }
//! }
//! ```
//!
//! ## Tek kutuphane, iki ABI
//!
//! Ayni kaynak hem **ELF** hem **PE** olarak derlenir; hangi tarafta
//! oldugunu hedef belirler:
//!
//! | | ELF (`i686-tcmk`) | PE (`i686-tcmk-win`) |
//! |---|---|---|
//! | sistem cagrisi | `int 0x80` (`sys`) | `int 0x2E` (`nt`) |
//! | pencere | `gui::Window` | `win32::Hwnd` |
//! | cikis | `sys_exit` | `NtTerminateProcess` |
//! | cizim | `canvas::Canvas` | `canvas::Canvas` |
//!
//! Son satir onemli: cizim kodu **ortaktir**. Iki uygulama arasindaki
//! tek gercek fark, cekirdege hangi kapidan girdikleridir -- ki bu da
//! zaten Level-0b1'in var olma sebebidir.
//!
//! ## Baglama modeli
//!
//! Her surec kendi adres uzayini aldigi icin (bkz. `core/mmu_i386.rs`)
//! tum uygulamalar ayni tabana linklenir. ELF tarafinda taban
//! `--image-base` ile dogrudan kullanici bolgesidir; PE tarafinda ise
//! Windows'un gelenegi olan `0x00400000` kullanilir ve cekirdek imaji
//! **taban yeniden yerlesimi** ile tasir (bkz. `binary_loader/pe32.rs`).
//! Yani PE ikilisi gercek bir Windows programi gibi gorunur.

#![no_std]

pub mod canvas;
pub mod font;

// --- Linux/POSIX tarafi (ELF ikilisi) ---
#[cfg(not(target_os = "windows"))]
pub mod gui;
#[cfg(not(target_os = "windows"))]
pub mod io;
#[cfg(not(target_os = "windows"))]
pub mod signal;
#[cfg(not(target_os = "windows"))]
pub mod sys;

// --- Windows/NT tarafi (PE ikilisi) ---
#[cfg(target_os = "windows")]
pub mod nt;
#[cfg(target_os = "windows")]
pub mod win32;
/// Ithal tablosu uzerinden Win32 API (`KERNEL32.dll`, `TCMKGUI.dll`).
/// `win32` ile ayni islevleri sunar, ama cagrilar elle `int 0x2E` yerine
/// **IAT** uzerinden gider -- yani derleyicinin urettigi siradan bir
/// Windows ikilisinin yaptigi gibi.
#[cfg(target_os = "windows")]
pub mod winapi;

/// Uygulamanin giris noktasini tanimlar.
///
/// GRUB/`iret` ile Ring 3'e girildiginde yigin hazirdir ama `argc/argv`
/// yoktur; giris noktasi bu yuzden argumansizdir. `main` dondugunde
/// otomatik olarak `sys_exit(0)` cagrilir -- Ring 3'te "donulecek yer"
/// olmadigi icin bu sart.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[no_mangle]
        pub extern "C" fn _start() -> ! {
            // Tip kontrolu: $main gercekten `fn()` olmali.
            let main: fn() = $main;
            main();
            $crate::exit_process(0)
        }
    };
}

/// Sureci sonlandirir -- hangi ABI ile derlendiyse onun yoluyla.
///
/// `entry!` ve panik isleyicisi bunu kullanir, boylece uygulama kodunun
/// hangi ikili bicimde derlendigini bilmesi gerekmez.
#[cfg(not(target_os = "windows"))]
pub fn exit_process(code: i32) -> ! {
    sys::exit(code)
}

#[cfg(target_os = "windows")]
pub fn exit_process(code: i32) -> ! {
    nt::terminate_process(code as u32)
}

/// Panik: mesaji stdout'a yazip 101 ile cikar (Rust'in geleneksel panik
/// cikis kodu). Cekirdek bu cikisi normal bir surec sonlanmasi olarak
/// gorur -- yani bir uygulamanin panigi sistemi etkilemez.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;

    #[cfg(not(target_os = "windows"))]
    let mut out = io::Stdout;
    #[cfg(target_os = "windows")]
    let mut out = win32::Console;
    let _ = writeln!(out, "[tcmk] uygulama panigi: {}", info.message());
    if let Some(loc) = info.location() {
        let _ = writeln!(out, "[tcmk]   {}:{}", loc.file(), loc.line());
    }
    exit_process(101)
}
