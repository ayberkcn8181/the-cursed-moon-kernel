//! `masked` -- `sigprocmask` ve `alarm` gosterimi.
//!
//! `sigdemo` sinyalin **geldigini** gosteriyordu. Bu program sinyalin
//! **bekletilebildigini** gosterir.
//!
//! POSIX'te bir sinyali engellemek onu yok saymak degildir: engelli
//! sinyal `PENDING` maskesinde durur ve engel kalkinca teslim edilir.
//! Kritik bolge kalibi budur -- bolunmemesi gereken is maskeyle
//! cevrelenir, sinyal kaybolmaz, yalnizca **erteленir**.
//!
//! ```text
//!   sigprocmask(SIG_BLOCK,   1<<SIGUSR1)   -> artik gelmez, birikir
//!   ... kritik bolge ...
//!   sigprocmask(SIG_UNBLOCK, 1<<SIGUSR1)   -> biriken ANINDA teslim
//! ```
//!
//! Uygulama **SIGUSR1 engelli baslar**: gosterilecek sey maskeleme
//! oldugu icin baslangic durumu da o. Kabuktan denemek (kanit burada):
//!
//! ```text
//! tcmk> run masked
//! tcmk> ps                 # gorev numarasini ogren
//! tcmk> signal 1 10        # sayac ARTMAZ -- engelli
//! tcmk> sigs               # engelli: USR1   bekleyen: USR1
//! tcmk> focus 4            # odagi uygulamaya ver
//!       'u' -> engel kalkar, sayac ANINDA 1 olur
//! ```
//!
//! Ikinci bir kanit oradadir: kabuktan uc kez `signal 1 10` gonderilse
//! bile engel kalkinca sayac **bir** artar. Standart sinyaller siraya
//! girmez; maske bir bittir, sayac degil.
//!
//! `alarm` ayni tabloyu kullanir: cekirdek PIT tik'inde sureyi dolan
//! goreve `SIGALRM` gonderir. Yani zamanlayici bir **sinyal kaynagi**;
//! ayri bir mekanizma degil.
//!
//! Tuslar: `b` engelle, `u` engeli kaldir, `a` alarm(3), `c` alarmi
//!         iptal et, `s` kendine SIGUSR1, `q` cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::signal;

tcmk::entry!(main);

const BG: u32 = 0x0014_0C1E;
const PANEL: u32 = 0x0024_1838;
const FG: u32 = 0x00E4_DCF0;
const ACCENT: u32 = 0x00C0_90FF;
const OK: u32 = 0x0060_E0A0;
const WARN: u32 = 0x00FF_A050;

