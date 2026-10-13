//! `jobctl` -- terminali kim okuyabilir: oturum, on plan, `SIGTTIN`.
//!
//! Surec gruplari onceki bir batida geldi: `setpgid`, `kill(-pgid)`,
//! `waitpid(-pgid)`, `SIGSTOP`/`SIGCONT`. Eksik kalan **terminal**
//! tarafiydi, ve is denetiminin asil anlami orada:
//!
//! ```text
//!   $ make &          <- arka plan isi
//!   $ vim             <- on plan isi, klavyeyi o okuyor
//! ```
//!
//! Iki is ayni terminali paylasiyor. Klavyenin `vim`e gitmesi, `make`in
//! ise ona **dokunamamasi** gerekiyor:
//!
//! ```text
//!   oturum (sid)         bir terminali paylasan islerin kumesi
//!   on plan grubu        terminali OKUYABILEN tek grup
//!   SIGTTIN / SIGTTOU    arka plandan okumaya / ayara dokunmaya
//!                        calisan grubu durdurur
//! ```
//!
//! ## TCMK'de terminal: liderin penceresi
//!
//! Bu sinav **kendi penceresini** terminal olarak kullaniyor ve o
//! yuzden pencereyi ilk is olarak aciyor: oturum liderinin penceresi
//! yoksa terminal de yoktur. Cocuklarin penceresi yok; okuduklari sey
//! ebeveynin penceresi, yani paylasilan terminal.
//!
//! ## Sekiz sinav
//!
//! ```text
//!   A  setsid            -> grup lideri EPERM; lider olmayan cocuk yeni
//!                           oturum kurar (sid = pgid = pid)
//!   B  oturum mirasi     -> cocuk oturumu devralir; setsid sonrasi
//!                           terminali yok (ENOTTY, okuma = dosya sonu)
//!   C  on plan sahibi    -> tcgetpgrp = kendi grubu; on plani BASKA
//!                           oturumdaki gruba vermek EPERM
//!   D  ON PLAN okur      -> on plana alinan cocugun okumasi izinli
//!   E  ARKA PLAN DURUR   -> arka plandan okuyan cocuk SIGTTIN ile durur
//!   F  yok sayilinca EIO -> SIGTTIN yok sayiliysa durmaz, okuma EIO
//!   G  SIGTTOU           -> arka plandan tcsetpgrp SIGTTOU ile durur;
//!                           SIGTTOU engelliyse IZIN verilir
//!   H  yeniden baslar    -> durdurulan okuma, on plana alinip devam
//!                           edince EINTR degil, yeniden calisir
//!   I  SIGCHLD bolmez    -> cocuk olunce baska bir sinyali bekleyen
//!                           sigtimedwait BOLUNMUYOR (varsayilan yok sayma)
//!   J  akis grubu        -> is parcacigi surecin grubunda; terminali
//!                           okurken arka planda sanilmiyor
//!   K  olen on plan      -> on plan grubu olunce on plan BOSALIR; ayni
//!                           numarayla gelen yeni is onu miras almaz
//! ```
//!
//! K bir bozma sinavinin buldugu hata. Ebeveyn terminali geri
//! alamayinca on plan olmus bir cocugun grubunda kaldi; sonraki cocuk o
//! yuvayi -- ve dolayisiyla o grup numarasini -- alinca hic kimse
//! `tcsetpgrp` cagirmadan **on plana gecmis** oldu ve arka plandan
//! okuyup durmadi. TCMK'de grup bir sayi oldugu icin numaranin yeniden
//! kullanilmasi tek basina yetiyordu.
//!
//! I ve J bu batinin yan urunu olan iki duzeltmeyi olcuyor. Ikisi de
//! eskiden beri duran hatalardi ama terminal gelene kadar gorunmuyordu:
//!
//! * Varsayilani yok saymak olan bir sinyal (`SIGCHLD`) bekleyen
//!   cagrilari **boluyordu**. Cocugu olan ve klavyeyi bekleyen her
//!   surec, cocuk bittiginde istemedigi bir `EINTR` aliyordu. Bu
//!   batida iki katmandan kapandi ve I yalnizca birini olcebiliyor
//!   (bkz. I'nin aciklamasi).
//! * Is parcaciklari kendi numaralarini surec grubu olarak
//!   tasiyordu. Terminal gelince bunun sonucu belirdi: ana akis
//!   disindaki bir akis klavyeyi okuyunca kendini arka planda sanip
//!   `SIGTTIN` alirdi.
//!
//! F ile G'nin ikinci yarisi ayni soruyu soruyor ve **zit** cevap
//! bekliyor: sinyal yok sayili/engelliyken okuma reddediliyor, ayar
//! degisikligi kabul ediliyor. Kabuklar ikinciye dayaniyor -- bir isi on
//! plana verdikten sonra kabuk kendisi arka plandadir ve terminali geri
//! almak icin `SIGTTOU`yu engeller. Bu sinav da ayni seyi yapiyor
//! (`take_terminal_back`).
//!
//! H bu batinin ortaya cikardigi bir hatayi olcuyor: varsayilani
//! durdurmak olan bir sinyalin boldugu cagri, `SIGCONT` sonrasi
//! `EINTR` ile donuyordu. Linux onu sessizce yeniden baslatir. Fark
//! is denetiminin kalbinde: arka plandan okuyan bir is `fg` ile on
//! plana alindiginda okumasi kaldigi yerden surmeli.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::signal;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0016_1418;
const PANEL: u32 = 0x0024_2028;
const FG: u32 = 0x00EC_E6EA;
const DIM: u32 = 0x009A_8E96;
const ACCENT: u32 = 0x00F0_B080;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

