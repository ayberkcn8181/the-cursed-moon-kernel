//! `winalert.exe` -- uyarilabilir bekleme **bolunur**, APC ic ice kosar.
//!
//! APC batisi bir randevu kurdu: APC yalnizca uyarilabilir bir bekleme
//! noktasinda kosar. Ama o noktayi yalnizca **girerken** tanidi --
//! `SleepEx(5000, TRUE)` cagiran bir akisa uykunun ikinci milisaniyesinde
//! gelen APC, bes saniye sonra kosuyordu. Windows'ta bekleme bolunur:
//!
//! ```text
//!   SleepEx(5000, TRUE)       APC gelir  -> hemen kosar,
//!   ......|.....................          WAIT_IO_COMPLETION
//!         ^ QueueUserAPC
//!
//!   SleepEx(5000, FALSE)      APC gelir  -> HICBIR SEY olmaz,
//!                                           APC kuyrukta bekler
//! ```
//!
//! Ikinci eksik ic ice teslimdi: bir APC yordami kendi icinde uyarilabilir
//! bekleme cagirirsa kuyruktaki sonraki APC **icte** kosmali. TCMK'de
//! saklanan baglam tek yuvaliydi ve ikinci APC disari birakiliyordu.
//!
//! ## Sekiz sinav
//!
//! ```text
//!   A  ic ice APC           -> dis APC'nin beklemesi ic APC'yi kosturdu:
//!                              sira O-I-o, ic bekleme WAIT_IO_COMPLETION
//!   B  kat siniri APC'yi    -> kendini kuyruklayan alti APC: en fazla dort
//!      kaybetmez               kat ic ice, sinirda bekleme bos dondu, ama
//!                              altisi de kostu
//!   C  APC'siz uyku surer   -> SleepEx(300, TRUE), APC yok: erken donmedi
//!   D  uyku bolunur         -> SleepEx(2500, TRUE) icindeki akisa 300 ms'de
//!                              APC: hemen kalkti
//!   E  izinsiz uyku         -> SleepEx(800, FALSE): APC kosmadi, uyku
//!      bolunmez                suresince surdu, APC kaybolmadi
//!   F  surec beklemesi      -> WaitForSingleObjectEx(cocuk, INFINITE, TRUE)
//!      bolunur                 APC ile kalkti, cocuk hala calisiyordu
//!   G  cocuk kaybolmaz      -> bolunen beklemeden sonra ikinci bekleme
//!                              cocugun cikis kodunu (7) aldi
//!   H  izinsiz bekleme      -> WaitForSingleObject: APC bolmedi, bekleme
//!      bolunmez                cocuk bitene kadar surdu
//! ```
//!
//! Ciftler: D/E ve F/H. "Bolunur" tek basina "her sey boler"i de gecerdi;
//! izinsiz bicimin bolunmedigini gormek gerekiyor. C de ayni soruyu
//! tersten soruyor: APC'siz bir uyarilabilir uyku erken donmemeli.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use tcmk::winapi::{self, Dword, Handle, Window};

tcmk::entry!(main);

const BG: u32 = 0x0012_1622;
const PANEL: u32 = 0x001E_2636;
const FG: u32 = 0x00E2_E8F2;
const DIM: u32 = 0x0086_94A6;
const ACCENT: u32 = 0x0090_C0FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

const WAIT_IO: Dword = winapi::WAIT_IO_COMPLETION;
/// Cocugun yasadigi sure ve cikis kodu.
const CHILD_MS: Dword = 1200;
const CHILD_CODE: Dword = 7;
/// Ic ice sinavda kac APC kosmali.
const DEEP_TOTAL: usize = 6;
/// Cekirdegin ic ice kat siniri (`apc::MAX_DEPTH`).
const DEEP_LIMIT: usize = 4;

/// Kac APC yordami kostu.
static RAN: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn on_apc(_data: usize) {
    RAN.fetch_add(1, Ordering::SeqCst);
}

