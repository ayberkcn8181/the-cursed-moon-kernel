//! `waiter` -- `pause` / `sigsuspend` gosterimi: sinyali **beklemek**.
//!
//! `masked` sinyalin bekletilebildigini gosteriyordu. Bu program bir
//! adim otesini gosterir: sinyali **uyuyarak** beklemek.
//!
//! Bu cagrilar gelene kadar bekleyisin tek yolu yoklamaydi:
//!
//! ```text
//!   while !bayrak { yield_now(); }      // sinyal gelene kadar CPU yakar
//! ```
//!
//! Dongu her turda cekirdege girip cikiyor, yani gorev sirekli
//! zamanlaniyor. `pause` ise gorevi **uyutuyor**: sinyal gelene kadar
//! hic zamanlanmiyor.
//!
//! ## Olcu kabukta
//!
//! Iki kip ayni programda ve **kip degistirme sinyalle** yapilir, yani
//! olcumun tamami kabuktan surulur -- odak hic degismez, dolayisiyla
//! `ps` sayaclari yalnizca bekleyis bicimini olcer:
//!
//! ```text
//! tcmk> run waiter
//! tcmk> ps            # durum: sinyal, cpu ~0
//! tcmk> ps            # on saniye sonra: cpu HALA ~0
//! tcmk> signal 5 10   # SIGUSR1 -> yoklama kipine gec
//! tcmk> ps            # cpu hizla artiyor
//! tcmk> signal 5 10   # geri uyku kipine
//! ```
//!
//! Program acilisla birlikte uyku kipindedir: saniyede bir `alarm(1)`
//! kurup `pause()` cagirir. Arada hic zamanlanmaz.
//!
//! ## `sigsuspend` neden ayri bir cagri
//!
//! Klasik kalip: sinyali engelle, bayragi kontrol et, gelmemisse bekle.
//! `sigprocmask` + `pause` olarak yazilirsa arada bir **pencere** kalir
//! -- sinyal tam o araliktaysa `pause` onu kacirir ve surec sonsuza
//! kadar uyur. `sigsuspend` maskeyi degistirmeyi ve beklemeyi tek,
//! bolunmez adimda yapar.
//!
//! Program bunu **acilista kendiliginden** sinar: SIGALRM engelliyken
//! `alarm(1)` kurup `sigsuspend(0)` cagirir. Maske gecici bosaldigi
//! icin alarm teslim edilir, isleyici kosar, ve donusten sonra maske
//! eski haline **geri gelmis** olmali. Ekrandaki "maske once/sonra"
//! satirlari o kontroldur; alarm her zaman geldigi icin sinav
//! belirlenimlidir (deterministik).
//!
//! Tuslar: `q` cik. Kip degistirmek icin `signal <id> 10`.

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::signal;

tcmk::entry!(main);

const BG: u32 = 0x0014_101C;
const PANEL: u32 = 0x0024_1C30;
const FG: u32 = 0x00E4_DCF0;
const DIM: u32 = 0x0088_7C98;
const ACCENT: u32 = 0x00C0_90FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Isleyicilerin artirdigi sayaclar. `AtomicU32`, cunku isleyici normal
/// akisin **ortasinda** kosar -- derleyicinin degeri bir registerda
/// onbelleklemesi burada gercek bir hata olurdu.
static ALARMS: AtomicU32 = AtomicU32::new(0);
static USR1: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_alarm(_signo: u32) {
    ALARMS.fetch_add(1, Ordering::SeqCst);
}

extern "C" fn on_usr1(_signo: u32) {
    USR1.fetch_add(1, Ordering::SeqCst);
    // Kip degistir: kabuktan `signal <id> 10` ile surulur.
    BUSY.fetch_xor(1, Ordering::SeqCst);
}

/// Ekranda gosterilen son sinavin sonucu.
#[derive(Clone, Copy)]
struct Report {
    text: &'static str,
    color: u32,
    /// `sigsuspend` oncesi ve sonrasi maske (sinav icin).
    before: u32,
    after: u32,
}

