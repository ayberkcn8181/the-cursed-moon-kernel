//! `winsync.exe` -- `WaitOnAddress`: ayni ilkel, baska bir imza.
//!
//! POSIX ikizi (`sync`) ayni cekirdek yoluna iniyor. Windows'ta bu
//! ilkel `SRWLOCK` ve `CONDITION_VARIABLE`in altinda duruyor; Linux'ta
//! ayni isi `futex` yapiyor. Ikisi de gercek, ikisi de ayni sey.
//!
//! ## Ucu de kucuk, ucu de gercek: ayrisan noktalar
//!
//! ```text
//!   POSIX  futex(uaddr, FUTEX_WAIT, beklenen, timeout)
//!            beklenen deger SAYI olarak gecer
//!            deger uymazsa -EAGAIN (bir hata kodu)
//!            FUTEX_WAKE kac tane oldugunu ALIR ve kac uyandirdigini DONER
//!
//!   Win32  WaitOnAddress(adres, karsilastirma_adresi, boy, ms)
//!            beklenen deger ADRES olarak gecer, boyu ayrica soylenir
//!            deger uymazsa TRUE (basari)
//!            WakeByAddressSingle / WakeByAddressAll: iki ayri cagri,
//!              ikisi de void -- "kimseyi bulamadim" bilgisi YOK
//! ```
//!
//! Ucuncu satir en ilginci. Win32'de `void` donmek bir eksiklik gibi
//! gorunuyor ama degil: cagiran zaten kosulu yeniden sinamak zorunda,
//! yani sayiyi bilse de yapacagi sey degismezdi. POSIX sayiyi vererek
//! **olcum** imkani birakiyor; TCMK bu yuzden sayiyi cekirdekte tutup
//! Win32 tarafina vermiyor -- iki sozlesme de aynen korunuyor.
//!
//! ## Bes sinav
//!
//! ```text
//!   A  uyandirma        -> kardes uyandirana kadar UYUNUR
//!   B  zaman asimi      -> kimse uyandirmazsa FALSE + ERROR_TIMEOUT
//!   C  deger degismis   -> beklenen deger tutmuyorsa TRUE, hic uyunmaz
//!   D  GetExitCodeThread-> kosarken STILL_ACTIVE, bitince gercek kod
//!   E  sayac dogru      -> iki akis kilitle artirir, toplam TAM cikar
//! ```
//!
//! C, POSIX'te `-EAGAIN` olarak gorunen durumun Win32 yuzu ve ayri bir
//! sinav olmayi hak ediyor: ayni olayin iki ABI'de **zit isaretli**
//! bildirildigi tek yer burasi.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use tcmk::winapi::{self, Dword, Window};

tcmk::entry!(main);

const BG: u32 = 0x0016_1420;
const PANEL: u32 = 0x0026_2434;
const FG: u32 = 0x00E6_E2F2;
const DIM: u32 = 0x008A_86A2;
const ACCENT: u32 = 0x00B0_A0FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// A sinavinin bayragi.
static FLAG: AtomicU32 = AtomicU32::new(0);

/// E sinavinin paylasilan sayaci ve kilidi.
static COUNTER: AtomicUsize = AtomicUsize::new(0);
static LOCK: AtomicU32 = AtomicU32::new(0);

const ROUNDS: usize = 200;

/// D sinavinin kardesi bunu dondurecek.
const THREAD_EXIT_CODE: Dword = 42;

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

/// A sinavinin kardesi: bekleyip bayragi kaldirir ve uyandirir.
unsafe extern "system" fn waker(_param: *mut c_void) -> Dword {
    winapi::Sleep(120);
    FLAG.store(1, Ordering::SeqCst);
    winapi::WakeByAddressSingle(FLAG.as_ptr() as *const c_void);
    0
}

/// D sinavinin kardesi: kisa surer ve **belirli** bir kodla biter.
unsafe extern "system" fn coded(_param: *mut c_void) -> Dword {
    winapi::Sleep(80);
    THREAD_EXIT_CODE
}

