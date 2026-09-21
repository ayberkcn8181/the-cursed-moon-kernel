//! `blocking` -- borudan okumak artik **bekliyor**.
//!
//! Bu, TCMK'nin uzun sure tasidigi ve README'de acikca yazili bir
//! sadelestirmenin sonu: "boru okumasi bloke etmez, veri yoksa 0 doner."
//! Gerekcesi GUI idi -- bloke olan bir surec penceresini de dondururdu.
//!
//! Ama sadelestirme bir **hata** uretiyordu, cunku iki ayri durumu ayni
//! sayiyla bildiriyordu:
//!
//! ```text
//!   veri yok, yazan var  -> "simdilik yok"       \  ikisi de 0
//!   veri yok, yazan yok  -> "bir daha gelmeyecek" /
//! ```
//!
//! POSIX'te ikincisi **dosya sonu**dur ve okuyan taraf oradan cikar.
//! Birincisini de 0 dondurmek, gercek bir Linux ikilisini erken
//! cikmaya ikna etmek demekti. Artik ilki bekliyor, ikincisi 0 donuyor.
//!
//! ## Bloke olmamak hala mumkun -- ama acikca istenerek
//!
//! POSIX'in tercihi dikkat cekici: **varsayilan beklemektir**, bloke
//! olmamak icin `O_NONBLOCK` koymak gerekir. Ve o zaman bos boru 0
//! degil `-EAGAIN` doner -- yani "sonra tekrar dene", dosya sonu degil.
//! Uc durum, uc ayri cevap.
//!
//! ## Bes sinav
//!
//! ```text
//!   A  bekleme       -> bos borudan okumak kardes yazana kadar BEKLER
//!   B  dosya sonu    -> yazan uc kapaninca okuma 0 doner (beklemez)
//!   C  O_NONBLOCK    -> bos boruda -EAGAIN, bekleme YOK
//!   D  uc durum ayri -> EAGAIN ile 0 ayni sey DEGIL
//!   E  pipe2         -> bayrak yaratma aninda konabiliyor
//! ```
//!
//! D sinavi asil meselenin kendisi: C ile B'nin ayni cagriyi ayni bos
//! boruda farkli cevaplar vermesi. Ayni sayiyi dondurselerdi sadeleşme
//! surerdi.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0014_1A22;
const PANEL: u32 = 0x0020_2C38;
const FG: u32 = 0x00E0_E8F0;
const DIM: u32 = 0x0084_94A4;
const ACCENT: u32 = 0x0068_C8E0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// A sinavinin kardesinin yazacagi damga.
const MARK: &[u8] = b"gec";

/// Kardesin yazmadan once bekleyecegi sure. Ana akisin gercekten
/// **uyumus** olmasi icin yeterince uzun.
const DELAY_MS: usize = 150;

/// A sinavinin borusunun yazma ucu -- kardes buradan yazacak.
static WRITE_END: AtomicUsize = AtomicUsize::new(usize::MAX);

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

