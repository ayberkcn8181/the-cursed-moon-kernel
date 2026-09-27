//! `sigfault` -- ayni hata, iki yuz: POSIX de artik yakalayip duzeltiyor.
//!
//! TCMK'nin Windows yuzu uzun suredir bir sayfa hatasini yakalayip
//! **duzeltebiliyordu**: `winseh` bilerek sifir adrese yaziyor, isleyici
//! hatali isaretciyi tutan registeri duzeltiyor ve komut tekrarlaniyor.
//! POSIX yuzunde ayni hata surecin sonuydu -- `SIGSEGV` yalnizca bir
//! olum sebebi olarak kaydediliyor, hic teslim edilmiyordu.
//!
//! Bu, projenin tezinde acik bir gedikti: iki ABI esit vatandas olacaksa
//! ayni donanim olayina ayni gucte cevap verebilmeliler.
//!
//! ## Ayni is, iki bicim
//!
//! ```text
//!   Windows  EXCEPTION_RECORD + CONTEXT   -> isleyici(&pointers)
//!            duzeltme: CONTEXT'e yaz, EXCEPTION_CONTINUE_EXECUTION don
//!
//!   POSIX    siginfo_t + ucontext_t       -> isleyici(signo, &si, &uc)
//!            duzeltme: ucontext_t'ye yaz, ISLEYICIDEN DON
//! ```
//!
//! Sag sutunun ikinci satiri POSIX'in daha yalin oldugu yer: "devam et"
//! demek icin ayri bir donus degeri yok, donusun kendisi o anlama
//! geliyor. Cekirdek `sigreturn`da baglami `ucontext_t`den geri okuyor
//! -- isleyici orada ne birakmissa o yuruyor.
//!
//! ## `SA_SIGINFO`: sinyalin iki sorusu daha
//!
//! ```text
//!   tek argumanli  handler(signo)                       "hangi sinyal"
//!   uc argumanli   handler(signo, &siginfo, &ucontext)  "+ neden, + nerede"
//! ```
//!
//! `siginfo_t` nedeni tasiyor: `si_code` sinyalin **kaynagini**
//! (eslenmemis sayfa mi, izin ihlali mi, yoksa bir surecin gonderdigi
//! `kill` mi), `si_addr` ise hataya yol acan adresi. Ayrim uydurma
//! degil: tembel bir ayirici icin eslenmemis sayfa **beklenen** bir
//! olaydir, izin ihlali degildir.
//!
//! ## Yedi sinav
//!
//! ```text
//!   A  yakalandi      -> SIGSEGV isleyicisi kostu ve surec OLMEDI
//!   B  si_addr        -> iki olayda da erisilmek istenen adres geldi
//!   C  si_code        -> eslenmemis adres SEGV_MAPERR diye geliyor
//!   D  DUZELTILDI     -> ucontext'teki register degistirildi, komut
//!                        tekrarlandi, yazma DOGRU yere dustu
//!   E  sifira bolme   -> SIGFPE de ayni yoldan; boleni duzeltmek yetti
//!   F  kill'in kimligi-> SI_USER ve si_pid gonderen surec
//!   G  izolasyon      -> isleyici YOKSA surec yine SIGSEGV ile oluyor
//! ```
//!
//! D bu sinavin sebebi. A tek basina "isleyici cagrildi" demek olurdu ve
//! bu, yakalamanin yarisi: bir sayfa hatasi isleyicisinden **duzeltmeden**
//! donmek ayni komutu yeniden calistirir, yani sonsuz dongu. Yakalamanin
//! bir ise yaramasi icin baglamin yazilabilir olmasi sart.
//!
//! G ters yonden bakiyor ve en az digerleri kadar onemli: yeni yol, bir
//! isleyici **yokken** hata izolasyonunu bozmamali. Cocuk surec bilerek
//! isleyicisiz cokuyor ve ebeveyn `WIFSIGNALED`/`WTERMSIG` ile olum
//! sebebini okuyor.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::signal::{self, Reg, SigInfo, UContext};
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0016_1C26;
const PANEL: u32 = 0x0024_2E3C;
const FG: u32 = 0x00E6_EAF2;
const DIM: u32 = 0x0090_98A8;
const ACCENT: u32 = 0x00F0_B060;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Isleyicinin yazmayi yonlendirecegi gecerli hedef.
///
/// `winseh`teki `SCRATCH` ile ayni rol: duzeltmenin **gerceklestigini**
/// gosteren yer. Sifir adrese giden yazma buraya dusmeli.
static mut SCRATCH: usize = 0;

