//! `death` -- bir cocugun **nasil** oldugunu ogrenmek.
//!
//! `waitpid` bir sayi dondurur, ama o sayi bir sayi degil: iki ayri
//! soruyu ayni kelimede cevaplayan **paketlenmis** bir durumdur.
//!
//! ```text
//!   normal cikis  -> (kod & 0xFF) << 8    WIFEXITED,   WEXITSTATUS
//!   sinyalle olum ->  signo & 0x7F        WIFSIGNALED, WTERMSIG
//! ```
//!
//! Ayrim sussuz gecilemez. Uzun sure TCMK sinyalle olen bir surec icin
//! `128 + signo`yu **cikis koduna** yaziyordu -- o sayi kabuklarin
//! gosterim gelenegidir, cekirdegin kodlamasi degil. `WIFSIGNALED`
//! soran bir program "normal cikti, kodu 141" cevabini alirdi.
//!
//! ## Cokme ayri bir kavram degil
//!
//! POSIX'te "cocuk coktu mu" diye ayri bir soru yok: cekirdek sayfa
//! hatasini `SIGSEGV`e, gecersiz komutu `SIGILL`e ceviriyor. Cokme,
//! sinyalle olumun bir turu -- ve bu sadelik bilincli.
//!
//! Windows tam tersini yapmis: orada sinyal yok, o yuzden "nasil oldu"
//! bilgisi **cikis kodunun degerine** gomulu. Coken bir surec
//! `GetExitCodeProcess`te `0xC0000005` (ACCESS_VIOLATION) gosterir --
//! yani NTSTATUS araligi "bu normal bir kod degil" demenin yolu.
//! Karsilastirmasi `windeath`te.
//!
//! ## Alti sinav
//!
//! ```text
//!   A  normal cikis  -> WIFEXITED dogru, kod 42
//!   B  sinyal degil  -> normal cikista WIFSIGNALED yanlis
//!   C  SIGKILL       -> WIFSIGNALED dogru, WTERMSIG 9
//!   D  cokme         -> gecersiz bellek erisimi SIGSEGV olur
//!   E  sifira bolme  -> SIGFPE olur (SIGSEGV degil)
//!   F  ayrim         -> cikis kodu 9 ile SIGKILL ayni sey DEGIL
//! ```
//!
//! F asil meselenin kendisi. `exit(9)` ile `SIGKILL` (9) ayni sayiyi
//! tasiyor; ayrimi yapmayan bir kodlama ikisini karistirirdi. Sinav
//! ikisini de kosuyor ve durum kelimelerinin **farkli** oldugunu
//! gosteriyor.
//!
//! E bilerek ayri: cokmenin tek bir sinyale duz eslenmedigini
//! gostermek, eslemeyi gercekten yapildigini kanitliyor.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x001A_1216;
const PANEL: u32 = 0x002C_2028;
const FG: u32 = 0x00EC_E0E6;
const DIM: u32 = 0x0094_8490;
const ACCENT: u32 = 0x00FF_A0B0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

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

/// Bir cocuk catallar, `body` kosturur ve durum kelimesini dondurur.
///
/// Cocuk hicbir kosulda geri donmemeli: `body` ya cikar ya oler.
fn run_child(body: fn() -> !) -> Option<u32> {
    match sys::fork() {
        0 => body(),
        id if id > 0 => {
            let mut status = 0u32;
            if sys::waitpid(id as usize, &mut status, 0) < 0 {
                return None;
            }
            Some(status)
        }
        _ => None,
    }
}

/// A/B: kendi istegiyle 42 ile cikar.
fn child_exit_42() -> ! {
    sys::exit(42);
}

/// F: kendi istegiyle **9** ile cikar -- SIGKILL ile ayni sayi.
fn child_exit_9() -> ! {
    sys::exit(9);
}

/// D: gecersiz bellege yazar. Cekirdek bunu SIGSEGV'e cevirmeli.
fn child_segfault() -> ! {
    unsafe {
        // Sifir sayfasi kullaniciya ait degil; yazma sayfa hatasi uretir.
        core::ptr::write_volatile(0usize as *mut u32, 1);
    }
    // Buraya ulasilmamali; ulasilirsa ayirt edilebilir bir kodla cik.
    sys::exit(3);
}

