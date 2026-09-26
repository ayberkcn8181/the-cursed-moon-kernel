//! `jobs` -- is denetimi: durdurma, devam ettirme, surec gruplari.
//!
//! POSIX'te bir surec olmenin disinda **durabilir** de. Uzun sure
//! TCMK'de bu kavram yoktu: `waitpid` yalnizca olumu bekliyor,
//! `SIGSTOP` diye bir sinyal bulunmuyordu. Bir kabuk yazilamazdi --
//! Ctrl-Z'nin karsiligi yoktu.
//!
//! ## Iki kavram, surekli karistirilan
//!
//! ```text
//!   is parcacigi grubu (TGID)  ayni surecin akislari -- bellek/fd paylasir
//!   surec grubu        (PGID)  kabugun ayni IS olarak gordugu surecler
//! ```
//!
//! Bir boru hatti (`a | b | c`) uc ayri surec, uc ayri TGID -- ama tek
//! bir PGID. Ctrl-C ucunu birden bitirebilsin diye. TCMK'de ikisi ayri
//! alanlar (`Task.group` ve `Task.pgid`) ve karistirilmalari kolay
//! oldugu icin aciktan ayri tutuluyorlar.
//!
//! ## Durum kelimesinin dorduncu hali
//!
//! `waitpid`in durum kelimesi artik dort seyi birden kodluyor ve
//! hepsi ayni 16 bite siginiyor:
//!
//! ```text
//!   normal cikis  -> (kod & 0xFF) << 8      WIFEXITED
//!   sinyalle olum ->  signo & 0x7F          WIFSIGNALED
//!   durduruldu    -> (signo << 8) | 0x7F    WIFSTOPPED
//!   devam etti    ->  0xFFFF                WIFCONTINUED
//! ```
//!
//! `0x7F` alt bayti "olumle gitmedi, durdu" demenin yolu -- gecerli bir
//! sinyal numarasi olmadigi icin ayirt edilebiliyor. Tasarim
//! 1970'lerden kalma ve hala calisiyor.
//!
//! ## Alti sinav
//!
//! ```text
//!   A  durdu          -> SIGSTOP sonrasi cocuk ilerlemiyor
//!   B  waitpid bildirir-> WUNTRACED ile WIFSTOPPED, WSTOPSIG = 19
//!   C  devam etti     -> SIGCONT sonrasi cocuk yeniden ilerliyor
//!   D  yakalanamaz    -> SIGSTOP'a isleyici kurulamiyor (SIGTSTP'ye kuruluyor)
//!   E  grup           -> kill(-pgid) gruptaki IKI cocugu da durduruyor
//!   F  ayrim          -> durmus cocuk WIFSIGNALED DEGIL
//! ```
//!
//! A'nin olcusu "ilerlemiyor". Yalnizca `waitpid`in "durdu" demesine
//! bakmak yetmezdi: cekirdek bayragi kurup gorevi gercekten
//! durdurmasaydi sinav yine gecerdi. Boru uzerinden gelen baytlarin
//! **kesilmesi** durmanin kendisini olcuyor.
//!
//! F, bu batiyi yazarken gercekten yapilan bir hatanin sinavi.
//! `WIFSIGNALED` uzun sure `status & 0x7F != 0` diye denetleniyordu ve
//! dogruydu -- durma kavrami gelene kadar. Durmus bir cocukta alt bayt
//! `0x7F`tir, yani o denetim "sinyalle oldu" der. Gercek POSIX de tam
//! bu yuzden `0x7F`i disliyor.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::signal;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0014_1A24;
const PANEL: u32 = 0x0022_2C38;
const FG: u32 = 0x00E4_EAF2;
const DIM: u32 = 0x008C_98A8;
const ACCENT: u32 = 0x0080_C0F0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Cocugun iki yazma arasinda bekledigi sure.
const TICK_MS: usize = 40;
/// Durmanin olculdugu pencere -- cocuk kosuyor olsaydi bu surede en az
/// dort bayt gelirdi.
const QUIET_MS: isize = 250;
/// Devam ettikten sonra bayt beklenen en uzun sure.
const RESUME_MS: isize = 600;

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

/// A/B/C cocugu: surekli bayt yazar.
fn ticker(write_end: usize) -> ! {
    loop {
        sys::write(write_end, b"x");
        sys::sleep_ms(TICK_MS);
    }
}

/// E cocugu: yalnizca yasar.
fn idler() -> ! {
    loop {
        sys::sleep_ms(50);
    }
}

