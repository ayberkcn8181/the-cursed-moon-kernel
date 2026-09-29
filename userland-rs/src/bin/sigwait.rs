//! `sigwait` -- sinyali **okumak**: kesintinin ucuncu yuzu.
//!
//! Iki bati once gercek-zamanli sinyaller geldi, bir bati once Windows'un
//! APC kuyruklari. APC batisinin sonucu suydu:
//!
//! ```text
//!   sinyal ->  kesinti. "su an bolunebilirsin" VARSAYILAN.
//!   APC    ->  randevu. "su an bolunebilirsin" ozel olarak SOYLENIR.
//! ```
//!
//! Ve orada yarim kalan bir soru vardi: POSIX'te randevu yuzu **yok mu**?
//! Var. Adi `sigwaitinfo` ve bu sinav onu olcuyor.
//!
//! ## Sinyalin uc yuzu
//!
//! Bir sinyalin bir surece yapabilecegi seyler:
//!
//! ```text
//!   1. varsayilan davranis  ->  cogunlukla olum
//!   2. isleyici             ->  cekirdek CAGIRIR, akis bolunur
//!   3. sigwaitinfo          ->  program OKUR, hicbir sey bolunmez
//! ```
//!
//! Ucuncusu sinyali bir **mesaja** cevirir. Fark gorunur: isleyici
//! yolunda kod "ne zaman kosacagimi bilmiyorum" durumundadir ve bu
//! yuzden ne yapabilecegi sinirlidir (POSIX'in async-signal-safe
//! listesi; `malloc` o listede **degil**). `sigwaitinfo` yolunda kod
//! siradan bir dongudur -- her sey serbesttir.
//!
//! ## Sart: sinyal once ENGELLENMELI
//!
//! Engellenmezse sinyal isleyiciye gider ve buraya hic ulasmaz. Yani
//! POSIX'te randevu yuzu vardir ama **varsayilan degildir**: program
//! ondan yararlanmak icin ozel olarak calismak zorundadir. Windows'ta
//! tersi -- APC randevudur ve baska turlusu yoktur.
//!
//! ## Yedi sinav
//!
//! ```text
//!   A  isleyici KOSMADI  -> engelli sinyal sigwaitinfo ile alindi,
//!                           isleyici hic calismadi
//!   B  deger geldi       -> si_value ve si_code senkron yolda da dogru
//!   C  kuyruk sirasi     -> uc sigqueue, uc sigwaitinfo, 11-22-33
//!   D  zaman asimi       -> sigtimedwait EAGAIN dondu ve CPU YAKMADI
//!   E  kume disi         -> kumedeki olmayan sinyal ALINMADI ve
//!                           tuketilmedi
//!   F  bekleyip uyaniyor -> bekleme sirasinda gelen sinyal alindi
//!   G  tuketiliyor       -> alinan sinyal PENDING'den dustu
//! ```
//!
//! A bu sinavin sebebi. "Sinyal alindi" tek basina yetmez -- isleyici de
//! kosmus olabilirdi ve sonuc ayni gorunurdu. Olcum, isleyicinin
//! sayacinin **sifir kalmasi**.
//!
//! D'nin ikinci yarisi ayri bir sey olcuyor: zaman asimi bir yoklama
//! dongusuyle de "calisir" gorunurdu. Gorevin `cpu` sayaci bekleme
//! boyunca artmamali, yani gorev gercekten uyumali.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::signal::{self, SigInfoBuf};

tcmk::entry!(main);

const BG: u32 = 0x0014_1020;
const PANEL: u32 = 0x0022_1C34;
const FG: u32 = 0x00E6_E2F2;
const DIM: u32 = 0x008E_88A4;
const ACCENT: u32 = 0x00B0_A0FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Sinavda kullanilan gercek-zamanli sinyaller.
const RT_A: u32 = signal::SIGRTMIN;
const RT_B: u32 = signal::SIGRTMIN + 1;

/// `-EAGAIN`: `sigtimedwait` suresi doldu.
const EAGAIN: isize = -11;

/// D sinavinin cocugunun cikis kodundaki bitler.
///
/// Isaretci biti (`D_DONE`) ayri tutuluyor: cocuk hic cevap veremezse
/// cikis kodu sifir olur ve "her sey basarisiz" ile "cocuk hic
/// konusmadi" birbirinden ayirt edilemezdi.
const D_DONE: u32 = 0x8;
const D_EAGAIN: u32 = 0x1;
const D_SLEPT_LONG: u32 = 0x2;
const D_NO_SPIN: u32 = 0x4;

