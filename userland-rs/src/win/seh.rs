//! `winseh.exe` -- Windows istisna dagitimi, ucundan ucuna.
//!
//! Bu program bilerek **coker**: sifir adrese yazar, sifira boler, kendi
//! istisnasini atar. Her seferinde bir isleyici devreye girer, hatayi
//! duzeltir ve program kaldigi yerden devam eder. Bir surec bittikten
//! sonra degil, *icinde* olculen bir sey bu -- yani coken surec
//! yasiyorsa dagitim gercekten calisiyor demektir.
//!
//! ## Neden onemli
//!
//! TEB kurulduktan sonra bir PE ikilisi `fs:[0]`da bir zincir
//! **gorebiliyordu** ama zincir yurutulmuyordu: bir sayfa hatasi surecin
//! sonuydu. Windows programlarinin buyuk kismi bunu varsayamaz --
//! `__try`/`__except` derleyicinin urettigi siradan bir yapidir.
//!
//! ## Iki mekanizma
//!
//! ```text
//!   VEH   AddVectoredExceptionHandler   surec genelinde, 32 ve 64 bit
//!   SEH   fs:[0] zinciri                yiginda, YALNIZCA 32 bit
//! ```
//!
//! Ikinci satirin "yalnizca 32 bit" olmasi TCMK'nin eksigi degil,
//! Windows'un kendi tercihi: Microsoft 64-bit'te tablo tabanli
//! (`.pdata`) cozüme gecti. TCMK ayni ayrimi tasiyor, o yuzden F ve H
//! sinavlari x86_64'te **atlanir**.
//!
//! Bir baska Windows tuhafligi da olculuyor: ayni anlami tasiyan donus
//! degerleri iki mekanizmada **farkli sayilardir**. VEH'te "devam et"
//! `-1`, zincirde `0`. Cekirdek bunlari karistirsaydi, zincirdeki bir
//! "sirakine gec" (1) yanlis okunurdu.
//!
//! ## Sekiz sinav
//!
//! ```text
//!   A  VEH erisim ihlali   -> isleyici cagrildi, kod 0xC0000005
//!   B  EXCEPTION_RECORD    -> parametreler: [0]=yazma, [1]=hedef adres
//!   C  isleyici sirasi     -> once eklenen once calisir, "sirakine gec"
//!                             gercekten sonrakine gecer
//!   D  Remove...Handler    -> kaldirilan isleyici bir daha cagrilmaz
//!   E  RaiseException      -> yazilim istisnasi ayni yoldan dagitilir
//!   F  SEH zinciri         -> fs:[0] kaydi calisir  (x64'te atlandi)
//!   G  sifira bolme        -> isleyici BOLENI duzeltir, komut tekrarlanir
//!   H  zincir geri alma    -> kayit dustugunde fs:[0] eski haline doner
//! ```
//!
//! G ve A birlikte tek bir seyi soyluyor: isleyici CONTEXT'i
//! degistirebiliyor. Degistiremeseydi "devam et" demenin anlami olmazdi
//! -- ayni komut ayni hatayi verirdi.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

#[cfg(target_arch = "x86")]
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use tcmk::seh::{self, ExceptionPointers, Reg};
use tcmk::winapi::{self, Window};

tcmk::entry!(main);

const BG: u32 = 0x0018_1424;
const PANEL: u32 = 0x0028_2038;
const FG: u32 = 0x00E4_DEF0;
const DIM: u32 = 0x008C_84A0;
const ACCENT: u32 = 0x00B0_A0FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Sifir adrese yazilmaya calisilan deger; isleyici hedefi duzeltince
/// [`SCRATCH`]e duser.
const MARK: usize = 0x5E48_1234;
/// `RaiseException` icin uydurma bir kod. Ust bit takimi (0xE0000000)
/// Windows'un "uygulama tanimli hata" bicimidir.
const RAISE_CODE: u32 = 0xE0BA_D001;
const RAISE_ARG0: usize = 0xC0FF_EE01;
const RAISE_ARG1: usize = 0x0BAD_F00D;

/// Isleyicinin duzelttigi isaretcinin gosterdigi yer.
static mut SCRATCH: usize = 0;

