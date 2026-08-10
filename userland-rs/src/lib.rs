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
//! ## Baglama modeli
//!
//! Cekirdekte henuz **surec basina adres uzayi yok** (doc Faz 9+): tum
//! Ring 3 uygulamalari ayni 2 MiB'lik kullanici bolgesini paylasir. Bu
//! yuzden her uygulama kendi "slot"una linklenir; slot tabani
//! `--image-base` ile derleme aninda verilir (bkz. Makefile'daki
//! `userland` hedefi). Ayni tabana linklenen iki uygulama birbirinin
//! kodunu ezerdi.
//!
//! Su an yalnizca i386 (ELF32) hedefi desteklenir; x86_64 userland'i
//! cekirdek tarafinda ELF64 yukleyicisiyle zaten calisiyor, Rust'a
//! tasinmasi ayri bir adimdir.

#![no_std]

pub mod gui;
pub mod io;
pub mod sys;

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
            $crate::sys::exit(0)
        }
    };
}

/// Panik: mesaji stdout'a yazip 101 ile cikar (Rust'in geleneksel panik
/// cikis kodu). Cekirdek bu cikisi normal bir surec sonlanmasi olarak
/// gorur -- yani bir uygulamanin panigi sistemi etkilemez.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    let mut out = io::Stdout;
    let _ = writeln!(out, "[tcmk] uygulama panigi: {}", info.message());
    if let Some(loc) = info.location() {
        let _ = writeln!(out, "[tcmk]   {}:{}", loc.file(), loc.line());
    }
    sys::exit(101)
}