/// Yazilmaya calisilan deger -- rastgele degil, taninabilir olsun diye.
const MARK: usize = 0x5A5A_1234;

/// Hatali erisimin **iki** ayri hedefi: biri cok dusuk, oteki cok
/// yuksek bir adres.
///
/// Sifir kullanilmiyor: `si_addr`in tam bu sayilar oldugunu sinamak,
/// "sifir mi" demekten cok daha dar bir olcu -- sifir bircok yolla
/// ortaya cikabilir. Ikisinin birlikte olculmesi de kasitli: tek bir
/// adres, raporlananin gercekten **o** adres oldugunu gosteremezdi.
///
/// Ikisi de surec acisindan eslenmemis, yani ikisinde de beklenen
/// `si_code` `SEGV_MAPERR`. Donanim acisindan ayni degiller ve sinavin
/// ilginc yani burada: cekirdek dusuk bellegi kendi icin her adres
/// uzayina esliyor, yani `LOW_ADDR` donanima **var** gorunuyor. Dogru
/// cevap yine de "yok" -- cunku Ring 3'e kapali (bkz.
/// `exceptions::fault_info`).
const LOW_ADDR: usize = 0x0000_1234;
const HIGH_ADDR: usize = 0x0700_0000;

static HITS: AtomicUsize = AtomicUsize::new(0);
static SEEN_CODE: AtomicUsize = AtomicUsize::new(usize::MAX);
static SEEN_ADDR: AtomicUsize = AtomicUsize::new(usize::MAX);
static SEEN_SIGNO: AtomicUsize = AtomicUsize::new(0);

static FPE_HITS: AtomicUsize = AtomicUsize::new(0);
static FPE_CODE: AtomicUsize = AtomicUsize::new(usize::MAX);

static KILL_CODE: AtomicUsize = AtomicUsize::new(usize::MAX);
static KILL_PID: AtomicUsize = AtomicUsize::new(usize::MAX);

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

/// `SIGSEGV` isleyicisi: kaydi okur, baglami **duzeltir**.
///
/// Duzeltme tek satir: hatali isaretciyi tutan register gecerli bir
/// adrese cevriliyor. Donusten sonra cekirdek baglami `ucontext_t`den
/// geri okuyor, yani ayni komut bu sefer `SCRATCH`e yaziyor.
extern "C" fn on_segv(signo: u32, info: *const SigInfo, context: *mut UContext) {
    let hit = HITS.fetch_add(1, Ordering::SeqCst);
    SEEN_SIGNO.store(signo as usize, Ordering::SeqCst);

    // --- Dongu freni ---
    //
    // Bir sayfa hatasi isleyicisinden **duzeltmeden** donmek ayni komutu
    // yeniden calistirir: sonsuz dongu. Duzeltme yurumuyorsa sinav bunu
    // asili kalarak degil, **soyleyerek** bildirmeli -- daha once iki
    // kez ogrenilmis bir ders (bkz. README, "sinavin temizligi").
    //
    // Cikis kodu tek isaret: hicbir bit kurulu degil, LOOPED kurulu.
    if hit >= LOOP_LIMIT {
        tcmk::sys::exit(BIT_LOOPED);
    }
    unsafe {
        SEEN_CODE.store((*info).code as usize, Ordering::SeqCst);
        SEEN_ADDR.store((*info).addr(), Ordering::SeqCst);
        signal::set_reg(
            context,
            Reg::Cx,
            core::ptr::addr_of_mut!(SCRATCH) as usize,
        );
    }
}

/// `SIGFPE` isleyicisi: boleni sifirdan farkli yapar.
///
/// `SIGSEGV`den ayri bir isleyici olmasi gerekli degildi ama aciklayici:
/// duzeltilen sey her iki durumda da **bir register**, degisen yalnizca
/// hangisi ve niye.
extern "C" fn on_fpe(_signo: u32, info: *const SigInfo, context: *mut UContext) {
    FPE_HITS.fetch_add(1, Ordering::SeqCst);
    unsafe {
        FPE_CODE.store((*info).code as usize, Ordering::SeqCst);
        signal::set_reg(context, Reg::Cx, 4);
    }
}