/// A sinavinin kardesi: bekleyip yazar.
extern "C" fn late_writer(_param: usize) -> usize {
    sys::sleep_ms(DELAY_MS);
    let fd = WRITE_END.load(Ordering::SeqCst);
    if fd != usize::MAX {
        sys::write(fd, MARK);
    }
    0
}

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;
    let mut checks = [EMPTY; 5];

    // --- A: bekleme ---
    //
    // Boru bos; kardes 150 ms sonra yazacak. Okuma beklemezse hemen 0
    // donerdi ve gecen sure sifira yakin olurdu.
    let mut waited_ms = 0usize;
    let a = match sys::pipe() {
        Some((read_end, write_end)) => {
            WRITE_END.store(write_end, Ordering::SeqCst);
            let helper = sys::clone_thread(late_writer, 0, 0);
            let started = sys::ticks();
            let mut buf = [0u8; 8];
            let read = sys::read(read_end, &mut buf);
            // Tik cozunurlugu 10 ms; 150 ms en az 12 tik etmeli.
            waited_ms = sys::ticks().wrapping_sub(started) * 10;
            let ok = helper > 0 && read == MARK.len() as isize && &buf[..MARK.len()] == MARK;
            sys::close(read_end);
            sys::close(write_end);
            ok && waited_ms >= 100
        }
        None => false,
    };
    checks[0] = Check {
        name: "A bekleme",
        detail: if !a && waited_ms < 100 {
            "beklemeden dondu (bloke etmiyor)"
        } else if a {
            "kardes yazana kadar beklendi"
        } else {
            "okunan veri yanlis"
        },
        passed: a,
    };

    // --- B: dosya sonu ---
    //
    // Yazan ucu **kapatiyoruz**. Bu, "bir daha veri gelmeyecek"
    // demektir ve okuma beklemeden 0 donmeli. Beklerse program burada
    // sonsuza kadar asilirdi -- sinavin kendisi de o yuzden degerli.
    let b = match sys::pipe() {
        Some((read_end, write_end)) => {
            sys::close(write_end);
            let started = sys::ticks();
            let mut buf = [0u8; 8];
            let read = sys::read(read_end, &mut buf);
            let instant = sys::ticks().wrapping_sub(started) < 5;
            sys::close(read_end);
            read == 0 && instant
        }
        None => false,
    };
    checks[1] = Check {
        name: "B dosya sonu",
        detail: if b {
            "yazan uc kapali, 0 dondu ve beklemedi"
        } else {
            "dosya sonu dogru bildirilmedi"
        },
        passed: b,
    };

    // --- C: O_NONBLOCK ---
    let mut nonblock_result = 0isize;
    let c = match sys::pipe() {
        Some((read_end, write_end)) => {
            let set = sys::set_flags(read_end, sys::O_NONBLOCK) == 0;
            let flags = sys::get_flags(read_end);
            let started = sys::ticks();
            let mut buf = [0u8; 8];
            nonblock_result = sys::read(read_end, &mut buf);
            let instant = sys::ticks().wrapping_sub(started) < 5;
            sys::close(read_end);
            sys::close(write_end);
            set && flags == sys::O_NONBLOCK as isize && nonblock_result == -EAGAIN && instant
        }
        None => false,
    };
    checks[2] = Check {
        name: "C O_NONBLOCK",
        detail: if nonblock_result == 0 {
            "bos boruda 0 dondu (dosya sonu sanilir)"
        } else if c {
            "bos boruda -EAGAIN, bekleme yok"
        } else {
            "bayrak kurulamadi ya da kod yanlis"
        },
        passed: c,
    };

    // --- D: uc durum uc cevap ---
    //
    // Asil mesele bu. Ayni cagri, ayni **bos** boru, iki farkli cevap:
    // yazan varsa -EAGAIN (sonra tekrar dene), yazan yoksa 0 (bitti).
    // Bunlar ayni sayi olsaydi okuyan taraf ikisini ayirt edemezdi.
    let d = match sys::pipe() {
        Some((read_end, write_end)) => {
            sys::set_flags(read_end, sys::O_NONBLOCK);
            let mut buf = [0u8; 8];
            // Yazan uc acik: "simdilik yok".
            let while_open = sys::read(read_end, &mut buf);
            sys::close(write_end);
            // Yazan uc kapandi: "bir daha gelmeyecek".
            let after_close = sys::read(read_end, &mut buf);
            sys::close(read_end);
            while_open == -EAGAIN && after_close == 0
        }
        None => false,
    };
    checks[3] = Check {
        name: "D uc durum ayri",
        detail: if d {
            "yazan varken -EAGAIN, kapaninca 0"
        } else {
            "iki durum AYIRT EDILEMIYOR"
        },
        passed: d,
    };

    // --- E: pipe2 ---
    let e = match sys::pipe2(sys::O_NONBLOCK) {
        Some((read_end, write_end)) => {
            let flags = sys::get_flags(read_end);
            let mut buf = [0u8; 8];
            let read = sys::read(read_end, &mut buf);
            sys::close(read_end);
            sys::close(write_end);
            flags == sys::O_NONBLOCK as isize && read == -EAGAIN
        }
        None => false,
    };
    checks[4] = Check {
        name: "E pipe2",
        detail: if e {
            "bayrak yaratma aninda kondu"
        } else {
            "pipe2 bayragi uygulamadi"
        },
        passed: e,
    };

    for check in &checks {
        let _ = writeln!(
            out,
            "[blocking] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }

    let mut win = match Window::open("blocking -- bekleyen okuma", 300, 200, 460, 170) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks, waited_ms);
        win.flush();
    }
}

/// "Simdilik yok, sonra tekrar dene" -- dosya sonu **degil**.
const EAGAIN: isize = 11;

fn draw(win: &mut Window, checks: &[Check; 5], waited: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "varsayilan beklemektir", ACCENT);

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
    win.text(6, h - 30, "beklenen ms:", DIM);
    win.number(130, h - 30, waited, FG);
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
