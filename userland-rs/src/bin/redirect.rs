//! `redirect` -- `dup` / `dup2` gosterimi: stdout'u kacirmak.
//!
//! `relay` iki surecin bir boruyla nasil konustugunu gosteriyordu. Bu
//! program borunun **ikinci** kullanimini gosterir: bir tanimlayiciyi
//! baska bir numaraya tasiyip, ondan haberi olmayan kodu kandirmak.
//!
//! ```text
//!   pipe()            ->  (r, w)
//!   dup(r)            ->  r2          (en kucuk bos numara)
//!   dup2(w, STDOUT)                   ARTIK 1 = borunun yazma ucu
//!   close(w)
//!   rapor_yaz()                       stdout'a yazar; boruyu BILMEZ
//!   close(STDOUT)                     varsayilan konsol geri gelir
//!   read(r2, ...)                     yazilanlar burada
//! ```
//!
//! Isin puf noktasi `rapor_yaz`'in hicbir seyi degismemesidir: `println!`
//! cagirir, o da 1 numaraya yazar. Yonlendirmeyi yapan cagiran taraftir.
//! UNIX kabugunun `komut > dosya` yonlendirmesi de tam olarak budur --
//! kabuk `fork`'tan sonra, `execve`'den once `dup2` yapar; calisan program
//! nereye yazdigini hic bilmez.
//!
//! Okuma `r` ile degil **`r2`** ile yapilir: `dup`'in dondurdugu kopyanin
//! ayni boruya baktiginin kaniti budur.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0010_141E;
const PANEL: u32 = 0x001C_2436;
const FG: u32 = 0x00D8_E2F0;
const ACCENT: u32 = 0x0060_D0FF;
const OK: u32 = 0x0060_E890;
const WARN: u32 = 0x00FF_C24A;

/// Yakalanan cikis icin tampon. Boru kapasitesi 1024 bayt; bundan
/// buyugu zaten boruya sigmaz.
const CAPTURE: usize = 512;

fn main() {
    use core::fmt::Write;
    let mut console = Stdout;

    // Bu satir HENUZ yonlendirme yokken yazildi: konsola gider.
    let _ = writeln!(console, "[redirect] baslarken stdout = konsol.");

    let (read_fd, write_fd) = match sys::pipe() {
        Some(pair) => pair,
        None => {
            let _ = writeln!(console, "[redirect] boru acilamadi.");
            return;
        }
    };

    // `dup`: numara secmez, en kucuk bos olani verir. Kopya ayni boruya
    // bakar -- asagida okuma bunun uzerinden yapilarak kanitlanir.
    let copy_fd = sys::dup(read_fd);
    if copy_fd < 0 {
        let _ = writeln!(console, "[redirect] dup basarisiz: {}", copy_fd);
        return;
    }
    let copy_fd = copy_fd as usize;

    // `dup2`: numarayi ISTEYEREK secer. 1 numaranin normalde bos duran
    // yuvasi dolar; cekirdek artik once tabloya baktigi icin stdout'a
    // yazilanlar konsola degil boruya gider.
    if sys::dup2(write_fd, sys::STDOUT) < 0 {
        let _ = writeln!(console, "[redirect] dup2 basarisiz.");
        return;
    }
    // Orijinal yazma ucu artik gereksiz: 1 numara ayni ucu tutuyor.
    // Kapatilmazsa boruda "iki yazan" gorunur ve dosya sonu hic anlasilmaz.
    sys::close(write_fd);

    // --- Bu noktadan sonra yazilan her sey boruya gidiyor ---
    let written = write_report(read_fd, copy_fd);

    // Yonlendirmeyi geri al: 1 numarali yuva bosalir, varsayilan konsol
    // yeniden devreye girer.
    sys::close(sys::STDOUT);

    // Okuma KOPYA uzerinden: `dup`'in verdigi numara ayni boru.
    let mut capture = [0u8; CAPTURE];
    let mut captured = 0usize;
    loop {
        if captured >= capture.len() {
            break;
        }
        let n = sys::read(copy_fd, &mut capture[captured..]);
        if n <= 0 {
            break;
        }
        captured += n as usize;
    }

    sys::close(read_fd);
    sys::close(copy_fd);

    let _ = writeln!(
        console,
        "[redirect] stdout geri geldi; boruda {} bayt yakalandi ({} yazilmisti).",
        captured, written
    );

    show(read_fd, write_fd, copy_fd, &capture[..captured], written);
}

