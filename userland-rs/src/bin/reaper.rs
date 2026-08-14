//! `reaper` -- `waitpid(-1)` ve gorev yuvasi geri kazanimi gosterimi.
//!
//! Sirayla UC cocuk uretir, her biri farkli bir kodla cikar ve ebeveyn
//! onlari **hangisi once biterse** sirasiyla toplar. Uc cocuk arka
//! arkaya degil, gorev tablosunda **ayni yuvalari** kullanarak dogar --
//! yuva geri kazanilmasaydi tablo birkac cocuk sonra dolardi.
//!
//! Tuslar: ESC -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x000A_1418;
const PANEL: u32 = 0x0014_2632;
const FG: u32 = 0x00E0_E8F0;
const ACCENT: u32 = 0x0070_D0FF;
const OK: u32 = 0x0060_E0A0;

const ROUNDS: usize = 3;

fn main() {
    let mut win = match Window::open("reaper -- waitpid(-1)", 340, 200, 330, 210) {
        Some(w) => w,
        None => return,
    };

    // Her tur: bir cocuk uret, ESC'ye bakarak bekle, topla.
    let mut collected: [(usize, u32); ROUNDS] = [(0, 0); ROUNDS];
    let mut done = 0usize;

    for round in 0..ROUNDS {
        let pid = sys::fork();
        if pid == 0 {
            // Cocuk: kisa bir is yapip kendine ozgu bir kodla cikar.
            for _ in 0..(round + 1) * 20 {
                sys::yield_now();
            }
            sys::exit((round as i32 + 1) * 7);
        }
        if pid < 0 {
            break;
        }

        // Ebeveyn: hangi cocuk biterse onu topla. Beklerken cizim
        // yapabilmek icin once WNOHANG ile yoklanir.
        loop {
            let mut status = 0u32;
            let got = sys::waitpid(sys::WAIT_ANY, &mut status, sys::WNOHANG);
            if got > 0 {
                collected[done] = (got as usize, sys::exit_status(status));
                done += 1;
                break;
            }
            if win.poll_key() == 0x1B {
                return;
            }
            draw(&mut win, &collected[..done], round);
            win.frame(30);
        }
    }

    // Toplama bitti: sonuc ekranda kalsin.
    loop {
        if win.poll_key() == 0x1B {
            return;
        }
        draw(&mut win, &collected[..done], ROUNDS);
        win.frame(60);
    }
}

fn draw(win: &mut Window, collected: &[(usize, u32)], round: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 20, PANEL);
    win.text(6, 2, "tur", FG);
    win.number(40, 2, round.min(ROUNDS), ACCENT);
    win.text(70, 2, "/", FG);
    win.number(82, 2, ROUNDS, FG);
    win.text(120, 2, "toplanan", FG);
    win.number(196, 2, collected.len(), OK);

    win.text(8, 30, "pid   cikis kodu", FG);
    let mut y = 48;
    for (pid, code) in collected {
        win.number(16, y, *pid, ACCENT);
        win.number(90, y, *code as usize, OK);
        y += 18;
    }

    win.fill(0, h - 20, w, 20, PANEL);
    win.text(6, h - 18, "waitpid(-1) -- ESC = cik", FG);
}
