//! `orphan` -- terminalin sahibi gidince: `SIGHUP` ve yetim gruplar.
//!
//! Is denetimi batisi terminali kimin okuyabilecegini getirdi. Eksik
//! kalan, terminalin ya da isi denetleyen tarafin **gittigi** an:
//!
//! ```text
//!   oturum lideri cikti      -> on plan grubuna SIGHUP + SIGCONT
//!   grup YETIM kaldi ve      -> gruptaki herkese SIGHUP + SIGCONT
//!   icinde durmus biri var
//!   yetim grup arka plandan  -> SIGTTIN degil, EIO
//!   terminali okudu
//!   yetim gruba SIGTSTP      -> ATILIR (SIGSTOP atilmaz)
//! ```
//!
//! Hepsinin ortak sebebi tek: bir grubu **disaridan** devam ettirebilecek
//! taraf -- ayni oturumda baska bir gruptaki ebeveyn, tipik olarak
//! kabuk -- artik yok. Durdurulan bir yetim sonsuza kadar durur.
//!
//! ## Once yeniden ebeveynleme gerekti
//!
//! TCMK'de ebeveyni olen canli bir surec `parent` alaninda olunun
//! numarasini tutmaya devam ediyordu. "Ebeveyn canli mi" sorusu o numarayla
//! cevaplanamaz: yuva yeniden kullanilinca numara **ilgisiz** yeni bir
//! sureci gosterir. Bu kendi basina bir hataydi -- yeni surec baskasinin
//! yetimini cocugu sanir, `SIGCHLD`ini alir, `waitpid(-1)` ile toplayabilir.
//! I sinavi onu olcuyor.
//!
//! ## On sinav
//!
//! ```text
//!   A  lider olunce HUP    -> terminalli lider cikti, on plan SIGHUP aldi
//!   B  ardindan CONT       -> ayni gruba SIGHUP'tan sonra SIGCONT geldi
//!   C  arka plan HUP ALMAZ -> ayni oturumdaki arka plan grubu dokunulmadi
//!   D  yetim+durmus grup   -> ebeveyn cikinca durmus cocuk HUP+CONT aldi
//!   E  yetim okuma EIO     -> arka plandaki yetim grup terminali okuyunca
//!                             EIO (SIGTTIN ile durmadi)
//!   F  yetim SIGTSTP ATILIR-> yetim grupta SIGTSTP sureci durdurmadi
//!   G  SIGSTOP atilmaz     -> yetim grupta SIGSTOP yine durdurdu
//!   H  yetim degilse durur -> ebeveyni canli bir grupta SIGTSTP durdurdu
//!   I  yetim baskasina     -> yetimin ebeveyni 0; olunun yuvasini alan yeni
//!      gecmez                 surec onu cocugu sanmiyor
//!   J  zaten yetim HUP     -> ZATEN yetim olan grupta bir uye cikinca
//!      ALMAZ                  durmus uye yeniden SIGHUP almadi
//! ```
//!
//! F'nin anlami ancak G ve H ile birlikte belli oluyor: F tek basina "hic
//! durdurulmadi"yi da gecerdi. G (SIGSTOP yine durdurur) ve H (yetim
//! olmayan grupta SIGTSTP durdurur) atmanin **secici** oldugunu gosteriyor.
//! D ile J de bir cift: D "yeni yetim kalan gruba HUP"u olcuyor, J ise
//! "yeni" kosulunu -- yoksa yetim bir grubun her cikisinda kalan durmus
//! uyeler yeniden HUP alirdi.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::signal;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0018_1416;
const PANEL: u32 = 0x0028_201E;
const FG: u32 = 0x00EE_E6E2;
const DIM: u32 = 0x009E_928C;
const ACCENT: u32 = 0x00FF_A070;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

const ECHILD: isize = 10;
const EIO: isize = 5;

