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
//! ## Yazma da bekliyor
//!
//! Okumanin aynasi tamamlandi: dolu bir boruya yazmak da yer acilana
//! kadar bekliyor. Ve ucuncu durumda POSIX'in en sert varsayilani
//! devreye giriyor -- okuyan uc kapaliysa yazmak yalnizca `EPIPE`
//! dondurmuyor, **`SIGPIPE`** de gonderiyor ve yakalanmazsa surec
//! **oluyor**.
//!
//! Kaba gorunuyor ama kabuk boru hatlarinin calismasi buna bagli:
//! `uretici | head` kaliginda `head` ilk on satiri alip cikar; uretici
//! durdurulmazsa sonsuza kadar kosardi.
//!
//! Windows'ta bunun karsiligi **yok** (bkz. `winpipe` F sinavi).
//!
//! ## Sekiz sinav
//!
//! ```text
//!   A  bekleme       -> bos borudan okumak kardes yazana kadar BEKLER
//!   B  dosya sonu    -> yazan uc kapaninca okuma 0 doner (beklemez)
//!   C  O_NONBLOCK    -> bos boruda -EAGAIN, bekleme YOK
//!   D  uc durum ayri -> EAGAIN ile 0 ayni sey DEGIL
//!   E  pipe2         -> bayrak yaratma aninda konabiliyor
//!   F  yazma bekler  -> dolu boruya yazmak kardes okuyana kadar BEKLER
//!   G  varsayilan olum-> yakalamayan cocuk 141 (128+SIGPIPE) ile OLER
//!   H  SIGPIPE        -> yakalanirsa sinyal GELIR, surec yasar
//!   I  EPIPE          -> yakalandiginda yazma -EPIPE doner
//! ```
//!
//! D sinavi asil meselenin kendisi: C ile B'nin ayni cagriyi ayni bos
//! boruda farkli cevaplar vermesi. Ayni sayiyi dondurselerdi sadelesme
//! surerdi.
//!
//! G ile H ayni olayin iki yuzu ve sirasi onemli. G, sinyali
//! **yakalamayan** bir `fork` cocugunun gercekten oldugunu olcuyor --
//! ebeveyn cikis kodunu `waitpid` ile topluyor ve 141 (128+13)
//! bekliyor. Kendi surecimizde olcemezdik: olcen taraf da olurdu.
//!
//! H ve I ise sinyali yakaliyor, boylece surec yasiyor ve `write`in
//! donus degeri gorulebiliyor. Yakalamasaydik o satirlara hic
//! gelinmezdi. Bu yuzden G once kosuyor: isleyici kurulmadan once.
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

/// F sinavinin borusunun okuma ucu.
static READ_END: AtomicUsize = AtomicUsize::new(usize::MAX);
/// G sinavinda sinyal geldi mi?
static SIGPIPE_SEEN: AtomicUsize = AtomicUsize::new(0);

/// `SIGPIPE` isleyicisi -- yalnizca "geldim" demek icin.
extern "C" fn on_sigpipe(_signo: u32) {
    SIGPIPE_SEEN.store(1, Ordering::SeqCst);
}

