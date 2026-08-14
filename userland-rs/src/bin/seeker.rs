//! `seeker` -- `lseek` / `fstat` gosterimi: dosyanin ortasindan okumak.
//!
//! Bu iki cagri gelene kadar dosyalar yalnizca **bastan sona**
//! okunabiliyordu: konum sadece `read`/`write` tarafindan ilerliyor, geri
//! alinamiyordu. Ortadan okumak, basa sarmak, sona eklemek -- hicbiri
//! mumkun degildi. Boyutu ogrenmenin de yolu yoktu; tek yol dosyayi sonuna
//! kadar okuyup saymakti.
//!
//! Program dort sinav yapar ve her birinin sonucunu ekranda gosterir:
//!
//! ```text
//!   1. yaz      -> 16 baytlik bilinen bir desen
//!   2. boyut    -> fstat 16 demeli; lseek(0, SEEK_END) de 16
//!   3. ortadan  -> lseek(4, SEEK_SET) + read(4) -> "4567"
//!   4. ekle     -> lseek(0, SEEK_END) + write -> dosya 24 bayt
//! ```
//!
//! Ucuncu sinav olcunun kendisi: dogru dort bayt geldiyse konum gercekten
//! tasinmis demektir. Dorduncusu de "ekleme" kalibinin -- her gunluk
//! dosyasinin ihtiyaci olan seyin -- calistigini gosterir.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0012_1018;
const PANEL: u32 = 0x0020_1C2C;
const FG: u32 = 0x00E0_DCE8;
const ACCENT: u32 = 0x00FF_A0D0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

const PATH: &str = "/seek.txt";
const PATTERN: &[u8] = b"0123456789ABCDEF";
const APPENDED: &[u8] = b"++EKLENDI";

/// Tek bir sinavin sonucu.
#[derive(Clone, Copy)]
struct Check {
    name: &'static str,
    expected: usize,
    got: usize,
}

impl Check {
    fn ok(&self) -> bool {
        self.expected == self.got
    }
}

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;

    let mut checks = [Check {
        name: "",
        expected: 0,
        got: 0,
    }; 5];
    let mut count = 0usize;
    let mut middle = [0u8; 4];

    let mut record = |name: &'static str, expected: usize, got: usize| {
        if count < checks.len() {
            checks[count] = Check {
                name,
                expected,
                got,
            };
            count += 1;
        }
    };

    // --- 1. Bilinen bir deseni yaz ---
    let mut name = [0u8; 32];
    name[..PATH.len()].copy_from_slice(PATH.as_bytes());
    let fd = unsafe { sys::open_raw(name.as_ptr(), sys::O_CREAT) };
    if fd < 0 {
        let _ = writeln!(out, "[seeker] dosya acilamadi: {}", fd);
        return;
    }
    let fd = fd as usize;
    let written = sys::write(fd, PATTERN);
    record("yazilan bayt", PATTERN.len(), written.max(0) as usize);

    // --- 2. Boyut: iki ayri yoldan ayni cevap gelmeli ---
    let by_stat = sys::file_size(fd).max(0) as usize;
    record("fstat boyut", PATTERN.len(), by_stat);
    let by_seek = sys::lseek(fd, 0, sys::SEEK_END).max(0) as usize;
    record("lseek(SEEK_END)", PATTERN.len(), by_seek);

    // --- 3. Ortadan oku: olcunun kendisi ---
    let position = sys::lseek(fd, 4, sys::SEEK_SET).max(0) as usize;
    record("lseek(4) konumu", 4, position);
    let n = sys::read(fd, &mut middle);
    let matches = n == 4 && &middle == b"4567";
    record("ortadan 4 bayt", 1, matches as usize);
    let _ = writeln!(
        out,
        "[seeker] ortadan okunan: {}{}{}{}",
        middle[0] as char, middle[1] as char, middle[2] as char, middle[3] as char
    );

    // --- 4. Sona ekle ---
    sys::lseek(fd, 0, sys::SEEK_END);
    sys::write(fd, APPENDED);
    let grown = sys::file_size(fd).max(0) as usize;
    sys::close(fd);

    let _ = writeln!(
        out,
        "[seeker] {} sinavdan {} gecti, dosya {} bayt.",
        count,
        checks[..count].iter().filter(|c| c.ok()).count(),
        grown
    );

    let mut win = match Window::open("seeker -- lseek / fstat", 280, 150, 400, 250) {
        Some(w) => w,
        None => return,
    };

    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks[..count], &middle, grown);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check], middle: &[u8; 4], grown: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "lseek / fstat", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.number(200, y, check.got, if check.ok() { OK } else { WARN });
        if !check.ok() {
            win.text(240, y, "beklenen:", FG);
            win.number(320, y, check.expected, WARN);
        }
        y += 16;
    }

    y += 8;
    win.text(6, y, "ortadan okunan:", FG);
    if let Ok(text) = core::str::from_utf8(middle) {
        win.text(140, y, text, ACCENT);
    }
    win.text(200, y, "(beklenen 4567)", FG);

    y += 20;
    win.text(6, y, "eklemeden sonra:", FG);
    win.number(148, y, grown, if grown == 25 { OK } else { WARN });
    win.text(190, y, "bayt", FG);

    let passed = checks.iter().filter(|c| c.ok()).count();
    win.fill(6, h - 44, w - 12, 20, PANEL);
    win.text(
        12,
        h - 41,
        if passed == checks.len() {
            "butun sinavlar gecti"
        } else {
            "BIR SINAV KALDI"
        },
        if passed == checks.len() { OK } else { WARN },
    );

    win.text(6, h - 16, "q = cik", FG);
}
