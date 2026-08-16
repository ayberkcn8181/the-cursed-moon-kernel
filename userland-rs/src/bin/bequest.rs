//! `bequest` -- ortam degiskeni `fork` ile devrediliyor, `execve` ile
//! korunuyor, kardese sizmiyor mu?
//!
//! Ortam ilk geldiginde **sistem geneli** tek bir tabloydu: her surec
//! baslangicta onun anlik goruntusunu aliyordu, ama kendi ortamini
//! degistiremiyordu -- `setenv` yoktu, cunku yazacak yer yoktu.
//!
//! Artik her gorev yuvasinin kendi tablosu var. POSIX'in uc kurali:
//!
//! | olay | ortam |
//! |---|---|
//! | `setenv` | yalnizca **kendi** tablosu degisir |
//! | `fork` | cocuk ebeveynin **kopyasini** alir |
//! | `execve` | yeni imaj **ayni** ortamla baslar |
//!
//! ## Dort sinav, hepsi belirlenimli
//!
//! ```text
//!   A  setenv kendi surecte gorunuyor mu?
//!      set("TCMK_MIRAS", "alfa") -> get() == "alfa"
//!
//!   B  fork ile devrediliyor mu?
//!      cocuk get("TCMK_MIRAS") okur, esitse 1 ile cikar
//!
//!   C  cocugun degisikligi ebeveyne sizmiyor mu?
//!      cocuk "cocuk" yazar; ebeveyn hala "alfa" gormeli
//!
//!   D  execve'den sonra yasiyor mu?
//!      "beta" yazilir, /bin/bequest `exec` argumaniyla yuklenir;
//!      yeni imaj degeri yigindaki environ dizisinden okur
//!
//!   E  cagiranin verdigi envp tabloyu YERINE geciyor mu?
//!      execve(..., ["TCMK_MIRAS=gamma"]) -> yeni imaj "gamma" gormeli
//!      ve HOME GORMEMELI: verilen dizi ortamin tamamidir
//! ```
//!
//! D sinavi programin **kendisini** yeniden yukler: ayni ikili, iki
//! asama. Ayrimi arguman yapiyor -- `run bequest exec` ile calisan surum
//! yalnizca degeri okuyup bildiriyor. Boylece sinav ikinci bir programa
//! bagimli olmuyor.
//!
//! Cocugun cevabi **cikis koduyla** geliyor, ekrana yazarak degil: iki
//! surecin ciktisi ayni konsolda karisabilir, cikis kodu karisamaz.
//!
//! Tuslar: `x` -> miras sinavi (D), `e` -> envp sinavi (E), `q` -> cik

#![no_std]
#![no_main]