// --- A: ic ice --------------------------------------------------------

static LOG: [AtomicU32; 8] = [const { AtomicU32::new(0) }; 8];
static LOG_N: AtomicUsize = AtomicUsize::new(0);
static INNER_RET: AtomicU32 = AtomicU32::new(0xFFFF);

fn log(b: u8) {
    let i = LOG_N.fetch_add(1, Ordering::SeqCst);
    if i < LOG.len() {
        LOG[i].store(b as u32, Ordering::SeqCst);
    }
}

fn log_is(want: &[u8]) -> bool {
    LOG_N.load(Ordering::SeqCst) == want.len()
        && want
            .iter()
            .enumerate()
            .all(|(i, &b)| LOG[i].load(Ordering::SeqCst) == b as u32)
}

unsafe extern "system" fn outer(_data: usize) {
    log(b'O');
    let r = winapi::SleepEx(0, 1);
    INNER_RET.store(r, Ordering::SeqCst);
    log(b'o');
}

unsafe extern "system" fn inner(_data: usize) {
    log(b'I');
}

// --- B: kat siniri -----------------------------------------------------

static LIVE: AtomicUsize = AtomicUsize::new(0);
static MAX_LIVE: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
static LIMIT_HITS: AtomicUsize = AtomicUsize::new(0);

/// Kendini yeniden kuyruklayip uyarilabilir bekleyen APC.
unsafe extern "system" fn deep(_data: usize) {
    let live = LIVE.fetch_add(1, Ordering::SeqCst) + 1;
    MAX_LIVE.fetch_max(live, Ordering::SeqCst);
    let n = TOTAL.fetch_add(1, Ordering::SeqCst) + 1;
    if n < DEEP_TOTAL {
        winapi::QueueUserAPC(deep, winapi::CURRENT_THREAD, 0);
        // Sinira gelinmisse bu bekleme APC kosturamaz ve bos doner.
        if winapi::SleepEx(0, 1) == 0 {
            LIMIT_HITS.fetch_add(1, Ordering::SeqCst);
        }
    }
    LIVE.fetch_sub(1, Ordering::SeqCst);
}

// --- Akis sinavlari: sonuc yuvalari -------------------------------------

/// Akisin olctugu: ilk beklemenin donusu, suresi, o an kac APC kostu,
/// ikinci adimin donusu, sonunda kac APC kostu, cocugun ara ve son kodu.
static R_RET: AtomicU32 = AtomicU32::new(0);
static R_MS: AtomicU32 = AtomicU32::new(0);
static R_RAN_MID: AtomicUsize = AtomicUsize::new(0);
static R_RET2: AtomicU32 = AtomicU32::new(0);
static R_RAN_END: AtomicUsize = AtomicUsize::new(0);
static R_CODE_MID: AtomicU32 = AtomicU32::new(0);
static R_CODE_END: AtomicU32 = AtomicU32::new(0);
static CHILD: AtomicU32 = AtomicU32::new(0);

fn reset() {
    RAN.store(0, Ordering::SeqCst);
    for r in [&R_RET, &R_MS, &R_RET2, &R_CODE_MID, &R_CODE_END] {
        r.store(0xFFFF, Ordering::SeqCst);
    }
    R_RAN_MID.store(usize::MAX, Ordering::SeqCst);
    R_RAN_END.store(usize::MAX, Ordering::SeqCst);
}

fn now() -> Dword {
    unsafe { winapi::GetTickCount() }
}

fn exit_code(h: Handle) -> Dword {
    let mut code = 0;
    unsafe { winapi::GetExitCodeProcess(h, &mut code) };
    code
}

