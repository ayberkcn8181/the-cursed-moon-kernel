//! `winargv.exe` -- Win32'nin komut satiri ve ortam blogu sozlesmesi.
//!
//! POSIX ikizi (`quoted`) ayni cekirdek yoluna iniyor; ayrisan **bilginin
//! bicimi**, ve iki satirda ozetlenebilir:
//!
//! ```text
//!   execve(yol, argv[], envp[])          -> ayrilmis DIZILER
//!   CreateProcessA(.., lpCommandLine,       tek DIZE
//!                  .., lpEnvironment, ..)   duz BLOK
//! ```
//!
//! Fark kozmetik degil. Dizide `"iki kelime"` tek elemandir ve icindeki
//! bosluk hicbir sey ifade etmez; dizede bosluk **ayiricidir**, o yuzden
//! Windows'un alintilama kurallari vardir. Ayni sey ortamda da olur:
//! POSIX `char *[]` gecirir, Windows `AD=deger\0AD=deger\0\0` seklinde
//! duz bir blok.
//!
//! Bu program ikisinin de **kayipsiz** tasindigini olcuyor.
//!
//! ## Nasil olculuyor
//!
//! Cevap ekrandan degil, cocugun **cikis kodundan** okunuyor:
//!
//! ```text
//!   ebeveyn: CreateProcessA -> cocuk ayni ikilidir
//!   cocuk  : gordugu argv/ortami sinar, bit maskesiyle cikar
//!   ebeveyn: WaitForSingleObject + GetExitCodeProcess
//! ```
//!
//! POSIX ikizinde ayni is `fork`+`execve`+`waitpid` ile yapiliyor. Uc
//! cagri yerine uc **baska** cagri -- ama ayni zamanlayici, ayni gorev
//! tablosu.
//!
//! ## Dort sinav
//!
//! ```text
//!   A  alintilanmis eleman -> "iki kelime" TEK arguman kalir
//!   B  ters bolulu yol     -> C:\yol\x kacisa ugramaz
//!   C  lpEnvironment       -> blok ortamin TAMAMI olur
//!   D  argv[0]             -> komut satirinin ilk elemani korunur
//! ```
//!
//! B, Windows'un bilinen tuhafligini olcuyor: yol ayiricisi ile kacis
//! karakteri **ayni**. Kural ters boluyu yalnizca bir tirnaktan
//! onceyken ozel sayarak bunu cozer -- yani `C:\yol\x` hicbir sey
//! kaybetmez ama `C:\yol\"` kaybeder.
//!
//! C'de "tamami" vurgulu: blok verildiginde devralinan ortam **silinir**.
//! Sinav bunu, ebeveynde kurulmus bir degiskenin cocukta kaybolmasini
//! bekleyerek olcuyor -- yoksa "ekledi mi yerine mi gecti" ayirt
//! edilemezdi.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::ffi::c_void;

use tcmk::winapi::{self, ProcessInformation, Window};
use tcmk::{args, env};

tcmk::entry!(main);

const BG: u32 = 0x0014_1020;
const PANEL: u32 = 0x0024_1C38;
const FG: u32 = 0x00E4_DEF0;
const DIM: u32 = 0x008C_84A0;
const ACCENT: u32 = 0x00C0_A8FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Ikilinin Windows yoluyla kendi adi -- cekirdek bunu `/bin/...`e cevirir.
const SELF_PATH: &[u8] = b"C:\\bin\\winargv.exe\0";

/// Cocuga gecirilen komut satiri.
///
/// `argv[0]` bilerek yoldan **farkli** bir ad; Windows'ta da komut
/// satirinin ilk elemani calisan imajla ayni olmak zorunda degildir.
const CHILD_LINE: &[u8] = b"kabuk cocuk \"iki kelime\" C:\\yol\\x\0";

/// Cocuga gecirilen ortam blogu: girdiler NUL ile ayrilir, **cift NUL**
/// blogu bitirir. Dizinin degil blogun kullanilmasi Windows'un secimi.
const CHILD_ENV: &[u8] = b"ROL=cocuk\0YOL=C:\\bin\0\0";

const SPACED: &str = "iki kelime";
const WINPATH: &str = "C:\\yol\\x";
const ALIAS: &str = "kabuk";

