//! `sigchld` -- cocugun durumu: itilen bilgi mi, cekilen mi?
//!
//! Bu batiya kadar bir ebeveyn, cocugunun durum degistirdigini yalnizca
//! `waitpid` cagirarak ogrenebiliyordu. Yani bilgi **cekiliyordu**:
//! ebeveyn sormadan hicbir sey ogrenmiyordu, ve sormak da ya bloke
//! oluyordu ya da `WNOHANG` ile bir yoklama dongusu gerektiriyordu.
//!
//! `SIGCHLD` bunu tersine ceviriyor -- bilgi **itiliyor**.
//!
//! ```text
//!   POSIX    cocuk oldu  ->  ebeveyne SIGCHLD GONDERILIR   (itme)
//!   Windows  cocuk oldu  ->  surec nesnesi ISARETLENIR     (cekme)
//! ```
//!
//! Windows'un karsiligi yok ve bu bir eksiklik degil, baska bir secim.
//! Orada cocuk oldugunde surec nesnesi isaretlenir ve ebeveyn onu
//! `WaitForSingleObject` ile ceker -- yani Win32'de cocuk olumu de
//! **randevu**dur, tipki APC gibi. POSIX iter; Windows beklenir.
//!
//! ## `SIGCHLD` iki yerde kural disi
//!
//! **Bir:** varsayilani **yok saymak**. Cogu sinyalin varsayilani olum;
//! bu ise gelmesi beklenen ve cogu programin umursamadigi bir bildirim.
//! Varsayilani olum olsaydi `fork` eden her program cocugu bitince
//! olurdu.
//!
//! **Iki:** onu `SIG_IGN` yapmak "sinyali at" demekten fazlasini yapar
//! -- cekirdek cocuklari **kendisi toplar** ve `waitpid` artik `ECHILD`
//! doner. Yani yok saymak burada sinyali degil **kaydi** siliyor.
//! POSIX'in en bilinen tuhafliklarindan biri ve G sinavi tam olarak
//! bunu olcuyor.
//!
//! ## Yedi sinav
//!
//! ```text
//!   A  waitpid'siz geldi -> cocuk cikti, isleyici kostu (sorulmadan)
//!   B  kim ve nasil      -> si_pid cocugun kimligi, CLD_EXITED,
//!                           si_status cikis kodu
//!   C  sinyalle olum     -> CLD_KILLED ve si_status olduren sinyal
//!   D  varsayilan yok    -> isleyicisiz bir SIGCHLD sureci OLDURMUYOR
//!      saymak
//!   E  durdu / devam etti-> CLD_STOPPED ve CLD_CONTINUED geliyor
//!   F  SA_NOCLDSTOP      -> durma bildirilmiyor, olum bildiriliyor
//!   G  SIG_IGN           -> zombi kalmiyor, waitpid ECHILD doner
//! ```
//!
//! A bu sinavin sebebi. B onun tamamlayicisi ve ayri bir sey olcuyor:
//! "sinyal geldi" ile "dogru sinyal geldi" ayni sey degil -- kayit
//! bosken de isleyici kosardi.
//!
//! D kolayca atlanabilecek olani. Varsayilani yanlis olan bir cekirdekte
//! `fork` eden **her** program cocugu bitince olurdu; yani D aslinda
//! butun otekilerin zeminini olcuyor.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::signal::{self, SigInfo, UContext};
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0010_1A16;
const PANEL: u32 = 0x001C_2A24;
const FG: u32 = 0x00E2_EEE8;
const DIM: u32 = 0x0088_9C94;
const ACCENT: u32 = 0x0070_E0C0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// `-ECHILD`: beklenecek cocuk yok.
const ECHILD: isize = -10;