/// Cocugu **sinirli** sure bekler; donmezse 0 doner.
///
/// `waitpid`in suresiz bicimi burada kullanilamaz: olculen sey zaten
/// bir beklemenin bitip bitmedigi, ve cocuk asili kalirsa ebeveyn de
/// asili kalirdi.
fn reap_bounded(child: isize) -> u32 {
    if child <= 0 {
        return 0;
    }
    let mut status = 0u32;
    for _ in 0..50 {
        if tcmk::sys::waitpid(child as usize, &mut status, tcmk::sys::WNOHANG) > 0 {
            return if tcmk::sys::exited(status) {
                tcmk::sys::exit_status(status)
            } else {
                0
            };
        }
        tcmk::sys::sleep_ms(50);
    }
    0
}

/// Isleyici kac kez kostu.
///
/// Sinavin can damari: `sigwaitinfo` calisiyorsa bu sayac **sifir**
/// kalmali. Sifir olmayan bir deger, sinyalin okunmadigini teslim
/// edildigini soyler.
static HANDLER_RAN: AtomicUsize = AtomicUsize::new(0);

/// Isleyici -- kosmamasi gerekiyor.
extern "C" fn on_rt(_signo: u32) {
    HANDLER_RAN.fetch_add(1, Ordering::SeqCst);
}

struct Check {
    name: &'static str,
    detail: &'static str,
    passed: bool,
}

/// Bir sinavin sonucunu **hesaplandigi anda** yazar.
///
/// Toplu rapor yetmiyor. Bu sinav bir beklemeyi olcuyor ve beklemeler
/// asilabilir: yalnizca sonda yazan bir sinav, bozuk bir cekirdekte
/// "hicbir cikti" veriyor -- hangi sinavda takildigi bile
/// gorunmuyordu. Satir satir yazmak, susan sinavin **nerede** sustugunu
/// soyluyor.
fn say(check: &Check) {
    use core::fmt::Write;
    let mut console = Stdout;
    let _ = writeln!(
        console,
        "[sigwait] {}: {} ({})",
        check.name,
        if check.passed { "gecti" } else { "KALDI" },
        check.detail
    );
}

const EMPTY: Check = Check {
    name: "",
    detail: "",
    passed: false,
};

const NAMES: [&str; 7] = [
    "A isleyici KOSMADI",
    "B deger + si_code",
    "C kuyruk sirasi",
    "D zaman asimi",
    "E kume disi",
    "F bekleyip uyaniyor",
    "G tuketiliyor",
];