/// Torunlarin cevap yazdigi boru (yazma ucu).
static PIPE_W: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Isleyicilerin yazacagi bayt (`SIGHUP` / `SIGCONT` icin ayri).
static HUP_BYTE: AtomicUsize = AtomicUsize::new(0);
static CONT_BYTE: AtomicUsize = AtomicUsize::new(0);

fn send(byte: u8) {
    let w = PIPE_W.load(Ordering::SeqCst);
    if w != usize::MAX {
        sys::write(w, &[byte]);
    }
}

extern "C" fn on_hup(_signo: u32) {
    let b = HUP_BYTE.load(Ordering::SeqCst) as u8;
    if b != 0 {
        send(b);
    }
}

extern "C" fn on_cont(_signo: u32) {
    let b = CONT_BYTE.load(Ordering::SeqCst) as u8;
    if b != 0 {
        send(b);
    }
}

/// `SIGHUP`/`SIGCONT` icin yazacak baytlari kurar.
fn listen(hup: u8, cont: u8) {
    HUP_BYTE.store(hup as usize, Ordering::SeqCst);
    CONT_BYTE.store(cont as usize, Ordering::SeqCst);
    signal::install(signal::SIGHUP, on_hup);
    if cont != 0 {
        signal::install(signal::SIGCONT, on_cont);
    }
}

/// Ebeveyn olene kadar bekler (`getppid` 0). Doner: oldu mu.
fn wait_orphaned(ms: usize) -> bool {
    for _ in 0..(ms / 20).max(1) {
        if sys::getppid() == 0 {
            return true;
        }
        sys::sleep_ms(20);
    }
    false
}

/// Borudan `ms` boyunca gelen baytlari toplar.
struct Bytes {
    buf: [u8; 32],
    len: usize,
}

impl Bytes {
    fn has(&self, b: u8) -> bool {
        self.buf[..self.len].contains(&b)
    }
    fn index(&self, b: u8) -> Option<usize> {
        self.buf[..self.len].iter().position(|&x| x == b)
    }
}

fn collect(fd: usize, ms: usize) -> Bytes {
    let mut out = Bytes {
        buf: [0; 32],
        len: 0,
    };
    let start = sys::ticks();
    let limit = ms / 10;
    while sys::ticks().saturating_sub(start) < limit && out.len < out.buf.len() {
        let mut watch = [sys::PollFd::new(fd, sys::POLLIN)];
        if sys::poll(&mut watch, 20) > 0 && watch[0].ready(sys::POLLIN) {
            let mut b = [0u8; 1];
            if sys::read(fd, &mut b) == 1 {
                out.buf[out.len] = b[0];
                out.len += 1;
            }
        }
    }
    out
}

/// Dogrudan cocugun cikis kodunu sinirli sure bekler.
fn reap(child: usize, ms: usize) -> Option<u32> {
    let mut status = 0u32;
    for _ in 0..(ms / 20).max(1) {
        if sys::waitpid(child, &mut status, sys::WNOHANG) > 0 {
            return if sys::exited(status) {
                Some(sys::exit_status(status))
            } else {
                None
            };
        }
        sys::sleep_ms(20);
    }
    signal::kill(child, signal::SIGKILL);
    sys::waitpid(child, &mut status, 0);
    None
}

/// Kalan torunlari temizler (cikis kodlarindan gelen numaralar).
fn sweep(pids: &[usize]) {
    for &p in pids {
        if p != 0 && p < 64 {
            signal::kill(p, signal::SIGKILL);
        }
    }
}

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

const NAMES: [&str; 10] = [
    "A lider olunce HUP",
    "B ardindan CONT",
    "C arka plan HUP ALMAZ",
    "D yetim+durmus grup",
    "E yetim okuma EIO",
    "F yetim SIGTSTP ATILIR",
    "G SIGSTOP atilmaz",
    "H yetim degilse durur",
    "I yetim baskasina gecmez",
    "J zaten yetim HUP ALMAZ",
];