/// Duzeltici isleyicinin gordukleri.
static FIXER_HITS: AtomicU32 = AtomicU32::new(0);
static FIXER_ORDER: AtomicU32 = AtomicU32::new(0);
static SEEN_CODE: AtomicU32 = AtomicU32::new(0);
static SEEN_ACCESS: AtomicUsize = AtomicUsize::new(usize::MAX);
static SEEN_ADDRESS: AtomicUsize = AtomicUsize::new(usize::MAX);
static SEEN_PARAM_COUNT: AtomicU32 = AtomicU32::new(0);

/// Yalnizca "gordum, benim degil" diyen isleyici.
static SPY_HITS: AtomicU32 = AtomicU32::new(0);
static SPY_ORDER: AtomicU32 = AtomicU32::new(0);

/// `RaiseException` sinavinin gordukleri.
static RAISED_CODE: AtomicU32 = AtomicU32::new(0);
static RAISED_ARG0: AtomicUsize = AtomicUsize::new(0);
static RAISED_ARG1: AtomicUsize = AtomicUsize::new(0);

/// Cagrilma sirasini damgalamak icin.
static SEQUENCE: AtomicU32 = AtomicU32::new(1);

/// Zincir isleyicisinin gordukleri (yalnizca i386).
#[cfg(target_arch = "x86")]
static CHAIN_HITS: AtomicU32 = AtomicU32::new(0);
#[cfg(target_arch = "x86")]
static CHAIN_CODE: AtomicU32 = AtomicU32::new(0);