const EPERM: isize = 1;
const ESRCH: isize = 3;
const EINTR: isize = 4;
const EIO: isize = 5;
const EAGAIN: isize = 11;
const ENOTTY: isize = 25;

/// Cocuk cevaplarinin isaret biti: "cocuk cevap verdi".
///
/// Sifir bir cevap olamaz: cocuk olurse ya da hic cikamazsa cikis kodu
/// zaten sifir olur ve "her sey kaldi" ile "cocuk oldu" ayirt
/// edilemezdi.
const DONE: u32 = 0x40;

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

const NAMES: [&str; 11] = [
    "A setsid",
    "B oturum mirasi",
    "C on plan sahibi",
    "D ON PLAN okur",
    "E ARKA PLAN DURUR",
    "F yok sayilinca EIO",
    "G SIGTTOU",
    "H yeniden baslar",
    "I SIGCHLD bolmez",
    "J akis grubu",
    "K olen on plan",
];

fn say(check: &Check) {
    use core::fmt::Write;
    let mut console = Stdout;
    let _ = writeln!(
        console,
        "[jobctl] {}: {} ({})",
        check.name,
        if check.passed { "gecti" } else { "KALDI" },
        check.detail
    );
}

/// Cocugun bir sonraki **durum degisikligini** sinirli sure bekler.
///
/// `WUNTRACED`: durma da bir durum degisikligi. Doner: ham durum
/// kelimesi, ya da hic degismediyse `None`.
fn wait_change(child: usize, ms: usize) -> Option<u32> {
    let mut status = 0u32;
    for _ in 0..(ms / 20).max(1) {
        if sys::waitpid(child, &mut status, sys::WNOHANG | sys::WUNTRACED) > 0 {
            return Some(status);
        }
        sys::sleep_ms(20);
    }
    None
}

/// Cocugun cikis kodunu sinirli sure bekler. Durursa ya da olurse
/// `None`; cevap vermezse oldurulup `None`.
fn wait_exit(child: usize, ms: usize) -> Option<u32> {
    match wait_change(child, ms) {
        Some(status) if sys::exited(status) => Some(sys::exit_status(status)),
        Some(status) if sys::stopped(status) => {
            // Cikmasi beklenirken durdu: birakilirsa sonsuza kadar
            // durur ve yuvasi bosalmaz.
            signal::kill(child, signal::SIGKILL);
            let _ = wait_change(child, 500);
            None
        }
        Some(_) => None,
        None => {
            signal::kill(child, signal::SIGKILL);
            let _ = wait_change(child, 500);
            None
        }
    }
}

/// Cocuk icin: kendi grubuna gec, cevap kodunu cikis koduna yaz.
fn own_group() {
    signal::setpgid(0, 0);
}

/// Hata kodunu cocugun cikis koduna sigdirir: `DONE | errno`.
fn encode(r: isize) -> i32 {
    let low = if r < 0 { (-r) as u32 & 0x3F } else { 0x3F };
    (DONE | low) as i32
}

/// Cevap kodunu cozer: okuma sonucu. `0x3F` "veri geldi".
fn decode(code: u32) -> Option<isize> {
    if code & DONE == 0 {
        return None;
    }
    let low = (code & 0x3F) as isize;
    Some(if low == 0x3F { 1 } else { -low })
}

/// Terminali **geri alir**: kabugun bir isi on plana verdikten sonra
/// yaptigi sey.
///
/// Cagiran artik arka planda; `tcsetpgrp` duz cagrilsaydi kendi grubu
/// `SIGTTOU` ile dururdu. Kabuklar sinyali engelleyerek bunu asar, ve
/// cekirdek tam o durumda **izin** vermek zorunda (G sinavi).
fn take_terminal_back(my_pgid: usize) -> isize {
    let old = signal::sigprocmask(signal::SIG_BLOCK, signal::mask_of(signal::SIGTTOU));
    let r = signal::tcsetpgrp(sys::STDIN, my_pgid);
    signal::sigprocmask(signal::SIG_SETMASK, old);
    r
}