/// D: uzun uyarilabilir uyku.
unsafe extern "system" fn sleep_alertable(_p: *mut c_void) -> Dword {
    let t0 = now();
    let r = winapi::SleepEx(2500, 1);
    R_MS.store(now().wrapping_sub(t0), Ordering::SeqCst);
    R_RET.store(r, Ordering::SeqCst);
    R_RAN_MID.store(RAN.load(Ordering::SeqCst), Ordering::SeqCst);
    0
}

/// E: izinsiz uyku, ardindan izinli bosaltma.
unsafe extern "system" fn sleep_plain(_p: *mut c_void) -> Dword {
    let t0 = now();
    let r = winapi::SleepEx(800, 0);
    R_MS.store(now().wrapping_sub(t0), Ordering::SeqCst);
    R_RET.store(r, Ordering::SeqCst);
    R_RAN_MID.store(RAN.load(Ordering::SeqCst), Ordering::SeqCst);
    R_RET2.store(winapi::SleepEx(0, 1), Ordering::SeqCst);
    R_RAN_END.store(RAN.load(Ordering::SeqCst), Ordering::SeqCst);
    0
}

/// F + G: uyarilabilir surec beklemesi, sonra izinsiz ikinci bekleme.
unsafe extern "system" fn wait_alertable(_p: *mut c_void) -> Dword {
    let child = CHILD.load(Ordering::SeqCst);
    let t0 = now();
    let r = winapi::WaitForSingleObjectEx(child, winapi::INFINITE, 1);
    R_MS.store(now().wrapping_sub(t0), Ordering::SeqCst);
    R_RET.store(r, Ordering::SeqCst);
    R_CODE_MID.store(exit_code(child), Ordering::SeqCst);
    R_RAN_MID.store(RAN.load(Ordering::SeqCst), Ordering::SeqCst);
    R_RET2.store(winapi::WaitForSingleObject(child, winapi::INFINITE), Ordering::SeqCst);
    R_CODE_END.store(exit_code(child), Ordering::SeqCst);
    0
}

/// H: izinsiz surec beklemesi, ardindan izinli bosaltma.
unsafe extern "system" fn wait_plain(_p: *mut c_void) -> Dword {
    let child = CHILD.load(Ordering::SeqCst);
    let t0 = now();
    let r = winapi::WaitForSingleObject(child, winapi::INFINITE);
    R_MS.store(now().wrapping_sub(t0), Ordering::SeqCst);
    R_RET.store(r, Ordering::SeqCst);
    R_RAN_MID.store(RAN.load(Ordering::SeqCst), Ordering::SeqCst);
    R_RET2.store(winapi::SleepEx(0, 1), Ordering::SeqCst);
    R_RAN_END.store(RAN.load(Ordering::SeqCst), Ordering::SeqCst);
    0
}

/// Bir akis baslatir, 300 ms sonra ona APC kuyruklar ve **sinirli** sure
/// bekler. Doner: (kuyruga girdi mi, akis bitti mi).
fn run_with_apc(f: unsafe extern "system" fn(*mut c_void) -> Dword, limit_ms: Dword) -> (bool, bool) {
    let thread = unsafe {
        winapi::CreateThread(
            core::ptr::null_mut(),
            0,
            Some(f),
            core::ptr::null_mut(),
            0,
            core::ptr::null_mut(),
        )
    };
    if thread == 0 {
        return (false, false);
    }
    // Akis bekleme cagrisinin **icine** girsin: once kuyruklamak "girerken
    // kostu"yu olcerdi, bolunmeyi degil.
    unsafe { winapi::Sleep(300) };
    let queued = unsafe { winapi::QueueUserAPC(on_apc, thread, 1) } != 0;
    let mut done = false;
    for _ in 0..(limit_ms / 20).max(1) {
        if unsafe { winapi::WaitForSingleObject(thread, 0) } == winapi::WAIT_OBJECT_0 {
            done = true;
            break;
        }
        unsafe { winapi::Sleep(20) };
    }
    unsafe { winapi::CloseHandle(thread) };
    (queued, done)
}

