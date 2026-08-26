//! `threads` -- `clone`: `fork` kopyalar, is parcacigi paylasir.
//!
//! Buraya kadar TCMK'de **bir gorev = bir surec = bir akis** idi.
//! `gettid` ile `getpid` ayni sayiyi donduruyordu, cunku ayirt edecek
//! bir sey yoktu. Artik var.
//!
//! ## Fark tek cumlede
//!
//! ```text
//!   fork          -> adres uzayi KOPYALANIR, cocuk ayri gorur
//!   clone(VM|..)  -> adres uzayi PAYLASILIR, ikisi ayni sey gorur
//! ```
//!
//! Sinav bunu en dogrudan yoldan olcuyor: is parcacigi bir global
//! degiskene yazar, ana akis onu okur. `fork`ta bu **imkansizdir** --
//! cocugun yazdigi kendi kopyasina gider.
//!
//! ## Dort sinav
//!
//! ```text
//!   A  bellek paylasimi  -> is parcaciginin yazdigini ana akis GORUR
//!   B  kimlikler         -> gettid AYRISIR, getpid AYNI kalir
//!   C  tanimlayicilar    -> is parcaciginin actigi dosyayi ana akis gorur
//!   D  fork ile karsit   -> cocugun yazdigi ebeveyne SIZMAZ
//!   E  once ana akis     -> ana akis cikinca kardes YASAMAYA DEVAM EDER
//! ```
//!
//! D bilerek burada: paylasimi olcmenin tek durust yolu, **paylasmayan**
//! yolu da ayni programda olcmek. Ikisi ayni satirlari kullaniyor;
//! ayrisan yalnizca hangi cagriyla dogduklari.
//!
//! E ise paylasimin tehlikeli tarafini olcuyor. Adres uzayi ortaksa,
//! ana akis cikarken onu yikmak hala kosan kardesin altindan zemini
//! cekmek olur. Sinav bunu bir `fork` cocugunda yapiyor: cocuk bir is
//! parcacigi baslatip **hemen** cikiyor. Kendi surecinde denemek
//! imkansizdi -- olcen taraf da olurdu.
//!
//! Kanit bir **boru** uzerinden geliyor ve bu bilincli bir secim.
//! "Ebeveyn ayakta mi" diye sormak yetmezdi: cekirdek hatali sayfayi
//! yakalayip yalnizca coken gorevi durduruyor, yani kardes olse de
//! ebeveyn saglam kalirdi ve sinav **her iki durumda da gecerdi**. Kardes
//! ana akis ciktiktan sonra boruya bir bayt yaziyor; o bayt geldiyse
//! kardes gercekten yasamis demektir.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0018_1220;
const PANEL: u32 = 0x0028_2038;
const FG: u32 = 0x00E4_DEF0;
const DIM: u32 = 0x008C_84A0;
const ACCENT: u32 = 0x00A0_C0FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Is parcaciginin yazacagi deger. Paylasilan bellekte oldugu icin ana
/// akis onu **gorur**; `fork` cocugunda gormezdi.
const MARK: usize = 0xC0DE_1234;

/// Is parcaciginin yazdigi yer.
static SHARED: AtomicUsize = AtomicUsize::new(0);
/// Is parcaciginin gordugu kimlikler.
static CHILD_TID: AtomicUsize = AtomicUsize::new(0);
static CHILD_PID: AtomicUsize = AtomicUsize::new(0);
/// Is parcaciginin actigi tanimlayici.
static CHILD_FD: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Is parcacigi bitti mi?
static DONE: AtomicUsize = AtomicUsize::new(0);

const PATH: &[u8] = b"/boot/msg.txt\0";

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

