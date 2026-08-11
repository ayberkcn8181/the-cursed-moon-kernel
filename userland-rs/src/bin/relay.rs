//! `relay` -- `pipe()` + `fork()` gosterimi.
//!
//! `twins` iki surecin **ayri** oldugunu gosteriyordu: bellek kopyalanir,
//! sayaclar ayrisir. Bu program tam tersini gosterir -- ayri adres
//! uzaylarindaki iki surec nasil **konusur**.
//!
//! Kalip UNIX'in klasigidir: once boru, sonra catallanma.
//!
//! ```text
//!   pipe()  ->  (okuma_fd, yazma_fd)
//!   fork()
//!     cocuk  : okuma ucunu kapat, olctugu degerleri yazma ucuna yaz
//!     ebeveyn: yazma ucunu kapat, okuma ucundan okuyup ekrana ciz
//! ```
//!
//! Boru tamponu **cekirdektedir**, yani `fork`'ta kopyalanmaz; iki surec
//! de ayni halka tamponu gorur. Paylasilan bir degisken olsaydi zaten
//! kopyalanirdi ve is gormezdi -- `twins`'teki sayaclarin ayrismasi bunun
//! kanitiydi.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x000E_1A24;
const PANEL: u32 = 0x0018_2C3A;
const FG: u32 = 0x00DC_E8F4;
const ACCENT: u32 = 0x0050_E0C0;
const WARN: u32 = 0x00FF_C24A;

/// Cocugun gonderecegi olcum sayisi.
const SAMPLES: u32 = 40;
/// Son gorunen olcum sayisi (grafik genisligi).
const HISTORY: usize = 40;

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;

    // ONCE boru: `fork`'tan sonra acilsaydi her surec kendi borusunu
    // acardi ve ikisi birbirini duymazdi.
    let (read_fd, write_fd) = match sys::pipe() {
        Some(pair) => pair,
        None => {
            let _ = writeln!(out, "[relay] boru acilamadi.");
            return;
        }
    };
    let _ = writeln!(out, "[relay] boru: okuma fd={} yazma fd={}", read_fd, write_fd);

    let result = sys::fork();
    if result < 0 {
        let _ = writeln!(out, "[relay] fork basarisiz: {}", result);
        return;
    }

    if result == 0 {
        child(read_fd, write_fd);
    } else {
        parent(read_fd, write_fd, result as usize);
    }
}

/// Cocuk: olctugu degerleri boruya yazar, sonra cikar.
///
/// Yazma bitince **yazma ucunu kapatir**; ebeveyn bunu "artik veri
/// gelmeyecek" olarak yorumlar (POSIX'te dosya sonu).
fn child(read_fd: usize, write_fd: usize) -> ! {
    use core::fmt::Write;
    let mut out = Stdout;

    // Cocuk okumayacak: kullanmadigi ucu kapatmak UNIX gelenegidir ve
    // gereklidir -- acik kalan bir okuma ucu, borunun hic kapanmamasina
    // yol acar.
    sys::close(read_fd);

    let _ = writeln!(out, "[relay] cocuk {} olcum gonderiyor.", SAMPLES);

    for i in 0..SAMPLES {
        // Basit ama duzensiz bir "olcum": ebeveynin cizdigi grafikte
        // desen gorunsun diye.
        let value = ((i * 37 + 11) % 64) as u8;
        let byte = [value];
        sys::write(write_fd, &byte);
        sys::sleep_ms(80);
    }

    sys::close(write_fd);
    let _ = writeln!(out, "[relay] cocuk bitirdi, yazma ucu kapandi.");
    sys::exit(0)
}

/// Ebeveyn: borudan okur, gelen degerleri cizer.
fn parent(read_fd: usize, write_fd: usize, child_pid: usize) {
    use core::fmt::Write;
    let mut out = Stdout;

    // Ebeveyn yazmayacak. Bu ucu kapatmak sart: acik kalirsa boruda
    // "hala bir yazan var" gorunur ve dosya sonu hic anlasilmaz.
    sys::close(write_fd);

    let mut win = match Window::open("relay -- pipe + fork", 300, 200, 360, 220) {
        Some(w) => w,
        None => return,
    };

    let mut history = [0u8; HISTORY];
    let mut count = 0usize;
    let mut total = 0u32;
    let mut done = false;

    loop {
        if win.poll_key() == b'q' {
            break;
        }

        // Bloke etmeyen okuma: veri yoksa 0 doner ve pencere donmaz.
        let mut chunk = [0u8; 16];
        let n = sys::read(read_fd, &mut chunk);
        if n > 0 {
            for &value in &chunk[..n as usize] {
                history[count % HISTORY] = value;
                count += 1;
                total += value as u32;
            }
        }

        // Cocuk bitti mi? Bittiyse ve boruda veri kalmadiysa is tamam.
        if !done && count as u32 >= SAMPLES {
            let mut status: u32 = 0;
            if sys::waitpid(child_pid, &mut status, sys::WNOHANG) == child_pid as isize {
                done = true;
                let _ = writeln!(
                    out,
                    "[relay] ebeveyn {} olcum aldi, cocuk {} ile bitti.",
                    count,
                    sys::exit_status(status)
                );
            }
        }

        draw(&mut win, &history, count, total, done);
        win.frame(40);
    }

    sys::close(read_fd);
}

fn draw(win: &mut Window, history: &[u8; HISTORY], count: usize, total: u32, done: bool) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "pipe() -> fork() -> read()", ACCENT);

    // Gelen degerlerin cubuk grafigi -- verinin gercekten aktigini
    // gostermenin en dogrudan yolu.
    let shown = core::cmp::min(count, HISTORY);
    let base = h - 40;
    for i in 0..shown {
        let value = history[i] as usize;
        let bar = value * 60 / 64;
        win.fill(8 + i * 8, base - bar, 6, bar.max(1), ACCENT);
    }

    win.text(6, 34, "alinan:", FG);
    win.number(66, 34, count, ACCENT);

    win.text(6, 52, "toplam:", FG);
    win.number(66, 52, total as usize, FG);

    win.text(
        6,
        h - 32,
        if done {
            "cocuk bitti, boru kapandi"
        } else {
            "borudan okunuyor..."
        },
        if done { ACCENT } else { WARN },
    );
    win.text(6, h - 16, "q = cik", FG);
}
