//! `winthread.exe` -- `CreateThread`: ayni paylasim, baska bir sozlesme.
//!
//! POSIX ikizi (`threads`) ayni cekirdek yoluna iniyor: iki cagri da
//! yaratanin adres uzayini paylasan yeni bir gorev aciyor. Ayrisan,
//! **kimin ne ayirdigi** ve **nasil beklendigi**:
//!
//! ```text
//!   clone(VM|FS|FILES|THREAD, yigin, ..)
//!       -> yigini CAGIRAN ayirir
//!       -> geriye yalnizca bir SAYI (tid) doner
//!       -> beklemek icin ayri bir yol gerekir (futex / pthread_join)
//!
//!   CreateThread(.., dwStackSize, ..)
//!       -> yigini CEKIRDEK ayirir, cagiran yalnizca boyu soyler
//!       -> geriye beklenebilir bir TANITICI doner
//!       -> WaitForSingleObject dogrudan calisir
//! ```
//!
//! Ikinci satir onemli: Windows'ta is parcacigi da bir **nesne**, yani
//! surecle ayni bekleme yuzeyini kullanir. POSIX'te is parcacigi bir
//! nesne degil; `waitpid` onu gormez. TCMK'de is parcacigi da bir gorev
//! oldugu icin Win32 tarafinda bu bedavaya geliyor -- ve sinav D bunu
//! olcuyor.
//!
//! ## Dort sinav
//!
//! ```text
//!   A  bellek paylasimi  -> is parcaciginin yazdigini ana akis GORUR
//!   B  kimlikler         -> GetCurrentThreadId AYRISIR, ProcessId AYNI
//!   C  taniticilar       -> is parcaciginin actigi dosyayi ana akis okur
//!   D  WaitForSingleObject -> is parcacigi taniticisi beklenebilir
//! ```
//!
//! B, TCMK'de uzun sure dogru olamayan bir sinav: `GetCurrentThreadId`
//! ile `GetCurrentProcessId` ayni sayiyi donduruyordu, cunku bir gorev
//! bir surec bir akis idi. Artik ayrisiyorlar.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use tcmk::winapi::{self, Dword, Window};

tcmk::entry!(main);

const BG: u32 = 0x0014_1826;
const PANEL: u32 = 0x0022_2A3C;
const FG: u32 = 0x00E4_E8F4;
const DIM: u32 = 0x0088_90A4;
const ACCENT: u32 = 0x0090_B8FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Is parcaciginin yazacagi deger; `lpParameter` ile geciyor.
const MARK: usize = 0x00BA_DA55;

/// Is parcaciginin cikis kodu -- `GetExitCodeThread` yok, ama
/// tramplenin cikis kodunu tasidigini yine de kayit altina aliyoruz.
const EXIT_CODE: Dword = 7;

static SHARED: AtomicUsize = AtomicUsize::new(0);
static CHILD_TID: AtomicU32 = AtomicU32::new(0);
static CHILD_PID: AtomicU32 = AtomicU32::new(0);
static CHILD_FILE: AtomicU32 = AtomicU32::new(winapi::INVALID_HANDLE_VALUE);
static DONE: AtomicU32 = AtomicU32::new(0);

/// Windows yoluyla verilen dosya; cekirdek onu `/boot/msg.txt`e ceviriyor.
const PATH: &[u8] = b"C:\\boot\\msg.txt\0";

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

/// Is parcacigi govdesi.
///
/// `extern "system"` sart: i386'da stdcall, yani argumani **cagrilan**
/// temizler. Cekirdegin kurdugu cerceve bunu bekliyor.
unsafe extern "system" fn worker(parameter: *mut c_void) -> Dword {
    SHARED.store(parameter as usize, Ordering::SeqCst);
    CHILD_TID.store(winapi::GetCurrentThreadId(), Ordering::SeqCst);
    CHILD_PID.store(winapi::GetCurrentProcessId(), Ordering::SeqCst);

    // Tanitici tablosu paylasilir: burada acilan dosyayi ana akis
    // okuyabilmeli. Windows'ta bu tabloya "surec tanitici tablosu"
    // deniyor -- adi bile paylasimin surece ait oldugunu soyluyor.
    let file = winapi::CreateFileA(
        PATH.as_ptr(),
        winapi::GENERIC_READ,
        0,
        core::ptr::null_mut(),
        winapi::OPEN_EXISTING,
        0,
        0,
    );
    CHILD_FILE.store(file, Ordering::SeqCst);

    DONE.store(1, Ordering::SeqCst);
    EXIT_CODE
}

