//! `winapc.exe` -- APC kuyruklari: kesinti degil, **randevu**.
//!
//! Bir onceki bati POSIX tarafinda gercek-zamanli sinyalleri getirdi ve
//! sonunda su cumle vardi: "Windows'ta en yakin karsilik APC
//! kuyruklaridir." Bu sinav o cumleyi olcuyor.
//!
//! Ikisi de ayni ise yariyor -- bir akisa "su an ne yapiyorsan, sunu da
//! calistir" demek. Ama **teslim ani** keskin bicimde ayri:
//!
//! ```text
//!   POSIX sinyal  ->  her sistem cagrisi donusunde teslim edilir
//!                     (surece sorulmaz)
//!   Windows APC   ->  yalnizca UYARILABILIR bir bekleme noktasinda
//!                     (surec acikca izin vermis olmali)
//! ```
//!
//! Varsayilanlar **ters**:
//!
//! ```text
//!   sinyal ->  bolunmek varsayilan.  Istemiyorsan MASKELE.
//!   APC    ->  bolunmemek varsayilan. Istiyorsan IZIN VER.
//! ```
//!
//! Sonucu somut ve B sinavinda goruluyor: hicbir zaman
//! `SleepEx(_, TRUE)` cagirmayan bir akis, kuyrugunda kac APC olursa
//! olsun hicbirini calistirmaz. Bir POSIX programinin "sinyali hic
//! gormedim" demesi icin onu maskelemis olmasi gerekir; bir Windows
//! programinin APC'yi hic gormemesi icin **hicbir sey yapmamasi**
//! yeterli.
//!
//! ## Hangisi daha iyi
//!
//! Ikisi de degil; ikisi de bir sorunu otekine takas ediyor.
//!
//! Sinyal, yeniden-girilebilirlik sorununu **programa** yikiyor: bir
//! isleyici kritik bolgenin ortasinda kosabilir, o yuzden isleyicide ne
//! yapilabilecegi kati bicimde sinirlidir. APC o sorunu tasarim geregi
//! yok ediyor -- yordam yalnizca programin "su an uygunum" dedigi anda
//! kosar, yani kritik bolgede hic kosmaz. Bedeli: bir akisin APC'sini
//! **hic** gormeme ihtimali, ve onunla gelen bir sinif kilitlenme.
//!
//! ## Yedi sinav
//!
//! ```text
//!   A  kuyruga girdi     -> QueueUserAPC sifirdan farkli dondu
//!   B  KENDILIGINDEN     -> siradan Sleep + is: yordam KOSMADI
//!      kosmuyor
//!   C  izin verilince     -> SleepEx(_, TRUE) kosturdu ve
//!      kosuyor              WAIT_IO_COMPLETION dondu
//!   D  deger tasindi     -> dwData yordama ulasti
//!   E  sira ve toplu      -> uc APC, 1-2-3 sirasiyla, TEK beklemede
//!      bosaltma
//!   F  izinsiz bekleme    -> SleepEx(_, FALSE) kosturmadi ve APC
//!                            KAYBOLMADI
//!   G  baska akisa        -> kardes akis kendi beklemesinde kosturdu
//! ```
//!
//! B bu sinavin sebebi. A tek basina yalnizca "cagri kabul edildi"
//! diyor; asil soru kabul edilen seyin **ne zaman** oldugu. F ayni
//! soruyu tersten soruyor ve bir tuzagi kapatiyor: izinsiz bekleme
//! APC'yi kosturmuyorsa, onu **atmis** da olabilirdi. Kaybolmadigini
//! gostermek icin hemen ardindan izinli bir bekleme geliyor.
//!
//! G cagrinin varlik sebebi: `QueueUserAPC`nin asil isi bir akisa is
//! yaptirmaktir, kendine degil.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::winapi::{self, Dword, Window};

tcmk::entry!(main);

const BG: u32 = 0x0012_1622;
const PANEL: u32 = 0x001E_2636;
const FG: u32 = 0x00E2_E8F2;
const DIM: u32 = 0x0086_94A6;
const ACCENT: u32 = 0x0090_C0FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Kac APC yordami kostu (ana akis).
static RAN: AtomicUsize = AtomicUsize::new(0);
/// Gelen degerler, gelis sirasinda.
static VALUES: [AtomicUsize; 4] = [
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
];
static VALUE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Kardes akista kosan APC'nin gordugu deger.
static SIBLING_VALUE: AtomicUsize = AtomicUsize::new(0);
/// Kardes akis dongusunu bitirdi mi.
static SIBLING_DONE: AtomicUsize = AtomicUsize::new(0);
/// Kardes akisin kendi tutamaci -- kuyruklamak icin gerekiyor.
static SIBLING_READY: AtomicUsize = AtomicUsize::new(0);

/// Olculen APC yordami: sayar ve degeri kaydeder.
unsafe extern "system" fn on_apc(data: usize) {
    RAN.fetch_add(1, Ordering::SeqCst);
    let slot = VALUE_COUNT.fetch_add(1, Ordering::SeqCst);
    if slot < VALUES.len() {
        VALUES[slot].store(data, Ordering::SeqCst);
    }
}