fn main() {
    use core::fmt::Write;
    let _ = writeln!(Stdout, "[jobctl] sinav basliyor");
    let mut checks = [EMPTY; 11];

    // Pencere **ilk is**: oturumun terminali liderinin penceresi.
    let mut win = Window::open("jobctl -- terminali kim okur", 260, 120, 480, 250);
    if win.is_none() {
        for (i, check) in checks.iter_mut().enumerate() {
            *check = Check {
                name: NAMES[i],
                detail: "pencere acilamadi: terminal yok",
                passed: false,
            };
            say(check);
        }
        return;
    }

    let me = signal::getpid();
    let my_pgid = signal::getpgid(0) as usize;
    let my_sid = signal::getsid(0);

    // Ebeveyn `SIGTTOU`yu **bastan sona** engelliyor -- etkilesimli bir
    // kabugun yaptigi gibi.
    //
    // Ilk yazilista yalnizca terminali geri alirken engelliyordu. Bozma
    // sinavi (engelli `SIGTTOU`ya izin verilmeyen bir cekirdek) bunun
    // neden yetmedigini gosterdi: geri alma basarisiz oldu, ebeveyn
    // arka planda kaldi, ve bir sonraki sinavda bir cocuga on plani
    // vermek isterken **kendisi** `SIGTTOU` ile durdu. Sinav tek satir
    // yazmadan sustu. Kabuklar bu yuzden is denetimi sinyallerini
    // surekli engeller/yok sayar: terminali yoneten taraf hicbir
    // zaman durdurulmamali.
    //
    // Cocuklar maskeyi devraliyor; durmasi **beklenen** G1 onu acikca
    // kaldiriyor.
    let saved_mask = signal::sigprocmask(signal::SIG_BLOCK, signal::mask_of(signal::SIGTTOU));
    // Ayni gerekceyle `SIGTTIN` de **yok sayiliyor**. Ikinci bir bozma
    // sinavinda ebeveyn terminali geri alamadi ve kendi grubundaki bir
    // cocuk terminali okudu: `SIGTTIN` butun gruba gidiyor, yani
    // **sinavin kendisi** de durdu ve yine sustu. Etkilesimli kabuklar
    // bu yuzden `SIGTTIN`/`SIGTTOU`/`SIGTSTP`yi yok sayar. Durmasi
    // beklenen cocuklar (E/H, J, K) varsayilani geri koyuyor.
    signal::ignore(signal::SIGTTIN);

    // --- A, B, C: oturum --------------------------------------------
    //
    // Cocuk S oturumun disina cikiyor. Cevabini cikis kodunda veriyor,
    // ama ebeveynin ona C icin **canliyken** ihtiyaci var: on plani
    // baska oturumdaki bir gruba vermeye calismak. Bu yuzden S
    // setsid'den sonra bir sure bekliyor.
    const S_INHERITED: u32 = 0x01;
    const S_NEW_SESSION: u32 = 0x02;
    const S_NO_TTY: u32 = 0x04;
    const S_EOF: u32 = 0x08;

    let leader_setsid = signal::setsid();
    let s = sys::fork();
    if s == 0 {
        let mut bits = DONE;
        if signal::getsid(0) == my_sid {
            bits |= S_INHERITED;
        }
        let sid = signal::setsid();
        let pid = signal::getpid() as isize;
        if sid == pid && signal::getpgid(0) == pid && signal::getsid(0) == pid {
            bits |= S_NEW_SESSION;
        }
        if signal::tcgetpgrp(sys::STDIN) == -ENOTTY {
            bits |= S_NO_TTY;
        }
        // Terminali yok: okuma **dosya sonu** dondurmeli, uyumamali.
        // Uyusaydi kimse uyandirmazdi -- tus gelecek bir yer yok.
        let mut buf = [0u8; 4];
        if sys::read(sys::STDIN, &mut buf) == 0 {
            bits |= S_EOF;
        }
        sys::sleep_ms(800);
        sys::exit(bits as i32);
    }

    // C: once kendi terminalimizin on plani, sonra baska oturuma devir.
    let fg_is_mine = signal::tcgetpgrp(sys::STDIN) == my_pgid as isize;
    let mut other_session = false;
    let mut cross = 0isize;
    if s > 0 {
        for _ in 0..40 {
            if signal::getsid(s as usize) == s {
                other_session = true;
                break;
            }
            sys::sleep_ms(10);
        }
        if other_session {
            cross = signal::tcsetpgrp(sys::STDIN, s as usize);
        }
    }
    let s_bits = if s > 0 {
        wait_exit(s as usize, 3000).unwrap_or(0)
    } else {
        0
    };
    let s_answered = s_bits & DONE != 0;

    let a = leader_setsid == -EPERM && s_answered && s_bits & S_NEW_SESSION != 0;
    checks[0] = Check {
        name: NAMES[0],
        detail: if a {
            "lider EPERM aldi, cocuk yeni oturum kurdu"
        } else if leader_setsid >= 0 {
            "grup LIDERI setsid yapabildi: grup iki oturuma yayildi"
        } else if !s_answered {
            "cocuk cevap vermedi"
        } else {
            "cocugun yeni oturumu sid = pgid = pid degil"
        },
        passed: a,
    };
    say(&checks[0]);

    let b = s_answered && s_bits & (S_INHERITED | S_NO_TTY | S_EOF) == S_INHERITED | S_NO_TTY | S_EOF;
    checks[1] = Check {
        name: NAMES[1],
        detail: if b {
            "oturum devralindi; setsid sonrasi terminal yok, okuma dosya sonu"
        } else if !s_answered {
            "cocuk cevap vermedi: terminalsiz okuma UYUYOR olabilir"
        } else if s_bits & S_INHERITED == 0 {
            "cocuk ebeveynin oturumunu DEVRALMADI"
        } else if s_bits & S_NO_TTY == 0 {
            "setsid sonrasi terminal HALA bagli"
        } else {
            "terminalsiz okuma dosya sonu DONMEDI"
        },
        passed: b,
    };
    say(&checks[1]);

    let c = fg_is_mine && other_session && cross == -EPERM;
    checks[2] = Check {
        name: NAMES[2],
        detail: if c {
            "on plan kendi grubum; baska oturuma devir EPERM"
        } else if !fg_is_mine {
            "tcgetpgrp kendi grubumu VERMEDI"
        } else if !other_session {
            "cocugun yeni oturumu gorulemedi"
        } else if cross == 0 {
            "on plan BASKA OTURUMDAKI gruba verildi"
        } else if cross == -ESRCH {
            "grup bulunamadi (ESRCH): oturum denetimi yerine var olma denetimi"
        } else {
            "beklenmedik hata"
        },
        passed: c,
    };
    say(&checks[2]);

    // --- D: on plana alinan cocuk okuyabilir --------------------------
    //
    // Cocuk kendi grubuna geciyor, ebeveyn ona on plani veriyor, cocuk
    // on planda oldugunu **gordukten sonra** okuyor. Bloke olmayan bir
    // okuma: tus yoksa `EAGAIN` -- "izinli ama bos". Arka planda olsaydi
    // `EAGAIN` degil `SIGTTIN` olurdu, cunku is denetimi veri olup
    // olmadigindan once bakiliyor.
    let d_child = sys::fork();
    if d_child == 0 {
        own_group();
        sys::set_flags(sys::STDIN, sys::O_NONBLOCK);
        let pgid = signal::getpgid(0);
        let mut mine = false;
        for _ in 0..100 {
            if signal::tcgetpgrp(sys::STDIN) == pgid {
                mine = true;
                break;
            }
            sys::sleep_ms(10);
        }
        if !mine {
            sys::exit(DONE as i32);
        }
        let mut buf = [0u8; 4];
        let r = sys::read(sys::STDIN, &mut buf);
        sys::exit(encode(r));
    }
    let mut d_given = -1isize;
    if d_child > 0 {
        signal::setpgid(d_child as usize, d_child as usize);
        d_given = signal::tcsetpgrp(sys::STDIN, d_child as usize);
    }
    let d_code = if d_child > 0 {
        wait_exit(d_child as usize, 3000)
    } else {
        None
    };
    let d_back = take_terminal_back(my_pgid);
    let d_read = d_code.and_then(decode);
    let d = d_given == 0 && matches!(d_read, Some(r) if r == -EAGAIN || r > 0);
    checks[3] = Check {
        name: NAMES[3],
        detail: if d {
            "on plandaki cocugun okumasi izinli (bos: EAGAIN)"
        } else if d_given != 0 {
            "on plan cocuga VERILEMEDI"
        } else if d_code.is_none() {
            "cocuk cikmadi: on planda okurken DURDURULDU"
        } else if d_code == Some(DONE) {
            "cocuk kendini on planda hic gormedi"
        } else if d_read == Some(-EIO) {
            "on plandaki okuma EIO aldi"
        } else {
            "beklenmedik okuma sonucu"
        },
        passed: d,
    };
    say(&checks[3]);

    // --- E + H: arka plandan okumak, sonra on plana gelmek ------------
    //
    // Cocuk kendi grubunda (arka planda) okuyor ve `SIGTTIN` ile
    // durmali (E). Ebeveyn sonra ona on plani verip devam ettiriyor;
    // okuma **yeniden baslamali** ve bu kez izinli olmali (H). `EINTR`
    // gelirse cagri yeniden baslatilmamis demektir.
    let r_child = sys::fork();
    if r_child == 0 {
        own_group();
        signal::default(signal::SIGTTIN);
        sys::set_flags(sys::STDIN, sys::O_NONBLOCK);
        let mut buf = [0u8; 4];
        let r = sys::read(sys::STDIN, &mut buf);
        sys::exit(encode(r));
    }
    let mut stop_sig = 0u32;
    let mut r_code: Option<u32> = None;
    let mut r_given = -1isize;
    let mut r_never_stopped = false;
    if r_child > 0 {
        let r = r_child as usize;
        signal::setpgid(r, r);
        match wait_change(r, 2000) {
            Some(status) if sys::stopped(status) => {
                stop_sig = sys::stop_signal(status);
                r_given = signal::tcsetpgrp(sys::STDIN, r);
                signal::kill(r, signal::SIGCONT);
                r_code = wait_exit(r, 3000);
            }
            Some(status) if sys::exited(status) => {
                r_never_stopped = true;
                r_code = Some(sys::exit_status(status));
            }
            _ => {
                signal::kill(r, signal::SIGKILL);
                let _ = wait_change(r, 500);
            }
        }
    }
    take_terminal_back(my_pgid);

    let e = stop_sig == signal::SIGTTIN;
    checks[4] = Check {
        name: NAMES[4],
        detail: if e {
            "arka plandan okuyan cocuk SIGTTIN ile durdu"
        } else if r_never_stopped {
            "cocuk DURMADI: arka plandan terminali okudu"
        } else if stop_sig != 0 {
            "cocuk durdu ama SIGTTIN ile degil"
        } else {
            "cocuk ne durdu ne cikti"
        },
        passed: e,
    };
    say(&checks[4]);

    let h_read = r_code.and_then(decode);
    let h = e && r_given == 0 && matches!(h_read, Some(r) if r == -EAGAIN || r > 0);
    checks[7] = Check {
        name: NAMES[7],
        detail: if h {
            "on plana alinip devam edince okuma yeniden calisti"
        } else if !e {
            "cocuk durmadi: olculemedi"
        } else if r_given != 0 {
            "on plan cocuga verilemedi"
        } else if h_read == Some(-EINTR) {
            "okuma EINTR dondu: cagri YENIDEN BASLATILMADI"
        } else if r_code.is_none() {
            "devam ettirilen cocuk cikmadi: yine durdu"
        } else {
            "beklenmedik okuma sonucu"
        },
        passed: h,
    };

    // --- F: SIGTTIN yok sayiliysa EIO ---------------------------------
    let f_child = sys::fork();
    if f_child == 0 {
        own_group();
        signal::ignore(signal::SIGTTIN);
        let mut buf = [0u8; 4];
        let r = sys::read(sys::STDIN, &mut buf);
        sys::exit(encode(r));
    }
    let mut f_stopped = false;
    let mut f_hung = false;
    let mut f_code: Option<u32> = None;
    if f_child > 0 {
        let f = f_child as usize;
        signal::setpgid(f, f);
        match wait_change(f, 2000) {
            Some(status) if sys::exited(status) => f_code = Some(sys::exit_status(status)),
            Some(status) if sys::stopped(status) => {
                f_stopped = true;
                signal::kill(f, signal::SIGKILL);
                let _ = wait_change(f, 500);
            }
            Some(_) => {}
            None => {
                // Ne durdu ne cikti: terminali **bekliyor**. Yani arka
                // plandan okuma hic engellenmedi -- cocuk on plan isinin
                // klavyesini bekleyen siradan bir okuyucu gibi uyudu.
                f_hung = true;
                signal::kill(f, signal::SIGKILL);
                let _ = wait_change(f, 500);
            }
        }
    }
    let f_read = f_code.and_then(decode);
    let f = f_read == Some(-EIO);
    checks[5] = Check {
        name: NAMES[5],
        detail: if f {
            "SIGTTIN yok sayili: durmadi, okuma EIO"
        } else if f_stopped {
            "yok sayilan SIGTTIN yine de DURDURDU"
        } else if f_hung {
            "cocuk terminali BEKLEDI: arka plandan okuma engellenmedi"
        } else if f_code.is_none() {
            "cocuk cikmadi"
        } else if matches!(f_read, Some(r) if r == -EAGAIN || r >= 0) {
            "arka plan cocugu terminali OKUYABILDI"
        } else {
            "beklenmedik okuma sonucu"
        },
        passed: f,
    };
    say(&checks[5]);

    // --- G: arka plandan tcsetpgrp ------------------------------------
    //
    // Iki yari, ayni cagri, zit beklenti. Birinci cocuk sinyali
    // engellemeden deniyor ve durmali; ikinci engelleyip deniyor ve
    // **basarmali**. Ikinci yari kabuklarin terminali geri alma yolu.
    let g1 = sys::fork();
    if g1 == 0 {
        own_group();
        // Ebeveynin engelini devraldi; durabilmesi icin kaldiriyor.
        signal::sigprocmask(signal::SIG_UNBLOCK, signal::mask_of(signal::SIGTTOU));
        let r = signal::tcsetpgrp(sys::STDIN, signal::getpgid(0) as usize);
        sys::exit(encode(r));
    }
    let mut g1_sig = 0u32;
    if g1 > 0 {
        let g = g1 as usize;
        signal::setpgid(g, g);
        match wait_change(g, 2000) {
            Some(status) if sys::stopped(status) => {
                g1_sig = sys::stop_signal(status);
                signal::kill(g, signal::SIGKILL);
                let _ = wait_change(g, 500);
            }
            Some(_) => {}
            None => {
                signal::kill(g, signal::SIGKILL);
                let _ = wait_change(g, 500);
            }
        }
    }
    take_terminal_back(my_pgid);

    const G2_TOOK: u32 = 0x01;
    const G2_HOLDS: u32 = 0x02;
    let g2 = sys::fork();
    if g2 == 0 {
        own_group();
        signal::sigprocmask(signal::SIG_BLOCK, signal::mask_of(signal::SIGTTOU));
        let pgid = signal::getpgid(0);
        let mut bits = DONE;
        if signal::tcsetpgrp(sys::STDIN, pgid as usize) == 0 {
            bits |= G2_TOOK;
        }
        if signal::tcgetpgrp(sys::STDIN) == pgid {
            bits |= G2_HOLDS;
        }
        sys::exit(bits as i32);
    }
    let g2_bits = if g2 > 0 {
        signal::setpgid(g2 as usize, g2 as usize);
        wait_exit(g2 as usize, 2000).unwrap_or(0)
    } else {
        0
    };
    let g_back = take_terminal_back(my_pgid);

    let g = g1_sig == signal::SIGTTOU && g2_bits & (G2_TOOK | G2_HOLDS) == G2_TOOK | G2_HOLDS;
    checks[6] = Check {
        name: NAMES[6],
        detail: if g {
            "arka plandan tcsetpgrp: SIGTTOU ile durdu; engelliyken izin verildi"
        } else if g1_sig == 0 {
            "arka plandan tcsetpgrp DURDURULMADI"
        } else if g1_sig != signal::SIGTTOU {
            "durdu ama SIGTTOU ile degil"
        } else if g2_bits & DONE == 0 {
            "SIGTTOU engelli cocuk cevap vermedi: yine de durduruldu"
        } else {
            "SIGTTOU engelliyken devir REDDEDILDI"
        },
        passed: g,
    };
    say(&checks[6]);
    say(&checks[7]);

    // --- I: varsayilani yok saymak olan sinyal beklemeyi bolmuyor -----
    //
    // Cocugun torunu 100 ms sonra oluyor -> cocuga `SIGCHLD` geliyor,
    // isleyicisi yok, varsayilani yok saymak. Cocuk o sirada
    // `sigtimedwait` ile **baska** bir sinyali (`SIGUSR1`) 1 saniye
    // bekliyor. Bolunmemeli; suresi dolmali (`EAGAIN`).
    //
    // Neden okuma degil de `sigtimedwait`: ilk yazilista olcu terminal
    // okumasiydi ve bozma sinavi onun **baska** bir duzeltmeyi
    // olctugunu gosterdi. Bu batida iki duzeltme ayni hatayi iki
    // katmandan kapatiyor:
    //
    //   interrupts_call      yok sayilan sinyal cagriyi BOLMESIN
    //   restart_after_signal bolunen cagri varsayilan davranistan
    //                        sonra YENIDEN BASLASIN
    //
    // Okuma ikincisinden geciyor: birincisi geri alininca okuma yine
    // bolunuyor ama sessizce yeniden basliyor, ve dis gozlem hic
    // degismiyor. `sigtimedwait` yeniden baslamaz -- bolunmesi
    // dogrudan `EINTR`dir. Yani birinciyi yalnizca o olcebilir.
    //
    // Olcu hem kod hem **sure**: bolunme 100 ms'de olur (hata), dogru
    // davranis 1000 ms'de zaman asimi.
    let i_child = sys::fork();
    if i_child == 0 {
        if sys::fork() == 0 {
            sys::sleep_ms(100);
            sys::exit(0);
        }
        let before = sys::ticks();
        let r = signal::sigtimedwait(signal::mask_of(signal::SIGUSR1), None, 1000);
        let waited = sys::ticks().saturating_sub(before);
        {
            use core::fmt::Write;
            let _ = writeln!(
                Stdout,
                "[jobctl/I] sigtimedwait {}  bekleme {} tik",
                r, waited
            );
        }
        // Kodlama: DONE | (yeterince bekledi ? 0x20 : 0) | (EAGAIN ? 0x10 : 0)
        //               | (EINTR ? 0x08 : 0)
        let mut bits = DONE;
        if waited >= I_MIN_TICKS {
            bits |= 0x20;
        }
        if r == -EAGAIN {
            bits |= 0x10;
        }
        if r == -EINTR {
            bits |= 0x08;
        }
        sys::exit(bits as i32);
    }
    let i_bits = if i_child > 0 {
        wait_exit(i_child as usize, 4000).unwrap_or(0)
    } else {
        0
    };
    let i = i_bits & (DONE | 0x30) == DONE | 0x30;
    checks[8] = Check {
        name: NAMES[8],
        detail: if i {
            "cocuk oldu, bekleme bolunmedi; suresi doldu"
        } else if i_bits & DONE == 0 {
            "cocuk cevap vermedi"
        } else if i_bits & 0x08 != 0 {
            "bekleme EINTR ile BOLUNDU: yok sayilan SIGCHLD cagriyi kesti"
        } else if i_bits & 0x20 == 0 {
            "bekleme erken bitti"
        } else {
            "zaman asimi EAGAIN donmedi"
        },
        passed: i,
    };
    say(&checks[8]);

    // --- J: is parcacigi surecin grubunda -----------------------------
    //
    // Akis `getpgid(0)` ile kendi grubunu soruyor ve terminali bloke
    // etmeden okuyor. Surecin grubunda (on planda) olmali ve okumasi
    // izinli olmali (`EAGAIN`). Eski hal: akis kendi numarasini grup
    // sanir, arka planda gorunur ve `SIGTTIN` ile durur -- o zaman
    // birlestirme zaman asimina ugrar.
    // J'nin cocugu **ebeveynin grubunda** kosuyor -- olculen sey zaten
    // akisin surecin grubunda olmasi. Ebeveyn terminali o an elinde
    // tutmuyorsa (daha onceki bir sinav geri alamadiysa) olcu kurulamaz:
    // cocuk arka planda olur ve bu, akisin degil ebeveynin durumu.
    let j_measurable = signal::tcgetpgrp(sys::STDIN) == my_pgid as isize;
    let j_child = if j_measurable { sys::fork() } else { -1 };
    if j_child == 0 {
        signal::default(signal::SIGTTIN);
        sys::set_flags(sys::STDIN, sys::O_NONBLOCK);
        let slot = core::ptr::addr_of_mut!(J_TID);
        let spawned = unsafe { sys::clone_joinable(thread_probe, 0, slot) };
        let joined = spawned > 0 && unsafe { sys::join_thread(slot, 1500) };
        let mut bits = DONE;
        if joined {
            bits |= 0x01;
        }
        let tp = J_PGID.load(core::sync::atomic::Ordering::SeqCst) as isize;
        if tp == signal::getpgid(0) {
            bits |= 0x02;
        }
        let tr = J_READ.load(core::sync::atomic::Ordering::SeqCst) as isize;
        if tr == -EAGAIN || tr > 0 {
            bits |= 0x04;
        }
        sys::exit(bits as i32);
    }
    let j_bits = if j_child > 0 {
        wait_exit(j_child as usize, 4000).unwrap_or(0)
    } else {
        0
    };
    let j = j_bits & 0x47 == 0x47;
    checks[9] = Check {
        name: NAMES[9],
        detail: if j {
            "akis surecin grubunda ve terminali okuyabildi"
        } else if !j_measurable {
            "terminal ebeveynde degil: olculemedi"
        } else if j_bits & DONE == 0 {
            "cocuk cevap vermedi"
        } else if j_bits & 0x01 == 0 {
            "akis bitmedi: arka planda sanilip DURDURULDU"
        } else if j_bits & 0x02 == 0 {
            "akisin surec grubu surecinkinden FARKLI"
        } else {
            "akisin okumasi reddedildi"
        },
        passed: j,
    };
    say(&checks[9]);

    // --- K: olen on plan grubu on plani birakir -----------------------
    //
    // X'e on plan veriliyor, X cikiyor ve toplaniyor. Sonra Y catallaniyor;
    // TCMK ilk bos yuvayi kullandigi icin Y buyuk olasilikla X'in yuvasini
    // ve **grup numarasini** aliyor. Y kendi grubunda, bloke etmeden
    // okuyor: on plan bosalmissa arka plandadir ve `SIGTTIN` ile durur.
    // On plan hala X'in numarasini gosteriyorsa Y okur (`EAGAIN`) --
    // kimse ona on plan vermedigi halde.
    //
    // Y X'in yuvasini almazsa olcu kurulamaz ve sinav bunu soyluyor.
    let x = sys::fork();
    if x == 0 {
        own_group();
        sys::exit(DONE as i32);
    }
    let mut x_given = -1isize;
    let mut fg_after_x = -1isize;
    if x > 0 {
        signal::setpgid(x as usize, x as usize);
        x_given = signal::tcsetpgrp(sys::STDIN, x as usize);
        let _ = wait_exit(x as usize, 2000);
        fg_after_x = signal::tcgetpgrp(sys::STDIN);
    }
    let y = sys::fork();
    if y == 0 {
        own_group();
        signal::default(signal::SIGTTIN);
        sys::set_flags(sys::STDIN, sys::O_NONBLOCK);
        let mut buf = [0u8; 4];
        let r = sys::read(sys::STDIN, &mut buf);
        sys::exit(encode(r));
    }
    let same_slot = x > 0 && y == x;
    let mut y_sig = 0u32;
    let mut y_read: Option<isize> = None;
    if y > 0 {
        let yy = y as usize;
        signal::setpgid(yy, yy);
        match wait_change(yy, 2000) {
            Some(status) if sys::stopped(status) => {
                y_sig = sys::stop_signal(status);
                signal::kill(yy, signal::SIGKILL);
                let _ = wait_change(yy, 500);
            }
            Some(status) if sys::exited(status) => {
                y_read = decode(sys::exit_status(status));
            }
            _ => {
                signal::kill(yy, signal::SIGKILL);
                let _ = wait_change(yy, 500);
            }
        }
    }
    let k_back = take_terminal_back(my_pgid);
    let k = x_given == 0 && fg_after_x != x && same_slot && y_sig == signal::SIGTTIN;
    checks[10] = Check {
        name: NAMES[10],
        detail: if k {
            "on plan grubu oldu, ayni numarali yeni is arka planda kaldi"
        } else if x_given != 0 {
            "on plan X'e verilemedi"
        } else if !same_slot {
            "Y X'in yuvasini almadi: olculemedi"
        } else if fg_after_x == x {
            "olen grup HALA on planda; yeni is onu MIRAS ALDI"
        } else if y_read.is_some() {
            "yeni is arka planda okuyabildi"
        } else {
            "yeni is SIGTTIN ile durmadi"
        },
        passed: k,
    };
    say(&checks[10]);

    // Terminal geri bizde mi -- butun sinavlar boyunca devredilip
    // geri alindi; son durumu raporda gorunur olsun.
    let restored = d_back == 0
        && g_back == 0
        && k_back == 0
        && signal::tcgetpgrp(sys::STDIN) == my_pgid as isize;

    let passed = checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(Stdout, "[jobctl] sonuc: {}/11 gecti", passed);
    let _ = writeln!(
        Stdout,
        "[jobctl] pid {}  pgid {}  sid {}  E durma sinyali {}  terminal geri: {}",
        me,
        my_pgid,
        my_sid,
        stop_sig,
        if restored { "evet" } else { "HAYIR" }
    );

    signal::sigprocmask(signal::SIG_SETMASK, saved_mask);
    signal::default(signal::SIGTTIN);
    if let Some(w) = win.as_mut() {
        show(w, &checks);
    }
}

/// I: 1 saniyelik beklemenin en az ne kadari gecmis olmali (tik =
/// 10 ms). `SIGCHLD` 100 ms'de geliyor; aradaki fark genis tutuldu.
const I_MIN_TICKS: usize = 50;

static mut J_TID: u32 = 0;
static J_PGID: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(usize::MAX);
static J_READ: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// J'nin akisi: grubunu sorar, terminali bloke etmeden okur.
extern "C" fn thread_probe(_param: usize) -> usize {
    J_PGID.store(signal::getpgid(0) as usize, core::sync::atomic::Ordering::SeqCst);
    let mut buf = [0u8; 4];
    let r = sys::read(sys::STDIN, &mut buf);
    J_READ.store(r as usize, core::sync::atomic::Ordering::SeqCst);
    0
}

fn show(win: &mut Window, checks: &[Check; 11]) {
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(win, checks);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 11]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Terminali yalnizca on plan okur", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            410,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    win.text(6, h - 30, "arka plan okursa: SIGTTIN, yok sayilirsa EIO", DIM);
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
