//! `intr` -- sinyal, bekleyen bir cagriyi boler mi?
//!
//! Bu soru ancak bir onceki batidan **sonra** anlamli oldu: bloke eden
//! cagrilar gelene kadar bolunecek bir sey yoktu. Simdi bos bir borudan
//! okumak bekliyor, ve o bekleyisin ortasinda bir sinyal gelirse ne
//! olacagi POSIX'in en ince -- ve en cok hataya yol acan -- ayrintisi.
//!
//! ## `EINTR`: POSIX'in meshur puruzu
//!
//! ```text
//!   read(bos_boru)          <- bekliyor
//!     ...sinyal gelir...
//!   isleyici calisir
//!   read -> -EINTR          <- veri gelmedi, cagri YARIDA kesildi
//! ```
//!
//! Program bunu denetlemezse veriyi kaybeder ya da yanlis dallanir.
//! `EINTR` denetlemeyi unutmak, UNIX tarihinin en yaygin hatalarindan
//! biri.
//!
//! ## `SA_RESTART`: puruzu gorunmez kilmak
//!
//! Isleyici `SA_RESTART` ile kurulursa cekirdek cagriyi **kendisi**
//! yeniden baslatiyor:
//!
//! ```text
//!   read(bos_boru)          <- bekliyor
//!     ...sinyal gelir...
//!   isleyici calisir
//!   read yeniden calisir    <- cagiran hicbir sey fark etmez
//!   read -> veri
//! ```
//!
//! Uygulamasi zarif: cekirdek cerceveyi **geri sariyor** -- komut
//! isaretcisi iki bayt geri aliniyor ve cagri numarasi geri yaziliyor.
//! Isleyici o cerceveyi gorup donuyor, `sigreturn` onu yukluyor ve
//! cagri kendiliginden yeniden calisiyor. Gercek Linux'ta da ayni
//! mekanizma var (`ERESTARTSYS`).
//!
//! Tarihsel not: eski `signal(2)` yuzu bayragi kendiliginden koyar, ham
//! `sigaction` koymaz. Ayni programin farkli libc'lerde farkli
//! davranmasinin sebebi tam olarak buydu.
//!
//! ## Windows'ta bunun karsiligi yok
//!
//! Win32'de sinyal diye bir sey olmadigi icin `ReadFile` boyle
//! bolunmez. En yakin kavram **alertable bekleme** (`ReadFileEx`,
//! `SleepEx` ile APC teslimi) ve o da acikca istenmeli -- varsayilan
//! bekleme bolunmez. Yani POSIX'te bolunme varsayilan, Win32'de opt-in.
//!
//! ## Alti sinav
//!
//! ```text
//!   A  EINTR          -> bekleyen okuma sinyalle bolunur, -EINTR doner
//!   B  isleyici calisti-> bolunme sinyali gercekten teslim etti
//!   C  veri gelmedi   -> bolunen okuma veriyi TUKETMEDI
//!   D  SA_RESTART     -> cagri yeniden baslar, cagiran EINTR GORMEZ
//!   E  yok sayilan    -> SIG_IGN olan sinyal cagriyi BOLMEZ
//!   F  yazma da boluner-> dolu boruya yazma da ayni kurala uyar
//! ```
//!
//! C bilerek burada: bolunen bir okumanin veriyi tuketmedigini
//! gostermek sart, yoksa `EINTR` sessiz veri kaybi olurdu.
//!
//! E, ayrimi olcuyor: yok sayilan bir sinyal hicbir sey yapmiyor
//! demektir, bekleyen bir okumayi kaldirmasinin sebebi yok.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0018_1418;
const PANEL: u32 = 0x0028_2430;
const FG: u32 = 0x00E6_E2E8;
const DIM: u32 = 0x008A_8496;
const ACCENT: u32 = 0x00D0_A0E0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Kardesin sinyali gondermeden once bekleyecegi sure. Ana akisin
/// gercekten **uyumus** olmasi icin yeterince uzun.
const DELAY_MS: usize = 120;

/// Isleyici kac kez calisti?
static HANDLER_RUNS: AtomicUsize = AtomicUsize::new(0);
/// Sinyalin gonderilecegi gorev.
static TARGET: AtomicUsize = AtomicUsize::new(0);
/// D sinavinda kardesin yazacagi boru ucu.
static WRITE_END: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Kardeslerin `pthread_t` yuvalari.
///
/// Her sinav kendi kardesini **bitmesini bekleyerek** birakmak zorunda.
/// Ilk halinde beklemiyordu ve bu gercek bir olcum hatasina yol acti:
/// D'nin kardesi hala kosarken E'nin borusu aciliyor, tanimlayici
/// numaralari geri donusturuldugu icin kardes **E'nin borusuna**
/// yaziyordu. E boylece yanlis sebeple geciyordu -- olculen sey
/// "yok sayilan sinyal bolmedi" degil, "baska bir kardes veri yazdi"
/// idi.
static mut JOIN_A: u32 = 0;
static mut JOIN_D: u32 = 0;
static mut JOIN_E: u32 = 0;
static mut JOIN_F: u32 = 0;

