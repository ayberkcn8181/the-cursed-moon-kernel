//! `nested` -- `sigaction` bayraklari ve **ic ice** sinyal teslimi.
//!
//! Bugune kadar TCMK'de bir isleyici koşarken **hicbir** sinyal teslim
//! edilmiyordu. Sebep tasarimdaydi: saklanan Ring 3 baglami tek yuvaydi,
//! ikinci bir teslim onu ezer ve surec asla eski yerine donemezdi. Yani
//! suren bir `SIGUSR1` isleyicisi, gelen bir `SIGALRM`i bekletiyordu.
//!
//! POSIX kurali daha dar: **yalnizca ayni sinyal** engellenir. Farkli
//! sinyaller ic ice teslim edilir. Baglam artik dort katmanli bir yigin
//! oldugu icin TCMK de bunu yapabiliyor.
//!
//! ## Dort sinav, hepsi kendiliginden
//!
//! ```text
//!   A  farkli sinyal ic ice gelir mi?
//!      alarm(1) + kill(kendine, SIGUSR1); isleyici 2 sn uyur
//!      -> SIGALRM, SIGUSR1'in ICINDE kosmali
//!
//!   B  ayni sinyal varsayilan olarak engellenir mi?
//!      isleyici kendine SIGUSR2 gonderir
//!      -> ic ice GIRMEMELI; teslim isleyici bitince olmali
//!
//!   C  SA_NODEFER o korumayi kaldirir mi?
//!      ayni sinav, bayrakli
//!      -> bu kez ic ice GIRMELI
//!
//!   D  SA_RESETHAND tek atimlik mi?
//!      SIGHUP bir kez teslim edilir, yerlestirme SIG_DFL'e donmeli
//! ```
//!
//! Hepsi belirlenimli: sinyaller programin kendisinden geliyor, `alarm`
//! da her zaman firliyor. Sonuclar hem ekranda hem seri gunlukte.
//!
//! D sinavinda ikinci bir `SIGHUP` **gonderilmez**: yerlestirme
//! varsayilana dondugu icin ikincisi sureci oldururdu -- ki bu da
//! `SA_RESETHAND`in ne yaptiginin bir baska kaniti.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::signal;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0018_1220;
const PANEL: u32 = 0x0028_2038;
const FG: u32 = 0x00E8_E0F4;
const DIM: u32 = 0x0090_84A0;
const ACCENT: u32 = 0x00B8_98FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// `SIGUSR1` isleyicisi su an koşuyor mu (kac katman)?
static IN_USR1: AtomicU32 = AtomicU32::new(0);
/// `SIGALRM`, `SIGUSR1`in **icinde** mi kostu?
static ALARM_NESTED: AtomicU32 = AtomicU32::new(0);

/// `SIGUSR2` isleyicisinin su anki derinligi ve gorulen en derini.
static USR2_DEPTH: AtomicU32 = AtomicU32::new(0);
static USR2_MAX: AtomicU32 = AtomicU32::new(0);
static USR2_COUNT: AtomicU32 = AtomicU32::new(0);
/// Isleyici kendine kac kez sinyal gondersin (sonsuz ozyineleme olmasin).
static USR2_LIMIT: AtomicU32 = AtomicU32::new(0);

static HUP_COUNT: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_alarm(_signo: u32) {
    // Bu isleyici `SIGUSR1`in icinde kostuysa sinav A gecti.
    if IN_USR1.load(Ordering::SeqCst) > 0 {
        ALARM_NESTED.store(1, Ordering::SeqCst);
    }
}

extern "C" fn on_usr1(_signo: u32) {
    IN_USR1.fetch_add(1, Ordering::SeqCst);
    // Uyku bir sistem cagrisidir; donusunde cekirdek bekleyen sinyalleri
    // teslim eder. Alarmin bu araliga dusmesi icin yeterince uzun.
    sys::sleep_ms(2000);
    IN_USR1.fetch_sub(1, Ordering::SeqCst);
}

extern "C" fn on_usr2(_signo: u32) {
    let depth = USR2_DEPTH.fetch_add(1, Ordering::SeqCst) + 1;
    USR2_MAX.fetch_max(depth, Ordering::SeqCst);
    USR2_COUNT.fetch_add(1, Ordering::SeqCst);

    // Kendine gonder -- ama sinirli. `SA_NODEFER` varken bu gercek bir
    // ozyineleme, o yuzden bir kez ile birakiliyor.
    if USR2_LIMIT.fetch_sub(1, Ordering::SeqCst) > 0 {
        signal::kill(signal::getpid(), signal::SIGUSR2);
        // Teslimin **bu isleyicinin icinde** olabilmesi icin bir cekirdek
        // donusu gerekiyor.
        sys::sleep_ms(300);
    }

    USR2_DEPTH.fetch_sub(1, Ordering::SeqCst);
}

