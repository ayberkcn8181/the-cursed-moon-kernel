//! `race` -- zamanlayici onceliginin **olculebilir** gosterimi.
//!
//! Iki kopya ayni anda calistirilip farkli `nice` degerleri verilirse,
//! tur sayaclari ayrisir: yuksek oncelikli olan daha uzun zaman dilimi
//! aldigi icin daha cok is bitirir.
//!
//! ```text
//! tcmk> run race
//! tcmk> run race
//! tcmk> nice 4 -20      # birine en yuksek oncelik
//! tcmk> nice 5 19       # otekine en dusuk
//! ```
//!
//! Olcumun anlamli olmasi icin uygulama **CPU'ya bagli** olmali: her
//! karede sabit miktarda hesap yapilir. Yalnizca `win_flush` cagirip
//! bekleyen bir program CPU'yu zaten gonullu birakir, o zaman dilim
//! uzunlugunun bir onemi kalmazdi.
//!
//! Pencere konumu surec kimliginden turetilir; boylece iki kopya
//! ust uste binmez.
//!
//! Tuslar: ESC -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::signal;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0012_0A18;
const PANEL: u32 = 0x0026_1636;
const FG: u32 = 0x00E8_E0F0;
const ACCENT: u32 = 0x00FF_C060;
const BAR: u32 = 0x0050_D0A0;

/// Kare basina yapilan is. Buyuk olmali ki gorev zaman diliminin
/// sonuna kadar CPU'da kalsin -- kucuk olsaydi is bitip `win_flush`'a
/// varilir, CPU gonullu birakilir ve oncelik farki olculemezdi.
const WORK_PER_FRAME: u32 = 240_000;

fn main() {
    let pid = signal::getpid();
    // Kopyalar ust uste binmesin: konum kimlikten turetilir. Kabuk
    // penceresinin ustunu kapatmamasi icin sag alta yerlesirler --
    // olcum sirasinda `ps` ciktisinin okunabilir kalmasi gerekiyor.
    let x = 700 + (pid % 2) * 158;
    let y = 545;

    let mut win = match Window::open("race", x, y, 150, 120) {
        Some(w) => w,
        None => return,
    };

    let mut rounds: usize = 0;
    let mut accumulator: u32 = 1;

    loop {
        loop {
            let key = win.poll_key();
            if key == 0 {
                break;
            }
            if key == 0x1B {
                return;
            }
        }

        // --- olculen is ---
        let mut i = 0u32;
        while i < WORK_PER_FRAME {
            // Derleyicinin donguyu silmemesi icin sonuc kullanilir.
            accumulator = accumulator.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            i += 1;
        }
        rounds += 1;

        draw(&mut win, pid, rounds, accumulator);
        // `frame(ms)` degil `flush`: `frame` kareyi bitirdikten sonra
        // UYUR, uyuyan gorev ise zamanlayici tarafindan hic secilmez --
        // yani butun gorevler ayni hizda uyanir ve oncelik farki
        // olculemez hale gelir. Burada olculen sey CPU payi oldugu icin
        // uyku yok.
        win.flush();
    }
}

fn draw(win: &mut Window, pid: usize, rounds: usize, accumulator: u32) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 20, PANEL);
    win.text(6, 2, "surec", FG);
    win.number(56, 2, pid, ACCENT);
    win.text(86, 2, "nice", FG);
    // Oncelik disaridan (kabuktan) degistirilebildigi icin her karede
    // yeniden okunur -- degisikligin ekranda gorunmesi icin.
    let nice = sys::getpriority(0);
    if nice < 0 {
        win.text(126, 2, "-", ACCENT);
        win.number(134, 2, (-nice) as usize, ACCENT);
    } else {
        win.number(126, 2, nice as usize, ACCENT);
    }

    win.text(6, 26, "tur", FG);
    win.number_scaled(6, 42, rounds, ACCENT, 2);

    // Ilerlemeyi gozle gorulur kilan cubuk: her 10 turda bir dolar.
    let filled = (rounds % 40) * w / 40;
    win.fill(0, h - 26, filled, 6, BAR);

    // Hesabin gercekten yapildiginin kaniti; derleyici donguyu
    // atsaydi bu deger sabit kalirdi.
    win.text(6, h - 16, "t", FG);
    win.number(16, h - 16, (accumulator >> 24) as usize, FG);
}