fn say(check: &Check) {
    use core::fmt::Write;
    let _ = writeln!(
        Stdout,
        "[orphan] {}: {} ({})",
        check.name,
        if check.passed { "gecti" } else { "KALDI" },
        check.detail
    );
}

fn main() {
    use core::fmt::Write;
    let _ = writeln!(Stdout, "[orphan] sinav basliyor");
    let mut checks = [EMPTY; 10];

    // Pencere ilk is: E icin bu surecin terminali olmali (oturumun
    // terminali liderin penceresi).
    let mut win = Window::open("orphan -- terminalin sahibi gidince", 260, 120, 480, 250);

    let Some((pipe_r, pipe_w)) = sys::pipe() else {
        let _ = writeln!(Stdout, "[orphan] boru acilamadi");
        return;
    };
    PIPE_W.store(pipe_w, Ordering::SeqCst);

    // --- A, B, C: oturum lideri cikiyor --------------------------------
    //
    // L yeni bir oturum kuruyor ve **kendi penceresini** aciyor -- o
    // pencere L'nin oturumunun terminali. Iki torun: G L'nin grubunda (on
    // plan), G2 kendi grubunda (arka plan). L cikinca yalnizca G'nin
    // grubu `SIGHUP` almali.
    let l = sys::fork();
    if l == 0 {
        signal::setsid();
        let _term = Window::open("orphan L", 20, 20, 120, 60);
        let g = sys::fork();
        if g == 0 {
            listen(b'H', b'C');
            sys::sleep_ms(1500);
            sys::exit(0);
        }
        let g2 = sys::fork();
        if g2 == 0 {
            signal::setpgid(0, 0);
            listen(b'X', 0);
            sys::sleep_ms(1200);
            send(b'q');
            sys::exit(0);
        }
        signal::setpgid(g2 as usize, g2 as usize);
        sys::sleep_ms(300);
        sys::exit((g as i32 & 0xF) | ((g2 as i32 & 0xF) << 4));
    }
    let l_code = if l > 0 { reap(l as usize, 3000) } else { None };
    let abc = collect(pipe_r, 1600);
    if let Some(code) = l_code {
        sweep(&[(code & 0xF) as usize, ((code >> 4) & 0xF) as usize]);
    }

    let a = abc.has(b'H');
    checks[0] = Check {
        name: NAMES[0],
        detail: if a {
            "lider cikti, on plandaki torun SIGHUP aldi"
        } else if l_code.is_none() {
            "lider cevap vermedi"
        } else {
            "on plan grubu SIGHUP ALMADI"
        },
        passed: a,
    };
    say(&checks[0]);
    let b = matches!((abc.index(b'H'), abc.index(b'C')), (Some(h), Some(c)) if h < c);
    checks[1] = Check {
        name: NAMES[1],
        detail: if b {
            "SIGHUP'tan sonra SIGCONT geldi"
        } else if !abc.has(b'C') {
            "SIGCONT GELMEDI"
        } else {
            "sira yanlis: SIGCONT once geldi"
        },
        passed: b,
    };
    say(&checks[1]);
    let c = abc.has(b'q') && !abc.has(b'X');
    checks[2] = Check {
        name: NAMES[2],
        detail: if c {
            "arka plan grubu sinyal almadi, kendi suresince yasadi"
        } else if abc.has(b'X') {
            "arka plan grubu da SIGHUP ALDI"
        } else {
            "arka plan torunu cevap vermedi"
        },
        passed: c,
    };
    say(&checks[2]);

    // --- D: yetim kalan ve icinde durmus biri olan grup ----------------
    //
    // P kendi grubunu kuruyor; cocugu C ayni grupta ve kendini
    // durduruyor. P cikinca grubu disaridan devam ettirebilecek kimse
    // kalmiyor: grup yetim ve icinde durmus bir surec var. Cekirdek
    // gruba `SIGHUP` ve `SIGCONT` gondermeli.
    let p = sys::fork();
    if p == 0 {
        signal::setpgid(0, 0);
        let child = sys::fork();
        if child == 0 {
            listen(b'h', b'c');
            signal::kill(signal::getpid(), signal::SIGSTOP);
            // Buraya ancak devam ettirilince gelinir.
            sys::sleep_ms(300);
            sys::exit(0);
        }
        // Cocuk gercekten durana kadar bekle -- durmadan cikmak olcuyu
        // bosa cikarirdi (grup yetim kalir ama durmus kimse yoktur).
        let mut status = 0u32;
        for _ in 0..50 {
            if sys::waitpid(child as usize, &mut status, sys::WNOHANG | sys::WUNTRACED) > 0 {
                break;
            }
            sys::sleep_ms(20);
        }
        sys::exit(child as i32 & 0xFF);
    }
    if p > 0 {
        signal::setpgid(p as usize, p as usize);
    }
    let p_code = if p > 0 { reap(p as usize, 3000) } else { None };
    let d_bytes = collect(pipe_r, 800);
    if let Some(code) = p_code {
        sweep(&[code as usize]);
    }
    let d = matches!((d_bytes.index(b'h'), d_bytes.index(b'c')), (Some(h), Some(c)) if h < c);
    checks[3] = Check {
        name: NAMES[3],
        detail: if d {
            "ebeveyn cikti, durmus cocuk SIGHUP sonra SIGCONT aldi"
        } else if d_bytes.has(b'h') {
            "SIGHUP geldi ama SIGCONT gelmedi"
        } else {
            "yetim kalan durmus grup SINYAL ALMADI: sonsuza kadar durur"
        },
        passed: d,
    };
    say(&checks[3]);

    // --- E, F, G: yetim grupta terminal ve durdurma --------------------
    //
    // Uc senaryo ayni kalip: ara surec kendi grubunu kuruyor, cocugunu
    // birakip cikiyor; cocuk ebeveyninin oldugunu gordukten sonra
    // (`getppid` 0) olcuyu yapiyor.
    let orphan_case = |role: u8| -> (Option<u32>, isize) {
        let mid = sys::fork();
        if mid == 0 {
            signal::setpgid(0, 0);
            let child = sys::fork();
            if child == 0 {
                if !wait_orphaned(1500) {
                    send(b'?');
                    sys::exit(0);
                }
                match role {
                    b'E' => {
                        let mut buf = [0u8; 4];
                        sys::set_flags(sys::STDIN, sys::O_NONBLOCK);
                        let r = sys::read(sys::STDIN, &mut buf);
                        send(if r == -EIO { b'E' } else { b'e' });
                    }
                    b'F' => {
                        let before = sys::ticks();
                        signal::kill(signal::getpid(), signal::SIGTSTP);
                        let waited = sys::ticks().saturating_sub(before);
                        send(if waited < 10 { b'F' } else { b'f' });
                    }
                    _ => {
                        let before = sys::ticks();
                        signal::kill(signal::getpid(), signal::SIGSTOP);
                        let waited = sys::ticks().saturating_sub(before);
                        send(if waited >= 20 { b'G' } else { b'g' });
                    }
                }
                sys::exit(0);
            }
            sys::exit(child as i32 & 0xFF);
        }
        if mid > 0 {
            signal::setpgid(mid as usize, mid as usize);
        }
        let code = if mid > 0 { reap(mid as usize, 3000) } else { None };
        (code, mid)
    };

    let (e_code, _) = orphan_case(b'E');
    let e_bytes = collect(pipe_r, 800);
    if let Some(code) = e_code {
        sweep(&[code as usize]);
    }
    let e = e_bytes.has(b'E');
    checks[4] = Check {
        name: NAMES[4],
        detail: if e {
            "yetim grubun okumasi EIO aldi, durdurulmadi"
        } else if e_bytes.has(b'e') {
            "yetim grup okudu ama EIO ALMADI"
        } else if e_bytes.has(b'?') {
            "torun yetim kalmadi: getppid 0 olmadi"
        } else {
            "torun cevap vermedi: SIGTTIN ile DURDURULMUS olabilir"
        },
        passed: e,
    };
    say(&checks[4]);

    let (f_code, _) = orphan_case(b'F');
    let f_bytes = collect(pipe_r, 800);
    if let Some(code) = f_code {
        sweep(&[code as usize]);
    }
    let f = f_bytes.has(b'F');
    checks[5] = Check {
        name: NAMES[5],
        detail: if f {
            "yetim grupta SIGTSTP atildi, surec durmadi"
        } else if f_bytes.has(b'?') {
            "torun yetim kalmadi: getppid 0 olmadi"
        } else {
            "yetim surec SIGTSTP ile DURDU: kimse devam ettiremez"
        },
        passed: f,
    };
    say(&checks[5]);

    // G: SIGSTOP yine durdurmali. Torun durunca sinav onu disaridan
    // devam ettiriyor -- numarasi ara surecin cikis kodunda.
    let (g_code, _) = orphan_case(b'G');
    let mut g_stopped_pid = 0usize;
    if let Some(code) = g_code {
        g_stopped_pid = code as usize;
        sys::sleep_ms(300);
        signal::kill(g_stopped_pid, signal::SIGCONT);
    }
    let g_bytes = collect(pipe_r, 800);
    sweep(&[g_stopped_pid]);
    let g = g_bytes.has(b'G');
    checks[6] = Check {
        name: NAMES[6],
        detail: if g {
            "yetim grupta SIGSTOP yine durdurdu, SIGCONT ile surdu"
        } else if g_bytes.has(b'g') {
            "SIGSTOP de ATILDI: atma secici degil"
        } else {
            "torun cevap vermedi"
        },
        passed: g,
    };
    say(&checks[6]);

    // --- H: yetim olmayan grupta SIGTSTP durdurur ----------------------
    let q = sys::fork();
    if q == 0 {
        signal::setpgid(0, 0);
        signal::kill(signal::getpid(), signal::SIGTSTP);
        sys::exit(0);
    }
    let mut h_sig = 0u32;
    if q > 0 {
        signal::setpgid(q as usize, q as usize);
        let mut status = 0u32;
        for _ in 0..100 {
            if sys::waitpid(q as usize, &mut status, sys::WNOHANG | sys::WUNTRACED) > 0 {
                if sys::stopped(status) {
                    h_sig = sys::stop_signal(status);
                }
                break;
            }
            sys::sleep_ms(20);
        }
        signal::kill(q as usize, signal::SIGKILL);
        sys::waitpid(q as usize, &mut status, 0);
    }
    let h = h_sig == signal::SIGTSTP;
    checks[7] = Check {
        name: NAMES[7],
        detail: if h {
            "ebeveyni canli grupta SIGTSTP durdurdu"
        } else {
            "SIGTSTP yetim OLMAYAN grupta da atildi"
        },
        passed: h,
    };
    say(&checks[7]);

    // --- I: yetim baskasina gecmez -------------------------------------
    //
    // P3 cocugu C3'u birakip cikiyor. C3 ebeveyninin `getppid` ile 0
    // oldugunu bildiriyor. Sonra sinav yeni bir N catalliyor: TCMK ilk bos
    // yuvayi kullandigi icin N buyuk olasilikla P3'un yuvasini aliyor.
    // N'nin hicbir cocugu yok; `waitpid(-1)` `ECHILD` demeli. Eski halde
    // C3'un `parent` alani hala o numarayi gosterdigi icin N, C3'u kendi
    // cocugu sanardi.
    let p3 = sys::fork();
    if p3 == 0 {
        let c3 = sys::fork();
        if c3 == 0 {
            if wait_orphaned(1000) {
                send(b'P');
            } else {
                send(b'p');
            }
            sys::sleep_ms(1200);
            sys::exit(0x5C);
        }
        sys::exit(c3 as i32 & 0xFF);
    }
    let c3 = if p3 > 0 { reap(p3 as usize, 2000) } else { None };
    sys::sleep_ms(100);
    let n = sys::fork();
    if n == 0 {
        let mut status = 0u32;
        let r = sys::waitpid(sys::WAIT_ANY, &mut status, sys::WNOHANG);
        sys::exit(if r == -ECHILD { 1 } else { 2 });
    }
    let n_code = if n > 0 { reap(n as usize, 2000) } else { None };
    let i_bytes = collect(pipe_r, 400);
    if let Some(code) = c3 {
        sweep(&[code as usize]);
    }
    let same_slot = n > 0 && n == p3;
    let i = i_bytes.has(b'P') && n_code == Some(1);
    checks[8] = Check {
        name: NAMES[8],
        detail: if i && same_slot {
            "yetimin ebeveyni 0; olunun yuvasini alan surecin cocugu yok"
        } else if i {
            "yetimin ebeveyni 0; yeni surec yuvayi almadi (kismi olcu)"
        } else if !i_bytes.has(b'P') {
            "yetimin getppid'i 0 OLMADI: olu ebeveyn numarasi duruyor"
        } else {
            "yeni surec baskasinin yetimini COCUGU SANDI"
        },
        passed: i,
    };
    say(&checks[8]);

    // --- J: zaten yetim grup yeniden HUP almaz -------------------------
    //
    // M kendi grubunu kuruyor, iki cocuk birakip cikiyor: grup yetim
    // kaliyor ama o an durmus kimse yok, sinyal gitmiyor. Sonra S kendini
    // durduruyor (`SIGSTOP` yetim grupta da durdurur, G) ve X cikiyor.
    // Grup X'in cikisindan **once** de yetimdi; "yeni yetim" degil, S
    // `SIGHUP` almamali. Olcu ancak S durmus ve X cikmissa anlamli: ikisi
    // de boruya ayrica haber veriyor.
    let m = sys::fork();
    if m == 0 {
        signal::setpgid(0, 0);
        let s_pid = sys::fork();
        if s_pid == 0 {
            listen(b'j', b'k');
            if wait_orphaned(1500) {
                send(b's');
                signal::kill(signal::getpid(), signal::SIGSTOP);
            }
            sys::sleep_ms(1500);
            sys::exit(0);
        }
        let x = sys::fork();
        if x == 0 {
            if wait_orphaned(1500) {
                sys::sleep_ms(300);
                send(b'x');
            }
            sys::exit(0);
        }
        sys::exit((s_pid as i32 & 0xF) | ((x as i32 & 0xF) << 4));
    }
    if m > 0 {
        signal::setpgid(m as usize, m as usize);
    }
    let m_code = if m > 0 { reap(m as usize, 2000) } else { None };
    let j_bytes = collect(pipe_r, 1200);
    if let Some(code) = m_code {
        sweep(&[(code & 0xF) as usize, ((code >> 4) & 0xF) as usize]);
    }
    let measured = j_bytes.has(b's') && j_bytes.has(b'x');
    let j = measured && !j_bytes.has(b'j');
    checks[9] = Check {
        name: NAMES[9],
        detail: if j {
            "uye cikti, zaten yetim grubun durmus uyesi sinyal almadi"
        } else if !measured {
            "olcu kurulamadi: uye durmadi ya da diger uye cikmadi"
        } else {
            "zaten yetim grup her cikista YENIDEN SIGHUP ALIYOR"
        },
        passed: j,
    };
    say(&checks[9]);

    let passed = checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(Stdout, "[orphan] sonuc: {}/10 gecti", passed);

    if let Some(w) = win.as_mut() {
        show(w, &checks);
    }
}

fn show(win: &mut Window, checks: &[Check; 10]) {
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(win, checks);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 10]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Denetleyen gidince: SIGHUP, yetim gruplar", ACCENT);
    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            400,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }
    win.text(6, h - 30, "yetim = disaridan devam ettirecek kimse yok", DIM);
    let passed = checks.iter().filter(|c| c.passed).count();
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