extern "C" fn on_hup(_signo: u32) {
    HUP_COUNT.fetch_add(1, Ordering::SeqCst);
}

/// Tek bir sinavin sonucu.
#[derive(Clone, Copy)]
struct Check {
    name: &'static str,
    detail: &'static str,
    passed: bool,
}

const EMPTY: Check = Check {
    name: "",
    detail: "",
    passed: false,
};

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;

    let mut checks = [EMPTY; 4];

    signal::install(signal::SIGALRM, on_alarm);
    signal::install(signal::SIGUSR1, on_usr1);

    // --- A: farkli sinyal ic ice gelir mi? ---
    signal::alarm(1);
    signal::kill(signal::getpid(), signal::SIGUSR1);
    let nested = ALARM_NESTED.load(Ordering::SeqCst) == 1;
    checks[0] = Check {
        name: "A ic ice farkli sinyal",
        detail: if nested {
            "SIGALRM, SIGUSR1'in icinde kostu"
        } else {
            "SIGALRM bekledi -- ic ice gelmedi"
        },
        passed: nested,
    };

    // --- B: ayni sinyal varsayilan olarak engelli mi? ---
    USR2_LIMIT.store(1, Ordering::SeqCst);
    signal::action(signal::SIGUSR2, on_usr2, 0, 0);
    signal::kill(signal::getpid(), signal::SIGUSR2);
    // Ikinci teslim isleyici bittikten sonra olur; onu da toplayalim.
    sys::sleep_ms(400);
    let b_max = USR2_MAX.load(Ordering::SeqCst);
    let b_count = USR2_COUNT.load(Ordering::SeqCst);
    checks[1] = Check {
        name: "B ayni sinyal engelli",
        detail: if b_max == 1 && b_count >= 2 {
            "ic ice girmedi, teslim ertelendi"
        } else if b_max > 1 {
            "IC ICE GIRDI -- engelleme calismiyor"
        } else {
            "ikinci teslim hic olmadi"
        },
        passed: b_max == 1 && b_count >= 2,
    };

    // --- C: SA_NODEFER korumayi kaldiriyor mu? ---
    USR2_DEPTH.store(0, Ordering::SeqCst);
    USR2_MAX.store(0, Ordering::SeqCst);
    USR2_COUNT.store(0, Ordering::SeqCst);
    USR2_LIMIT.store(1, Ordering::SeqCst);
    signal::action(signal::SIGUSR2, on_usr2, signal::SA_NODEFER, 0);
    signal::kill(signal::getpid(), signal::SIGUSR2);
    sys::sleep_ms(400);
    let c_max = USR2_MAX.load(Ordering::SeqCst);
    checks[2] = Check {
        name: "C SA_NODEFER",
        detail: if c_max >= 2 {
            "ayni sinyal kendi icinde teslim edildi"
        } else {
            "ic ice GIRMEDI -- bayrak etkisiz"
        },
        passed: c_max >= 2,
    };

    // --- D: SA_RESETHAND tek atimlik mi? ---
    signal::action(signal::SIGHUP, on_hup, signal::SA_RESETHAND, 0);
    signal::kill(signal::getpid(), signal::SIGHUP);
    let reset = signal::current_handler(signal::SIGHUP) == signal::SIG_DFL;
    let ran = HUP_COUNT.load(Ordering::SeqCst) == 1;
    checks[3] = Check {
        name: "D SA_RESETHAND",
        detail: if ran && reset {
            "bir kez kostu, yerlestirme SIG_DFL'e dondu"
        } else if !ran {
            "isleyici hic kosmadi"
        } else {
            "YERLESTIRME DURUYOR -- sifirlanmadi"
        },
        passed: ran && reset,
    };
    // Ikinci SIGHUP gonderilmez: yerlestirme varsayilana dondugu icin
    // sureci oldururdu.

    for check in &checks {
        let _ = writeln!(
            out,
            "[nested] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }

    let mut win = match Window::open("nested -- sigaction bayraklari", 240, 130, 460, 220) {
        Some(w) => w,
        None => return,
    };

    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 4]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "sigaction: ic ice teslim ve bayraklar", ACCENT);

    let mut y = 32;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            180,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 14;
        win.text(16, y, check.detail, DIM);
        y += 18;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.fill(6, h - 42, w - 12, 20, PANEL);
    win.text(
        12,
        h - 39,
        if passed == checks.len() {
            "dort sinav da gecti"
        } else {
            "BIR SINAV KALDI"
        },
        if passed == checks.len() { OK } else { WARN },
    );
    win.text(6, h - 14, "q cik", DIM);
}
