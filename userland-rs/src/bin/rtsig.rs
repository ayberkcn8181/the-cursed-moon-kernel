//! `rtsig` -- birlesen sinyal ile kuyruklanan sinyal.
//!
//! POSIX'te iki sinif sinyal vardir ve aralarindaki fark bir bayrak ya
//! da bir numara degil, **bekleme bicimi**:
//!
//! ```text
//!   standart (1..=31)        bir BIT MASKESINDE bekler
//!   gercek-zamanli (32..=63) bir KUYRUKTA bekler
//! ```
//!
//! Sonucu tek bir olcumle gorunur:
//!
//! ```text
//!   engelle; kill(SIGUSR1) x3;   engeli kaldir  ->  isleyici 1 kez kosar
//!   engelle; sigqueue(RTMIN) x3; engeli kaldir  ->  isleyici 3 kez kosar
//! ```
//!
//! Ikinci satirdaki her teslim ayrica **kendi degerini** tasir
//! (`si_value`), cunku kuyrukta her kopyanin kendi kaydi var. Birlesen
//! bir sinyalde degerin bir anlami olamazdi: uc gonderim tek teslime
//! dusunce hangi degerin tasinacagi cevapsiz kalirdi.
//!
//! ## Neden bedava degil
//!
//! Bit maskesi sabit yer tutar ve **hicbir zaman dolmaz**. Kuyruk ise
//! cekirdek bellegidir, sinirlidir ve dolabilir -- doldugunda `sigqueue`
//! `EAGAIN` doner. Yani gonderen taraf ilk kez "sinyal gonderemedim"
//! cevabi alabiliyor. POSIX bunu bir ariza olarak degil **sozlesme**
//! olarak yazar, cunku aksi halde bir surec baska bir surece sinirsiz
//! sinyal gondererek cekirdegi tuketebilirdi.
//!
//! ## Windows'ta bu ayrim yok
//!
//! En yakin karsilik APC kuyruklaridir (`QueueUserAPC`) ve onlar **her
//! zaman** kuyruklu, **her zaman** deger tasir. Yani Windows'ta "ucuz ve
//! birlesen" bir bildirim yolu hic olmadi; POSIX ikisini de tutuyor ve
//! secimi uygulamaya birakiyor.
//!
//! ## Yedi sinav
//!
//! ```text
//!   A  standart birlesir -> 3 kill -> 1 teslim
//!   B  rt kuyruklanir    -> 3 sigqueue -> 3 teslim, hepsi engel KALKINCA
//!   C  deger tasiniyor   -> 11,22,33 sirasiyla geldi (FIFO)
//!   D  si_code=SI_QUEUE  -> ve si_pid gonderenin kimligi
//!   E  kuyruk dolar      -> 8 kabul, sonrasi EAGAIN
//!   F  numara sirasi     -> sonra gelen RTMIN, once gelen RTMIN+1'den ONCE
//!   G  maske 64 bit      -> 63 numarali sinyal maskeye girip geri okunuyor
//! ```
//!
//! B'nin "hepsi engel kalkinca" kismi bos bir ayrinti degil: maske 32
//! bitte kalmis olsaydi `1<<32` sifira dusecek, hicbir sey engellenmeyecek
//! ve uc sinyal gonderildigi anda teslim edilecekti -- sayac yine 3
//! olurdu. Yani "3 teslim" tek basina kuyrugu olcmuyor. Olcen sey,
//! teslimlerin **ne zaman** oldugu.
//!
//! G ayni soruyu en ciplak haliyle soruyor: 32'den buyuk bir bit
//! maskeden geri okunabiliyor mu.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::signal::{self, SigInfo, UContext};

tcmk::entry!(main);

const BG: u32 = 0x0010_1A22;
const PANEL: u32 = 0x001C_2A34;
const FG: u32 = 0x00E0_EAF0;
const DIM: u32 = 0x0088_98A4;
const ACCENT: u32 = 0x0070_D0FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Sinavda kullanilan gercek-zamanli sinyaller.
const RT_A: u32 = signal::SIGRTMIN;
const RT_B: u32 = signal::SIGRTMIN + 1;