/// Boruda bekleyen ne varsa tuketir (bloke etmeden).
fn drain(read_end: usize) -> usize {
    let mut total = 0usize;
    let mut buf = [0u8; 64];
    loop {
        let n = sys::read(read_end, &mut buf);
        if n <= 0 {
            return total;
        }
        total += n as usize;
    }
}

/// `read_end`te `timeout` icinde veri var mi.
fn waits_for_data(read_end: usize, timeout: isize) -> bool {
    let mut watch = [sys::PollFd::new(read_end, sys::POLLIN)];
    sys::poll(&mut watch, timeout) > 0 && watch[0].ready(sys::POLLIN)
}

/// Bir surec grubunu bekler (`waitpid(-pgid, ...)`).
fn waitpid_group(pgid: usize, status: &mut u32, options: usize) -> isize {
    let target = -(pgid as isize);
    sys::waitpid(target as usize, status, options)
}

/// Gruptan bir **durma** bildirimi bekler; bulursa duran cocugun
/// kimligini doner.
///
/// Zaman asimli olmasi sart ve bunu olcum ogretti. Ilk hali bloke eden
/// bir `waitpid` kullaniyordu; bilerek bozulmus bir cekirdekte (yayin
/// yalnizca ilk uyeye gidiyor) ikinci cocuk hic durmuyor ve sinav
/// **asili kaliyordu**. Kalan bir sinav, asili kalan bir sinavdan
/// iyidir: biri sebebi yazar, oteki yalnizca sessizligi.
fn wait_group_stop(pgid: usize, tries: usize) -> Option<usize> {
    for _ in 0..tries {
        let mut status = 0u32;
        let pid = waitpid_group(pgid, &mut status, sys::WUNTRACED | sys::WNOHANG);
        if pid > 0 && sys::stopped(status) {
            return Some(pid as usize);
        }
        if pid < 0 {
            // Cocuk kalmadi.
            return None;
        }
        sys::sleep_ms(25);
    }
    None
}

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;
    let mut checks = [EMPTY; 6];

    // --- A + B + C + F: tek cocuk, boru uzerinden olculen ilerleme ---
    let mut stop_status = 0u32;
    let (a, b, c, f) = match sys::pipe2(sys::O_NONBLOCK) {
        Some((read_end, write_end)) => match sys::fork() {
            0 => {
                sys::close(read_end);
                ticker(write_end);
            }
            child if child > 0 => {
                let child = child as usize;
                // Cocuk gercekten kosuyor mu -- durmadan once bunu
                // bilmek sart, yoksa "durdu" sonucu bos bir gozlem olur.
                let running = waits_for_data(read_end, RESUME_MS);
                drain(read_end);

                signal::kill(child, signal::SIGSTOP);
                // Durdurma anindan hemen once yazilmis olabilecek
                // baytlari at; olculen sey BUNDAN SONRASI.
                sys::sleep_ms(TICK_MS * 2);
                drain(read_end);
                let quiet = !waits_for_data(read_end, QUIET_MS);

                let reported =
                    sys::waitpid(child, &mut stop_status, sys::WUNTRACED) == child as isize;
                let b = reported
                    && sys::stopped(stop_status)
                    && sys::stop_signal(stop_status) == signal::SIGSTOP;
                // Durmus cocuk "sinyalle oldu" gorunmemeli.
                let f = reported && sys::stopped(stop_status) && !sys::signalled(stop_status);

                signal::kill(child, signal::SIGCONT);
                let resumed = waits_for_data(read_end, RESUME_MS);

                signal::kill(child, signal::SIGKILL);
                let mut gone = 0u32;
                sys::waitpid(child, &mut gone, 0);
                sys::close(read_end);
                sys::close(write_end);
                (running && quiet, b, resumed, f)
            }
            _ => (false, false, false, false),
        },
        None => (false, false, false, false),
    };

    checks[0] = Check {
        name: "A durdu",
        detail: if a {
            "SIGSTOP sonrasi cocuk ilerlemedi"
        } else {
            "cocuk durdurulamadi ya da hic kosmadi"
        },
        passed: a,
    };
    checks[1] = Check {
        name: "B waitpid bildirir",
        detail: if b {
            "WIFSTOPPED dogru, WSTOPSIG 19"
        } else if sys::stopped(stop_status) {
            "durdu ama yanlis sinyal"
        } else {
            "WUNTRACED durmayi BILDIRMEDI"
        },
        passed: b,
    };
    checks[2] = Check {
        name: "C devam etti",
        detail: if c {
            "SIGCONT sonrasi cocuk yeniden ilerledi"
        } else {
            "SIGCONT cocugu KALDIRMADI"
        },
        passed: c,
    };

    // --- D: SIGSTOP yakalanamaz ---
    //
    // Reddin **secici** oldugunu gostermek icin SIGTSTP de deneniyor:
    // o yakalanabilir olmali, yoksa sinav "isleyici kurma hic
    // calismiyor" ile "SIGSTOP korunuyor"u ayirt edemezdi.
    let stop_rejected = signal::install(signal::SIGSTOP, on_signal) < 0;
    let tstp_ok = signal::install(signal::SIGTSTP, on_signal) >= 0;
    let d = stop_rejected && tstp_ok;
    checks[3] = Check {
        name: "D yakalanamaz",
        detail: if !stop_rejected {
            "SIGSTOP'a isleyici KURULDU"
        } else if !tstp_ok {
            "SIGTSTP de reddedildi (ayrim yok)"
        } else {
            "SIGSTOP reddedildi, SIGTSTP kabul edildi"
        },
        passed: d,
    };

    // --- E: surec grubu ---
    //
    // Iki cocuk tek gruba aliniyor ve **tek** bir cagri ikisini birden
    // durduruyor. Kabugun Ctrl-Z'si tam olarak budur.
    let mut group_size = 0usize;
    let e = match (sys::fork(), 0) {
        (0, _) => idler(),
        (first, _) if first > 0 => {
            let first = first as usize;
            match sys::fork() {
                0 => idler(),
                second if second > 0 => {
                    let second = second as usize;
                    // Ikisini de birincinin grubuna al.
                    signal::setpgid(first, first);
                    signal::setpgid(second, first);
                    let grouped = signal::getpgid(first) == first as isize
                        && signal::getpgid(second) == first as isize;

                    // Tek cagri, iki hedef.
                    signal::kill_group(first, signal::SIGSTOP);

                    let mut stopped = 0usize;
                    let mut seen = [usize::MAX; 2];
                    for slot in 0..2 {
                        if let Some(pid) = wait_group_stop(first, 40) {
                            seen[slot] = pid;
                            stopped += 1;
                        }
                    }
                    group_size = stopped;
                    let distinct = seen[0] != seen[1];

                    // Temizlik **tek tek** yapiliyor, grup uzerinden
                    // degil. Sebebi olcum: sinanan sey yayinin kendisi,
                    // yani bozuk bir cekirdekte `kill_group` yalnizca
                    // bir cocuga ulasir -- ve grup uzerinden temizleyen
                    // ilk hal, hayatta kalan ikinci cocugu beklerken
                    // sinavi ASILI birakiyordu. Temizlik, sinanan seye
                    // bagli olmamali.
                    signal::kill(first, signal::SIGCONT);
                    signal::kill(second, signal::SIGCONT);
                    signal::kill(first, signal::SIGKILL);
                    signal::kill(second, signal::SIGKILL);
                    let mut gone = 0u32;
                    sys::waitpid(first, &mut gone, 0);
                    sys::waitpid(second, &mut gone, 0);
                    grouped && stopped == 2 && distinct
                }
                _ => false,
            }
        }
        _ => false,
    };
    checks[4] = Check {
        name: "E grup",
        detail: if e {
            "tek cagri gruptaki iki cocugu da durdurdu"
        } else if group_size == 1 {
            "yalnizca BIR cocuk durdu (yayin yok)"
        } else if group_size == 0 {
            "gruptaki hicbir cocuk durmadi"
        } else {
            "grup kurulamadi"
        },
        passed: e,
    };

    checks[5] = Check {
        name: "F ayrim",
        detail: if f {
            "durmus cocuk WIFSIGNALED degil"
        } else {
            "durma SINYALLE OLUM gibi gorundu"
        },
        passed: f,
    };

    for check in &checks {
        let _ = writeln!(
            out,
            "[jobs] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let _ = writeln!(
        out,
        "[jobs] durum kelimesi: 0x{:04x}  kendi pgid: {}",
        stop_status,
        signal::getpgid(0)
    );

    let mut win = match Window::open("jobs -- is denetimi", 255, 165, 470, 190) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks, stop_status);
        win.frame(30);
    }
}

extern "C" fn on_signal(_signo: u32) {}

fn draw(win: &mut Window, checks: &[Check; 6], status: u32) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "olum degil: surec DURABILIR de", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            365,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, "durum kelimesi:", DIM);
    win.number(150, h - 30, status as usize, FG);
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