/// `kill` ile gelen sinyalin isleyicisi -- burada duzeltilecek bir sey
/// yok, okunacak bir **kimlik** var.
extern "C" fn on_usr1(_signo: u32, info: *const SigInfo, _context: *mut UContext) {
    unsafe {
        KILL_CODE.store((*info).code as usize, Ordering::SeqCst);
        KILL_PID.store((*info).pid(), Ordering::SeqCst);
    }
}

/// Gecersiz bir adrese yazar.
///
/// Hedef adres bilerek **belirli bir registerde** (`ecx`/`rcx`)
/// tutuluyor: isleyicinin duzeltecegi sey tam olarak o register.
/// `inout(...) => _` yazilmasinin sebebi de bu -- isleyici registeri
/// degistirdigi icin derleyici eski degerin korundugunu varsaymamali.
///
/// Windows ikizi (`winseh::write_through_null`) ile **ayni komut**.
#[inline(never)]
unsafe fn write_through(addr: usize, value: usize) {
    #[cfg(target_arch = "x86")]
    core::arch::asm!("mov [ecx], edx", inout("ecx") addr => _, inout("edx") value => _);
    #[cfg(target_arch = "x86_64")]
    core::arch::asm!("mov [rcx], rdx", inout("rcx") addr => _, inout("rdx") value => _);
}

/// Sifira boler; bolen `ecx`te.
#[inline(never)]
unsafe fn divide_by_zero(numerator: u32) -> u32 {
    let quotient: u32;
    core::arch::asm!(
        "div ecx",
        inout("eax") numerator => quotient,
        inout("edx") 0u32 => _,
        inout("ecx") 0u32 => _,
    );
    quotient
}

/// Isleyicinin ayni hataya kac kez girmeyi kabul ettigi.
const LOOP_LIMIT: usize = 8;

// Cocugun cikis kodunda tasidigi sonuclar. Tek bir bayt, alti bit:
// boru kurmaya gerek birakmiyor ve cocugun **hic** donmemesi de bir
// cevap oluyor.
const BIT_CAUGHT: i32 = 1;
const BIT_ADDR: i32 = 2;
const BIT_CODE: i32 = 4;
const BIT_FIXED: i32 = 8;
const BIT_FPE: i32 = 16;
/// Isleyici donguye girdi: duzeltme yurumedi.
const BIT_LOOPED: i32 = 64;

/// Hatalari **cocukta** uretip sonucu cikis kodunda dondurur.
///
/// Ayri bir surecte olmasinin tek sebebi dayaniklilik: duzeltme
/// yurumezse bu surec sonsuza kadar ayni komutu tekrarlar. Ebeveyn onu
/// sinirli bir sure bekleyip **rapor edebilir**; ayni islemi kendi
/// icinde yapsaydi sinav asili kalirdi.
fn faulting_child() -> ! {
    let mut bits = 0i32;

    unsafe { SCRATCH = 0 };
    unsafe { write_through(LOW_ADDR, MARK) };
    let landed = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(SCRATCH)) };
    let hits = HITS.load(Ordering::SeqCst);
    let low_code = SEEN_CODE.load(Ordering::SeqCst);
    let low_addr = SEEN_ADDR.load(Ordering::SeqCst);

    unsafe { write_through(HIGH_ADDR, MARK) };
    let high_code = SEEN_CODE.load(Ordering::SeqCst);
    let high_addr = SEEN_ADDR.load(Ordering::SeqCst);

    if hits == 1 && SEEN_SIGNO.load(Ordering::SeqCst) == signal::SIGSEGV as usize {
        bits |= BIT_CAUGHT;
    }
    if low_addr == LOW_ADDR && high_addr == HIGH_ADDR {
        bits |= BIT_ADDR;
    }
    if low_code == signal::SEGV_MAPERR as usize && high_code == signal::SEGV_MAPERR as usize {
        bits |= BIT_CODE;
    }
    if landed == MARK {
        bits |= BIT_FIXED;
    }

    let quotient = unsafe { divide_by_zero(100) };
    if FPE_HITS.load(Ordering::SeqCst) == 1
        && quotient == 25
        && FPE_CODE.load(Ordering::SeqCst) == signal::FPE_INTDIV as usize
    {
        bits |= BIT_FPE;
    }

    tcmk::sys::exit(bits)
}