/// Cekirdegin gorev basina kuyrukta tuttugu en fazla kopya.
///
/// Cekirdekteki `signal::SIGQUEUE_LEN` ile ayni olmak zorunda. Iki
/// yerde durmasi hos degil ama sayi ABI'nin parcasi degil: sinav onu bir
/// **beklenti** olarak degil, "bir yerde dolmali" diye kullaniyor
/// (bkz. E).
const QUEUE_LEN: usize = 8;

/// `sigqueue`in kuyruk dolu oldugunda dondurdugu errno.
const EAGAIN: isize = -11;

/// Kac teslim oldu -- sinyal basina degil, toplam.
static DELIVERED: AtomicUsize = AtomicUsize::new(0);
/// Engel **kalkmadan once** kac teslim oldu.
///
/// Sifirdan buyuk olmasi, engellemenin hic yurumedigini soyler: B'nin
/// asil olcusu bu.
static EARLY: AtomicUsize = AtomicUsize::new(0);

/// Asama: 1 = engelli bolge, 2 = engel kalkti.
static PHASE: AtomicUsize = AtomicUsize::new(1);

/// Gelen degerler, gelis sirasinda.
static VALUES: [AtomicUsize; 4] = [
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
];
/// Kac deger kaydedildi.
static VALUE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Ilk teslimde gorulen `si_code` ve `si_pid`.
static SEEN_CODE: AtomicUsize = AtomicUsize::new(usize::MAX);
static SEEN_PID: AtomicUsize = AtomicUsize::new(usize::MAX);

/// F sinavi: gelen sinyal numaralari, gelis sirasinda.
static ORDER: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
static ORDER_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Standart sinyalin isleyicisi -- yalnizca sayar.
extern "C" fn on_standard(_signo: u32) {
    DELIVERED.fetch_add(1, Ordering::SeqCst);
    if PHASE.load(Ordering::SeqCst) == 1 {
        EARLY.fetch_add(1, Ordering::SeqCst);
    }
}

