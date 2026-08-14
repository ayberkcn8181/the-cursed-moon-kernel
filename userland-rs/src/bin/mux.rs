//! `mux` -- `poll()` gosterimi: iki kaynagi ayni anda beklemek.
//!
//! `relay` boruyu her karede yokluyordu: `read` cagir, sifir donerse uyu.
//! Bu kalibin iki sorunu var -- veri gelse bile uyku suresi kadar
//! gecikiyor, ve **ikinci** bir kaynak (klavye) eklendiginde ikisini
//! sirayla yoklamaktan baska yol kalmiyor.
//!
//! `poll` soruyu tersine cevirir: "sen bekle, hangisi hazir olursa beni
//! uyandir."
//!
//! ```text
//!   poll([ boru: POLLIN, stdin: POLLIN ], 120 ms)
//!     -> 0            zaman asimi (kimse konusmadi)
//!     -> fds[0]       borudan veri geldi
//!     -> fds[1]       tusa basildi
//!     -> POLLHUP      cocuk cikti, boru kapandi
//! ```
//!
//! Ekrandaki uc sayac tam olarak bu dallari sayar. Cocuk bittikten sonra
//! boru **izlemeden cikarilir** (`fd = -1`): POLLHUP kalici oldugu icin
//! izlemeye devam etmek `poll`'u her turda hemen dondurur -- yani mesgul
//! bekleme. POSIX'in negatif tanimlayiciyi yok saymasi tam da bunun
//! icindir. Cikarildiktan sonra zaman asimi sayaci artmaya baslar; bu,
//! borunun gercekten susmus oldugunun kanitidir.
//!
//! Tuslar: yazilanlar sayilir, `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys::{self, PollFd};

tcmk::entry!(main);

const BG: u32 = 0x000C_1620;
const PANEL: u32 = 0x0018_2838;
const FG: u32 = 0x00D8_E2F0;
const ACCENT: u32 = 0x0070_D8FF;
const OK: u32 = 0x0060_E890;
const WARN: u32 = 0x00FF_C24A;

/// Cocugun gonderecegi bayt sayisi.
const SAMPLES: u32 = 24;
/// `poll` zaman asimi. Pencere bu araliklarla yeniden ciziliyor, yani
/// aslinda kare hizidir.
const TIMEOUT_MS: isize = 120;

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;

    let (read_fd, write_fd) = match sys::pipe() {
        Some(pair) => pair,
        None => {
            let _ = writeln!(out, "[mux] boru acilamadi.");
            return;
        }
    };

    let child = sys::fork();
    if child < 0 {
        let _ = writeln!(out, "[mux] fork basarisiz: {}", child);
        return;
    }

    if child == 0 {
        writer(read_fd, write_fd);
    } else {
        multiplexer(read_fd, write_fd, child as usize);
    }
}

/// Cocuk: boruya araliklarla yazar. Bilerek YAVAS -- ebeveynin cogu
/// turda zaman asimina dusmesi, `poll`'un gercekten bekledigini gosterir.
fn writer(read_fd: usize, write_fd: usize) -> ! {
    sys::close(read_fd);
    for i in 0..SAMPLES {
        let byte = [(b'a' + (i % 26) as u8)];
        sys::write(write_fd, &byte);
        sys::sleep_ms(300);
    }
    sys::close(write_fd);
    sys::exit(3)
}