/// Cocugu **sinirli** bir sure bekler.
///
/// `waitpid`in bloke eden hali burada kullanilamaz: cocuk donguye
/// girmisse hic bitmez ve sinav donmez.
fn wait_bounded(child: usize, tries: usize) -> Option<u32> {
    let mut status = 0u32;
    for _ in 0..tries {
        if sys::waitpid(child, &mut status, sys::WNOHANG) > 0 {
            return Some(status);
        }
        sys::sleep_ms(25);
    }
    None
}

fn main() {
    let mut checks = [EMPTY; 7];

    signal::action_info(signal::SIGSEGV, on_segv, signal::SA_RESTART, 0);
    signal::action_info(signal::SIGFPE, on_fpe, signal::SA_RESTART, 0);
    signal::action_info(signal::SIGUSR1, on_usr1, signal::SA_RESTART, 0);

    // --- A / B / C / D / E: hatalar **cocukta** ---
    //
    // Duzeltme yurumezse hatali komut sonsuza kadar tekrarlanir. Ayri
    // bir surecte kosmasi, sinavin o durumda asili kalmak yerine
    // **rapor edebilmesi** icin: ebeveyn sinirli bir sure bekliyor ve
    // cevap gelmezse sebebi soyluyor.
    let child = sys::fork();
    if child == 0 {
        faulting_child();
    }
    let outcome = wait_bounded(child as usize, 80);
    if outcome.is_none() {
        // Cocuk hala kosuyor: dongude. Temizlik sinanan seye bagli
        // olmamali, o yuzden dogrudan oldurulur.
        signal::kill(child as usize, signal::SIGKILL);
    }
    let bits = match outcome {
        Some(status) if sys::exited(status) => sys::exit_status(status) as i32,
        _ => 0,
    };
    let looped = outcome.is_none() || bits & BIT_LOOPED != 0;
    // Cocuk bir sinyalle olduyse ayrimi yapmak gerekiyor: yakalama hic
    // calismamis demektir.
    let died = matches!(outcome, Some(status) if sys::signalled(status));

    /// Bir bitin neden kurulu olmadigini anlatan ortak aciklama.
    fn why(looped: bool, died: bool) -> Option<&'static str> {
        if looped {
            Some("isleyici donguye girdi -- duzeltme YURUMEDI")
        } else if died {
            Some("cocuk sinyalle oldu -- hata yakalanmadi")
        } else {
            None
        }
    }

    checks[0] = Check {
        name: NAMES[0],
        detail: match why(looped, died) {
            Some(reason) => reason,
            None if bits & BIT_CAUGHT != 0 => "SIGSEGV yakalandi, surec yasiyor",
            None => "isleyici cagrilmadi ya da birden fazla kez cagrildi",
        },
        passed: bits & BIT_CAUGHT != 0,
    };

    checks[1] = Check {
        name: NAMES[1],
        detail: match why(looped, died) {
            Some(reason) => reason,
            None if bits & BIT_ADDR != 0 => "her iki olayda da erisilen adres geldi",
            None => "si_addr BASKA bir adres gosterdi",
        },
        passed: bits & BIT_ADDR != 0,
    };

    // `si_code` sinyalin **kaynagini** soyluyor mu. Beklenen ikisinde de
    // `SEGV_MAPERR`: iki adres de surec acisindan eslenmemis.
    // `LOW_ADDR`in donanima "var" gorunmesi cevabi degistirmemeli --
    // degistirseydi, surecin hic goremedigi bir adres "izin ihlali" diye
    // raporlanirdi. Sinav hem eski davranisi (`SI_KERNEL`) hem de ham
    // donanim bitini okumayi (`SEGV_ACCERR`) dusurur.
    checks[2] = Check {
        name: NAMES[2],
        detail: match why(looped, died) {
            Some(reason) => reason,
            None if bits & BIT_CODE != 0 => "iki eslenmemis adres de SEGV_MAPERR",
            None => "si_code yanlis sebebi gosteriyor",
        },
        passed: bits & BIT_CODE != 0,
    };

    // D: duzeltmenin yurudugu tek kanit -- yazma DOGRU yere dustu.
    // Bu bit, cekirdegin `ucontext_t`yi geri okudugunu ve hatali komutu
    // tekrarladigini **birlikte** gosteriyor.
    checks[3] = Check {
        name: NAMES[3],
        detail: match why(looped, died) {
            Some(reason) => reason,
            None if bits & BIT_FIXED != 0 => "register duzeltildi, yazma dogru yere dustu",
            None => "komut tekrarlandi ama yazma dogru yere DUSMEDI",
        },
        passed: bits & BIT_FIXED != 0,
    };

    checks[4] = Check {
        name: NAMES[4],
        detail: match why(looped, died) {
            Some(reason) => reason,
            None if bits & BIT_FPE != 0 => "bolen 4 yapildi, 100/4 = 25",
            None => "SIGFPE yakalanmadi ya da bolum yanlis",
        },
        passed: bits & BIT_FPE != 0,
    };

    // --- F: kill ile gelen sinyalin kimligi ---
    //
    // Hatadan farkli bir yol: bu sinyal donanimdan degil bir **surecten**
    // geliyor. `si_code` ikisini ayirt ediyor, `si_pid` gonderen.
    let me = sys::getpid() as usize;
    signal::kill(me, signal::SIGUSR1);
    let kill_code = KILL_CODE.load(Ordering::SeqCst);
    let kill_pid = KILL_PID.load(Ordering::SeqCst);
    let kill_ok = kill_code == signal::SI_USER as usize && kill_pid == me;
    checks[5] = Check {
        name: NAMES[5],
        detail: if kill_code == usize::MAX {
            "SIGUSR1 isleyicisi cagrilmadi"
        } else if kill_code != signal::SI_USER as usize {
            "si_code SI_USER degil"
        } else if kill_ok {
            "SI_USER ve si_pid gonderen surec"
        } else {
            "si_pid gonderenin kimligi DEGIL"
        },
        passed: kill_ok,
    };

    // --- G: isleyici yoksa izolasyon bozulmadi mi ---
    //
    // Yeni yolun en kolay bozacagi sey bu: hatayi sinyale cevirirken
    // isleyicisiz surecin olmemesi ya da yanlis sebeple olmesi. Cocuk
    // bilerek cokuyor, ebeveyn olum **sebebini** okuyor.
    let child = sys::fork();
    if child == 0 {
        // Miras alinan isleyici kaldirilir: sinav tam da "isleyici yok"
        // durumunu olcuyor.
        signal::default(signal::SIGSEGV);
        unsafe { write_through(HIGH_ADDR, MARK) };
        // Buraya gelinmemeli. Gelinirse ayirt edilebilir bir kodla cik.
        sys::exit(7);
    }
    let mut status = 0u32;
    sys::waitpid(child as usize, &mut status, 0);
    let died_right = sys::signalled(status) && sys::term_signal(status) == signal::SIGSEGV;
    checks[6] = Check {
        name: NAMES[6],
        detail: if sys::exited(status) && sys::exit_status(status) == 7 {
            "cocuk cokmedi -- hata YUTULDU"
        } else if died_right {
            "isleyicisiz cocuk SIGSEGV ile oldu"
        } else if sys::signalled(status) {
            "oldu ama sebep SIGSEGV degil"
        } else {
            "cocugun olum sebebi okunamadi"
        },
        passed: died_right,
    };

    report(&checks, bits);
    show(&checks, bits);
}

const NAMES: [&str; 7] = [
    "A yakalandi",
    "B si_addr",
    "C si_code",
    "D DUZELTILDI",
    "E sifira bolme",
    "F kill'in kimligi",
    "G izolasyon",
];

fn report(checks: &[Check; 7], bits: i32) {
    use core::fmt::Write;
    let mut console = Stdout;
    for check in checks {
        let _ = writeln!(
            console,
            "[sigfault] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let _ = writeln!(console, "[sigfault] cocugun cevabi: 0x{:02x}", bits);
}

fn show(checks: &[Check; 7], bits: i32) {
    let mut win = match Window::open("sigfault -- POSIX da duzeltebiliyor", 250, 160, 500, 216) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, checks, bits);
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 7], bits: i32) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Sayfa hatasi artik POSIX'te de duzeltilebilir", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            395,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, "cocugun cevabi:", DIM);
    win.number(
        160,
        h - 30,
        bits as usize,
        if bits & BIT_FIXED != 0 { OK } else { WARN },
    );
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
