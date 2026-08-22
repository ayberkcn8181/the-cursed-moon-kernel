//! `quoted` -- `execve(yol, argv[], envp[])`: dizi, dize degil.
//!
//! TCMK'nin `execve`si bugune kadar kendi numarasindan (`0x509`) ve
//! **tek bir dizeden** ibaretti. Cekirdek o dizeyi boslugundan boluyordu,
//! yani `"iki kelime"` yazan bir cagiran iki arguman gonderiyordu.
//! Derleyicinin urettigi gercek bir Linux ikilisi ise `execve`yi 11 (ya
//! da x86_64'te 59) numarayla ve bir **dizi** ile cagirir; dizide bosluk
//! hicbir sey ifade etmez.
//!
//! Ikisi de artik var ve bu program ikisini de olcuyor.
//!
//! ## Nasil olculuyor
//!
//! Cevap ekrandan degil, **cikis kodundan** okunuyor:
//!
//! ```text
//!   ebeveyn: fork -> cocuk kendini yeniden yukler (execve)
//!   cocuk  : gordugu argv/ortami sinar, sonucu bit maskesi olarak cikar
//!   ebeveyn: waitpid ile maskeyi alir
//! ```
//!
//! Cocuk ayni ikilidir; hangi rolde oldugunu `argv[1]`den anlar. Bu,
//! ayri bir yardimci program yazmaktan daha durust: sinanan sey
//! **argv'nin kendisi** oldugu icin, onu tasiyan da argv olmali.
//!
//! ## Dort sinav
//!
//! ```text
//!   A  bosluklu eleman  -> dizide TEK arguman kalir
//!   B  argv[0]          -> cagiranin verdigi ad korunur (yol degil)
//!   C  envp             -> verilen dizi ortamin TAMAMI olur
//!   D  eski bicim       -> tek dize, ama artik alintilamayi anliyor
//! ```
//!
//! B'nin neden onemli oldugu tek ornekte gorunur: busybox tek bir
//! ikilidir ve hangi komut oldugunu `argv[0]`dan anlar. `argv[0]`i yola
//! esitleyen bir cekirdek onu calistiramazdi.
//!
//! C'de "tamami" vurgulu: gercek `execve`de `envp` eskisine **eklenmez**,
//! yerine gecer. Sinav bunu, ebeveynde kurulmus bir degiskenin cocukta
//! **kaybolmasini** bekleyerek olcuyor.
//!
//! D, eski bicimi de kazanan tarafa gecirdi: cekirdek artik iki bicimi
//! de ayni bloga ceviriyor, ve bolme Windows'un alintilama kurallariyla
//! yapiliyor. Yani `0x509` da bosluklu bir argumani koruyor.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::{args, env, sys};

tcmk::entry!(main);

const BG: u32 = 0x0014_1020;
const PANEL: u32 = 0x0024_1C38;
const FG: u32 = 0x00E4_DEF0;
const DIM: u32 = 0x008C_84A0;
const ACCENT: u32 = 0x00C0_A8FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Ikilinin kendi yolu -- cocuk bunu yeniden yukleyecek.
const SELF_PATH: &str = "/bin/quoted";

/// Bosluk iceren arguman. Dizi biciminde **tek** eleman olmali.
const SPACED: &str = "iki kelime";
/// Ters bolulu bir Windows yolu: kacisa ugramamali.
const WINPATH: &str = "C:\\yol\\x";
/// `argv[0]` olarak gecirilen ad -- bilerek yoldan **farkli**.
const ALIAS: &str = "kabuk";

