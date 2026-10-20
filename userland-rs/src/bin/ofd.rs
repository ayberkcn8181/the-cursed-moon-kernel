//! `ofd` -- tanimlayici ile acik dosya tanimi: iki ayri katman.
//!
//! POSIX'te bir tanimlayici iki seydir:
//!
//! ```text
//!   tanimlayici (fd)         surec basina bir numara; tek bayragi
//!                            FD_CLOEXEC
//!   acik dosya tanimi        open() basina bir kayit; KONUM, O_NONBLOCK,
//!                            O_APPEND burada
//! ```
//!
//! `dup`, `dup2` ve `fork` yeni bir tanimlayici yaratir ama **ayni tanima**
//! baglar: iki tanimlayici tek bir konumu paylasir. Kabugun
//! `(echo a; echo b) > f` kalibi buna dayaniyor. TCMK uzun sure ikisini
//! tek bir degerde tutuyordu ve kopyalar kopyadan sonra ayrisiyordu.
//!
//! ## On sinav
//!
//! ```text
//!   A  dup konumu paylasir    -> kopya asliyla, asil kopyayla devam etti
//!   B  lseek paylasilir       -> kopyada lseek, aslinin konumunu tasidi
//!   C  ayri open ayri konum   -> ikinci open bastan okudu, ilkine dokunmadi
//!   D  fork konumu paylasir   -> cocugun okumasi ebeveynin konumunu ilerletti
//!   E  O_NONBLOCK dup         -> kopyaya konan bayrak aslinda goruldu
//!   F  O_NONBLOCK fork        -> cocugun koydugu bayrak ebeveynde: okuma EAGAIN
//!   G  FD_CLOEXEC yuvaya ait  -> dup kopyasinda bayrak YOK; F_DUPFD_CLOEXEC'te var
//!   H  exec kapatir           -> O_CLOEXEC'li tanimlayici yeni imajda yok,
//!                                digeri acik ve KONUMU korunmus
//!   I  O_APPEND (disk)        -> lseek(0) sonrasi yazma yine sona gitti
//!   J  paylasilan yazma       -> cocuk ve ebeveyn ayni tanima sirayla yazdi:
//!      (disk)                    ikisi de dosyada, ust uste degil
//! ```
//!
//! C bir tuzagi kapatiyor: "her sey paylasiliyor" da A, B ve D'yi gecerdi.
//! G de ayni soruyu bayraklar icin soruyor -- `FD_CLOEXEC` tanima degil
//! yuvaya ait, `dup` onu tasimamali.
//!
//! I ve J yazma ister; RAMFS salt okunur oldugu icin disk yoksa atlaniyor.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0016_1A14;
const PANEL: u32 = 0x0024_2C20;
const FG: u32 = 0x00E6_EEE0;
const DIM: u32 = 0x0094_A08C;
const ACCENT: u32 = 0x00B0_E070;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;
const SKIP: u32 = 0x00C0_C060;

const EBADF: isize = 9;
const EAGAIN: isize = 11;

const MSG: &[u8] = b"/boot/msg.txt\0";
const DISK_FILE: &[u8] = b"/home/ofd.txt\0";
/// `execve` edilen rolun cikis kodundaki "gercekten bu rol kostu" biti.
const ROLE_DONE: i32 = 0x40;

fn open(path: &[u8], flags: usize) -> isize {
    unsafe { sys::open_raw(path.as_ptr(), flags) }
}

fn pos(fd: usize) -> isize {
    sys::lseek(fd, 0, sys::SEEK_CUR)
}

/// `n` bayt okur; okunani doner.
fn take(fd: usize, n: usize) -> ([u8; 16], usize) {
    let mut buf = [0u8; 16];
    let got = sys::read(fd, &mut buf[..n.min(16)]);
    (buf, got.max(0) as usize)
}

#[derive(Clone, Copy, PartialEq)]
enum Verdict {
    Pass,
    Fail,
    Skip,
}

struct Check {
    name: &'static str,
    detail: &'static str,
    verdict: Verdict,
}

const EMPTY: Check = Check {
    name: "",
    detail: "",
    verdict: Verdict::Fail,
};