/// Hicbir sey duzeltmeyen isleyici: gordugunu not eder ve sirayi
/// devreder. Windows'ta gunlukleme/telemetri isleyicileri boyledir.
unsafe extern "system" fn spy(info: *mut ExceptionPointers) -> i32 {
    let record = &*(*info).exception_record;
    SPY_HITS.fetch_add(1, Ordering::Relaxed);
    SPY_ORDER.store(SEQUENCE.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
    let _ = record.code;
    seh::EXCEPTION_CONTINUE_SEARCH
}

/// Asil is: hatayi **duzeltip** yurutmeyi surdurur.
///
/// Duzeltme her zaman CONTEXT uzerinden olur. Iki ornek:
///
///   * erisim ihlalinde hatali isaretciyi tutan register (`ecx`/`rcx`)
///     gecerli bir adrese cevrilir;
///   * sifira bolmede bolen register sifirdan farkli yapilir.
///
/// Ikisinde de cekirdek hatali **komutu tekrarlar**; duzeltilmis
/// registerlarla bu sefer basarili olur.
unsafe extern "system" fn fixer(info: *mut ExceptionPointers) -> i32 {
    let record = &*(*info).exception_record;
    let context = (*info).context_record;

    FIXER_HITS.fetch_add(1, Ordering::Relaxed);
    FIXER_ORDER.store(SEQUENCE.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
    SEEN_CODE.store(record.code, Ordering::Relaxed);
    SEEN_PARAM_COUNT.store(record.number_parameters, Ordering::Relaxed);
    if record.number_parameters >= 2 {
        SEEN_ACCESS.store(record.information[0], Ordering::Relaxed);
        SEEN_ADDRESS.store(record.information[1], Ordering::Relaxed);
    }

    match record.code {
        seh::STATUS_ACCESS_VIOLATION => {
            seh::set_reg(context, Reg::C, core::ptr::addr_of_mut!(SCRATCH) as usize);
            seh::EXCEPTION_CONTINUE_EXECUTION
        }
        seh::STATUS_INTEGER_DIVIDE_BY_ZERO => {
            seh::set_reg(context, Reg::C, 4);
            seh::EXCEPTION_CONTINUE_EXECUTION
        }
        RAISE_CODE => {
            RAISED_CODE.store(record.code, Ordering::Relaxed);
            if record.number_parameters >= 2 {
                RAISED_ARG0.store(record.information[0], Ordering::Relaxed);
                RAISED_ARG1.store(record.information[1], Ordering::Relaxed);
            }
            // Yazilim istisnasinda duzeltilecek bir register yok:
            // yurutme cagriyi izleyen komuttan devam eder.
            seh::EXCEPTION_CONTINUE_EXECUTION
        }
        _ => seh::EXCEPTION_CONTINUE_SEARCH,
    }
}

/// Zincir isleyicisi (`fs:[0]`). Imzasi ve donus degerleri VEH'ten
/// **farkli**: dort parametre alir ve "devam et" icin `0` doner.
#[cfg(target_arch = "x86")]
unsafe extern "C" fn chain_handler(
    record: *mut seh::ExceptionRecord,
    _establisher: *mut c_void,
    context: *mut c_void,
    _dispatcher: *mut c_void,
) -> i32 {
    let record = &*record;
    CHAIN_HITS.fetch_add(1, Ordering::Relaxed);
    CHAIN_CODE.store(record.code, Ordering::Relaxed);
    if record.code == seh::STATUS_INTEGER_DIVIDE_BY_ZERO {
        seh::set_reg(context, Reg::C, 4);
        return seh::EXCEPTION_CONTINUE_EXECUTION_SEH;
    }
    seh::EXCEPTION_CONTINUE_SEARCH_SEH
}

/// Sifir adrese yazmayi dener.
///
/// Hedef adres bilerek **belirli bir registerde** (`ecx`/`rcx`) tutulur:
/// isleyicinin duzeltecegi sey tam olarak o register. `inout(... ) => _`
/// yazilmasinin sebebi de bu -- isleyici registeri degistirdigi icin
/// derleyici eski degerin korundugunu varsaymamali.
#[inline(never)]
unsafe fn write_through_null(value: usize) {
    #[cfg(target_arch = "x86")]
    core::arch::asm!("mov [ecx], edx", inout("ecx") 0usize => _, inout("edx") value => _);
    #[cfg(target_arch = "x86_64")]
    core::arch::asm!("mov [rcx], rdx", inout("rcx") 0usize => _, inout("rdx") value => _);
}

/// Sifira boler. Bolen yine `ecx`te; isleyici onu duzeltince ayni komut
/// bu sefer calisir.
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

#[derive(Clone, Copy)]
struct Check {
    name: &'static str,
    detail: &'static str,
    passed: bool,
    skipped: bool,
}

const EMPTY: Check = Check {
    name: "",
    detail: "",
    passed: false,
    skipped: false,
};

fn result(check: &Check) -> &'static str {
    if check.skipped {
        "atlandi"
    } else if check.passed {
        "gecti"
    } else {
        "KALDI"
    }
}

fn main() {
    let mut console = winapi::Console;
    let mut checks = [EMPTY; 8];

    let fixer_handle = unsafe { winapi::AddVectoredExceptionHandler(0, Some(fixer)) };
    if fixer_handle.is_null() {
        let _ = core::fmt::Write::write_str(&mut console, "[winseh] isleyici eklenemedi\n");
        return;
    }

    // --- A: erisim ihlali yakalandi mi ---
    unsafe {
        SCRATCH = 0;
        write_through_null(MARK);
    }
    let landed = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(SCRATCH)) };
    let a = FIXER_HITS.load(Ordering::Relaxed) == 1
        && SEEN_CODE.load(Ordering::Relaxed) == seh::STATUS_ACCESS_VIOLATION
        && landed == MARK;
    checks[0] = Check {
        name: "A VEH erisim ihlali",
        detail: if FIXER_HITS.load(Ordering::Relaxed) == 0 {
            "isleyici HIC cagrilmadi"
        } else if SEEN_CODE.load(Ordering::Relaxed) != seh::STATUS_ACCESS_VIOLATION {
            "istisna kodu 0xC0000005 degil"
        } else if landed != MARK {
            "yurutme surdu ama yazma GITMEDI"
        } else {
            "yakalandi, duzeltildi, komut tekrarlandi"
        },
        passed: a,
        skipped: false,
    };

    // --- B: EXCEPTION_RECORD parametreleri ---
    //
    // Windows erisim ihlalinde iki parametre verir ve ikisi de
    // gerceginden gelir: erisim turu hata kodunun bir bitinden, hedef
    // adres CR2'den.
    let access = SEEN_ACCESS.load(Ordering::Relaxed);
    let address = SEEN_ADDRESS.load(Ordering::Relaxed);
    let b = SEEN_PARAM_COUNT.load(Ordering::Relaxed) == 2 && access == 1 && address == 0;
    checks[1] = Check {
        name: "B kayit parametreleri",
        detail: if SEEN_PARAM_COUNT.load(Ordering::Relaxed) != 2 {
            "parametre sayisi 2 degil"
        } else if access != 1 {
            "erisim turu YAZMA olarak gelmedi"
        } else if address != 0 {
            "hedef adres 0 degil"
        } else {
            "[0]=yazma, [1]=0x00000000"
        },
        passed: b,
        skipped: false,
    };

    // --- C: isleyici sirasi ---
    //
    // `first = 1` ile eklenen isleyici listenin **basina** gecer, yani
    // once o calisir. "Sirakine gec" dedigi icin duzeltici de calisir.
    let spy_handle = unsafe { winapi::AddVectoredExceptionHandler(1, Some(spy)) };
    FIXER_HITS.store(0, Ordering::Relaxed);
    SPY_HITS.store(0, Ordering::Relaxed);
    SEQUENCE.store(1, Ordering::Relaxed);
    unsafe {
        SCRATCH = 0;
        write_through_null(MARK);
    }
    let c = !spy_handle.is_null()
        && SPY_HITS.load(Ordering::Relaxed) == 1
        && FIXER_HITS.load(Ordering::Relaxed) == 1
        && SPY_ORDER.load(Ordering::Relaxed) < FIXER_ORDER.load(Ordering::Relaxed)
        && unsafe { core::ptr::read_volatile(core::ptr::addr_of!(SCRATCH)) } == MARK;
    checks[2] = Check {
        name: "C isleyici sirasi",
        detail: if spy_handle.is_null() {
            "ikinci isleyici eklenemedi"
        } else if SPY_HITS.load(Ordering::Relaxed) == 0 {
            "bastaki isleyici cagrilmadi"
        } else if FIXER_HITS.load(Ordering::Relaxed) == 0 {
            "'sirakine gec' SONRAKINE GECMEDI"
        } else if c {
            "once bastaki, sonra duzeltici"
        } else {
            "sira ters"
        },
        passed: c,
        skipped: false,
    };

    // --- D: kaldirilan isleyici ---
    let removed = unsafe { winapi::RemoveVectoredExceptionHandler(spy_handle) };
    FIXER_HITS.store(0, Ordering::Relaxed);
    SPY_HITS.store(0, Ordering::Relaxed);
    unsafe {
        SCRATCH = 0;
        write_through_null(MARK);
    }
    let d = removed != 0
        && SPY_HITS.load(Ordering::Relaxed) == 0
        && FIXER_HITS.load(Ordering::Relaxed) == 1;
    checks[3] = Check {
        name: "D isleyici kaldirma",
        detail: if removed == 0 {
            "kaldirma BASARISIZ"
        } else if SPY_HITS.load(Ordering::Relaxed) != 0 {
            "kaldirilan isleyici YINE cagrildi"
        } else if FIXER_HITS.load(Ordering::Relaxed) != 1 {
            "kalan isleyici cagrilmadi"
        } else {
            "kaldirildi, bir daha cagrilmadi"
        },
        passed: d,
        skipped: false,
    };

    // --- E: RaiseException ---
    //
    // Yazilim istisnasi donanim istisnasindan ayirt edilmez: ayni
    // dagitici, ayni isleyiciler, ayni kayit. Tek fark kodu **programin**
    // secmesi.
    let args = [RAISE_ARG0, RAISE_ARG1];
    FIXER_HITS.store(0, Ordering::Relaxed);
    unsafe { winapi::RaiseException(RAISE_CODE, 0, 2, args.as_ptr()) };
    let e = FIXER_HITS.load(Ordering::Relaxed) == 1
        && RAISED_CODE.load(Ordering::Relaxed) == RAISE_CODE
        && RAISED_ARG0.load(Ordering::Relaxed) == RAISE_ARG0
        && RAISED_ARG1.load(Ordering::Relaxed) == RAISE_ARG1;
    checks[4] = Check {
        name: "E RaiseException",
        detail: if FIXER_HITS.load(Ordering::Relaxed) == 0 {
            "isleyici cagrilmadi"
        } else if RAISED_CODE.load(Ordering::Relaxed) != RAISE_CODE {
            "kod degisti"
        } else if e {
            "ozel kod + iki parametre dogru geldi"
        } else {
            "parametreler gelmedi"
        },
        passed: e,
        skipped: false,
    };

    // --- G: sifira bolme (siralamada F'ten once yapiliyor) ---
    //
    // Erisim ihlalinden yapisal olarak ayri: orada hatali olan bir
    // *adres*, burada bir *deger*. Ikisi de CONTEXT uzerinden duzeliyor.
    FIXER_HITS.store(0, Ordering::Relaxed);
    let quotient = unsafe { divide_by_zero(100) };
    let g = FIXER_HITS.load(Ordering::Relaxed) == 1
        && SEEN_CODE.load(Ordering::Relaxed) == seh::STATUS_INTEGER_DIVIDE_BY_ZERO
        && quotient == 25;
    checks[6] = Check {
        name: "G sifira bolme",
        detail: if FIXER_HITS.load(Ordering::Relaxed) == 0 {
            "isleyici cagrilmadi"
        } else if quotient != 25 {
            "bolen duzeltildi ama sonuc yanlis"
        } else if g {
            "bolen 4 yapildi, 100/4 = 25"
        } else {
            "istisna kodu beklenen degil"
        },
        passed: g,
        skipped: false,
    };

    // Zincir sinavlari icin VEH temizlenir: vektorlu isleyiciler
    // zincirden ONCE calistigi icin duzeltici sirayi zincire hic
    // birakmazdi.
    let _ = unsafe { winapi::RemoveVectoredExceptionHandler(fixer_handle) };

    // --- F ve H: SEH zinciri ---
    let (f, h) = chain_checks();
    checks[5] = f;
    checks[7] = h;

    for check in &checks {
        let _ = core::fmt::Write::write_str(&mut console, "[winseh] ");
        let _ = core::fmt::Write::write_str(&mut console, check.name);
        let _ = core::fmt::Write::write_str(&mut console, ": ");
        let _ = core::fmt::Write::write_str(&mut console, result(check));
        let _ = core::fmt::Write::write_str(&mut console, " (");
        let _ = core::fmt::Write::write_str(&mut console, check.detail);
        let _ = core::fmt::Write::write_str(&mut console, ")\n");
    }

    let mut win = match Window::create("winseh -- istisna dagitimi", 300, 150, 460, 240) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, &checks);
        win.frame(60);
    }
}