/// Gercek-zamanli sinyalin isleyicisi -- degeri ve nedeni de kaydeder.
///
/// `SA_SIGINFO` sart: tek argumanli yuz yalnizca sinyal numarasini
/// goruyor, yani `si_value`yu **hic** gormez. Degerin varligi ile uc
/// argumanli yuzun varligi ayni seyin iki yuzu.
extern "C" fn on_rt(signo: u32, info: *const SigInfo, _context: *mut UContext) {
    DELIVERED.fetch_add(1, Ordering::SeqCst);
    if PHASE.load(Ordering::SeqCst) == 1 {
        EARLY.fetch_add(1, Ordering::SeqCst);
    }

    // SAFETY: cekirdek `SA_SIGINFO` isleyicisine gecerli bir kayit verir.
    let (code, pid, value) = unsafe { ((*info).code, (*info).pid(), (*info).value()) };

    if SEEN_CODE.load(Ordering::SeqCst) == usize::MAX {
        SEEN_CODE.store(code as isize as usize, Ordering::SeqCst);
        SEEN_PID.store(pid, Ordering::SeqCst);
    }

    let slot = VALUE_COUNT.fetch_add(1, Ordering::SeqCst);
    if slot < VALUES.len() {
        VALUES[slot].store(value, Ordering::SeqCst);
    }

    let at = ORDER_COUNT.fetch_add(1, Ordering::SeqCst);
    if at < ORDER.len() {
        ORDER[at].store(signo as usize, Ordering::SeqCst);
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
    "A standart birlesir",
    "B rt kuyruklanir",
    "C deger + FIFO",
    "D si_code=SI_QUEUE",
    "E kuyruk dolar",
    "F numara sirasi",
    "G maske 64 bit",
];

/// Sayaclari ve kayitlari sifirlar -- her sinav temiz baslar.
fn reset() {
    DELIVERED.store(0, Ordering::SeqCst);
    EARLY.store(0, Ordering::SeqCst);
    PHASE.store(1, Ordering::SeqCst);
    VALUE_COUNT.store(0, Ordering::SeqCst);
    for slot in VALUES.iter() {
        slot.store(usize::MAX, Ordering::SeqCst);
    }
    ORDER_COUNT.store(0, Ordering::SeqCst);
    for slot in ORDER.iter() {
        slot.store(0, Ordering::SeqCst);
    }
}

fn main() {
    let mut checks = [EMPTY; 7];
    let me = signal::getpid();

    // Isleyiciler **her seyden once** kuruluyor.
    //
    // Sirasi onemli: isleyicisi olmayan bir gercek-zamanli sinyalin
    // varsayilan davranisi sureci sonlandirmaktir (POSIX'te butun
    // gercek-zamanlilar icin boyledir). Once gonderip sonra kurmak,
    // sinavi ilk gonderimde bitirirdi.
    let installed_std = signal::install(signal::SIGUSR1, on_standard) >= 0;
    let installed_a = signal::action_info(RT_A, on_rt, 0, 0) >= 0;
    let installed_b = signal::action_info(RT_B, on_rt, 0, 0) >= 0;

    // --- A: standart sinyal birlesir ---------------------------------
    //
    // Uc gonderim, bir teslim. Sinyal kaybolmuyor -- **birlesiyor**:
    // maske bir bittir, sayac degil.
    if installed_std {
        reset();
        signal::sigprocmask(signal::SIG_BLOCK, signal::mask_of(signal::SIGUSR1));
        for _ in 0..3 {
            signal::kill(me, signal::SIGUSR1);
        }
        PHASE.store(2, Ordering::SeqCst);
        signal::sigprocmask(signal::SIG_UNBLOCK, signal::mask_of(signal::SIGUSR1));
        let count = DELIVERED.load(Ordering::SeqCst);
        checks[0] = Check {
            name: NAMES[0],
            detail: if count == 1 {
                "3 gonderim -> 1 teslim (bit birlesti)"
            } else if count == 0 {
                "teslim hic olmadi"
            } else {
                "3 gonderim -> 3 teslim: birlesme YOK"
            },
            passed: count == 1,
        };
    } else {
        checks[0] = Check {
            name: NAMES[0],
            detail: "SIGUSR1 isleyicisi kurulamadi",
            passed: false,
        };
    }

    // --- B ve C: gercek-zamanli sinyal kuyruklanir --------------------
    //
    // Ayni kalip, farkli sinyal. Iki sey birlikte olculuyor:
    //
    //   * teslim sayisi 3 (birlesme yok)
    //   * hepsi engel KALKTIKTAN sonra (yani gercekten kuyruktaydilar)
    //
    // Ikincisi olmadan olcum bos: 64 bitlik maske yurumemis olsaydi
    // `1<<32` sifira duser, hicbir sey engellenmez ve uc sinyal
    // gonderildigi anda teslim edilirdi -- sayac yine 3 olurdu.
    let mut rt_count = 0usize;
    let mut rt_early = usize::MAX;
    if installed_a {
        reset();
        signal::sigprocmask(signal::SIG_BLOCK, signal::mask_of(RT_A));
        let mut accepted = 0usize;
        for value in [11usize, 22, 33] {
            if signal::sigqueue(me, RT_A, value) == 0 {
                accepted += 1;
            }
        }
        PHASE.store(2, Ordering::SeqCst);
        signal::sigprocmask(signal::SIG_UNBLOCK, signal::mask_of(RT_A));
        rt_count = DELIVERED.load(Ordering::SeqCst);
        rt_early = EARLY.load(Ordering::SeqCst);

        checks[1] = Check {
            name: NAMES[1],
            detail: if accepted != 3 {
                "sigqueue kabul etmedi (rt sinyal destegi yok?)"
            } else if rt_count != 3 {
                "3 gonderim -> 3 teslim olmadi: kuyruk YOK"
            } else if rt_early != 0 {
                "teslimler engel KALKMADAN oldu: maske 32 bitte kalmis"
            } else {
                "3 gonderim -> 3 teslim, hepsi engel kalkinca"
            },
            passed: accepted == 3 && rt_count == 3 && rt_early == 0,
        };

        // C: degerler ve **sirasi**.
        //
        // Ayni numaradan kopyalar varis sirasinda teslim edilir; POSIX
        // gercek-zamanli sinyaller icin bunu garanti eder. Sira olcumu
        // ayri bir sey soyluyor: kuyruk gercekten bir kuyruk mu, yoksa
        // son gonderilenin sakladigi bir yuva mi.
        let values = [
            VALUES[0].load(Ordering::SeqCst),
            VALUES[1].load(Ordering::SeqCst),
            VALUES[2].load(Ordering::SeqCst),
        ];
        let fifo = values == [11, 22, 33];
        checks[2] = Check {
            name: NAMES[2],
            detail: if fifo {
                "11,22,33 gonderildi, 11,22,33 geldi"
            } else if values[0] == 33 {
                "sira TERS: kuyruk degil yigin"
            } else if values == [0, 0, 0] {
                "degerler sifir geldi: si_value tasinmiyor"
            } else {
                "degerler beklenenden farkli"
            },
            passed: fifo,
        };

        // D: sinyalin **nedeni**.
        //
        // `si_code` negatif olmasi tesadufi degil: POSIX kodun isaretini
        // "cekirdek mi uretti, surec mi gonderdi" ayrimi icin kullanir.
        let code = SEEN_CODE.load(Ordering::SeqCst) as isize as i32;
        let pid = SEEN_PID.load(Ordering::SeqCst);
        let right = code == signal::SI_QUEUE && pid == me;
        checks[3] = Check {
            name: NAMES[3],
            detail: if right {
                "si_code=SI_QUEUE, si_pid gonderen"
            } else if code == signal::SI_USER {
                "si_code=SI_USER: kill'den ayrilmiyor"
            } else if code == signal::SI_QUEUE {
                "si_code dogru, si_pid YANLIS"
            } else {
                "si_code beklenmedik"
            },
            passed: right,
        };
    } else {
        for i in 1..4 {
            checks[i] = Check {
                name: NAMES[i],
                detail: "SIGRTMIN isleyicisi kurulamadi",
                passed: false,
            };
        }
    }

    // --- E: kuyruk dolar ve EAGAIN doner -----------------------------
    //
    // Bir kuyrugun olculebilir en onemli ozelligi sinirli olmasi.
    // Sinirsiz olsaydi gonderen taraf cekirdek bellegini tuketebilirdi;
    // POSIX bu yuzden `sigqueue`a bir hata kodu verir. Yani dolmak bir
    // ariza degil, sozlesmenin yazili parcasi.
    let mut accepted = 0usize;
    let mut refused_with = 0isize;
    if installed_a {
        reset();
        signal::sigprocmask(signal::SIG_BLOCK, signal::mask_of(RT_A));
        for value in 0..QUEUE_LEN + 2 {
            let result = signal::sigqueue(me, RT_A, value);
            if result == 0 {
                accepted += 1;
            } else if refused_with == 0 {
                refused_with = result;
            }
        }
        PHASE.store(2, Ordering::SeqCst);
        signal::sigprocmask(signal::SIG_UNBLOCK, signal::mask_of(RT_A));
        let delivered = DELIVERED.load(Ordering::SeqCst);
        let full = accepted == QUEUE_LEN && refused_with == EAGAIN && delivered == accepted;
        checks[4] = Check {
            name: NAMES[4],
            detail: if full {
                "8 kabul, sonrasi EAGAIN, 8 teslim"
            } else if refused_with == 0 {
                "kuyruk hic dolmadi: sinir yok"
            } else if refused_with != EAGAIN {
                "dolu kuyruk EAGAIN degil baska hata donduruyor"
            } else {
                "kabul edilen sayi ile teslim sayisi uyusmuyor"
            },
            passed: full,
        };
    } else {
        checks[4] = Check {
            name: NAMES[4],
            detail: "SIGRTMIN isleyicisi kurulamadi",
            passed: false,
        };
    }

    // --- F: numara sirasi varis sirasini ezer ------------------------
    //
    // Ikisi de kuyrukta bekliyor. Teslim once **numaraya** bakar: kucuk
    // numara, sonra gonderilmis olsa bile once gelir. POSIX gercek-
    // zamanli sinyallerde numarayi bir **onceliktir** diye tanimlar --
    // adlarindaki "gercek-zamanli" kisminin karsiligi da budur.
    let mut order = [0usize; 2];
    if installed_a && installed_b {
        reset();
        let both = signal::mask_of(RT_A) | signal::mask_of(RT_B);
        signal::sigprocmask(signal::SIG_BLOCK, both);
        // Once BUYUK numara gonderiliyor: varis sirasi yanlis cevabi
        // verse onu goruruz.
        signal::sigqueue(me, RT_B, 1);
        signal::sigqueue(me, RT_A, 2);
        PHASE.store(2, Ordering::SeqCst);
        signal::sigprocmask(signal::SIG_UNBLOCK, both);
        order = [
            ORDER[0].load(Ordering::SeqCst),
            ORDER[1].load(Ordering::SeqCst),
        ];
        let right = order == [RT_A as usize, RT_B as usize];
        checks[5] = Check {
            name: NAMES[5],
            detail: if right {
                "kucuk numara once, gec gelmis olsa bile"
            } else if order == [RT_B as usize, RT_A as usize] {
                "varis sirasi numarayi ezdi"
            } else {
                "iki sinyalin ikisi de gelmedi"
            },
            passed: right,
        };
    } else {
        checks[5] = Check {
            name: NAMES[5],
            detail: "iki rt isleyicisi kurulamadi",
            passed: false,
        };
    }

    // --- G: maske gercekten 64 bit mi --------------------------------
    //
    // En ciplak soru ve bir onceki duzende cevabi **hayir**di: maske
    // `u32`ydi, yani 32'den buyuk hicbir sinyal engellenemezdi. Burada
    // olculen sey bir davranis degil, tasimanin genisligi: 63 numarali
    // bit maskeye girip geri okunabiliyor mu.
    let top = signal::mask_of(signal::SIGRTMAX);
    signal::sigprocmask(signal::SIG_BLOCK, top);
    let read_back = signal::current_mask();
    signal::sigprocmask(signal::SIG_UNBLOCK, top);
    let cleared = signal::current_mask();
    let wide = read_back & top != 0 && cleared & top == 0;
    checks[6] = Check {
        name: NAMES[6],
        detail: if wide {
            "63 numarali bit maskeye girdi ve geri okundu"
        } else if read_back & top == 0 {
            "63 numarali bit maskede DURMADI: maske 32 bit"
        } else {
            "bit girdi ama kaldirilamadi"
        },
        passed: wide,
    };

    report(&checks, rt_count, rt_early, accepted, order);
    show(&checks, accepted, refused_with);
}

fn report(
    checks: &[Check; 7],
    rt_count: usize,
    rt_early: usize,
    accepted: usize,
    order: [usize; 2],
) {
    use core::fmt::Write;
    let mut console = Stdout;
    let _ = writeln!(console, "[rtsig] sinav basliyor");
    for check in checks {
        let _ = writeln!(
            console,
            "[rtsig] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let _ = writeln!(
        console,
        "[rtsig] rt teslim: {} (erken {})  kuyruga sigan: {}  sira: {},{}",
        rt_count, rt_early, accepted, order[0], order[1]
    );
    let _ = writeln!(
        console,
        "[rtsig] SIGRTMIN={} SIGRTMAX={} maske={} bit",
        signal::SIGRTMIN,
        signal::SIGRTMAX,
        64
    );
}

fn show(checks: &[Check; 7], accepted: usize, refused_with: isize) {
    let mut win = match Window::open("rtsig -- birlesen mi, kuyruklanan mi", 250, 100, 470, 260) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, checks, accepted, refused_with);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 7], accepted: usize, refused_with: isize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Standart sinyal birlesir, gercek-zamanli kuyruklanir", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            380,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    win.text(6, h - 46, "kuyruga sigan:", DIM);
    win.number(130, h - 46, accepted, ACCENT);
    win.text(190, h - 46, "sonrasi:", DIM);
    win.text(
        265,
        h - 46,
        if refused_with == EAGAIN {
            "EAGAIN"
        } else {
            "hata yok"
        },
        if refused_with == EAGAIN { OK } else { WARN },
    );

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