/// Cocugun cikis kodundaki bitler.
const BIT_ALIAS: i32 = 1 << 0;
const BIT_SPACED: i32 = 1 << 1;
const BIT_WINPATH: i32 = 1 << 2;
const BIT_ENV_NEW: i32 = 1 << 3;
const BIT_ENV_GONE: i32 = 1 << 4;
const BIT_COUNT: i32 = 1 << 5;
/// Eski bicim (tek dize) icin ayri bir bit kumesi.
const BIT_LINE_ARG: i32 = 1 << 0;
const BIT_LINE_ARGV0: i32 = 1 << 1;

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

    // --- Cocuk rolleri ---
    //
    // `argv[1]` rolu soyluyor. Cocuk hicbir sey yazmaz; cevabi cikis
    // koduyla birakir, cunku olcumu ebeveyn yapiyor.
    // Cocuk gordugunu **gunluge** de yazar. Sinav cikis kodundan
    // okunuyor ama bir sinav kaldiginda "ne gordu" sorusunun cevabi
    // ancak boyle bulunur.
    if let Some(role) = args::get(1) {
        if role == "dizi" || role == "satir" {
            let _ = write!(out, "[quoted] cocuk({}) argc={}", role, args::count());
            for i in 0..args::count() {
                let _ = write!(out, " [{}]='{}'", i, args::get(i).unwrap_or("?"));
            }
            let _ = writeln!(out, " ROL={:?} KALAN={:?}", env::get("ROL"), env::get("KALAN"));
            let verdict = if role == "dizi" {
                vector_child_verdict()
            } else {
                line_child_verdict()
            };
            let _ = writeln!(out, "[quoted] cocuk({}) maske={}", role, verdict);
            sys::exit(verdict);
        }
    }

    let mut checks = [EMPTY; 4];

    // Ebeveynde iki degisken kurulur: biri cocukta **degismeli**, digeri
    // **kaybolmali**. Ikincisi olmadan "ekledi mi yoksa yerine mi gecti"
    // ayirt edilemezdi.
    env::set("ROL", "ebeveyn");
    env::set("KALAN", "evet");

    // --- Dizi bicimi ---
    let vector_mask = run_child(&mut out, true);
    let a = vector_mask.map(|m| m & BIT_SPACED != 0).unwrap_or(false);
    checks[0] = Check {
        name: "A bosluklu eleman",
        detail: match vector_mask {
            None => "cocuk yaratilamadi",
            Some(m) if m & BIT_SPACED != 0 => "dizide tek arguman kaldi",
            Some(_) => "bosluktan BOLUNDU",
        },
        passed: a,
    };

    let b = vector_mask
        .map(|m| m & BIT_ALIAS != 0 && m & BIT_COUNT != 0 && m & BIT_WINPATH != 0)
        .unwrap_or(false);
    checks[1] = Check {
        name: "B argv[0] ve sayi",
        detail: match vector_mask {
            None => "cocuk yaratilamadi",
            Some(m) if m & BIT_ALIAS == 0 => "argv[0] yola EZILDI",
            Some(m) if m & BIT_COUNT == 0 => "arguman sayisi yanlis",
            Some(m) if m & BIT_WINPATH == 0 => "ters bolu kacisa ugradi",
            Some(_) => "verilen ad korundu, 4 arguman geldi",
        },
        passed: b,
    };

    let c = vector_mask
        .map(|m| m & BIT_ENV_NEW != 0 && m & BIT_ENV_GONE != 0)
        .unwrap_or(false);
    checks[2] = Check {
        name: "C envp yerine gecti",
        detail: match vector_mask {
            None => "cocuk yaratilamadi",
            Some(m) if m & BIT_ENV_NEW == 0 => "verilen degisken gorunmedi",
            Some(m) if m & BIT_ENV_GONE == 0 => "eski ortam SILINMEDI",
            Some(_) => "yeni ortam var, eskisi yok",
        },
        passed: c,
    };

    // --- Eski bicim (tek dize) ---
    let line_mask = run_child(&mut out, false);
    let d = line_mask
        .map(|m| m & BIT_LINE_ARG != 0 && m & BIT_LINE_ARGV0 != 0)
        .unwrap_or(false);
    checks[3] = Check {
        name: "D eski bicim + tirnak",
        detail: match line_mask {
            None => "cocuk yaratilamadi",
            Some(m) if m & BIT_LINE_ARG == 0 => "tirnakli arguman bolundu",
            Some(m) if m & BIT_LINE_ARGV0 == 0 => "argv[0] yol degil",
            Some(_) => "tirnak korundu, argv[0] = yol",
        },
        passed: d,
    };

    for check in &checks {
        let _ = writeln!(
            out,
            "[quoted] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }

    let mut win = match Window::open("quoted -- execve dizisi", 300, 170, 440, 170) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks);
        win.flush();
    }
}

/// Cocugu baslatir ve cikis kodunu (bit maskesi) doner.
///
/// `fork` + `execve`: gercek POSIX deyimi de budur. Cocuk `execve`den
/// donmez, yani `fork`un cocuk dali ancak `execve` **basarisiz** olursa
/// devam eder -- o yuzden orada dogrudan cikiliyor.
fn run_child(out: &mut Stdout, vector_form: bool) -> Option<i32> {
    use core::fmt::Write;

    match sys::fork() {
        0 => {
            if vector_form {
                sys::execve_argv(
                    SELF_PATH,
                    &[ALIAS, "dizi", SPACED, WINPATH],
                    Some(&["ROL=cocuk"]),
                );
            } else {
                // Eski bicim: argumanlar tek dize, `argv[0]` cekirdek
                // tarafindan yoldan uretilir.
                sys::execve_args(SELF_PATH, "satir \"iki kelime\"");
            }
            // Buraya gelindiyse `execve` basarisiz oldu.
            sys::exit(0xFF);
        }
        id if id > 0 => {
            let mut status = 0u32;
            let waited = sys::waitpid(id as usize, &mut status, 0);
            if waited < 0 {
                let _ = writeln!(out, "[quoted] waitpid basarisiz: {}", waited);
                return None;
            }
            Some(sys::exit_status(status) as i32)
        }
        _ => None,
    }
}

/// Dizi biciminde yuklenen cocugun verdigi karar.
fn vector_child_verdict() -> i32 {
    let mut mask = 0i32;
    if args::get(0) == Some(ALIAS) {
        mask |= BIT_ALIAS;
    }
    if args::get(2) == Some(SPACED) {
        mask |= BIT_SPACED;
    }
    if args::get(3) == Some(WINPATH) {
        mask |= BIT_WINPATH;
    }
    if env::get("ROL") == Some("cocuk") {
        mask |= BIT_ENV_NEW;
    }
    if env::get("KALAN").is_none() {
        mask |= BIT_ENV_GONE;
    }
    if args::count() == 4 {
        mask |= BIT_COUNT;
    }
    mask
}

/// Eski (tek dizeli) bicimde yuklenen cocugun verdigi karar.
fn line_child_verdict() -> i32 {
    let mut mask = 0i32;
    if args::get(2) == Some(SPACED) {
        mask |= BIT_LINE_ARG;
    }
    if args::get(0) == Some(SELF_PATH) {
        mask |= BIT_LINE_ARGV0;
    }
    mask
}

fn draw(win: &mut Window, checks: &[Check; 4]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "execve: dizi mi, dize mi?", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(300, y, if check.passed { "gecti" } else { "KALDI" },
                 if check.passed { OK } else { WARN });
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, args::get(0).unwrap_or("?"), DIM);
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