use tcmk::args;
use tcmk::env;
use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0010_1A16;
const PANEL: u32 = 0x001C_2C26;
const FG: u32 = 0x00DC_E8E0;
const DIM: u32 = 0x0078_9084;
const ACCENT: u32 = 0x0068_E0B0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Sinavlarin uzerinde calistigi degisken. Adi bilerek TCMK onekli:
/// oturum tablosundan gelen `HOME`/`PATH`/`SHELL` ile karismasin.
const NAME: &str = "TCMK_MIRAS";

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

    // --- Ikinci asama: execve ile yuklenmis surum ---
    //
    // Burada yalnizca **okuma** var. Deger yigindaki `environ`
    // dizisinden geliyor ve o diziyi cekirdek, gorev yuvasinin
    // tablosundan yeniden kurdu -- yani exec'ten once yazilan deger
    // hala oradaysa, tablo yuvada kaldi demektir.
    let phase = args::first();
    if phase == Some("exec") || phase == Some("envp") {
        let value = env::get(NAME);
        let home = env::get("HOME");
        // Iki mod, iki ayri beklenti:
        //
        //   exec  ortam **korundu**: exec'ten once yazilan "beta" duruyor
        //         ve oturumdan gelen HOME de yerinde.
        //   envp  ortam **degistirildi**: cagiranin verdigi "gamma"
        //         gorunuyor ve HOME **yok** -- verilen dizi ortamin
        //         tamamidir, eskisinin uzerine eklenmez.
        let explicit = phase == Some("envp");
        let passed = if explicit {
            value == Some("gamma") && home.is_none()
        } else {
            value == Some("beta") && home.is_some()
        };
        let _ = writeln!(
            out,
            "[bequest] {} execve sonrasi: {} ({}={}, HOME={})",
            if explicit { "E envp" } else { "D miras" },
            if passed { "gecti" } else { "KALDI" },
            NAME,
            value.unwrap_or("<yok>"),
            home.unwrap_or("<yok>")
        );
        // Ekranda da kalsin: exec'ten sonra ayri bir pencere aciliyor.
        let mut win = match Window::open("bequest -- execve sonrasi", 300, 200, 430, 140) {
            Some(w) => w,
            None => return,
        };
        loop {
            if win.poll_key() == b'q' {
                break;
            }
            let (w, h) = (win.width(), win.height());
            win.clear(BG);
            win.fill(0, 0, w, 22, PANEL);
            win.text(
                6,
                3,
                if explicit {
                    "E: execve'ye verilen envp"
                } else {
                    "D: execve'den sonra miras"
                },
                ACCENT,
            );
            win.text(6, 34, NAME, FG);
            win.text(120, 34, value.unwrap_or("<yok>"), if passed { OK } else { WARN });
            win.text(6, 50, "HOME", DIM);
            win.text(120, 50, home.unwrap_or("<yok>"), DIM);
            win.text(
                6,
                72,
                if !passed {
                    "KALDI -- ortam beklenen gibi degil"
                } else if explicit {
                    "gecti -- verilen dizi ortamin tamami"
                } else {
                    "gecti -- deger exec'ten sagladi"
                },
                if passed { OK } else { WARN },
            );
            win.text(6, h - 14, "q cik", DIM);
            win.frame(60);
        }
        return;
    }

    // --- Birinci asama: A, B, C ---
    let mut checks = [EMPTY; 3];

    // A: kendi surecte gorunuyor mu?
    let wrote = env::set(NAME, "alfa");
    let seen = env::get(NAME);
    checks[0] = Check {
        name: "A setenv kendinde",
        detail: if !wrote {
            "setenv basarisiz"
        } else if seen == Some("alfa") {
            "get() yeni degeri gordu"
        } else {
            "get() ESKI degeri gordu"
        },
        passed: wrote && seen == Some("alfa"),
    };

    // B ve C: tek bir `fork` ikisini birden olcuyor.
    //
    // Cocuk once devraldigini bildiriyor (cikis kodu), sonra kendi
    // degerini yaziyor. Ebeveyn cocuk bittikten sonra kendi degerine
    // bakiyor -- hala "alfa" ise sizinti yok.
    let child = sys::fork();
    if child == 0 {
        let inherited = env::get(NAME) == Some("alfa");
        env::set(NAME, "cocuk");
        sys::exit(if inherited { 1 } else { 0 });
    }

    let mut status = 0u32;
    let mut inherited = false;
    if child > 0 {
        sys::waitpid(child as usize, &mut status, 0);
        inherited = sys::exit_status(status) == 1;
    }
    checks[1] = Check {
        name: "B fork mirasi",
        detail: if child <= 0 {
            "fork basarisiz"
        } else if inherited {
            "cocuk 'alfa' gordu"
        } else {
            "cocuk degeri GORMEDI"
        },
        passed: inherited,
    };

    let after = env::get(NAME);
    checks[2] = Check {
        name: "C kardes yalitimi",
        detail: if after == Some("alfa") {
            "cocugun yazdigi ebeveyne sizmadi"
        } else {
            "EBEVEYN cocugun degerini gordu"
        },
        passed: after == Some("alfa"),
    };

    for check in &checks {
        let _ = writeln!(
            out,
            "[bequest] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }

    let mut win = match Window::open("bequest -- ortam mirasi", 260, 140, 440, 210) {
        Some(w) => w,
        None => return,
    };

    loop {
        match win.poll_key() {
            b'q' => break,
            // D: kendini yeniden yukler; bu satirdan sonrasi calismaz.
            b'x' => {
                env::set(NAME, "beta");
                let _ = writeln!(out, "[bequest] D: {}=beta yazildi, execve...", NAME);
                sys::execve_args("/bin/bequest", "exec");
                let _ = writeln!(out, "[bequest] execve basarisiz");
            }
            // E: bu kez ortam **acikca** veriliyor. Yuvada duran tablo
            // (HOME dahil) tamamen yerini bu dizeye birakmali.
            b'e' => {
                env::set(NAME, "beta");
                let _ = writeln!(out, "[bequest] E: envp=[{}=gamma], execve...", NAME);
                sys::execve_env(
                    "/bin/bequest",
                    "envp",
                    Some(&["TCMK_MIRAS=gamma"]),
                );
                let _ = writeln!(out, "[bequest] execve basarisiz");
            }
            _ => {}
        }
        draw(&mut win, &checks);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 3]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "ortam: setenv / fork / execve", ACCENT);

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
            "uc sinav da gecti -- x / e ile execve"
        } else {
            "BIR SINAV KALDI"
        },
        if passed == checks.len() { OK } else { WARN },
    );
    win.text(6, h - 14, "x miras  e envp  q cik", DIM);
}