const NAMES: [&str; 10] = [
    "A dup konumu paylasir",
    "B lseek paylasilir",
    "C ayri open ayri konum",
    "D fork konumu paylasir",
    "E O_NONBLOCK dup",
    "F O_NONBLOCK fork",
    "G FD_CLOEXEC yuvaya ait",
    "H exec kapatir",
    "I O_APPEND",
    "J paylasilan yazma",
];

fn verdict(ok: bool) -> Verdict {
    if ok {
        Verdict::Pass
    } else {
        Verdict::Fail
    }
}

/// `execve` ile gelinen rol: "x <kapanmali> <acik> <konum>".
///
/// Cikis kodu bitleri: 1 = O_CLOEXEC'li tanimlayici kapanmis, 2 = digeri
/// acik, 4 = digerinin konumu korunmus. `ROLE_DONE` rolun gercekten
/// kostugunu soyluyor (yoksa exec hic olmamis olabilir).
fn exec_role() -> ! {
    let num = |i: usize| -> usize {
        tcmk::args::get(i)
            .map(|s| s.bytes().fold(0usize, |n, b| n * 10 + (b.wrapping_sub(b'0') as usize)))
            .unwrap_or(99)
    };
    let (closed, open, at) = (num(2), num(3), num(4));
    let mut code = ROLE_DONE;
    if sys::fcntl(closed, sys::F_GETFD, 0) == -EBADF {
        code |= 1;
    }
    if sys::fcntl(open, sys::F_GETFD, 0) == 0 {
        code |= 2;
    }
    if pos(open) == at as isize {
        code |= 4;
    }
    sys::exit(code);
}