/// Kardes akista kosan APC.
unsafe extern "system" fn on_sibling_apc(data: usize) {
    SIBLING_VALUE.store(data, Ordering::SeqCst);
}

/// Kardes akis: kendi uyarilabilir beklemesine girer ve APC'sini bekler.
///
/// Dongu sart: kuyruklama ana akista, bu akis kostuktan **sonra**
/// oluyor. Tek bir `SleepEx` cagirsa APC henuz kuyrukta olmayabilirdi.
unsafe extern "system" fn sibling(_param: *mut c_void) -> Dword {
    SIBLING_READY.store(1, Ordering::SeqCst);
    for _ in 0..40 {
        // Uyarilabilir: izin acikca veriliyor. `TRUE` yerine `FALSE`
        // yazmak bu dongunun APC'yi hic gormemesine yeterdi.
        if winapi::SleepEx(10, 1) == winapi::WAIT_IO_COMPLETION {
            break;
        }
    }
    SIBLING_DONE.store(1, Ordering::SeqCst);
    0
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
    "A kuyruga girdi",
    "B kendiliginden kosmuyor",
    "C izin verilince kosuyor",
    "D deger tasindi",
    "E sira + toplu bosaltma",
    "F izinsiz bekleme",
    "G baska akisa",
];

/// Sayaclari sifirlar -- her sinav temiz baslar.
fn reset() {
    RAN.store(0, Ordering::SeqCst);
    VALUE_COUNT.store(0, Ordering::SeqCst);
    for slot in VALUES.iter() {
        slot.store(usize::MAX, Ordering::SeqCst);
    }
}