/// Isleyici kac kez kostu.
static CHLD_COUNT: AtomicUsize = AtomicUsize::new(0);
/// Son gelen kaydin alanlari.
static LAST_PID: AtomicUsize = AtomicUsize::new(usize::MAX);
static LAST_CODE: AtomicUsize = AtomicUsize::new(usize::MAX);
static LAST_STATUS: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Gorulen butun `si_code`lar, gelis sirasinda.
static CODES: [AtomicUsize; 6] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
static CODE_COUNT: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_chld(_signo: u32, info: *const SigInfo, _context: *mut UContext) {
    CHLD_COUNT.fetch_add(1, Ordering::SeqCst);
    // SAFETY: cekirdek `SA_SIGINFO` isleyicisine gecerli bir kayit verir.
    let (code, pid, status) = unsafe { ((*info).code, (*info).pid(), (*info).status()) };
    LAST_PID.store(pid, Ordering::SeqCst);
    LAST_CODE.store(code as isize as usize, Ordering::SeqCst);
    LAST_STATUS.store(status as usize, Ordering::SeqCst);
    let slot = CODE_COUNT.fetch_add(1, Ordering::SeqCst);
    if slot < CODES.len() {
        CODES[slot].store(code as isize as usize, Ordering::SeqCst);
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

const NAMES: [&str; 7] = [
    "A waitpid'siz geldi",
    "B kim ve nasil",
    "C sinyalle olum",
    "D varsayilan yok saymak",
    "E durdu / devam etti",
    "F SA_NOCLDSTOP",
    "G SIG_IGN zombi birakmaz",
];

/// Bir sinavin sonucunu **hesaplandigi anda** yazar.
///
/// Bu sinav cocuk bekliyor ve beklemeler asilabilir; yalnizca sonda
/// yazan bir sinav, bozuk bir cekirdekte hangi adimda sustugunu bile
/// soyleyemez.
fn say(check: &Check) {
    use core::fmt::Write;
    let mut console = Stdout;
    let _ = writeln!(
        console,
        "[sigchld] {}: {} ({})",
        check.name,
        if check.passed { "gecti" } else { "KALDI" },
        check.detail
    );
}

fn reset() {
    CHLD_COUNT.store(0, Ordering::SeqCst);
    LAST_PID.store(usize::MAX, Ordering::SeqCst);
    LAST_CODE.store(usize::MAX, Ordering::SeqCst);
    LAST_STATUS.store(usize::MAX, Ordering::SeqCst);
    CODE_COUNT.store(0, Ordering::SeqCst);
    for slot in CODES.iter() {
        slot.store(0, Ordering::SeqCst);
    }
}

/// Isleyicinin kosmasini **sinirli** sure bekler.
///
/// Sinyal teslimi bir sistem cagrisinin donusunde olur, yani bekleme
/// bir syscall icermek zorunda. `sleep_ms` hem onu sagliyor hem de
/// yoklama dongusune dusmuyor.
fn wait_for_signals(count: usize) -> bool {
    for _ in 0..60 {
        if CHLD_COUNT.load(Ordering::SeqCst) >= count {
            return true;
        }
        sys::sleep_ms(50);
    }
    false
}

/// `exit(code)` ile cikan bir cocuk uretir.
fn spawn_exiting(code: i32) -> isize {
    let child = sys::fork();
    if child == 0 {
        sys::sleep_ms(30);
        sys::exit(code);
    }
    child
}

fn main() {
    use core::fmt::Write;
    let _ = writeln!(Stdout, "[sigchld] sinav basliyor");
    let mut checks = [EMPTY; 7];

    let installed = signal::action_info(signal::SIGCHLD, on_chld, 0, 0) >= 0;

    // --- A ve B: waitpid cagirmadan haber almak ----------------------
    //
    // Cocuk bilerek **toplanmiyor**: olculen sey tam olarak "sormadan
    // ogrendim mi". `waitpid` once cagrilsaydi sinyalin hicbir katkisi
    // gorunmezdi.
    reset();
    let child = spawn_exiting(42);
    let arrived = child > 0 && wait_for_signals(1);
    checks[0] = Check {
        name: NAMES[0],
        detail: if !installed {
            "SIGCHLD isleyicisi kurulamadi"
        } else if child < 0 {
            "cocuk surec acilamadi"
        } else if arrived {
            "cocuk cikti, isleyici waitpid'siz kostu"
        } else {
            "isleyici KOSMADI: bildirim yok"
        },
        passed: installed && arrived,
    };
    say(&checks[0]);

    let pid = LAST_PID.load(Ordering::SeqCst);
    let code = LAST_CODE.load(Ordering::SeqCst) as isize as i32;
    let status = LAST_STATUS.load(Ordering::SeqCst);
    let b_ok = arrived && pid == child as usize && code == signal::CLD_EXITED && status == 42;
    checks[1] = Check {
        name: NAMES[1],
        detail: if !arrived {
            "sinyal gelmedi, kayit okunamadi"
        } else if b_ok {
            "si_pid cocuk, CLD_EXITED, si_status 42"
        } else if pid != child as usize {
            "si_pid cocugun kimligi degil"
        } else if code != signal::CLD_EXITED {
            "si_code CLD_EXITED degil"
        } else {
            "si_status cikis kodunu tasimiyor"
        },
        passed: b_ok,
    };
    say(&checks[1]);
    // Zombiyi topla: sinavin geri kalani temiz yuvalarla kossun.
    let mut ignored = 0u32;
    if child > 0 {
        sys::waitpid(child as usize, &mut ignored, 0);
    }

    // --- C: sinyalle olen cocuk --------------------------------------
    //
    // Ayni olay, baska bir sebep. `waitpid`in durum kelimesinde bu ayrim
    // `WIFSIGNALED` ile yapiliyordu; burada `si_code` dogrudan soyluyor.
    reset();
    let victim = sys::fork();
    if victim == 0 {
        // Cocuk: oldurulene kadar uyu.
        for _ in 0..200 {
            sys::sleep_ms(50);
        }
        sys::exit(0);
    }
    let mut c_ok = false;
    let mut c_code = 0i32;
    let mut c_status = 0usize;
    if victim > 0 {
        sys::sleep_ms(60);
        signal::kill(victim as usize, signal::SIGKILL);
        if wait_for_signals(1) {
            c_code = LAST_CODE.load(Ordering::SeqCst) as isize as i32;
            c_status = LAST_STATUS.load(Ordering::SeqCst);
            c_ok = c_code == signal::CLD_KILLED && c_status == signal::SIGKILL as usize;
        }
        sys::waitpid(victim as usize, &mut ignored, 0);
    }
    checks[2] = Check {
        name: NAMES[2],
        detail: if victim < 0 {
            "cocuk surec acilamadi"
        } else if c_ok {
            "CLD_KILLED ve si_status olduren sinyal"
        } else if c_code == signal::CLD_EXITED {
            "olum CLD_EXITED diye bildirildi"
        } else if c_code == 0 {
            "sinyal hic gelmedi"
        } else {
            "si_status olduren sinyali tasimiyor"
        },
        passed: c_ok,
    };
    say(&checks[2]);

    // --- D: varsayilan davranis yok saymak ---------------------------
    //
    // Butun otekilerin zemini. Varsayilani yanlis olan bir cekirdekte
    // `fork` eden **her** program cocugu bitince olurdu -- yani bu sinav
    // kalirsa digerlerinin gectigi de kuskulu olurdu.
    //
    // Olcum cocuk surecte: varsayilan yanlissa oradaki surec oler ve
    // cikis kodu bunu soyler. Ebeveynde denemek sinavi bitirirdi.
    // Varsayilan **cocukta** geri aliniyor, ebeveynde degil. Ilk
    // yazilista ebeveynde yapiliyordu ve bozma sinavi bunu yakaladi:
    // varsayilan olum olunca prober'in cikisi **ebeveyni** olduruyor ve
    // sinav D'ye varamadan susuyordu. `fork` yerlestirmeleri devrettigi
    // icin cocukta yapmak yeterli.
    let prober = sys::fork();
    if prober == 0 {
        signal::default(signal::SIGCHLD);
        // Torun: hemen cikar ve dedenin degil, babanin cocugudur.
        let grandchild = sys::fork();
        if grandchild == 0 {
            sys::exit(7);
        }
        // Varsayilan yok saymaksa buraya kadar yasariz.
        for _ in 0..20 {
            sys::sleep_ms(20);
        }
        let mut st = 0u32;
        sys::waitpid(grandchild as usize, &mut st, 0);
        sys::exit(D_SURVIVED);
    }
    let d_code = reap_bounded(prober);
    let d_ok = d_code == D_SURVIVED as u32;
    checks[3] = Check {
        name: NAMES[3],
        detail: if prober < 0 {
            "cocuk surec acilamadi"
        } else if d_ok {
            "isleyicisiz SIGCHLD sureci oldurmedi"
        } else if d_code == D_KILLED {
            "isleyicisiz SIGCHLD sureci OLDURDU"
        } else if d_code == 0 {
            "cocuk cevap vermedi"
        } else {
            "cocuk beklenmedik bir kodla cikti"
        },
        passed: d_ok,
    };
    say(&checks[3]);

    // --- E: durdu ve devam etti --------------------------------------
    reset();
    let job = sys::fork();
    if job == 0 {
        for _ in 0..200 {
            sys::sleep_ms(50);
        }
        sys::exit(0);
    }
    let mut e_ok = false;
    if job > 0 {
        sys::sleep_ms(60);
        signal::kill(job as usize, signal::SIGSTOP);
        let stopped = wait_for_signals(1);
        signal::kill(job as usize, signal::SIGCONT);
        let both = stopped && wait_for_signals(2);
        let seen = [
            CODES[0].load(Ordering::SeqCst) as isize as i32,
            CODES[1].load(Ordering::SeqCst) as isize as i32,
        ];
        e_ok = both && seen == [signal::CLD_STOPPED, signal::CLD_CONTINUED];
        signal::kill(job as usize, signal::SIGKILL);
        sys::waitpid(job as usize, &mut ignored, 0);
    }
    checks[4] = Check {
        name: NAMES[4],
        detail: if job < 0 {
            "cocuk surec acilamadi"
        } else if e_ok {
            "CLD_STOPPED sonra CLD_CONTINUED geldi"
        } else if CHLD_COUNT.load(Ordering::SeqCst) == 0 {
            "durma/devam hic bildirilmedi"
        } else {
            "gelen kodlar beklenenden farkli"
        },
        passed: e_ok,
    };
    say(&checks[4]);

    // --- F: SA_NOCLDSTOP durmayi bastirir, olumu bastirmaz -----------
    //
    // Iki yari birlikte olcmek zorunda: yalnizca "durma gelmedi"
    // bakilsaydi, hicbir sey gondermeyen bir cekirdek de gecerdi.
    reset();
    signal::action_info(signal::SIGCHLD, on_chld, signal::SA_NOCLDSTOP, 0);
    let quiet = sys::fork();
    let mut f_ok = false;
    let mut after_stop = usize::MAX;
    if quiet > 0 {
        sys::sleep_ms(60);
        signal::kill(quiet as usize, signal::SIGSTOP);
        sys::sleep_ms(200);
        after_stop = CHLD_COUNT.load(Ordering::SeqCst);
        signal::kill(quiet as usize, signal::SIGCONT);
        sys::sleep_ms(100);
        signal::kill(quiet as usize, signal::SIGKILL);
        let died = wait_for_signals(1);
        f_ok = after_stop == 0 && died;
        sys::waitpid(quiet as usize, &mut ignored, 0);
    } else if quiet == 0 {
        for _ in 0..200 {
            sys::sleep_ms(50);
        }
        sys::exit(0);
    }
    checks[5] = Check {
        name: NAMES[5],
        detail: if quiet < 0 {
            "cocuk surec acilamadi"
        } else if f_ok {
            "durma bastirildi, olum yine bildirildi"
        } else if after_stop != 0 {
            "SA_NOCLDSTOP'a ragmen durma bildirildi"
        } else {
            "olum de bildirilmedi: bayrak fazlasini bastiriyor"
        },
        passed: f_ok,
    };
    say(&checks[5]);

    // --- G: SIG_IGN zombi birakmaz -----------------------------------
    //
    // POSIX'in tuhafligi: yok saymak burada sinyali degil **kaydi**
    // siliyor. Olcum `waitpid`in `ECHILD` donmesi -- yani cekirdegin
    // "boyle bir cocuk yok" demesi.
    signal::ignore(signal::SIGCHLD);
    let orphan = spawn_exiting(3);
    let mut g_result = 0isize;
    if orphan > 0 {
        sys::sleep_ms(200);
        let mut st = 0u32;
        g_result = sys::waitpid(orphan as usize, &mut st, 0);
    }
    let g_ok = orphan > 0 && g_result == ECHILD;
    checks[6] = Check {
        name: NAMES[6],
        detail: if orphan < 0 {
            "cocuk surec acilamadi"
        } else if g_ok {
            "zombi kalmadi, waitpid ECHILD dondu"
        } else if g_result > 0 {
            "cocuk ZOMBI kaldi: yok saymak toplamiyor"
        } else {
            "waitpid beklenmedik bir deger dondu"
        },
        passed: g_ok,
    };
    say(&checks[6]);
    signal::default(signal::SIGCHLD);

    let passed = checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(Stdout, "[sigchld] sonuc: {}/7 gecti", passed);
    let _ = writeln!(
        Stdout,
        "[sigchld] son kayit: pid={} code={} status={}  toplam sinyal={}",
        LAST_PID.load(Ordering::SeqCst),
        LAST_CODE.load(Ordering::SeqCst) as isize,
        LAST_STATUS.load(Ordering::SeqCst),
        CHLD_COUNT.load(Ordering::SeqCst)
    );
    show(&checks);
}

/// D cocugunun "hayatta kaldim" cevabi.
const D_SURVIVED: i32 = 0x55;

/// Cocuk bir **sinyalle** oldu.
///
/// "Hic cevap vermedi" ile ayri tutuluyor ve ayrim sinavin teshisini
/// belirliyor: ikisi de sifir donseydi D "cocuk cevap vermedi" derdi,
/// oysa asil cevap "isleyicisiz SIGCHLD sureci oldurdu".
const D_KILLED: u32 = 0xDEAD;

/// Cocugu **sinirli** sure bekler.
///
/// Doner: cikis kodu, sinyalle oldu ise `D_KILLED`, hic donmediyse 0.
fn reap_bounded(child: isize) -> u32 {
    if child <= 0 {
        return 0;
    }
    let mut status = 0u32;
    for _ in 0..60 {
        if sys::waitpid(child as usize, &mut status, sys::WNOHANG) > 0 {
            return if sys::exited(status) {
                sys::exit_status(status)
            } else {
                D_KILLED
            };
        }
        sys::sleep_ms(50);
    }
    0
}

fn show(checks: &[Check; 7]) {
    let mut win = match Window::open("sigchld -- itilen bilgi", 260, 130, 480, 250) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, checks);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 7]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Ebeveyn artik sormadan ogreniyor", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            390,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    win.text(6, h - 46, "POSIX:   cocuk oldu -> sinyal ITILIR", DIM);
    win.text(6, h - 30, "Windows: cocuk oldu -> nesne isaretlenir, CEKILIR", DIM);

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