fn main() {
    if tcmk::args::get(1) == Some("x") {
        exec_role();
    }

    use core::fmt::Write;
    let _ = writeln!(Stdout, "[ofd] sinav basliyor");
    let mut checks = [EMPTY; 10];
    let mut win = Window::open("ofd -- tanimlayici ve tanim", 260, 120, 480, 250);

    // Karsilastirma icin dosyanin basi, ayri bir tanimdan.
    let mut whole = [0u8; 16];
    let ref_fd = open(MSG, 0);
    let ref_n = if ref_fd >= 0 {
        let n = sys::read(ref_fd as usize, &mut whole);
        sys::close(ref_fd as usize);
        n.max(0) as usize
    } else {
        0
    };

    // --- A, B, C: dup ve ayri open ------------------------------------
    let fd = open(MSG, 0);
    let (a, b, c);
    if fd < 0 || ref_n < 16 {
        a = false;
        b = false;
        c = false;
    } else {
        let fd = fd as usize;
        let _ = take(fd, 4);
        let d = sys::dup(fd);
        let (got, n) = if d >= 0 { take(d as usize, 4) } else { ([0; 16], 0) };
        // Iki yon: kopya aslin konumundan devam etmeli, **sonra** asil de
        // kopyanin okudugu yerden. Yalnizca ilki, `dup` aninda konumu
        // kopyalayan (paylasmayan) bir tabloyu da gecirirdi.
        let (back, n2) = take(fd, 4);
        let forward = d >= 0 && n == 4 && got[..4] == whole[4..8];
        a = forward && n2 == 4 && back[..4] == whole[8..12];
        checks[0] = Check {
            name: NAMES[0],
            detail: if a {
                "kopya asliyla, asil kopyayla devam etti (4-8-12)"
            } else if n == 4 && got[..4] == whole[..4] {
                "kopya BASTAN okudu: konum ayrismis"
            } else if forward && n2 == 4 && back[..4] == whole[4..8] {
                "asil kopyanin okumasini GORMEDI: konum dup aninda kopyalanmis"
            } else {
                "dup ya da okuma basarisiz"
            },
            verdict: verdict(a),
        };

        let moved = d >= 0 && sys::lseek(d as usize, 10, sys::SEEK_SET) == 10;
        b = moved && pos(fd) == 10;
        checks[1] = Check {
            name: NAMES[1],
            detail: if b {
                "kopyada lseek(10), aslinin konumu da 10"
            } else if !moved {
                "kopyada lseek basarisiz"
            } else {
                "aslinin konumu DEGISMEDI: konum ayrismis"
            },
            verdict: verdict(b),
        };

        // C kendi olcusunu tutuyor: B kalmissa konum 10 olmayabilir ve
        // "10'da mi" diye sormak B'nin hatasini C'ye yazardi.
        let before = pos(fd);
        let fd2 = open(MSG, 0);
        let (got2, n2) = if fd2 >= 0 { take(fd2 as usize, 4) } else { ([0; 16], 0) };
        let untouched = pos(fd) == before;
        c = fd2 >= 0 && n2 == 4 && got2[..4] == whole[..4] && untouched;
        checks[2] = Check {
            name: NAMES[2],
            detail: if c {
                "ikinci open bastan okudu, ilkinin konumuna dokunmadi"
            } else if !untouched {
                "ayri open ilkinin konumunu TASIDI: fazla paylasim"
            } else {
                "ikinci open bastan okumadi"
            },
            verdict: verdict(c),
        };
        if fd2 >= 0 {
            sys::close(fd2 as usize);
        }
        if d >= 0 {
            sys::close(d as usize);
        }

        // --- D: fork konumu paylasir ----------------------------------
        sys::lseek(fd, 0, sys::SEEK_SET);
        let child = sys::fork();
        if child == 0 {
            let _ = take(fd, 5);
            sys::exit(0);
        }
        let mut status = 0u32;
        let reaped = child > 0 && sys::waitpid(child as usize, &mut status, 0) == child;
        let (got, n) = take(fd, 4);
        let dd = reaped && n == 4 && got[..4] == whole[5..9];
        checks[3] = Check {
            name: NAMES[3],
            detail: if dd {
                "cocuk 5 bayt okudu, ebeveyn 5. bayttan devam etti"
            } else if n == 4 && got[..4] == whole[..4] {
                "ebeveyn BASTAN okudu: cocugun okumasi konumu tasimadi"
            } else {
                "fork ya da okuma basarisiz"
            },
            verdict: verdict(dd),
        };
        sys::close(fd);
    }
    if fd < 0 || ref_n < 16 {
        for i in 0..4 {
            checks[i] = Check {
                name: NAMES[i],
                detail: "/boot/msg.txt acilamadi",
                verdict: Verdict::Fail,
            };
        }
    }

    // --- E: O_NONBLOCK dup kopyasinda paylasilir -----------------------
    let e = match sys::pipe() {
        Some((r, w)) => {
            let d = sys::dup(r);
            let set = d >= 0 && sys::set_flags(d as usize, sys::O_NONBLOCK) == 0;
            let seen = sys::get_flags(r) as usize & sys::O_NONBLOCK != 0;
            if d >= 0 {
                sys::close(d as usize);
            }
            sys::close(r);
            sys::close(w);
            set && seen
        }
        None => false,
    };
    checks[4] = Check {
        name: NAMES[4],
        detail: if e {
            "kopyaya konan O_NONBLOCK aslinda goruldu"
        } else {
            "bayrak kopyada KALDI: tanim paylasilmiyor"
        },
        verdict: verdict(e),
    };

    // --- F: O_NONBLOCK fork cocugundan ebeveyne -------------------------
    let (f, f_detail) = match sys::pipe() {
        Some((r, w)) => {
            let child = sys::fork();
            if child == 0 {
                sys::set_flags(r, sys::O_NONBLOCK);
                sys::exit(0);
            }
            let mut status = 0u32;
            let _ = child > 0 && sys::waitpid(child as usize, &mut status, 0) == child;
            let seen = sys::get_flags(r) as usize & sys::O_NONBLOCK != 0;
            // Bayrak gorunmuyorsa okumak sinavi kilitlerdi: yazan uc
            // acik ve boru bos.
            let read = if seen {
                let mut b = [0u8; 1];
                sys::read(r, &mut b)
            } else {
                0
            };
            sys::close(r);
            sys::close(w);
            let ok = seen && read == -EAGAIN;
            (
                ok,
                if ok {
                    "cocugun koydugu bayrak ebeveynde, bos boru EAGAIN"
                } else if !seen {
                    "cocugun bayragi ebeveyne GECMEDI"
                } else {
                    "bayrak goruldu ama okuma EAGAIN donmedi"
                },
            )
        }
        None => (false, "boru acilamadi"),
    };
    checks[5] = Check {
        name: NAMES[5],
        detail: f_detail,
        verdict: verdict(f),
    };

    // --- G: FD_CLOEXEC yuvaya ait ---------------------------------------
    let fd = open(MSG, 0);
    let g = if fd >= 0 {
        let fd = fd as usize;
        let set = sys::fcntl(fd, sys::F_SETFD, sys::FD_CLOEXEC) == 0;
        let on = sys::fcntl(fd, sys::F_GETFD, 0) == sys::FD_CLOEXEC as isize;
        let d = sys::dup(fd);
        let copy_off = d >= 0 && sys::fcntl(d as usize, sys::F_GETFD, 0) == 0;
        let high = sys::fcntl(fd, sys::F_DUPFD_CLOEXEC, 10);
        let high_on = high >= 10 && sys::fcntl(high as usize, sys::F_GETFD, 0) == sys::FD_CLOEXEC as isize;
        // `F_SETFD` durum bayraklarina dokunmamali.
        let status_clean = sys::get_flags(fd) as usize & sys::FD_CLOEXEC == 0;
        for x in [d, high] {
            if x >= 0 {
                sys::close(x as usize);
            }
        }
        sys::close(fd);
        let ok = set && on && copy_off && high_on && status_clean;
        checks[6] = Check {
            name: NAMES[6],
            detail: if ok {
                "dup kopyasinda yok, F_DUPFD_CLOEXEC'te >= 10 numarada var"
            } else if !on {
                "F_SETFD bayragi TUTMADI"
            } else if !copy_off {
                "dup bayragi TASIDI: tanima yazilmis"
            } else if !high_on {
                "F_DUPFD_CLOEXEC calismadi"
            } else {
                "F_SETFD durum bayraklarini da degistirdi"
            },
            verdict: verdict(ok),
        };
        ok
    } else {
        checks[6] = Check {
            name: NAMES[6],
            detail: "/boot/msg.txt acilamadi",
            verdict: Verdict::Fail,
        };
        false
    };
    let _ = g;

    // --- H: exec kapatir ------------------------------------------------
    let closing = open(MSG, sys::O_CLOEXEC);
    let keeping = open(MSG, 0);
    let (h, h_detail) = if closing >= 0 && keeping >= 0 {
        let _ = take(keeping as usize, 3);
        let child = sys::fork();
        if child == 0 {
            let mut args = [0u8; 32];
            let mut n = 0;
            for part in [b"x ".as_slice()] {
                args[..part.len()].copy_from_slice(part);
                n = part.len();
            }
            for (i, v) in [closing as usize, keeping as usize, 3].iter().enumerate() {
                if *v >= 10 {
                    args[n] = b'0' + (*v / 10) as u8;
                    n += 1;
                }
                args[n] = b'0' + (*v % 10) as u8;
                n += 1;
                if i < 2 {
                    args[n] = b' ';
                    n += 1;
                }
            }
            let line = core::str::from_utf8(&args[..n]).unwrap_or("x");
            sys::execve_args("/bin/ofd", line);
            sys::exit(0x3F);
        }
        let mut status = 0u32;
        let got = child > 0 && sys::waitpid(child as usize, &mut status, 0) == child;
        let code = if got && sys::exited(status) {
            sys::exit_status(status) as i32
        } else {
            0
        };
        let ok = code == ROLE_DONE | 7;
        (
            ok,
            if ok {
                "O_CLOEXEC'li kapandi, digeri acik ve konumu (3) korundu"
            } else if code & ROLE_DONE == 0 {
                "yeni imaj kosmadi: execve basarisiz"
            } else if code & 1 == 0 {
                "O_CLOEXEC'li tanimlayici yeni imajda ACIK"
            } else if code & 2 == 0 {
                "bayraksiz tanimlayici da KAPANDI"
            } else {
                "konum exec'te KAYBOLDU"
            },
        )
    } else {
        (false, "/boot/msg.txt acilamadi")
    };
    for x in [closing, keeping] {
        if x >= 0 {
            sys::close(x as usize);
        }
    }
    checks[7] = Check {
        name: NAMES[7],
        detail: h_detail,
        verdict: verdict(h),
    };

    // --- I, J: yazma (disk) -----------------------------------------------
    if sys::fs_total_blocks() == 0 {
        for i in 8..10 {
            checks[i] = Check {
                name: NAMES[i],
                detail: "disk bagli degil",
                verdict: Verdict::Skip,
            };
        }
    } else {
        // I: O_APPEND
        let base = open(DISK_FILE, sys::O_CREAT | sys::O_TRUNC);
        let app = open(DISK_FILE, sys::O_APPEND);
        let (i_ok, i_detail) = if base >= 0 && app >= 0 {
            sys::write(base as usize, b"aaaa");
            sys::lseek(app as usize, 0, sys::SEEK_SET);
            sys::write(app as usize, b"bb");
            let at = pos(app as usize);
            sys::lseek(base as usize, 0, sys::SEEK_SET);
            let (got, n) = take(base as usize, 16);
            let ok = n == 6 && &got[..6] == b"aaaabb" && at == 6;
            (
                ok,
                if ok {
                    "lseek(0) sonrasi yazma sona gitti: aaaabb"
                } else if n >= 2 && &got[..2] == b"bb" {
                    "yazma lseek konumuna gitti: O_APPEND YOK SAYILDI"
                } else {
                    "icerik beklenenden farkli"
                },
            )
        } else {
            (false, "diskte dosya acilamadi")
        };
        for x in [base, app] {
            if x >= 0 {
                sys::close(x as usize);
            }
        }
        checks[8] = Check {
            name: NAMES[8],
            detail: i_detail,
            verdict: verdict(i_ok),
        };

        // J: (echo cocuk; echo ebeveyn) > f
        let out = open(DISK_FILE, sys::O_CREAT | sys::O_TRUNC);
        let (j_ok, j_detail) = if out >= 0 {
            let out = out as usize;
            let child = sys::fork();
            if child == 0 {
                sys::write(out, b"cocuk\n");
                sys::exit(0);
            }
            let mut status = 0u32;
            let _ = child > 0 && sys::waitpid(child as usize, &mut status, 0) == child;
            sys::write(out, b"ebeveyn\n");
            sys::close(out);
            let rd = open(DISK_FILE, 0);
            let (got, n) = if rd >= 0 { take(rd as usize, 16) } else { ([0; 16], 0) };
            if rd >= 0 {
                sys::close(rd as usize);
            }
            let ok = n == 14 && &got[..14] == b"cocuk\nebeveyn\n";
            (
                ok,
                if ok {
                    "dosyada ikisi de, sirayla: cocuk + ebeveyn"
                } else if n >= 8 && &got[..8] == b"ebeveyn\n" {
                    "ebeveyn cocugun USTUNE yazdi: konum paylasilmiyor"
                } else {
                    "icerik beklenenden farkli"
                },
            )
        } else {
            (false, "diskte dosya acilamadi")
        };
        checks[9] = Check {
            name: NAMES[9],
            detail: j_detail,
            verdict: verdict(j_ok),
        };
    }

    let mut passed = 0;
    let mut skipped = 0;
    for check in &checks {
        let word = match check.verdict {
            Verdict::Pass => {
                passed += 1;
                "gecti"
            }
            Verdict::Fail => "KALDI",
            Verdict::Skip => {
                skipped += 1;
                "atlandi"
            }
        };
        let _ = writeln!(Stdout, "[ofd] {}: {} ({})", check.name, word, check.detail);
    }
    let _ = writeln!(
        Stdout,
        "[ofd] sonuc: {}/{} gecti, {} atlandi",
        passed,
        checks.len() - skipped,
        skipped
    );

    if let Some(w) = win.as_mut() {
        loop {
            if w.poll_key() == b'q' {
                break;
            }
            draw(w, &checks);
            w.frame(60);
        }
    }
}

fn draw(win: &mut Window, checks: &[Check; 10]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Tanimlayici ayri, acik dosya tanimi ayri", ACCENT);
    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        let (word, color) = match check.verdict {
            Verdict::Pass => ("gecti", OK),
            Verdict::Fail => ("KALDI", WARN),
            Verdict::Skip => ("atlandi", SKIP),
        };
        win.text(400, y, word, color);
        y += 16;
    }
    win.text(6, h - 14, "dup/fork: yeni numara, AYNI konum", DIM);
}