/// Is parcacigi govdesi.
///
/// Ayni adres uzayinda kostugu icin `static`lara dogrudan yaziyor --
/// paylasimin kaniti da bu. Dondugunde cekirdegin yigina yazdigi
/// tramplen cikis cagrisini yapar; program ayrica bir sey yapmaz.
extern "C" fn worker(param: usize) -> usize {
    SHARED.store(param, Ordering::SeqCst);
    CHILD_TID.store(sys::gettid(), Ordering::SeqCst);
    CHILD_PID.store(sys::getpid(), Ordering::SeqCst);
    // Tanimlayici tablosu da paylasilir (`CLONE_FILES`): burada acilan
    // dosyayi ana akis kapatabilmeli.
    let fd = unsafe { sys::open_raw(PATH.as_ptr(), 0) };
    CHILD_FD.store(if fd < 0 { usize::MAX } else { fd as usize }, Ordering::SeqCst);
    DONE.store(1, Ordering::SeqCst);
    0
}

/// E sinavinin is parcacigi: bilerek **yavas**, ve sonunda konusuyor.
///
/// Ana akis bunun ortasinda cikiyor; amac tam olarak o ani gecmek. Cikis
/// atlatildiktan sonra boruya bir bayt yaziliyor -- kanit o bayt.
/// `param` borunun yazma ucu.
extern "C" fn lingering(param: usize) -> usize {
    // Ana akisin cikmasi icin yeterince uzun: cikis hemen ardindan
    // geliyor, 150 ms fazlasiyla yetiyor.
    let mut spins = 0;
    while spins < 15 {
        sys::sleep_ms(10);
        spins += 1;
    }
    // Buraya ulasmak, adres uzayinin hala ayakta oldugunu gosteriyor.
    sys::write(param, MARKER);
    0
}