const BIT_SPACED: u32 = 1 << 0;
const BIT_WINPATH: u32 = 1 << 1;
const BIT_ENV_NEW: u32 = 1 << 2;
const BIT_ENV_GONE: u32 = 1 << 3;
const BIT_ALIAS: u32 = 1 << 4;
const BIT_COUNT: u32 = 1 << 5;

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
    let mut console = winapi::Console;

    // Cocuk rolu: hicbir sey yazmaz, karari cikis koduyla birakir.
    if args::get(1) == Some("cocuk") {
        use core::fmt::Write;
        let _ = write!(console, "[winargv] cocuk argc={}", args::count());
        for i in 0..args::count() {
            let _ = write!(console, " [{}]='{}'", i, args::get(i).unwrap_or("?"));
        }
        let verdict = child_verdict();
        let _ = writeln!(
            console,
            " ROL={:?} KALAN={:?} maske={}",
            env::get("ROL"),
            env::get("KALAN"),
            verdict
        );
        unsafe { winapi::ExitProcess(verdict) };
    }

    let mut checks = [EMPTY; 4];

    // Ebeveynde iki degisken: biri cocukta degismeli, digeri kaybolmali.
    env::set("ROL", "ebeveyn");
    env::set("KALAN", "evet");

    let mask = run_child();

    let a = mask.map(|m| m & BIT_SPACED != 0).unwrap_or(false);
    checks[0] = Check {
        name: "A alintilanmis eleman",
        detail: match mask {
            None => "cocuk yaratilamadi",
            Some(m) if m & BIT_SPACED != 0 => "tirnak korundu, tek arguman",
            Some(_) => "bosluktan BOLUNDU",
        },
        passed: a,
    };

    let b = mask.map(|m| m & BIT_WINPATH != 0).unwrap_or(false);
    checks[1] = Check {
        name: "B ters bolulu yol",
        detail: match mask {
            None => "cocuk yaratilamadi",
            Some(m) if m & BIT_WINPATH != 0 => "C:\\yol\\x oldugu gibi geldi",
            Some(_) => "ters bolu kacisa ugradi",
        },
        passed: b,
    };

    let c = mask
        .map(|m| m & BIT_ENV_NEW != 0 && m & BIT_ENV_GONE != 0)
        .unwrap_or(false);
    checks[2] = Check {
        name: "C lpEnvironment",
        detail: match mask {
            None => "cocuk yaratilamadi",
            Some(m) if m & BIT_ENV_NEW == 0 => "bloktaki degisken gorunmedi",
            Some(m) if m & BIT_ENV_GONE == 0 => "devralinan ortam SILINMEDI",
            Some(_) => "blok ortamin yerine gecti",
        },
        passed: c,
    };

    let d = mask
        .map(|m| m & BIT_ALIAS != 0 && m & BIT_COUNT != 0)
        .unwrap_or(false);
    checks[3] = Check {
        name: "D argv[0] ve sayi",
        detail: match mask {
            None => "cocuk yaratilamadi",
            Some(m) if m & BIT_ALIAS == 0 => "argv[0] yola EZILDI",
            Some(m) if m & BIT_COUNT == 0 => "arguman sayisi yanlis",
            Some(_) => "verilen ad korundu, 4 arguman geldi",
        },
        passed: d,
    };

    for check in &checks {
        let _ = core::fmt::Write::write_str(&mut console, "[winargv] ");
        let _ = core::fmt::Write::write_str(&mut console, check.name);
        let _ = core::fmt::Write::write_str(&mut console, ": ");
        let _ = core::fmt::Write::write_str(
            &mut console,
            if check.passed { "gecti" } else { "KALDI" },
        );
        let _ = core::fmt::Write::write_str(&mut console, " (");
        let _ = core::fmt::Write::write_str(&mut console, check.detail);
        let _ = core::fmt::Write::write_str(&mut console, ")\n");
    }

    let mut win = match Window::create("winargv -- komut satiri", 320, 190, 440, 170) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, &checks);
        win.frame(60);
    }
}

/// Cocugu baslatir, bitmesini bekler ve cikis kodunu doner.
///
/// `lpApplicationName` ve `lpCommandLine` **birlikte** veriliyor:
/// Windows'un kurali, calisan imajin ilkinden, `argv[0]` dahil butun
/// argumanlarin ikincisinden gelmesidir. Ayrimi olcen sinav D.
fn run_child() -> Option<u32> {
    let mut info = ProcessInformation::new();
    let created = unsafe {
        winapi::CreateProcessA(
            SELF_PATH.as_ptr(),
            CHILD_LINE.as_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            0,
            0,
            CHILD_ENV.as_ptr() as *mut c_void,
            core::ptr::null(),
            core::ptr::null_mut(),
            &mut info,
        )
    };
    if created == 0 {
        return None;
    }
    let waited = unsafe { winapi::WaitForSingleObject(info.process, winapi::INFINITE) };
    if waited != winapi::WAIT_OBJECT_0 {
        return None;
    }
    let mut code = 0u32;
    let read = unsafe { winapi::GetExitCodeProcess(info.process, &mut code) };
    unsafe { winapi::CloseHandle(info.process) };
    if read == 0 || code == winapi::STILL_ACTIVE {
        return None;
    }
    Some(code)
}

/// Cocugun verdigi karar.
fn child_verdict() -> u32 {
    let mut mask = 0u32;
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
    if args::get(0) == Some(ALIAS) {
        mask |= BIT_ALIAS;
    }
    if args::count() == 4 {
        mask |= BIT_COUNT;
    }
    mask
}

fn draw(win: &mut Window, checks: &[Check; 4]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "tek dize, duz blok", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            300,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
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