fn main() {
    let mut console = winapi::Console;
    let mut checks = [EMPTY; 4];

    let parent_tid = unsafe { winapi::GetCurrentThreadId() };
    let parent_pid = unsafe { winapi::GetCurrentProcessId() };

    let mut thread_id = 0u32;
    let thread = unsafe {
        winapi::CreateThread(
            core::ptr::null_mut(),
            0,
            Some(worker),
            MARK as *mut c_void,
            0,
            &mut thread_id,
        )
    };

    // --- D: beklenebilir tanitici ---
    //
    // Bilerek once: sinavin kendisi digerlerinin de dogru olmasini
    // sagliyor. POSIX ikizinde burada bayrak yoklamak zorunda kalmistik;
    // Win32'de bekleme yuzeyi hazir geliyor.
    let waited = thread != 0 && unsafe { winapi::WaitForSingleObject(thread, winapi::INFINITE) }
        == winapi::WAIT_OBJECT_0;
    let finished = DONE.load(Ordering::SeqCst) == 1;
    let d = waited && finished;
    checks[3] = Check {
        name: "D WaitForSingleObject",
        detail: if thread == 0 {
            "is parcacigi yaratilamadi"
        } else if !waited {
            "bekleme basarisiz (tanitici beklenebilir degil)"
        } else if !finished {
            "bekleme dondu ama is parcacigi bitmemis"
        } else {
            "tanitici beklendi, is parcacigi bitmisti"
        },
        passed: d,
    };

    // --- A: bellek paylasimi ---
    let seen = SHARED.load(Ordering::SeqCst);
    let a = finished && seen == MARK;
    checks[0] = Check {
        name: "A bellek paylasimi",
        detail: if !finished {
            "is parcacigi bitmedi"
        } else if a {
            "lpParameter ile yazilan deger ana akista gorundu"
        } else {
            "deger GORUNMEDI (uzay paylasilmiyor)"
        },
        passed: a,
    };

    // --- B: kimlikler ---
    let child_tid = CHILD_TID.load(Ordering::SeqCst);
    let child_pid = CHILD_PID.load(Ordering::SeqCst);
    let b = finished
        && child_tid != parent_tid
        && child_pid == parent_pid
        && thread_id == child_tid;
    checks[1] = Check {
        name: "B kimlikler",
        detail: if !finished {
            "is parcacigi bitmedi"
        } else if child_tid == parent_tid {
            "GetCurrentThreadId AYRISMADI"
        } else if child_pid != parent_pid {
            "GetCurrentProcessId ayristi (ayri surec sayiliyor)"
        } else if thread_id != child_tid {
            "lpThreadId kardesin kimligiyle ORTUSMUYOR"
        } else {
            "tid ayri, pid ayni, lpThreadId dogru"
        },
        passed: b,
    };

    // --- C: tanitici paylasimi ---
    let file = CHILD_FILE.load(Ordering::SeqCst);
    let mut buf = [0u8; 16];
    let mut read = 0u32;
    let readable = file != winapi::INVALID_HANDLE_VALUE
        && unsafe {
            winapi::ReadFile(
                file,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut read,
                core::ptr::null_mut(),
            )
        } != 0
        && read > 0;
    let c = finished && readable;
    checks[2] = Check {
        name: "C tanitici paylasimi",
        detail: if file == winapi::INVALID_HANDLE_VALUE {
            "is parcacigi dosyayi acamadi"
        } else if c {
            "kardesin actigi dosya ana akista okundu"
        } else {
            "tanitici GORUNMEDI"
        },
        passed: c,
    };
    if file != winapi::INVALID_HANDLE_VALUE {
        unsafe { winapi::CloseHandle(file) };
    }
    if thread != 0 {
        unsafe { winapi::CloseHandle(thread) };
    }

    for check in &checks {
        let _ = core::fmt::Write::write_str(&mut console, "[winthread] ");
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

    let mut win = match Window::create("winthread -- CreateThread", 340, 230, 460, 150) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, &checks, parent_tid as usize, child_tid as usize);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 4], parent: usize, child: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "yigini cekirdek ayirir, tanitici beklenir", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            340,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, "tid:", DIM);
    win.number(50, h - 30, parent, FG);
    win.text(100, h - 30, "kardes:", DIM);
    win.number(170, h - 30, child, FG);
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