static USR1: AtomicUsize = AtomicUsize::new(0);
static ALRM: AtomicUsize = AtomicUsize::new(0);
/// Isleyicinin gordugu kare numarasi -- sinyalin dongunun ortasinda
/// geldigini gosterir.
static AT_FRAME: AtomicUsize = AtomicUsize::new(0);
static FRAME: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_signal(signo: u32) {
    match signo {
        signal::SIGUSR1 => {
            USR1.fetch_add(1, Ordering::Relaxed);
        }
        signal::SIGALRM => {
            ALRM.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
    AT_FRAME.store(FRAME.load(Ordering::Relaxed), Ordering::Relaxed);
}

fn main() {
    use core::fmt::Write;
    let mut out = tcmk::io::Stdout;

    signal::install(signal::SIGUSR1, on_signal);
    signal::install(signal::SIGALRM, on_signal);

    // Engelli basla: gosterilen sey maskeleme, o yuzden ilk durum da o.
    signal::sigprocmask(signal::SIG_BLOCK, signal::mask_of(signal::SIGUSR1));

    let pid = signal::getpid();
    let _ = writeln!(
        out,
        "[masked] gorev #{}: SIGUSR1 ENGELLI basladi. 'u' ac, 'b' engelle, 'a' alarm(3), 'q' cik",
        pid
    );

    let mut win = match Window::open("masked -- sigprocmask / alarm", 300, 120, 400, 260) {
        Some(w) => w,
        None => return,
    };

    // Son `alarm(0)` cagrisinin dondugu kalan sure -- POSIX'in "onceki
    // alarmdan kalan saniye" sozlesmesinin kaniti.
    let mut cancelled_at = 0u32;
    let mut alarm_armed = false;

    loop {
        FRAME.fetch_add(1, Ordering::Relaxed);

        match win.poll_key() {
            b'q' => break,
            b'b' => {
                signal::sigprocmask(signal::SIG_BLOCK, signal::mask_of(signal::SIGUSR1));
                let _ = writeln!(out, "[masked] SIGUSR1 engellendi.");
            }
            b'u' => {
                // Engel kalkinca biriken sinyal ANINDA teslim edilir:
                // cekirdek Ring 3'e donerken bekleyenlere bakar, ve bu
                // syscall'in donusu tam olarak o an.
                signal::sigprocmask(signal::SIG_UNBLOCK, signal::mask_of(signal::SIGUSR1));
                let _ = writeln!(out, "[masked] SIGUSR1 engeli kalkti.");
            }
            b'a' => {
                let previous = signal::alarm(3);
                alarm_armed = true;
                let _ = writeln!(out, "[masked] alarm(3) kuruldu (onceki kalan: {}s)", previous);
            }
            b'c' => {
                cancelled_at = signal::alarm(0);
                alarm_armed = false;
                let _ = writeln!(out, "[masked] alarm iptal, kalan {}s idi.", cancelled_at);
            }
            b's' => {
                signal::kill(pid, signal::SIGUSR1);
            }
            _ => {}
        }

        if alarm_armed && ALRM.load(Ordering::Relaxed) > 0 {
            alarm_armed = false;
        }

        draw(&mut win, pid, cancelled_at, alarm_armed);
        win.frame(50);
    }

    let _ = writeln!(
        out,
        "[masked] cikis: {} SIGUSR1, {} SIGALRM.",
        USR1.load(Ordering::Relaxed),
        ALRM.load(Ordering::Relaxed)
    );
}

fn draw(win: &mut Window, pid: usize, cancelled_at: u32, alarm_armed: bool) {
    let (w, h) = (win.width(), win.height());
    let mask = signal::current_mask();
    let blocked = mask & signal::mask_of(signal::SIGUSR1) != 0;

    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "sigprocmask + alarm", ACCENT);

    win.text(6, 30, "gorev:", FG);
    win.number(64, 30, pid, ACCENT);
    win.text(120, 30, "kare:", FG);
    win.number(176, 30, FRAME.load(Ordering::Relaxed), FG);

    // Maske durumu: kirmiziysa sinyal birikiyor demektir.
    win.fill(6, 50, w - 12, 20, PANEL);
    win.text(
        12,
        53,
        if blocked {
            "SIGUSR1 ENGELLI -- gelenler birikiyor"
        } else {
            "SIGUSR1 acik -- geldigi anda teslim"
        },
        if blocked { WARN } else { OK },
    );

    win.text(6, 82, "SIGUSR1 sayaci:", FG);
    win.number(148, 82, USR1.load(Ordering::Relaxed), OK);
    win.text(6, 100, "SIGALRM sayaci:", FG);
    win.number(148, 100, ALRM.load(Ordering::Relaxed), OK);
    win.text(6, 118, "isleyici karesi:", FG);
    win.number(148, 118, AT_FRAME.load(Ordering::Relaxed), ACCENT);

    win.text(6, 144, "alarm:", FG);
    win.text(
        64,
        144,
        if alarm_armed { "kurulu" } else { "yok" },
        if alarm_armed { WARN } else { FG },
    );
    if cancelled_at > 0 {
        win.text(150, 144, "iptalde kalan:", FG);
        win.number(272, 144, cancelled_at as usize, ACCENT);
        win.text(300, 144, "s", FG);
    }

    win.text(6, h - 46, "b engelle  u ac  a alarm(3)", FG);
    win.text(6, h - 30, "c iptal    s kendine SIGUSR1", FG);
    win.text(6, h - 14, "q = cik", FG);
}
