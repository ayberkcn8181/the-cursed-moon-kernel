//! `sigdemo` -- POSIX sinyallerini goruntuleyen uygulama.
//!
//! Uc sinyal icin isleyici kurar (SIGUSR1, SIGUSR2, SIGTERM) ve her
//! geldiginde sayacini artirir. Sinyal, uygulama cizim dongusunun
//! ortasindayken gelir; isleyici doner ve dongu **hicbir sey olmamis
//! gibi** devam eder -- ekrandaki kare sayaci kesintisiz artmaya devam
//! ederek bunu gosterir.
//!
//! Kabuktan denemek icin:
//!
//! ```text
//! tcmk> run sigdemo
//! tcmk> ps                 # gorev numarasini ogren
//! tcmk> signal 4 10        # SIGUSR1
//! tcmk> signal 4 12        # SIGUSR2
//! tcmk> signal 4 15        # SIGTERM -> uygulama duzgunce cikar
//! tcmk> kill 4             # SIGKILL -> yakalanamaz, aninda sonlanir
//! ```
//!
//! Tuslar: 's' -> kendine SIGUSR1 gonder,  'i' -> SIGUSR2'yi yok say,
//!         ESC -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::signal;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0010_0818;
const PANEL: u32 = 0x0022_1430;
const FG: u32 = 0x00E8_E0F0;
const ACCENT: u32 = 0x00FF_B060;
const OK: u32 = 0x0060_E0A0;

/// Isleyici asil akisin ortasinda calisir; paylasilan durumu atomik
/// tutmak bu yuzden onemli.
static USR1: AtomicUsize = AtomicUsize::new(0);
static USR2: AtomicUsize = AtomicUsize::new(0);
static TERM: AtomicUsize = AtomicUsize::new(0);
/// Isleyicinin gordugu kare numarasi: sinyalin dongunun **ortasinda**
/// geldigini gosterir.
static AT_FRAME: AtomicUsize = AtomicUsize::new(0);
static FRAME: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_signal(signo: u32) {
    match signo {
        signal::SIGUSR1 => {
            USR1.fetch_add(1, Ordering::Relaxed);
        }
        signal::SIGUSR2 => {
            USR2.fetch_add(1, Ordering::Relaxed);
        }
        signal::SIGTERM => {
            TERM.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
    AT_FRAME.store(FRAME.load(Ordering::Relaxed), Ordering::Relaxed);
    let _ = sys::write(sys::STDOUT, b"[sigdemo] sinyal alindi\n");
}

fn main() {
    let mut win = match Window::open("sigdemo -- POSIX sinyalleri", 300, 200, 380, 230) {
        Some(w) => w,
        None => return,
    };

    signal::install(signal::SIGUSR1, on_signal);
    signal::install(signal::SIGUSR2, on_signal);
    signal::install(signal::SIGTERM, on_signal);

    let pid = signal::getpid();
    let mut ignoring = false;

    loop {
        let frame = FRAME.fetch_add(1, Ordering::Relaxed) + 1;

        loop {
            let key = win.poll_key();
            if key == 0 {
                break;
            }
            match key {
                0x1B => {
                    let _ = sys::write(sys::STDOUT, b"[sigdemo] cikiliyor.\n");
                    return;
                }
                b's' => {
                    // Kendine sinyal gondermek de gecerlidir: cekirdek
                    // bunu bekleyenlere yazar, teslim bu syscall'dan
                    // Ring 3'e donerken olur -- yani `kill` cagrisi
                    // donmeden once isleyici calisir.
                    signal::kill(pid, signal::SIGUSR1);
                }
                b'i' => {
                    ignoring = !ignoring;
                    if ignoring {
                        signal::ignore(signal::SIGUSR2);
                    } else {
                        signal::install(signal::SIGUSR2, on_signal);
                    }
                }
                _ => {}
            }
        }

        draw(&mut win, pid, frame, ignoring);
        win.frame(30);
    }
}

fn draw(win: &mut Window, pid: usize, frame: usize, ignoring: bool) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "surec:", ACCENT);
    win.number(56, 3, pid, FG);
    win.text(90, 3, "kare:", ACCENT);
    win.number(134, 3, frame, FG);

    let rows = [
        ("SIGUSR1 (10)", USR1.load(Ordering::Relaxed), OK),
        ("SIGUSR2 (12)", USR2.load(Ordering::Relaxed), OK),
        ("SIGTERM (15)", TERM.load(Ordering::Relaxed), ACCENT),
    ];
    let mut y = 38;
    for (name, count, color) in rows {
        win.text(12, y, name, FG);
        win.number(150, y, count, color);
        // Sayac kadar kucuk kare: gozle sayilabilsin.
        for i in 0..count.min(14) {
            win.fill(190 + i * 12, y + 2, 8, 10, color);
        }
        y += 22;
    }

    win.text(12, y + 8, "son sinyal kare:", FG);
    win.number(160, y + 8, AT_FRAME.load(Ordering::Relaxed), ACCENT);

    win.fill(0, h - 38, w, 38, PANEL);
    win.text(6, h - 34, "s = kendine SIGUSR1   i = SIGUSR2 yok say:", FG);
    win.text(
        6,
        h - 18,
        if ignoring { "ACIK" } else { "kapali" },
        if ignoring { ACCENT } else { OK },
    );
    win.text(120, h - 18, "ESC = cik", FG);
}