/// Kip: uyuyarak mi, yoklayarak mi bekleniyor?
///
/// SIGUSR1 ile degisir. Isleyiciden yazildigi icin atomiktir.
static BUSY: AtomicU32 = AtomicU32::new(0);

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;

    signal::install(signal::SIGALRM, on_alarm);
    signal::install(signal::SIGUSR1, on_usr1);

    let _ = writeln!(
        out,
        "[waiter] pid={} -- kip degistirmek icin: signal {} 10",
        signal::getpid(),
        signal::getpid()
    );

    // --- Acilis sinavi: `sigsuspend` maskeyi geri veriyor mu? ---
    //
    // SIGALRM engellenir, sonra `sigsuspend(0)` ile maske **gecici**
    // bosaltilir. Alarm bir saniye icinde mutlaka gelir, yani sinav
    // belirlenimlidir. Donusten sonra maske eski haline donmus olmali.
    signal::sigprocmask(signal::SIG_BLOCK, signal::mask_of(signal::SIGALRM));
    let before = signal::current_mask();
    signal::alarm(1);
    signal::sigsuspend(0);
    let after = signal::current_mask();
    let delivered = ALARMS.load(Ordering::SeqCst) > 0;
    let report = Report {
        text: if delivered && after == before {
            "sigsuspend: maske geri geldi"
        } else if delivered {
            "sigsuspend: MASKE BOZULDU"
        } else {
            "sigsuspend: sinyal gelmedi"
        },
        color: if delivered && after == before { OK } else { WARN },
        before,
        after,
    };
    let _ = writeln!(
        out,
        "[waiter] sigsuspend sinavi: maske {} -> {} ({})",
        before,
        after,
        if delivered && after == before {
            "gecti"
        } else {
            "KALDI"
        }
    );
    // Sinav bitti; engel kalksin ki kip degistirme sinyali gelebilsin.
    signal::sigprocmask(signal::SIG_SETMASK, 0);

    let mut win = match Window::open("waiter -- pause / sigsuspend", 260, 140, 430, 250) {
        Some(w) => w,
        None => return,
    };

    // Yoklama kipinde kac tur donuldu. Uyku kipinde bu sayi artmaz;
    // iki bekleyis arasindaki farkin kaynagi tam olarak budur.
    let mut spins = 0u32;

    loop {
        if win.poll_key() == b'q' {
            break;
        }

        let busy = BUSY.load(Ordering::SeqCst) != 0;
        draw(&mut win, busy, spins, &report);

        if busy {
            // Eski yol: sinyal gelene kadar cekirdege girip cikmak.
            // Sinyal teslimi sistem cagrisi donusunde oldugu icin bos
            // bir dongu isleyiciyi hic calistirmaz -- yani yoklama
            // **zorunlu** olarak CPU yakar.
            spins = spins.wrapping_add(1);
            win.frame(0);
        } else {
            // Yeni yol: alarm kur, uyu. Arada hic zamanlanmaz.
            signal::alarm(1);
            signal::pause();
        }
    }
}

fn draw(win: &mut Window, busy: bool, spins: u32, report: &Report) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "pause / sigsuspend", ACCENT);
    win.text(240, 3, if busy { "kip: YOKLAMA" } else { "kip: UYKU" }, if busy {
        WARN
    } else {
        OK
    });

    let mut y = 32;
    win.text(6, y, "SIGALRM sayaci", FG);
    win.number(200, y, ALARMS.load(Ordering::SeqCst) as usize, ACCENT);
    y += 18;
    win.text(6, y, "SIGUSR1 sayaci", FG);
    win.number(200, y, USR1.load(Ordering::SeqCst) as usize, ACCENT);
    y += 18;
    win.text(6, y, "yoklama turu", FG);
    win.number(200, y, spins as usize, if spins > 0 { WARN } else { DIM });

    y += 26;
    win.text(6, y, "maske once", DIM);
    win.number(200, y, report.before as usize, FG);
    y += 16;
    win.text(6, y, "maske sonra", DIM);
    win.number(200, y, report.after as usize, FG);

    win.fill(6, h - 46, w - 12, 20, PANEL);
    win.text(12, h - 43, report.text, report.color);

    win.text(6, h - 17, "kip: signal <id> 10   |   q cik", DIM);
}