/// E: sifira boler. Cekirdek bunu SIGFPE'ye cevirmeli.
///
/// Bolme **ham komutla** yapiliyor ve bu sart. Rust'in `/` isleci
/// sifira bolmeyi dilin kendi kurali olarak yakaliyor: derleyici bir
/// sifir denetimi uretiyor ve panige daliyor, yani `div` komutu hic
/// calismiyor ve CPU istisnasi **hic olusmuyor**.
///
/// Ilk halinde tam bu oldu -- sinav "istisna hic olusmadi" diye kaldi
/// ve hata cekirdekte degil, olcumun kendisindeydi. Cekirdegin
/// `#DE -> SIGFPE` eslemesini sinamak icin komutu dogrudan yurutmek
/// gerekiyor.
fn child_divide_by_zero() -> ! {
    unsafe {
        // EDX:EAX / bolen -- bolen sifir oldugu icin #DE (vektor 0).
        core::arch::asm!(
            "xor edx, edx",
            "div {divisor:e}",
            divisor = in(reg) 0u32,
            inout("eax") 100u32 => _,
            out("edx") _,
            options(nostack),
        );
    }
    // Buraya ulasilmamali.
    sys::exit(3);
}

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;
    let mut checks = [EMPTY; 6];

    // --- A + B: normal cikis ---
    let normal = run_child(child_exit_42);
    let a = normal
        .map(|s| sys::exited(s) && sys::exit_status(s) == 42)
        .unwrap_or(false);
    let b = normal.map(|s| !sys::signalled(s)).unwrap_or(false);
    checks[0] = Check {
        name: "A normal cikis",
        detail: if a {
            "WIFEXITED dogru, kod 42"
        } else {
            "cikis kodu okunamadi"
        },
        passed: a,
    };
    checks[1] = Check {
        name: "B sinyal degil",
        detail: if b {
            "normal cikista WIFSIGNALED yanlis"
        } else {
            "normal cikis SINYAL gibi gorundu"
        },
        passed: b,
    };

    // --- C: SIGKILL ---
    //
    // Cocuk sonsuza kadar uyur; ebeveyn onu olduruyor.
    let mut kill_signal = 0u32;
    let c = match sys::fork() {
        0 => {
            loop {
                sys::sleep_ms(50);
            }
        }
        id if id > 0 => {
            // Cocugun gercekten kosmaya baslamasini bekle.
            sys::sleep_ms(60);
            tcmk::signal::kill(id as usize, tcmk::signal::SIGKILL);
            let mut status = 0u32;
            let reaped = sys::waitpid(id as usize, &mut status, 0) >= 0;
            kill_signal = sys::term_signal(status);
            reaped && sys::signalled(status) && kill_signal == tcmk::signal::SIGKILL
        }
        _ => false,
    };
    checks[2] = Check {
        name: "C SIGKILL",
        detail: if c {
            "WIFSIGNALED dogru, WTERMSIG 9"
        } else if kill_signal == 0 {
            "olduruldugu halde normal cikis gorundu"
        } else {
            "yanlis sinyal"
        },
        passed: c,
    };

    // --- D: cokme SIGSEGV olur ---
    let crashed = run_child(child_segfault);
    let mut crash_signal = 0u32;
    let d = crashed
        .map(|s| {
            crash_signal = sys::term_signal(s);
            sys::signalled(s) && crash_signal == tcmk::signal::SIGSEGV
        })
        .unwrap_or(false);
    checks[3] = Check {
        name: "D cokme",
        detail: if d {
            "gecersiz erisim SIGSEGV oldu"
        } else if crash_signal == 0 {
            "cokme normal cikis gibi gorundu"
        } else {
            "yanlis sinyal"
        },
        passed: d,
    };

    // --- E: sifira bolme SIGFPE olur ---
    //
    // Ayri sinav olmasi sart: butun cokmeler SIGSEGV'e eslenseydi D
    // yine gecerdi ama esleme yapilmamis olurdu.
    let divided = run_child(child_divide_by_zero);
    let mut fpe_signal = 0u32;
    let e = divided
        .map(|s| {
            fpe_signal = sys::term_signal(s);
            sys::signalled(s) && fpe_signal == tcmk::signal::SIGFPE
        })
        .unwrap_or(false);
    checks[4] = Check {
        name: "E sifira bolme",
        detail: if e {
            "SIGFPE oldu (SIGSEGV degil)"
        } else if fpe_signal == tcmk::signal::SIGSEGV {
            "SIGSEGV oldu -- esleme yapilmamis"
        } else if fpe_signal == 0 {
            "istisna hic olusmadi"
        } else {
            "yanlis sinyal"
        },
        passed: e,
    };

    // --- F: ayni sayi, farkli anlam ---
    //
    // `exit(9)` ile `SIGKILL` (9) ayni sayiyi tasiyor. Durum kelimesi
    // ikisini ayirt edemeseydi bir kabuk "9 ile cikti" ile
    // "olduruldu"yu karistirirdi.
    let exited_nine = run_child(child_exit_9);
    let f = exited_nine
        .map(|s| sys::exited(s) && sys::exit_status(s) == 9 && !sys::signalled(s))
        .unwrap_or(false)
        && c;
    checks[5] = Check {
        name: "F ayrim",
        detail: if f {
            "exit(9) ile SIGKILL(9) ayri gorunuyor"
        } else {
            "ikisi AYIRT EDILEMIYOR"
        },
        passed: f,
    };

    for check in &checks {
        let _ = writeln!(
            out,
            "[death] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }

    let mut win = match Window::open("death -- nasil oldu", 300, 190, 460, 190) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks, crash_signal as usize, fpe_signal as usize);
        win.flush();
    }
}

fn draw(win: &mut Window, checks: &[Check; 6], segv: usize, fpe: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "cokme = sinyalle olumun bir turu", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            340,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, "cokme sig:", DIM);
    win.number(110, h - 30, segv, FG);
    win.text(170, h - 30, "bolme sig:", DIM);
    win.number(275, h - 30, fpe, FG);
    win.text(
        6,
        h - 14,
        if passed == checks.len() {
            "hepsi gecti   q cik"
        } else {
            "BIR SINAV KALDI   q cik"
        },
        if passed == checks.len() { OK } else { WARN },
    );
}