fn main() {
    let mut checks = [EMPTY; 7];
    let me = winapi::CURRENT_THREAD;

    // --- A ve B: kuyruga girdi, ama kendiliginden kosmadi ------------
    //
    // Ikisi tek bir kalipta olculuyor cunku ayni anin iki yuzu: cagri
    // kabul edildi (A) ve kabul edilen sey **henuz olmadi** (B).
    reset();
    let queued = unsafe { winapi::QueueUserAPC(on_apc, me, 7) };
    checks[0] = Check {
        name: NAMES[0],
        detail: if queued != 0 {
            "QueueUserAPC kabul etti"
        } else {
            "QueueUserAPC reddetti (destek yok?)"
        },
        passed: queued != 0,
    };

    // Uyarilabilir OLMAYAN is: siradan uyku, saat okuma, pencere yok.
    // Bir POSIX sinyali bunlarin hepsinde teslim edilirdi -- her biri
    // bir sistem cagrisi ve teslim noktasi tam da syscall donusu.
    for _ in 0..3 {
        unsafe { winapi::Sleep(10) };
        let _ = unsafe { winapi::GetTickCount() };
    }
    let ran_early = RAN.load(Ordering::SeqCst);
    checks[1] = Check {
        name: NAMES[1],
        detail: if queued == 0 {
            "kuyruga girmedi, olculemedi"
        } else if ran_early == 0 {
            "Sleep + is: yordam kosmadi (izin verilmedi)"
        } else {
            "yordam IZINSIZ kostu: sinyal gibi davraniyor"
        },
        passed: queued != 0 && ran_early == 0,
    };

    // --- C ve D: izin verilince kosuyor, degeriyle --------------------
    let woke = unsafe { winapi::SleepEx(0, 1) };
    let ran_now = RAN.load(Ordering::SeqCst);
    checks[2] = Check {
        name: NAMES[2],
        detail: if ran_now == 1 && woke == winapi::WAIT_IO_COMPLETION {
            "SleepEx(_,TRUE) kosturdu, WAIT_IO_COMPLETION dondu"
        } else if ran_now == 1 {
            "yordam kostu ama donus WAIT_IO_COMPLETION degil"
        } else if ran_now == 0 {
            "izin verildi ama yordam yine kosmadi"
        } else {
            "beklenenden fazla yordam kostu"
        },
        passed: ran_now == 1 && woke == winapi::WAIT_IO_COMPLETION,
    };

    let seen = VALUES[0].load(Ordering::SeqCst);
    checks[3] = Check {
        name: NAMES[3],
        detail: if seen == 7 {
            "dwData yordama ulasti"
        } else if seen == 0 {
            "deger sifir geldi: dwData tasinmiyor"
        } else {
            "deger beklenenden farkli"
        },
        passed: seen == 7,
    };

    // --- E: sira ve toplu bosaltma ------------------------------------
    //
    // Uc APC kuyruga giriyor ve **tek** bir uyarilabilir bekleme
    // hepsini bosaltiyor. Windows'un sozlesmesi budur: bekleme, kuyruk
    // bosalana kadar surer ve sonunda tek bir WAIT_IO_COMPLETION doner.
    reset();
    let mut accepted = 0;
    for value in [11usize, 22, 33] {
        if unsafe { winapi::QueueUserAPC(on_apc, me, value) } != 0 {
            accepted += 1;
        }
    }
    let batch = unsafe { winapi::SleepEx(0, 1) };
    let values = [
        VALUES[0].load(Ordering::SeqCst),
        VALUES[1].load(Ordering::SeqCst),
        VALUES[2].load(Ordering::SeqCst),
    ];
    let fifo = accepted == 3
        && RAN.load(Ordering::SeqCst) == 3
        && values == [11, 22, 33]
        && batch == winapi::WAIT_IO_COMPLETION;
    checks[4] = Check {
        name: NAMES[4],
        detail: if fifo {
            "uc APC, 11-22-33 sirasiyla, tek beklemede"
        } else if values[0] == 33 {
            "sira TERS: kuyruk degil yigin"
        } else if RAN.load(Ordering::SeqCst) == 1 {
            "tek bekleme yalnizca BIR APC kosturdu"
        } else {
            "sira ya da sayi beklenenden farkli"
        },
        passed: fifo,
    };

    // --- F: izinsiz bekleme kosturmaz, ama APC'yi de ATMAZ -----------
    //
    // Tek basina "kosmadi" yeterli degil: kosmamasinin sebebi APC'nin
    // atilmis olmasi da olabilirdi. Ikinci yari bunu kapatiyor.
    reset();
    let f_queued = unsafe { winapi::QueueUserAPC(on_apc, me, 99) } != 0;
    let plain = unsafe { winapi::SleepEx(10, 0) };
    let after_plain = RAN.load(Ordering::SeqCst);
    let alert = unsafe { winapi::SleepEx(0, 1) };
    let after_alert = RAN.load(Ordering::SeqCst);
    let kept = f_queued
        && after_plain == 0
        && plain == 0
        && after_alert == 1
        && alert == winapi::WAIT_IO_COMPLETION;
    checks[5] = Check {
        name: NAMES[5],
        detail: if kept {
            "SleepEx(_,FALSE) kosturmadi, APC kuyrukta kaldi"
        } else if after_plain != 0 {
            "izinsiz bekleme yordami KOSTURDU"
        } else if after_alert == 0 {
            "izinsiz bekleme APC'yi ATTI"
        } else {
            "donus degerleri beklenenden farkli"
        },
        passed: kept,
    };

    // --- G: baska bir akisa kuyruklama --------------------------------
    //
    // Cagrinin varlik sebebi. Kardes akis kendi uyarilabilir
    // beklemesinde kosuyor; ana akis yalnizca kuyruga koyuyor.
    let mut thread_id = 0u32;
    let thread = unsafe {
        winapi::CreateThread(
            core::ptr::null_mut(),
            0,
            Some(sibling),
            core::ptr::null_mut(),
            0,
            &mut thread_id,
        )
    };
    let mut g_detail = "kardes akis kurulamadi";
    let mut g_passed = false;
    if thread != 0 {
        // Kardes gercekten kosmaya baslasin: kuyruga onceden koymak da
        // calisirdi ama o zaman "beklemede kostu" degil "baslarken
        // kostu" olcerdik.
        for _ in 0..50 {
            if SIBLING_READY.load(Ordering::SeqCst) == 1 {
                break;
            }
            unsafe { winapi::Sleep(10) };
        }
        let sent = unsafe { winapi::QueueUserAPC(on_sibling_apc, thread, 55) } != 0;
        let _ = unsafe { winapi::WaitForSingleObject(thread, winapi::INFINITE) };
        let value = SIBLING_VALUE.load(Ordering::SeqCst);
        let done = SIBLING_DONE.load(Ordering::SeqCst) == 1;
        g_passed = sent && done && value == 55;
        g_detail = if g_passed {
            "kardes akis kendi beklemesinde kosturdu"
        } else if !sent {
            "baska akisa kuyruklama reddedildi"
        } else if value == 0 {
            "kardes akis APC'yi hic gormedi"
        } else {
            "kardes akiste deger yanlis"
        };
    }
    checks[6] = Check {
        name: NAMES[6],
        detail: g_detail,
        passed: g_passed,
    };

    report(&checks);
    show(&checks);
}

fn report(checks: &[Check; 7]) {
    use core::fmt::Write;
    let mut console = winapi::Console;
    let _ = writeln!(console, "[winapc] sinav basliyor");
    for check in checks {
        let _ = writeln!(
            console,
            "[winapc] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let _ = writeln!(
        console,
        "[winapc] toplam kosan yordam: {}  WAIT_IO_COMPLETION = 0x{:x}",
        RAN.load(Ordering::SeqCst),
        winapi::WAIT_IO_COMPLETION
    );
}

fn show(checks: &[Check; 7]) {
    let mut win = match Window::create("winapc -- kesinti degil, randevu", 250, 150, 500, 250) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, checks);
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 7]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "APC yalnizca IZIN VERILEN anda kosar", ACCENT);

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

    win.text(6, h - 46, "sinyal: bolunmek varsayilan, istemiyorsan maskele", DIM);
    win.text(6, h - 30, "APC:    bolunmemek varsayilan, istiyorsan izin ver", DIM);

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