fn main() {
    use core::fmt::Write;
    let _ = writeln!(Stdout, "[sigwait] sinav basliyor");
    let mut checks = [EMPTY; 7];
    let me = signal::getpid();

    // Isleyici **kuruluyor** -- ve kosmamasi gerekiyor.
    //
    // Kurmamak sinavi zayiflatirdi: isleyicisiz bir sinyalin
    // kosmamasi zaten kesin. Asil soru, isleyicisi **olan** bir sinyalin
    // senkron alindiginda isleyiciye ugramamasi.
    let installed = signal::install(RT_A, on_rt) >= 0 && signal::install(RT_B, on_rt) >= 0;

    // Engellemek sart: engellenmemis bir sinyal isleyiciye gider ve
    // `sigwaitinfo`ya hic ulasmaz. POSIX'te randevu yuzunun bedeli bu.
    let both = signal::mask_of(RT_A) | signal::mask_of(RT_B);
    signal::sigprocmask(signal::SIG_BLOCK, both);

    // --- A ve B: senkron alim, isleyici kosmadan --------------------
    //
    // Bekleme **sinirli**: `sigwaitinfo` suresizdir ve burada onu
    // kullanmak sinavi zayiflatirdi. Engelleme yurumezse sinyal
    // isleyiciye gider, kuyrukta hicbir sey kalmaz ve suresiz bir
    // bekleme sonsuza kadar asili kalir -- yani bozuk bir cekirdekte
    // sinav rapor vermek yerine **susardi**. Sinirli bekleme ayni
    // durumda zaman asimina dusup "isleyici KOSTU" diyor.
    let mut buf = SigInfoBuf::new();
    signal::sigqueue(me, RT_A, 1234);
    let got = signal::sigtimedwait(signal::mask_of(RT_A), Some(&mut buf), 500);
    let ran = HANDLER_RAN.load(Ordering::SeqCst);
    checks[0] = Check {
        name: NAMES[0],
        detail: if !installed {
            "rt isleyicisi kurulamadi"
        } else if got == RT_A as isize && ran == 0 {
            "sinyal alindi, isleyici hic kosmadi"
        } else if ran != 0 {
            "isleyici KOSTU: sinyal okunmadi, teslim edildi"
        } else if got < 0 {
            "sigwaitinfo hata dondurdu"
        } else {
            "beklenen sinyal gelmedi"
        },
        passed: installed && got == RT_A as isize && ran == 0,
    };
    say(&checks[0]);

    let info = buf.info();
    let value = info.value();
    let code = info.code;
    checks[1] = Check {
        name: NAMES[1],
        detail: if value == 1234 && code == signal::SI_QUEUE {
            "si_value ve si_code senkron yolda da dogru"
        } else if value == 1234 {
            "deger dogru ama si_code yanlis"
        } else if value == 0 {
            "si_value sifir: kayit yazilmadi"
        } else {
            "deger beklenenden farkli"
        },
        passed: value == 1234 && code == signal::SI_QUEUE,
    };
    say(&checks[1]);

    // --- C: kuyruk sirasi senkron yolda da korunuyor -----------------
    //
    // Uc kopya kuyrukta, uc ayri `sigwaitinfo`. Teslim yolundaki FIFO
    // garantisi burada da gecerli olmali -- iki yolun ayni kuyruktan
    // cektiginin kaniti.
    let mut order = [0usize; 3];
    signal::sigqueue(me, RT_A, 11);
    signal::sigqueue(me, RT_A, 22);
    signal::sigqueue(me, RT_A, 33);
    for slot in order.iter_mut() {
        let mut one = SigInfoBuf::new();
        // Yine sinirli: ayni gerekce (bkz. A).
        if signal::sigtimedwait(signal::mask_of(RT_A), Some(&mut one), 300) == RT_A as isize {
            *slot = one.info().value();
        }
    }
    let fifo = order == [11, 22, 33];
    checks[2] = Check {
        name: NAMES[2],
        detail: if fifo {
            "11-22-33 gonderildi, 11-22-33 okundu"
        } else if order[0] == 33 {
            "sira TERS: kuyruk degil yigin"
        } else if order == [0, 0, 0] {
            "hicbiri okunamadi"
        } else {
            "sira beklenenden farkli"
        },
        passed: fifo,
    };
    say(&checks[2]);

    // --- D: zaman asimi, ve bekleme sirasinda CPU yakilmiyor --------
    //
    // Ikinci yari bos bir ayrinti degil: zaman asimi bir yoklama
    // dongusuyle de "calisir" gorunurdu. Olcum, gorevin `cpu` sayacinin
    // bekleme boyunca artmamasi -- yani gorevin gercekten uyumasi.
    //
    // Olcum **cocuk surecte**: zaman asimi hic gerceklesmezse bekleme
    // suresiz olur ve sinav rapor vermek yerine asili kalirdi. Cocuk
    // cevabini cikis kodunda tasiyor, ebeveyn onu sinirli bir sure
    // yokluyor -- yani bozuk bir cekirdekte bile bir cevap var.
    let d_child = tcmk::sys::fork();
    if d_child == 0 {
        let cpu_before = tcmk::sys::self_cpu_ticks();
        let before = tcmk::sys::ticks();
        let timed = signal::sigtimedwait(signal::mask_of(RT_A), None, 300);
        let elapsed = tcmk::sys::ticks().saturating_sub(before);
        let cpu_used = tcmk::sys::self_cpu_ticks().saturating_sub(cpu_before);
        let mut bits = D_DONE;
        if timed == EAGAIN {
            bits |= D_EAGAIN;
        }
        if elapsed >= 20 {
            bits |= D_SLEPT_LONG;
        }
        // Bir tik 10 ms: 300 ms yaklasik 30 tik. Uyuyan bir gorev
        // zamanlanmaz, yani kendi CPU sayaci artmaz; yoklama dongusu
        // olsaydi otuz tikin cogunu o gorev harcardi.
        if cpu_used <= 5 {
            bits |= D_NO_SPIN;
        }
        tcmk::sys::exit(bits as i32);
    }
    let d_bits = reap_bounded(d_child);
    let d_ok = d_bits & D_DONE != 0
        && d_bits & D_EAGAIN != 0
        && d_bits & D_SLEPT_LONG != 0
        && d_bits & D_NO_SPIN != 0;
    checks[3] = Check {
        name: NAMES[3],
        detail: if d_child < 0 {
            "cocuk surec acilamadi"
        } else if d_bits & D_DONE == 0 {
            "cocuk cevap vermedi: bekleme ASILI KALDI"
        } else if d_ok {
            "EAGAIN dondu, sure doldu, CPU yakilmadi"
        } else if d_bits & D_EAGAIN == 0 {
            "zaman asiminda EAGAIN donmedi"
        } else if d_bits & D_NO_SPIN == 0 {
            "sure doldu ama gorev UYUMADI (yoklama dongusu)"
        } else {
            "sure beklenenden kisa"
        },
        passed: d_ok,
    };
    say(&checks[3]);

    // --- E: kume disi sinyal alinmiyor VE tuketilmiyor ---------------
    //
    // Iki yari ayri sey olcuyor. "Alinmadi" tek basina yetmez: sinyal
    // sessizce atilmis da olabilirdi. Ikinci yari onu kapatiyor.
    //
    // Iki bekleme de **yoklama bicimi** (`timeout = 0`): olculen sey
    // zamanlama degil, kume uyeligi ve tuketim. Uyuyan bir bicim
    // kullanmak, sinavi kendi olcmedigi bir mekanizmaya (zaman asimi
    // uyandirmasi) baglardi -- o mekanizma bozuldugunda E burada
    // asili kalirdi. Zaman asimini olcen sinav D ve o, cocukta kosuyor.
    signal::sigqueue(me, RT_B, 77);
    let wrong = signal::sigtimedwait(signal::mask_of(RT_A), None, 0);
    let mut kept = SigInfoBuf::new();
    let recovered = signal::sigtimedwait(signal::mask_of(RT_B), Some(&mut kept), 0);
    let survived = recovered == RT_B as isize && kept.info().value() == 77;
    checks[4] = Check {
        name: NAMES[4],
        detail: if wrong == EAGAIN && survived {
            "kume disi sinyal alinmadi ve kuyrukta kaldi"
        } else if wrong != EAGAIN {
            "kume disi sinyal ALINDI: maske yok sayiliyor"
        } else {
            "kume disi sinyal ATILDI"
        },
        passed: wrong == EAGAIN && survived,
    };
    say(&checks[4]);

    // --- F: bekleme sirasinda gelen sinyal --------------------------
    //
    // Simdiye kadarki sinavlarda sinyal **once** kuyruga giriyordu,
    // yani `sigwaitinfo` hic uyumadan donuyordu. Burada tersi: cocuk
    // surec ebeveyn uyurken gonderiyor.
    let child = tcmk::sys::fork();
    if child == 0 {
        // Cocuk: ebeveyn beklemeye girsin diye kisa bir gecikme.
        tcmk::sys::sleep_ms(150);
        signal::sigqueue(me, RT_A, 4242);
        tcmk::sys::exit(0);
    }
    let mut late = SigInfoBuf::new();
    let woke = signal::sigtimedwait(signal::mask_of(RT_A), Some(&mut late), 3000);
    let _ = reap_bounded(child);
    let from_child = woke == RT_A as isize && late.info().value() == 4242;
    checks[5] = Check {
        name: NAMES[5],
        detail: if child < 0 {
            "cocuk surec acilamadi"
        } else if from_child {
            "bekleme sirasinda gelen sinyal alindi"
        } else if woke == EAGAIN {
            "gonderilen sinyal bekleyeni UYANDIRMADI"
        } else {
            "uyanildi ama deger yanlis"
        },
        passed: from_child,
    };
    say(&checks[5]);

    // --- G: alinan sinyal tuketiliyor -------------------------------
    //
    // Okumanin **yan etkisi** var: sinyal kuyruktan cikiyor. Cikmasaydi
    // ayni sinyal sonsuza kadar okunabilirdi ve `sigwaitinfo` dongusu
    // hic bloke olmazdi.
    // Yoklama bicimi; gerekce E ile ayni.
    let again = signal::sigtimedwait(signal::mask_of(RT_A), None, 0);
    let still_quiet = HANDLER_RAN.load(Ordering::SeqCst) == 0;
    checks[6] = Check {
        name: NAMES[6],
        detail: if again == EAGAIN && still_quiet {
            "alinan sinyal kuyruktan dustu, isleyici hala sessiz"
        } else if again != EAGAIN {
            "ayni sinyal YENIDEN okundu: tuketilmiyor"
        } else {
            "isleyici bir noktada kostu"
        },
        passed: again == EAGAIN && still_quiet,
    };
    say(&checks[6]);

    report(&checks, order, d_bits);
    show(&checks);
}

fn report(checks: &[Check; 7], order: [usize; 3], d_bits: u32) {
    use core::fmt::Write;
    let mut console = Stdout;
    let passed = checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(console, "[sigwait] sonuc: {}/7 gecti", passed);
    let _ = writeln!(
        console,
        "[sigwait] isleyici kosma sayisi: {}  sira: {},{},{}  D bitleri: 0x{:x}",
        HANDLER_RAN.load(Ordering::SeqCst),
        order[0],
        order[1],
        order[2],
        d_bits
    );
}

fn show(checks: &[Check; 7]) {
    let mut win = match Window::open("sigwait -- sinyali okumak", 260, 130, 480, 250) {
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
    win.text(6, 3, "Cekirdek cagirmiyor -- program okuyor", ACCENT);

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

    win.text(6, h - 46, "isleyici kosma sayisi (0 olmali):", DIM);
    let ran = HANDLER_RAN.load(Ordering::SeqCst);
    win.number(290, h - 46, ran, if ran == 0 { OK } else { WARN });

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