/// `fs:[0]` zinciri: kur, coz, geri al.
///
/// Kaydin **yiginda** durmasi Windows'un kuralidir; bu yuzden
/// `ChainGuard` bir RAII nesnesi. Dustugunde `fs:[0]` eski degerine
/// doner -- H sinavi tam olarak bunu olcuyor. Geri alinmasaydi, bir
/// sonraki istisnada artik var olmayan bir yigin cercevesine
/// dallanilirdi.
#[cfg(target_arch = "x86")]
fn chain_checks() -> (Check, Check) {
    let before = tcmk::teb::exception_list();
    let quotient;
    {
        let mut guard = tcmk::seh::ChainGuard::new(chain_handler);
        unsafe { guard.install() };
        quotient = unsafe { divide_by_zero(100) };
    }
    let after = tcmk::teb::exception_list();

    let hit = CHAIN_HITS.load(Ordering::Relaxed);
    let f = hit == 1
        && CHAIN_CODE.load(Ordering::Relaxed) == seh::STATUS_INTEGER_DIVIDE_BY_ZERO
        && quotient == 25;
    let f_check = Check {
        name: "F SEH zinciri",
        detail: if hit == 0 {
            "zincir isleyicisi cagrilmadi"
        } else if quotient != 25 {
            "cagrildi ama duzeltme uygulanmadi"
        } else if f {
            "fs:[0] kaydi calisti, 100/4 = 25"
        } else {
            "istisna kodu beklenen degil"
        },
        passed: f,
        skipped: false,
    };

    let h = after == before;
    let h_check = Check {
        name: "H zincir geri alma",
        detail: if h {
            "kayit dustu, fs:[0] eski haline dondu"
        } else {
            "fs:[0] OLU bir kaydi gosteriyor"
        },
        passed: h,
        skipped: false,
    };
    (f_check, h_check)
}

/// x86_64'te zincir yok -- Windows'un kendi tercihi (tablo tabanli
/// cozum). Uydurma bir sonuc yazmaktansa acikca atlaniyor.
#[cfg(target_arch = "x86_64")]
fn chain_checks() -> (Check, Check) {
    let skipped = |name| Check {
        name,
        detail: "x64'te SEH zinciri yok (tablo tabanli)",
        passed: false,
        skipped: true,
    };
    (skipped("F SEH zinciri"), skipped("H zincir geri alma"))
}

fn draw(win: &mut Window, checks: &[Check; 8]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "istisna: yakala, duzelt, devam et", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            320,
            y,
            result(check),
            if check.skipped {
                DIM
            } else if check.passed {
                OK
            } else {
                WARN
            },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed || c.skipped).count();
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