/// Kardesin boruya yazdigi kanit.
const MARKER: &[u8] = b"T";

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;
    let mut checks = [EMPTY; 5];

    let parent_tid = sys::gettid();
    let parent_pid = sys::getpid();

    let tid = sys::clone_thread(worker, MARK, 0);
    // Is parcacigi ayri bir akis: bitmesini beklemek gerekiyor. Gercek
    // bir program `pthread_join` kullanirdi; TCMK'de bekleme yolu
    // `waitpid` ve o yalnizca cocuklari gorur, o yuzden burada bayrak
    // yoklaniyor. Sinir README'de yazili.
    let mut spins = 0;
    while DONE.load(Ordering::SeqCst) == 0 && spins < 200 {
        sys::sleep_ms(10);
        spins += 1;
    }
    let finished = DONE.load(Ordering::SeqCst) == 1;

    // --- A: bellek paylasimi ---
    let seen = SHARED.load(Ordering::SeqCst);
    let a = tid > 0 && finished && seen == MARK;
    checks[0] = Check {
        name: "A bellek paylasimi",
        detail: if tid <= 0 {
            "is parcacigi yaratilamadi"
        } else if !finished {
            "is parcacigi bitmedi"
        } else if a {
            "yazdigi deger ana akista gorundu"
        } else {
            "deger GORUNMEDI (uzay paylasilmiyor)"
        },
        passed: a,
    };

    // --- B: kimlikler ---
    //
    // `gettid` ayrisiyor cunku iki ayri akis var; `getpid` ayni kaliyor
    // cunku tek bir surec var. Ikisinin de dogru olmasi sart: yalnizca
    // birine bakmak "ayri surec mi ayri akis mi" sorusunu cevaplamazdi.
    let child_tid = CHILD_TID.load(Ordering::SeqCst);
    let child_pid = CHILD_PID.load(Ordering::SeqCst);
    let b = finished && child_tid != parent_tid && child_pid == parent_pid;
    checks[1] = Check {
        name: "B kimlikler",
        detail: if !finished {
            "is parcacigi bitmedi"
        } else if child_tid == parent_tid {
            "gettid AYRISMADI"
        } else if child_pid != parent_pid {
            "getpid ayristi (ayri surec sayiliyor)"
        } else {
            "tid ayri, pid ayni"
        },
        passed: b,
    };

    // --- C: tanimlayici paylasimi ---
    let child_fd = CHILD_FD.load(Ordering::SeqCst);
    let mut buf = [0u8; 16];
    let readable = child_fd != usize::MAX && sys::read(child_fd, &mut buf) > 0;
    let c = finished && readable;
    checks[2] = Check {
        name: "C tanimlayici paylasimi",
        detail: if child_fd == usize::MAX {
            "is parcacigi dosyayi acamadi"
        } else if c {
            "kardesin actigi dosya ana akista okundu"
        } else {
            "tanimlayici GORUNMEDI"
        },
        passed: c,
    };
    if child_fd != usize::MAX {
        sys::close(child_fd);
    }

    // --- D: `fork` karsiti ---
    //
    // Ayni satirlar, baska bir cagri. Cocuk paylasilan degiskene yazar;
    // ebeveyn onu **gormemeli**, cunku adres uzayi kopyalandi.
    SHARED.store(0, Ordering::SeqCst);
    let mut forked = false;
    match sys::fork() {
        0 => {
            SHARED.store(MARK, Ordering::SeqCst);
            sys::exit(0);
        }
        id if id > 0 => {
            let mut status = 0u32;
            forked = sys::waitpid(id as usize, &mut status, 0) >= 0;
        }
        _ => {}
    }
    let leaked = SHARED.load(Ordering::SeqCst) == MARK;
    let d = forked && !leaked;
    checks[3] = Check {
        name: "D fork ile karsit",
        detail: if !forked {
            "fork basarisiz"
        } else if leaked {
            "cocugun yazdigi ebeveyne SIZDI"
        } else {
            "cocugun yazdigi ebeveyne sizmadi"
        },
        passed: d,
    };

    // --- E: kardes kosarken cikmak ---
    //
    // Cocuk bir is parcacigi baslatir ve beklemeden `exit` eder. Paylasilan
    // adres uzayi o anda yikilirsa kardes bir sonraki komutunda coker;
    // ebeveyn bunu `waitpid`in donusunden ve kendi ayakta kalmasindan
    // anlar.
    let mut reaped = false;
    let mut heard = 0usize;
    if let Some((read_end, write_end)) = sys::pipe() {
        match sys::fork() {
            0 => {
                let _ = sys::clone_thread(lingering, write_end, 0);
                // Kardes hala kosuyor: bilerek beklemiyoruz.
                sys::exit(0);
            }
            id if id > 0 => {
                let mut status = 0u32;
                reaped = sys::waitpid(id as usize, &mut status, 0) >= 0;
                // Kardes yazacaksa bu araliktan sonra yazmis olur.
                sys::sleep_ms(400);
                let mut echo = [0u8; 4];
                let got = sys::read(read_end, &mut echo);
                if got > 0 && echo[0] == MARKER[0] {
                    heard = got as usize;
                }
            }
            _ => {}
        }
        sys::close(read_end);
        sys::close(write_end);
    }
    let e = reaped && heard > 0;
    checks[4] = Check {
        name: "E once ana akis",
        detail: if !reaped {
            "cocuk beklenemedi"
        } else if heard == 0 {
            "kardes ana akistan sonra YASAMADI"
        } else {
            "ana akis cikti, kardes yazmaya devam etti"
        },
        passed: e,
    };

    for check in &checks {
        let _ = writeln!(
            out,
            "[threads] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }

    let mut win = match Window::open("threads -- clone", 300, 210, 440, 170) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks, parent_tid, child_tid);
        win.flush();
    }
}

fn draw(win: &mut Window, checks: &[Check; 5], parent: usize, child: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "fork kopyalar, clone paylasir", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            320,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, "tid:", DIM);
    win.number(50, h - 30, parent, FG);
    win.text(100, h - 30, "kardes:", DIM);
    win.number(170, h - 30, child, FG);
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