/// Kilidi alir. Cekisme yoksa cekirdege inmez.
///
/// Uc degerli kilit: 0 bos, 1 alinmis, 2 alinmis ve bekleyen var.
/// Windows'un kendi `SRWLOCK`u da benzer bir kodlama kullanir.
fn lock_acquire() {
    if LOCK
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        return;
    }
    loop {
        let previous = LOCK.swap(2, Ordering::Acquire);
        if previous == 0 {
            return;
        }
        // Beklenen deger bir **adres** ile veriliyor: Win32'nin
        // POSIX'ten ayrisan imzasi bu.
        let expected: u32 = 2;
        unsafe {
            winapi::WaitOnAddress(
                LOCK.as_ptr() as *const c_void,
                &expected as *const u32 as *const c_void,
                4,
                winapi::INFINITE,
            )
        };
    }
}

fn lock_release() {
    if LOCK.swap(0, Ordering::Release) == 2 {
        unsafe { winapi::WakeByAddressSingle(LOCK.as_ptr() as *const c_void) };
    }
}

/// E sinavinin kardesi.
unsafe extern "system" fn adder(_param: *mut c_void) -> Dword {
    for _ in 0..ROUNDS {
        lock_acquire();
        let value = COUNTER.load(Ordering::Relaxed);
        // Kilidi zorlamak icin bilerek birakilan aralik.
        winapi::Sleep(0);
        COUNTER.store(value + 1, Ordering::Relaxed);
        lock_release();
    }
    0
}

