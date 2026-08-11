//! `twins` -- `fork()` gosterimi.
//!
//! Tek bir program calisir, `fork()` cagirir ve o noktadan sonra **iki
//! surec** olur. Ikisi de ayni koddan devam eder, ama:
//!
//!   * `fork()` ebeveyne cocugun kimligini, cocuga `0` dondurur;
//!   * bellek kopyalanmistir, paylasilmaz -- bu yuzden ayni degiskeni
//!     ayri ayri degistirirler ve birbirlerinin sayacini goremezler.
//!
//! Ikinci madde ekranda dogrudan gorunur: iki pencere ayni yerden
//! baslayan ama birbirinden bagimsiz sayaclar gosterir. Bellek gercekten
//! kopyalanmasaydi iki sayac kilitli adimda ilerlerdi.
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

    let mut win = match Window::open(title, x, y, 300, 150) {
        Some(w) => w,
        None => return,
    };

    loop {
        if win.poll_key() == b'q' {
            break;
        }

        // Her surec KENDI kopyasini artirir. Adimlar farkli (1'e karsi 7)
        // ki sayaclarin ayristigi bir bakista gorunsun.
        counter += step;

        win.clear(bg);
        win.text(10, 10, title, accent);
        win.text(10, 34, if is_child { "fork() -> 0" } else { "fork() -> cocuk id" }, FG);

        win.text(10, 62, "sayac:", FG);
        win.number_pad(70, 62, counter, 6, accent);

        win.text(10, 86, "adim:", FG);
        win.number(60, 86, step, FG);

        win.text(10, 116, "q = cik", FG);

        win.frame(60);
    }

    let _ = writeln!(
        out,
        "[twins] {} cikiyor (sayac={}).",
        if is_child { "cocuk" } else { "ebeveyn" },
        counter
    );
}