/// A/B sinavlarinin damgasi.
const MARK: &[u8] = b"veri";

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

/// Sinyal isleyicisi -- yalnizca sayiyor.
extern "C" fn on_signal(_signo: u32) {
    HANDLER_RUNS.fetch_add(1, Ordering::SeqCst);
}

/// Bekleyip ana akisa sinyal gonderen kardes.
extern "C" fn late_signaller(_param: usize) -> usize {
    sys::sleep_ms(DELAY_MS);
    tcmk::signal::kill(TARGET.load(Ordering::SeqCst), tcmk::signal::SIGUSR1);
    0
}

/// D sinavinin kardesi: once sinyal, sonra **veri**.
///
/// Sira onemli: cagri once bolunmeli, sonra yeniden baslayinca veriyi
/// bulmali. Veri once gelseydi bolunme hic olmazdi.
extern "C" fn signal_then_write(_param: usize) -> usize {
    sys::sleep_ms(DELAY_MS);
    tcmk::signal::kill(TARGET.load(Ordering::SeqCst), tcmk::signal::SIGUSR1);
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
    let mut checks = [EMPTY; 6];

    TARGET.store(sys::gettid(), Ordering::SeqCst);

    // --- A + B + C: EINTR ---
    //
    // Isleyici **bayraksiz** kuruluyor: bolunen cagri -EINTR donmeli.
    tcmk::signal::install(tcmk::signal::SIGUSR1, on_signal);
    let mut eintr_result = 0isize;
    let mut leftover = 0isize;
    let (a, b, c) = match sys::pipe() {
        Some((read_end, write_end)) => {
            let slot = core::ptr::addr_of_mut!(JOIN_A);
            let helper = unsafe { sys::clone_joinable(late_signaller, 0, slot) };
            let before = HANDLER_RUNS.load(Ordering::SeqCst);
            let mut buf = [0u8; 16];
            eintr_result = sys::read(read_end, &mut buf);
            let ran = HANDLER_RUNS.load(Ordering::SeqCst) > before;
            // Kardes bitmeden devam etmek, sonraki sinavin borusuna
            // sizmasi demek olurdu.
            unsafe { sys::join_thread(slot, 3000) };

            // Bolunen okuma veriyi tuketmemis olmali. Simdi yaziyoruz ve
            // ayni tanimlayicidan okuyoruz: tam gelmeli.
            sys::write(write_end, MARK);
            let mut after = [0u8; 16];
            leftover = sys::read(read_end, &mut after);

            let intact = leftover == MARK.len() as isize && &after[..MARK.len()] == MARK;
            sys::close(read_end);
            sys::close(write_end);
            (
                helper > 0 && eintr_result == -EINTR,
                ran,
                intact,
            )
        }
        None => (false, false, false),
    };
    checks[0] = Check {
        name: "A EINTR",
        detail: if eintr_result >= 0 {
            "okuma bolunmedi (veri dondu)"
        } else if a {
            "bekleyen okuma bolundu, -EINTR dondu"
        } else {
            "yanlis hata kodu"
        },
        passed: a,
    };
    checks[1] = Check {
        name: "B isleyici calisti",
        detail: if b {
            "bolen sinyal gercekten teslim edildi"
        } else {
            "isleyici HIC calismadi"
        },
        passed: b,
    };
    checks[2] = Check {
        name: "C veri tukenmedi",
        detail: if c {
            "bolunen okuma veriyi yemedi"
        } else {
            "veri KAYBOLDU ya da eksik geldi"
        },
        passed: c,
    };

    // --- D: SA_RESTART ---
    //
    // Ayni senaryo, tek fark isleyicinin bayragi. Cagiran artik `EINTR`
    // gormemeli: cekirdek cagriyi yeniden baslatmali ve okuma sonunda
    // veriyle donmeli.
    tcmk::signal::install_with(
        tcmk::signal::SIGUSR1,
        on_signal,
        tcmk::signal::SA_RESTART,
    );
    let mut restart_result = 0isize;
    let d = match sys::pipe() {
        Some((read_end, write_end)) => {
            WRITE_END.store(write_end, Ordering::SeqCst);
            let before = HANDLER_RUNS.load(Ordering::SeqCst);
            let slot = core::ptr::addr_of_mut!(JOIN_D);
            let helper = unsafe { sys::clone_joinable(signal_then_write, 0, slot) };
            let mut buf = [0u8; 16];
            restart_result = sys::read(read_end, &mut buf);
            let ran = HANDLER_RUNS.load(Ordering::SeqCst) > before;
            unsafe { sys::join_thread(slot, 3000) };
            let ok = helper > 0
                && ran
                && restart_result == MARK.len() as isize
                && &buf[..MARK.len()] == MARK;
            sys::close(read_end);
            sys::close(write_end);
            ok
        }
        None => false,
    };
    checks[3] = Check {
        name: "D SA_RESTART",
        detail: if restart_result == -EINTR {
            "cagri yeniden BASLAMADI (-EINTR gordu)"
        } else if d {
            "sinyal geldi, cagri yeniden basladi, veri geldi"
        } else {
            "veri yanlis ya da isleyici calismadi"
        },
        passed: d,
    };

    // --- E: yok sayilan sinyal bolmez ---
    //
    // `SIG_IGN` olan bir sinyal hicbir sey yapmiyor demektir; bekleyen
    // bir okumayi kaldirmasinin sebebi yok. Ayrimi yapmasaydik yok
    // sayilan bir sinyal bosuna `EINTR` uretirdi.
    tcmk::signal::ignore(tcmk::signal::SIGUSR1);
    let mut ignored_result = 0isize;
    let mut waited_ms = 0usize;
    let e = match sys::pipe() {
        Some((read_end, write_end)) => {
            WRITE_END.store(write_end, Ordering::SeqCst);
            let slot = core::ptr::addr_of_mut!(JOIN_E);
            let helper = unsafe { sys::clone_joinable(signal_then_write, 0, slot) };
            let started = sys::ticks();
            let mut buf = [0u8; 16];
            ignored_result = sys::read(read_end, &mut buf);
            // Zamanlama sarti sinavin belkemigi: kardes sinyali 120 ms'de,
            // veriyi 240 ms'de gonderiyor. Okuma sinyalde bolunseydi
            // erken donerdi. "Veri geldi" tek basina yetmez -- ne zaman
            // geldigi de olculmeli.
            waited_ms = sys::ticks().wrapping_sub(started) * 10;
            unsafe { sys::join_thread(slot, 3000) };
            let ok = helper > 0
                && ignored_result == MARK.len() as isize
                && &buf[..MARK.len()] == MARK
                && waited_ms >= 200;
            sys::close(read_end);
            sys::close(write_end);
            ok
        }
        None => false,
    };
    checks[4] = Check {
        name: "E yok sayilan bolmez",
        detail: if ignored_result == -EINTR {
            "SIG_IGN olan sinyal cagriyi BOLDU"
        } else if e {
            "sinyale ragmen veri gelene kadar beklendi"
        } else if waited_ms < 200 {
            "erken dondu (sinyal beklemeyi kisaltti)"
        } else {
            "veri yanlis geldi"
        },
        passed: e,
    };

    // --- F: yazma da bolunuyor ---
    //
    // Kural okuma/yazma ayrimi yapmiyor: bekleyen her cagri ayni.
    tcmk::signal::install(tcmk::signal::SIGUSR1, on_signal);
    let mut write_result = 0isize;
    let f = match sys::pipe() {
        Some((read_end, write_end)) => {
            // Tamponu doldur (bloke olmayan kipte, burada asilmamak icin).
            sys::set_flags(write_end, sys::O_NONBLOCK);
            let block = [b'x'; 256];
            let mut total = 0usize;
            loop {
                let n = sys::write(write_end, &block);
                if n <= 0 || total > 4096 {
                    break;
                }
                total += n as usize;
            }
            sys::set_flags(write_end, 0);

            let slot = core::ptr::addr_of_mut!(JOIN_F);
            let helper = unsafe { sys::clone_joinable(late_signaller, 0, slot) };
            write_result = sys::write(write_end, b"z");
            unsafe { sys::join_thread(slot, 3000) };
            let ok = helper > 0 && write_result == -EINTR;
            sys::close(read_end);
            sys::close(write_end);
            ok
        }
        None => false,
    };
    checks[5] = Check {
        name: "F yazma da boluner",
        detail: if write_result >= 0 {
            "dolu boruya yazma bolunmedi"
        } else if f {
            "bekleyen yazma bolundu, -EINTR dondu"
        } else {
            "yanlis hata kodu"
        },
        passed: f,
    };

    for check in &checks {
        let _ = writeln!(
            out,
            "[intr] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }

    let mut win = match Window::open("intr -- EINTR ve SA_RESTART", 290, 190, 470, 190) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks, HANDLER_RUNS.load(Ordering::SeqCst));
        win.flush();
    }
}

/// Bekleyen cagri sinyalle bolundu.
const EINTR: isize = 4;

fn draw(win: &mut Window, checks: &[Check; 6], runs: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "sinyal bekleyen cagriyi boler", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            350,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, "isleyici calisti:", DIM);
    win.number(170, h - 30, runs, FG);
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