/// Ebeveyn: boru ile klavyeyi **ayni** cagriyla bekler.
fn multiplexer(read_fd: usize, write_fd: usize, child: usize) {
    use core::fmt::Write;
    let mut out = Stdout;

    sys::close(write_fd);

    let mut win = match Window::open("mux -- poll()", 520, 40, 400, 250) {
        Some(w) => w,
        None => return,
    };

    let mut state = State::new();
    // Iki kaynak, tek cagri. Sirayla degil, AYNI ANDA.
    let mut fds = [
        PollFd::new(read_fd, sys::POLLIN),
        PollFd::new(sys::STDIN, sys::POLLIN),
    ];

    loop {
        let ready = sys::poll(&mut fds, TIMEOUT_MS);

        if ready == 0 {
            state.timeouts += 1;
        }

        // 1. Borudan veri.
        if fds[0].ready(sys::POLLIN) {
            let mut chunk = [0u8; 32];
            let n = sys::read(read_fd, &mut chunk);
            if n > 0 {
                state.pipe_events += 1;
                state.bytes += n as usize;
            }
        }

        // 2. Yazan uc kapandi. Kalan veri okundugu icin izlemeden
        //    cikariliyor -- POLLHUP kalicidir, birakilirsa `poll` bir
        //    daha hic beklemez.
        if fds[0].ready(sys::POLLHUP) && !state.hup {
            state.hup = true;
            fds[0].fd = -1;
            // Cocuk zaten cikmis olmali; toplanmazsa zombi kalirdi.
            let mut status: u32 = 0;
            if sys::waitpid(child, &mut status, 0) >= 0 {
                state.child_status = sys::exit_status(status);
            }
            let _ = writeln!(
                out,
                "[mux] boru kapandi: {} bayt, {} veri / {} tus / {} zaman asimi",
                state.bytes, state.pipe_events, state.key_events, state.timeouts
            );
            state.timeouts_at_hup = state.timeouts;
        }

        // 3. Klavye. Ayni `poll` cagrisindan geldi.
        if fds[1].ready(sys::POLLIN) {
            let mut keys = [0u8; 8];
            let n = sys::read(sys::STDIN, &mut keys);
            for &key in keys.iter().take(n.max(0) as usize) {
                if key == b'q' {
                    let _ = writeln!(
                        out,
                        "[mux] cikis: {} veri, {} tus, {} zaman asimi.",
                        state.pipe_events, state.key_events, state.timeouts
                    );
                    return;
                }
                state.key_events += 1;
                state.last_key = key;
            }
        }

        draw(&mut win, &state);
        // Uyku YOK: bekleme isini `poll` yapiyor.
        win.flush();
    }
}

struct State {
    pipe_events: usize,
    key_events: usize,
    timeouts: usize,
    timeouts_at_hup: usize,
    bytes: usize,
    last_key: u8,
    hup: bool,
    child_status: u32,
}

impl State {
    fn new() -> State {
        State {
            pipe_events: 0,
            key_events: 0,
            timeouts: 0,
            timeouts_at_hup: 0,
            bytes: 0,
            last_key: 0,
            hup: false,
            child_status: 0,
        }
    }
}

fn draw(win: &mut Window, state: &State) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "poll([boru, stdin], 120ms)", ACCENT);

    win.text(6, 30, "borudan veri:", FG);
    win.number(126, 30, state.pipe_events, ACCENT);
    win.text(180, 30, "bayt:", FG);
    win.number(232, 30, state.bytes, FG);

    win.text(6, 48, "tus olayi  :", FG);
    win.number(126, 48, state.key_events, OK);
    if state.last_key != 0 {
        let last = [state.last_key];
        if let Ok(text) = core::str::from_utf8(&last) {
            win.text(180, 48, "son:", FG);
            win.text(232, 48, text, OK);
        }
    }

    win.text(6, 66, "zaman asimi :", FG);
    win.number(126, 66, state.timeouts, WARN);

    // Cubuk: gelen baytlarin ilerlemesi.
    let progress = state.bytes * (w - 24) / SAMPLES as usize;
    win.fill(12, 92, w - 24, 10, PANEL);
    win.fill(12, 92, progress.min(w - 24), 10, if state.hup { OK } else { ACCENT });

    if state.hup {
        win.text(6, 118, "boru kapandi (POLLHUP), izlemeden cikarildi", OK);
        win.text(6, 134, "cocuk cikis kodu:", FG);
        win.number(160, 134, state.child_status as usize, OK);
        win.text(6, 150, "kapanistan sonraki zaman asimi:", FG);
        win.number(
            268,
            150,
            state.timeouts.saturating_sub(state.timeouts_at_hup),
            WARN,
        );
        win.text(6, 168, "artik yalnizca klavye bekleniyor.", FG);
    } else {
        win.text(6, 118, "iki kaynak da bekleniyor...", WARN);
        win.text(6, 134, "yaz: tus olayi sayaci artar", FG);
    }

    win.text(6, h - 16, "q = cik", FG);
}