/// stdout'a yazan, kac bayt yazdigini sayan yazici.
///
/// Sayaca ihtiyac var cunku dogrulama "boruda ne varsa, yazilanin aynisi
/// olmali" seklinde yapiliyor; `println!` bir sayi dondurmez.
struct CountingStdout(usize);

impl core::fmt::Write for CountingStdout {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = sys::write(sys::STDOUT, s.as_bytes());
        if n < 0 {
            return Err(core::fmt::Error);
        }
        self.0 += n as usize;
        Ok(())
    }
}

/// Yonlendirmeden habersiz "kutuphane" kodu.
///
/// Tek yaptigi stdout'a (1 numaraya) yazmak; borudan, `dup2`'den haberi
/// yok. Yakalanan metnin bu fonksiyondan cikmasi, yonlendirmenin cagiran
/// tarafta kuruldugunun kanitidir.
fn write_report(read_fd: usize, copy_fd: usize) -> usize {
    use core::fmt::Write;
    let mut out = CountingStdout(0);

    let _ = writeln!(out, "--- yakalanan rapor ---");
    let _ = writeln!(out, "boru okuma ucu : fd {}", read_fd);
    let _ = writeln!(out, "dup kopyasi    : fd {}", copy_fd);
    let _ = writeln!(out, "stdout         : fd {} (dup2 ile boru)", sys::STDOUT);
    let _ = writeln!(out, "surec          : pid {}", tcmk::signal::getpid());
    let _ = writeln!(out, "bu satirlar konsola GITMEDI.");

    out.0
}

fn show(read_fd: usize, write_fd: usize, copy_fd: usize, captured: &[u8], written: usize) {
    let mut win = match Window::open("redirect -- dup / dup2", 250, 150, 400, 260) {
        Some(w) => w,
        None => return,
    };

    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, read_fd, write_fd, copy_fd, captured, written);
        win.frame(60);
    }
}

fn draw(
    win: &mut Window,
    read_fd: usize,
    write_fd: usize,
    copy_fd: usize,
    captured: &[u8],
    written: usize,
) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "dup2(w, 1) -> stdout artik boru", ACCENT);

    win.text(6, 28, "pipe r/w:", FG);
    win.number(80, 28, read_fd, ACCENT);
    win.number(104, 28, write_fd, ACCENT);
    win.text(6, 44, "dup kopya:", FG);
    win.number(88, 44, copy_fd, OK);
    // Yazilan = yakalanan olmali: borudan tek bayt kaybolmadan gecti.
    let intact = written > 0 && captured.len() == written;
    win.text(6, 60, "yazilan/yakalanan:", FG);
    win.number(156, 60, written, FG);
    win.number(196, 60, captured.len(), if intact { OK } else { WARN });

    // Yakalanan metin: konsola gitmesi gereken satirlar burada.
    win.fill(6, 78, w - 12, h - 100, PANEL);
    let mut y = 82;
    let mut start = 0usize;
    for i in 0..=captured.len() {
        let line_end = i == captured.len() || captured[i] == b'\n';
        if !line_end {
            continue;
        }
        if y + 12 > h - 26 {
            break;
        }
        if let Ok(text) = core::str::from_utf8(&captured[start..i]) {
            if !text.is_empty() {
                win.text(10, y, text, FG);
            }
        }
        y += 12;
        start = i + 1;
    }

    win.text(6, h - 16, "q = cik", FG);
}