/// F sinavinin kardesi: bekleyip **okur** ve yer acar.
extern "C" fn late_reader(_param: usize) -> usize {
    sys::sleep_ms(DELAY_MS);
    let fd = READ_END.load(Ordering::SeqCst);
    if fd != usize::MAX {
        let mut buf = [0u8; 512];
        sys::read(fd, &mut buf);
    }
    0
}

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
    let mut checks = [EMPTY; 9];

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

    // --- F: yazma bekliyor ---
    //
    // Tamponu tamamen dolduruyoruz, sonra bir kardes 150 ms sonra
    // okuyup yer aciyor. Yazma beklemezse hemen kisa donerdi.
    let mut write_waited_ms = 0usize;
    let f = match sys::pipe() {
        Some((read_end, write_end)) => {
            // Tamponu doldur. `PIPE_CAPACITY` 1 KiB; kisa donusler
            // oldugu surece yazmaya devam ediyoruz.
            let block = [b'x'; 256];
            let mut total = 0usize;
            // Bloke olmayan kipte doldur ki burada asilmayalim.
            sys::set_flags(write_end, sys::O_NONBLOCK);
            loop {
                let n = sys::write(write_end, &block);
                if n <= 0 {
                    break;
                }
                total += n as usize;
                if total > 4096 {
                    break;
                }
            }
            // Simdi bloke eden kipe geri don ve bir bayt daha yazmayi
            // dene: tampon dolu, beklemeli.
            sys::set_flags(write_end, 0);
            READ_END.store(read_end, Ordering::SeqCst);
            let helper = sys::clone_thread(late_reader, 0, 0);
            let started = sys::ticks();
            let wrote = sys::write(write_end, b"z");
            write_waited_ms = sys::ticks().wrapping_sub(started) * 10;
            let ok = helper > 0 && wrote == 1 && write_waited_ms >= 100;
            sys::close(read_end);
            sys::close(write_end);
            ok
        }
        None => false,
    };
    checks[5] = Check {
        name: "F yazma bekler",
        detail: if f {
            "dolu boru, kardes okuyana kadar beklendi"
        } else if write_waited_ms < 100 {
            "beklemeden dondu (yazma bloke etmiyor)"
        } else {
            "yazma basarisiz"
        },
        passed: f,
    };

    // --- G: varsayilan davranis oldurur ---
    //
    // Bu sinav isleyici kurulmadan **once** kosmali. Kendi surecimizde
    // olcemezdik -- olcen taraf da olurdu -- o yuzden bir `fork` cocugu
    // yaziyor ve ebeveyn cikis kodunu topluyor.
    //
    // Beklenen kod 141 = 128 + SIGPIPE(13): POSIX'in sinyalle olen bir
    // surec icin kullandigi gelenek.
    let mut child_code = 0u32;
    let g = match sys::pipe() {
        Some((read_end, write_end)) => {
            let forked = sys::fork();
            match forked {
                0 => {
                    // Cocuk: okuyan ucu kapatip yaziyor. Isleyici yok,
                    // yani buradan geri donus de yok.
                    sys::close(read_end);
                    sys::write(write_end, b"kimse yok");
                    // Buraya ulasilmamali: sinyal sureci oldurmus
                    // olmali. Ulasilirsa ayirt edilebilir bir kodla cik.
                    sys::exit(7);
                }
                id if id > 0 => {
                    sys::close(read_end);
                    sys::close(write_end);
                    let reaped = sys::waitpid(id as usize, &mut child_code, 0) >= 0;
                    // Durum **paketlenmis** gelir ve iki ayri soruyu
                    // cevaplar. Burada sorulan "hangi kodla cikti" degil,
                    // "neyle oldu": `WIFSIGNALED` + `WTERMSIG`.
                    let killed = sys::signalled(child_code);
                    let by = sys::term_signal(child_code);
                    child_code = by;
                    reaped && killed && by == tcmk::signal::SIGPIPE
                }
                _ => false,
            }
        }
        None => false,
    };
    checks[6] = Check {
        name: "G varsayilan olum",
        detail: if g {
            "yakalamayan cocuk SIGPIPE ile oldu"
        } else if child_code == 0 {
            "cocuk OLMEDI, normal cikis gorundu"
        } else {
            "cocuk beklenemedi ya da sinyal yanlis"
        },
        passed: g,
    };

    // --- H: sinyal yakalanabiliyor ---
    //
    // Artik isleyiciyi kuruyoruz: surec yasayacak ve olcum surebilecek.
    tcmk::signal::install(tcmk::signal::SIGPIPE, on_sigpipe);
    let mut epipe_result = 0isize;
    let h = match sys::pipe() {
        Some((read_end, write_end)) => {
            // Okuyan ucu kapat: artik kimse okumayacak.
            sys::close(read_end);
            epipe_result = sys::write(write_end, b"kimse yok");
            // Sinyal Ring 3'e donusle teslim edilir; bir tik birakalim.
            sys::sleep_ms(20);
            sys::close(write_end);
            SIGPIPE_SEEN.load(Ordering::SeqCst) == 1
        }
        None => false,
    };
    checks[7] = Check {
        name: "H SIGPIPE",
        detail: if h {
            "yakalandi, surec yasiyor"
        } else {
            "sinyal GELMEDI"
        },
        passed: h,
    };

    // --- I: EPIPE ---
    //
    // Sinyal yakalandigi icin yazmanin donus degeri gorulebiliyor.
    // Yakalanmasaydi bu satira hic gelinmezdi -- G tam olarak onu
    // olcuyor.
    let i = epipe_result == -EPIPE;
    checks[8] = Check {
        name: "I EPIPE",
        detail: if i {
            "yazma -EPIPE dondu"
        } else if epipe_result >= 0 {
            "yazma BASARILI dondu (okuyan yokken)"
        } else {
            "yanlis hata kodu"
        },
        passed: i,
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

    let mut win = match Window::open("blocking -- bekleyen borular", 300, 170, 460, 240) {
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
/// Okuyan ucu kapali boruya yazmak. Cogu program bunu **hic gormez**:
/// `SIGPIPE` yakalanmazsa surec once oler.
const EPIPE: isize = 32;

fn draw(win: &mut Window, checks: &[Check; 9], waited: usize) {
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
