//! `twins` -- `fork()` + `waitpid()` gosterimi.
//!
//! Tek bir program calisir, `fork()` cagirir ve o noktadan sonra **iki
//! surec** olur. Ikisi de ayni koddan devam eder, ama:
//!
//!   * `fork()` ebeveyne cocugun kimligini, cocuga `0` dondurur;
//!   * bellek kopyalanmistir, paylasilmaz -- bu yuzden ayni degiskeni
//!     ayri ayri degistirirler ve birbirlerinin sayacini goremezler.
//!
//! Ikinci madde ekranda dogrudan gorunur: iki pencere ayni degerden
//! baslayan ama birbirinden bagimsiz sayaclar gosterir. Bellek gercekten
//! kopyalanmasaydi iki sayac kilitli adimda ilerlerdi.
//!
//! Cocuk sayili kare sonra `42` ile cikar; ebeveyn bunu `waitpid` ile
//! toplar ve ekranda cikis kodunu gosterir. Ebeveyn her karede `WNOHANG`
//! ile yoklar -- boylece beklerken penceresi donmaz.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG_PARENT: u32 = 0x0014_1E2E;
const BG_CHILD: u32 = 0x002A_1620;
const FG: u32 = 0x00E0_E8F0;
const ACCENT: u32 = 0x0060_D0FF;
const CHILD_ACCENT: u32 = 0x00FF_9060;
const OK: u32 = 0x0072_E08A;

/// Cocuk bu kadar kare sonra cikar -- ebeveynin `waitpid`'i gorunur
/// olsun diye kisa tutuldu.
const CHILD_FRAMES: usize = 60;
/// Cocugun cikis kodu; ebeveyn bunu ekranda gostermeli.
const CHILD_EXIT_CODE: u32 = 42;

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;

    // `fork`'tan ONCE yazilan deger: iki surecte de ayni baslar.
    let mut counter: usize = 1000;

    let result = sys::fork();
    if result < 0 {
        let _ = writeln!(out, "[twins] fork basarisiz: {}", result);
        return;
    }

    let is_child = result == 0;
    let child_pid = result as usize;
    let _ = writeln!(
        out,
        "[twins] fork dondu: {} -- ben {}",
        result,
        if is_child { "cocuk" } else { "ebeveyn" }
    );

    // Buradan itibaren iki ayri surec. Pencereler farkli yerlerde acilir
    // ki ikisi birden gorunsun; pencere adres uzayina degil goreve bagli
    // oldugu icin cocuk kendi penceresini acmak zorundadir.
    let (title, x, y, bg, accent, step) = if is_child {
        ("twins -- COCUK", 560, 120, BG_CHILD, CHILD_ACCENT, 7)
    } else {
        ("twins -- EBEVEYN", 120, 120, BG_PARENT, ACCENT, 1)
    };

    let mut win = match Window::open(title, x, y, 300, 170) {
        Some(w) => w,
        None => return,
    };

    let mut frames = 0usize;
    // Ebeveynin cocuktan topladigi cikis kodu (henuz yoksa None).
    let mut reaped: Option<u32> = None;

    loop {
        if win.poll_key() == b'q' {
            break;
        }

        // Cocuk sayili kare sonra ciker; ebeveyn kullanici cikana kadar
        // yasar.
        frames += 1;
        if is_child && frames >= CHILD_FRAMES {
            let _ = writeln!(out, "[twins] cocuk {} ile cikiyor.", CHILD_EXIT_CODE);
            sys::exit(CHILD_EXIT_CODE as i32);
        }

        // Her surec KENDI kopyasini artirir. Adimlar farkli (1'e karsi 7)
        // ki sayaclarin ayristigi bir bakista gorunsun.
        counter += step;

        // Ebeveyn cocugu yoklar. `WNOHANG` sayesinde cocuk hala
        // calisiyorken bloke olmaz, yani penceresi akici kalir.
        if !is_child && reaped.is_none() {
            let mut status: u32 = 0;
            if sys::waitpid(child_pid, &mut status, sys::WNOHANG) == child_pid as isize {
                let code = sys::exit_status(status);
                reaped = Some(code);
                let _ = writeln!(out, "[twins] ebeveyn: cocuk {} ile bitti.", code);
            }
        }

        win.clear(bg);
        win.text(10, 10, title, accent);
        win.text(
            10,
            34,
            if is_child { "fork() -> 0" } else { "fork() -> cocuk id" },
            FG,
        );

        win.text(10, 62, "sayac:", FG);
        win.number_pad(70, 62, counter, 6, accent);

        win.text(10, 86, "adim:", FG);
        win.number(60, 86, step, FG);

        if is_child {
            win.text(10, 112, "kalan kare:", FG);
            win.number(102, 112, CHILD_FRAMES - frames, CHILD_ACCENT);
        } else {
            match reaped {
                Some(code) => {
                    win.text(10, 112, "cocuk bitti, kod:", FG);
                    win.number(146, 112, code as usize, OK);
                }
                None => win.text(10, 112, "cocuk calisiyor...", FG),
            }
        }

        win.text(10, 140, "q = cik", FG);

        win.frame(60);
    }

    // Kullanici erken cikarsa cocuk hala kosuyor olabilir: bu kez
    // **bloke olarak** bekle. Ebeveyn bu sirada `bekliyor` durumundadir
    // ve zamanlayici tarafindan hic secilmez.
    if !is_child && reaped.is_none() {
        let mut status: u32 = 0;
        let _ = writeln!(out, "[twins] ebeveyn cocugu bekliyor (bloke)...");
        let got = sys::waitpid(child_pid, &mut status, 0);
        let _ = writeln!(
            out,
            "[twins] waitpid -> {} (kod {})",
            got,
            sys::exit_status(status)
        );
    }

    let _ = writeln!(
        out,
        "[twins] {} cikiyor (sayac={}).",
        if is_child { "cocuk" } else { "ebeveyn" },
        counter
    );
}