fn main() {
    let mut console = winapi::Console;
    let mut checks = [EMPTY; 5];

    // --- A: uyandirma ---
    let mut waker_id = 0u32;
    let waker_handle = unsafe {
        winapi::CreateThread(
            core::ptr::null_mut(),
            0,
            Some(waker),
            core::ptr::null_mut(),
            0,
            &mut waker_id,
        )
    };
    let zero: u32 = 0;
    let started = unsafe { winapi::GetTickCount() };
    let woke = unsafe {
        winapi::WaitOnAddress(
            FLAG.as_ptr() as *const c_void,
            &zero as *const u32 as *const c_void,
            4,
            winapi::INFINITE,
        )
    } != 0;
    let elapsed = unsafe { winapi::GetTickCount() }.wrapping_sub(started);
    let a = waker_handle != 0 && woke && FLAG.load(Ordering::SeqCst) == 1 && elapsed >= 80;
    checks[0] = Check {
        name: "A uyandirma",
        detail: if waker_handle == 0 {
            "kardes yaratilamadi"
        } else if !woke {
            "bekleme basarisiz dondu"
        } else if elapsed < 80 {
            "hic beklemeden dondu"
        } else if FLAG.load(Ordering::SeqCst) != 1 {
            "uyandi ama bayrak kalkmamis"
        } else {
            "uyundu ve kardes uyandirdi"
        },
        passed: a,
    };

    // --- B: zaman asimi ---
    let idle = AtomicU32::new(7);
    let seven: u32 = 7;
    let started = unsafe { winapi::GetTickCount() };
    let timed = unsafe {
        winapi::WaitOnAddress(
            idle.as_ptr() as *const c_void,
            &seven as *const u32 as *const c_void,
            4,
            200,
        )
    };
    let error = unsafe { winapi::GetLastError() };
    let waited = unsafe { winapi::GetTickCount() }.wrapping_sub(started);
    let b = timed == 0 && error == winapi::ERROR_TIMEOUT && waited >= 150;
    checks[1] = Check {
        name: "B zaman asimi",
        detail: if timed != 0 {
            "uyandirilmadigi halde TRUE dondu"
        } else if error != winapi::ERROR_TIMEOUT {
            "yanlis hata kodu"
        } else if waited < 150 {
            "sure dolmadan dondu"
        } else {
            "FALSE + ERROR_TIMEOUT"
        },
        passed: b,
    };

    // --- C: deger zaten degismis ---
    //
    // POSIX'te bu `-EAGAIN`, yani bir hata kodu; Win32'de `TRUE`, yani
    // basari. Ayni olayin iki ABI'de zit isaretle bildirildigi tek yer.
    let mismatch = AtomicU32::new(1);
    let expect_zero: u32 = 0;
    let started = unsafe { winapi::GetTickCount() };
    let changed = unsafe {
        winapi::WaitOnAddress(
            mismatch.as_ptr() as *const c_void,
            &expect_zero as *const u32 as *const c_void,
            4,
            winapi::INFINITE,
        )
    };
    let instant = unsafe { winapi::GetTickCount() }.wrapping_sub(started) < 50;
    let c = changed != 0 && instant;
    checks[2] = Check {
        name: "C deger degismis",
        detail: if changed == 0 {
            "TRUE beklenirken FALSE geldi"
        } else if !instant {
            "deger uymuyorken yine de uyudu"
        } else {
            "hic uyumadan TRUE dondu (POSIX -EAGAIN der)"
        },
        passed: c,
    };

    // --- D: GetExitCodeThread ---
    //
    // POSIX'te bunun karsiligi yok: `waitpid` is parcacigini gormez,
    // `pthread_join` donus degerini kutuphanenin kendi yapisindan alir.
    let mut coded_id = 0u32;
    let coded_handle = unsafe {
        winapi::CreateThread(
            core::ptr::null_mut(),
            0,
            Some(coded),
            core::ptr::null_mut(),
            0,
            &mut coded_id,
        )
    };
    let mut running_code = 0u32;
    let asked_running = coded_handle != 0
        && unsafe { winapi::GetExitCodeThread(coded_handle, &mut running_code) } != 0;
    let still_active = running_code == winapi::STILL_ACTIVE;

    let waited_ok = coded_handle != 0
        && unsafe { winapi::WaitForSingleObject(coded_handle, winapi::INFINITE) }
            == winapi::WAIT_OBJECT_0;
    let mut final_code = 0u32;
    let asked_final =
        waited_ok && unsafe { winapi::GetExitCodeThread(coded_handle, &mut final_code) } != 0;
    let d = asked_running && still_active && asked_final && final_code == THREAD_EXIT_CODE;
    checks[3] = Check {
        name: "D GetExitCodeThread",
        detail: if !asked_running {
            "kosarken sorulamadi"
        } else if !still_active {
            "kosarken STILL_ACTIVE donmedi"
        } else if !asked_final {
            "bitince sorulamadi"
        } else if final_code != THREAD_EXIT_CODE {
            "cikis kodu YANLIS"
        } else {
            "kosarken STILL_ACTIVE, bitince 42"
        },
        passed: d,
    };

    // --- E: kilit gercekten koruyor mu ---
    let mut helper_id = 0u32;
    let helper = unsafe {
        winapi::CreateThread(
            core::ptr::null_mut(),
            0,
            Some(adder),
            core::ptr::null_mut(),
            0,
            &mut helper_id,
        )
    };
    unsafe { adder(core::ptr::null_mut()) };
    if helper != 0 {
        unsafe { winapi::WaitForSingleObject(helper, winapi::INFINITE) };
    }
    let total = COUNTER.load(Ordering::Relaxed);
    let e = helper != 0 && total == 2 * ROUNDS;
    checks[4] = Check {
        name: "E sayac dogru",
        detail: if helper == 0 {
            "kardes yaratilamadi"
        } else if total < 2 * ROUNDS {
            "sayac EKSIK (kilit korumadi)"
        } else if total > 2 * ROUNDS {
            "sayac FAZLA"
        } else {
            "iki akis, 400 artirma, toplam tam"
        },
        passed: e,
    };

    if waker_handle != 0 {
        unsafe { winapi::CloseHandle(waker_handle) };
    }
    if coded_handle != 0 {
        unsafe { winapi::CloseHandle(coded_handle) };
    }
    if helper != 0 {
        unsafe { winapi::CloseHandle(helper) };
    }

    for check in &checks {
        let _ = core::fmt::Write::write_str(&mut console, "[winsync] ");
        let _ = core::fmt::Write::write_str(&mut console, check.name);
        let _ = core::fmt::Write::write_str(&mut console, ": ");
        let _ = core::fmt::Write::write_str(
            &mut console,
            if check.passed { "gecti" } else { "KALDI" },
        );
        let _ = core::fmt::Write::write_str(&mut console, " (");
        let _ = core::fmt::Write::write_str(&mut console, check.detail);
        let _ = core::fmt::Write::write_str(&mut console, ")\n");
    }

    let mut win = match Window::create("winsync -- WaitOnAddress", 330, 220, 470, 170) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, &checks, total, final_code as usize);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 5], total: usize, code: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "ayni ilkel, baska bir imza", ACCENT);

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
    win.text(6, h - 30, "sayac:", DIM);
    win.number(70, h - 30, total, FG);
    win.text(140, h - 30, "cikis kodu:", DIM);
    win.number(250, h - 30, code, FG);
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