/// Ayni ikiliyi cocuk rolunde baslatir. Doner: surec tutamaci (0 = olmadi).
fn spawn_child() -> Handle {
    let mut info = winapi::ProcessInformation::new();
    let created = unsafe {
        winapi::CreateProcessA(
            b"C:\\bin\\winalert.exe\0".as_ptr(),
            b"winalert.exe c\0".as_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            0,
            0,
            core::ptr::null_mut(),
            core::ptr::null(),
            core::ptr::null_mut(),
            &mut info,
        )
    };
    if created == 0 {
        return 0;
    }
    unsafe { winapi::CloseHandle(info.thread) };
    info.process
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

const NAMES: [&str; 8] = [
    "A ic ice APC",
    "B kat siniri kaybetmez",
    "C APC'siz uyku surer",
    "D uyku bolunur",
    "E izinsiz uyku bolunmez",
    "F surec beklemesi bolunur",
    "G cocuk kaybolmaz",
    "H izinsiz bekleme bolunmez",
];

fn main() {
    if tcmk::args::count() > 1 && tcmk::args::get(1) == Some("c") {
        unsafe {
            winapi::Sleep(CHILD_MS);
            winapi::ExitProcess(CHILD_CODE);
        }
    }

    use core::fmt::Write;
    let mut console = winapi::Console;
    let _ = writeln!(console, "[winalert] sinav basliyor");
    let mut checks = [EMPTY; 8];
    let me = winapi::CURRENT_THREAD;

    // --- A: ic ice ----------------------------------------------------
    //
    // Iki APC kuyrukta: `outer` kendi icinde uyarilabilir bekliyor.
    // Ic ice teslim varsa `inner` o beklemede kosar (O-I-o) ve ic bekleme
    // WAIT_IO_COMPLETION doner; yoksa `inner` disarida, `outer`dan sonra
    // kosar (O-o-I) ve ic bekleme bos doner.
    let qa = unsafe { winapi::QueueUserAPC(outer, me, 0) != 0 && winapi::QueueUserAPC(inner, me, 0) != 0 };
    let ra = unsafe { winapi::SleepEx(0, 1) };
    let a = qa && log_is(b"OIo") && INNER_RET.load(Ordering::SeqCst) == WAIT_IO && ra == WAIT_IO;
    checks[0] = Check {
        name: NAMES[0],
        detail: if a {
            "ic APC dis APC'nin beklemesinde kostu (O-I-o)"
        } else if !qa {
            "kuyruklama reddedildi"
        } else if log_is(b"OoI") {
            "ic ice teslim YOK: ic APC disari birakildi (O-o-I)"
        } else if log_is(b"OI") {
            "dis APC'nin beklemesi DONMEDI: saklanan baglam ezildi"
        } else {
            "sira ya da donus degerleri beklenenden farkli"
        },
        passed: a,
    };

    // --- B: kat siniri ------------------------------------------------
    let qb = unsafe { winapi::QueueUserAPC(deep, me, 0) } != 0;
    let rb = unsafe { winapi::SleepEx(0, 1) };
    let max_live = MAX_LIVE.load(Ordering::SeqCst);
    let total = TOTAL.load(Ordering::SeqCst);
    let b = qb
        && max_live == DEEP_LIMIT
        && total == DEEP_TOTAL
        && LIMIT_HITS.load(Ordering::SeqCst) >= 1
        && LIVE.load(Ordering::SeqCst) == 0
        && rb == WAIT_IO;
    checks[1] = Check {
        name: NAMES[1],
        detail: if b {
            "dort kat ic ice, sinirda bekleme bos dondu, alti APC de kostu"
        } else if !qb {
            "kuyruklama reddedildi"
        } else if total < DEEP_TOTAL {
            "sinirda APC KAYBOLDU"
        } else if max_live > DEEP_LIMIT {
            "kat siniri YOK: derinlik dordu asti"
        } else if max_live == 1 {
            "ic ice teslim yok: her APC ayri kostu"
        } else if max_live < DEEP_LIMIT {
            "ic ice kat siniri beklenenden SIG"
        } else if rb != WAIT_IO {
            "dis bekleme WAIT_IO_COMPLETION donmedi: baglam karisti"
        } else {
            "derinlik ya da donus beklenenden farkli"
        },
        passed: b,
    };

    // --- C: APC'siz uyarilabilir uyku suresince surer ------------------
    RAN.store(0, Ordering::SeqCst);
    let t0 = now();
    let rc = unsafe { winapi::SleepEx(300, 1) };
    let mc = now().wrapping_sub(t0);
    let c = rc == 0 && mc >= 250 && RAN.load(Ordering::SeqCst) == 0;
    checks[2] = Check {
        name: NAMES[2],
        detail: if c {
            "APC yokken uyku tam suresince surdu, 0 dondu"
        } else if rc == WAIT_IO {
            "kuyrukta ONCEKI sinavdan kalan bir APC kostu"
        } else if mc < 250 {
            "uyku ERKEN bitti: APC yokken kalkti"
        } else {
            "donus degeri beklenenden farkli"
        },
        passed: c,
    };

    // --- D: uyarilabilir uyku bolunur ---------------------------------
    reset();
    let (qd, dd) = run_with_apc(sleep_alertable, 3500);
    let (rd, md) = (R_RET.load(Ordering::SeqCst), R_MS.load(Ordering::SeqCst));
    let d = qd && dd && rd == WAIT_IO && md < 1500 && md >= 200 && R_RAN_MID.load(Ordering::SeqCst) == 1;
    checks[3] = Check {
        name: NAMES[3],
        detail: if d {
            "2500 ms'lik uyku APC gelince kalkti, WAIT_IO_COMPLETION"
        } else if !qd || !dd {
            "akis kurulamadi ya da donmedi"
        } else if rd == WAIT_IO && md >= 1500 {
            "APC uykuyu BOLMEDI: ancak sure dolunca kostu"
        } else if md < 200 {
            "uyku APC gelmeden bitti"
        } else {
            "donus degeri beklenenden farkli"
        },
        passed: d,
    };

    // --- E: izinsiz uyku bolunmez ---------------------------------------
    reset();
    let (qe, de) = run_with_apc(sleep_plain, 2500);
    let (re, me_ms) = (R_RET.load(Ordering::SeqCst), R_MS.load(Ordering::SeqCst));
    let e = qe
        && de
        && re == 0
        && me_ms >= 700
        && R_RAN_MID.load(Ordering::SeqCst) == 0
        && R_RET2.load(Ordering::SeqCst) == WAIT_IO
        && R_RAN_END.load(Ordering::SeqCst) == 1;
    checks[4] = Check {
        name: NAMES[4],
        detail: if e {
            "SleepEx(_,FALSE) APC'ye ragmen surdu; APC sonra kostu"
        } else if !qe || !de {
            "akis kurulamadi ya da donmedi"
        } else if me_ms < 700 {
            "izinsiz uyku BOLUNDU"
        } else if R_RAN_MID.load(Ordering::SeqCst) != 0 {
            "izinsiz uyku APC'yi KOSTURDU"
        } else if R_RAN_END.load(Ordering::SeqCst) == 0 {
            "APC KAYBOLDU: sonraki izinli bekleme gormedi"
        } else {
            "donus degerleri beklenenden farkli"
        },
        passed: e,
    };

    // --- F + G: uyarilabilir surec beklemesi bolunur, cocuk kaybolmaz ----
    reset();
    let child = spawn_child();
    CHILD.store(child, Ordering::SeqCst);
    let (qf, df) = if child != 0 {
        run_with_apc(wait_alertable, 4000)
    } else {
        (false, false)
    };
    let (rf, mf) = (R_RET.load(Ordering::SeqCst), R_MS.load(Ordering::SeqCst));
    let f = qf
        && df
        && rf == WAIT_IO
        && mf < 1000
        && R_CODE_MID.load(Ordering::SeqCst) == winapi::STILL_ACTIVE
        && R_RAN_MID.load(Ordering::SeqCst) == 1;
    checks[5] = Check {
        name: NAMES[5],
        detail: if f {
            "bekleme APC ile kalkti, cocuk hala calisiyordu"
        } else if child == 0 {
            "cocuk baslatilamadi"
        } else if !qf || !df {
            "akis kurulamadi ya da donmedi"
        } else if rf == winapi::WAIT_OBJECT_0 {
            "APC beklemeyi BOLMEDI: cocuk bitene kadar surdu"
        } else {
            "donus ya da sure beklenenden farkli"
        },
        passed: f,
    };
    let g = f
        && R_RET2.load(Ordering::SeqCst) == winapi::WAIT_OBJECT_0
        && R_CODE_END.load(Ordering::SeqCst) == CHILD_CODE;
    checks[6] = Check {
        name: NAMES[6],
        detail: if g {
            "ikinci bekleme cocugun cikis kodunu (7) aldi"
        } else if !f {
            "F bolunmedigi icin olculemedi"
        } else if R_RET2.load(Ordering::SeqCst) != winapi::WAIT_OBJECT_0 {
            "ikinci bekleme basarisiz: bolunen bekleme cocugu TOPLADI"
        } else {
            "cikis kodu KAYBOLDU"
        },
        passed: g,
    };
    if child != 0 {
        unsafe { winapi::CloseHandle(child) };
    }

    // --- H: izinsiz surec beklemesi bolunmez ----------------------------
    reset();
    let child = spawn_child();
    CHILD.store(child, Ordering::SeqCst);
    let (qh, dh) = if child != 0 {
        run_with_apc(wait_plain, 4000)
    } else {
        (false, false)
    };
    let (rh, mh) = (R_RET.load(Ordering::SeqCst), R_MS.load(Ordering::SeqCst));
    let h = qh
        && dh
        && rh == winapi::WAIT_OBJECT_0
        && mh >= 700
        && R_RAN_MID.load(Ordering::SeqCst) == 0
        && R_RET2.load(Ordering::SeqCst) == WAIT_IO
        && R_RAN_END.load(Ordering::SeqCst) == 1;
    checks[7] = Check {
        name: NAMES[7],
        detail: if h {
            "WaitForSingleObject cocuk bitene kadar surdu; APC sonra kostu"
        } else if child == 0 {
            "cocuk baslatilamadi"
        } else if !qh || !dh {
            "akis kurulamadi ya da donmedi"
        } else if rh == WAIT_IO || mh < 700 {
            "izinsiz bekleme APC ile BOLUNDU"
        } else if R_RAN_END.load(Ordering::SeqCst) == 0 {
            "APC KAYBOLDU"
        } else {
            "donus degerleri beklenenden farkli"
        },
        passed: h,
    };
    if child != 0 {
        unsafe { winapi::CloseHandle(child) };
    }

    for check in &checks {
        let _ = writeln!(
            console,
            "[winalert] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let passed = checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(
        console,
        "[winalert] olcu: D {} ms  E {} ms  F {} ms  H {} ms  en derin kat {}",
        md, me_ms, mf, mh, max_live
    );
    let _ = writeln!(console, "[winalert] sonuc: {}/8 gecti", passed);
    show(&checks);
}

fn show(checks: &[Check; 8]) {
    let mut win = match Window::create("winalert -- bekleme bolunur", 250, 150, 500, 250) {
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

fn draw(win: &mut Window, checks: &[Check; 8]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "APC uyarilabilir beklemeyi boler", ACCENT);
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
    win.text(6, h - 30, "izinli bekleme kalkar, izinsiz bekleme surer", DIM);
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
