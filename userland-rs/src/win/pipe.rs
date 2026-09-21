//! `winpipe.exe` -- ayni boru, baska bir "bakma" sozlesmesi.
//!
//! POSIX ikizi (`blocking`) ayni cekirdek nesnesine iniyor: `CreatePipe`
//! ile `pipe()` ayni halka tamponu aciyor, `ReadFile` ile `read()` ayni
//! yoldan okuyor ve ikisi de artik **bekliyor**.
//!
//! Ayrisan uc nokta var ve ucu de bu programda olculuyor.
//!
//! ## 1. Bakmak ile almak
//!
//! ```text
//!   POSIX  poll(fd, POLLIN)  -> "veri var mi?"
//!          read(fd, ...)     -> veriyi AL (tuketir)
//!
//!   Win32  PeekNamedPipe(..) -> veriyi GOR, tuketme
//!          ReadFile(..)      -> veriyi al
//! ```
//!
//! POSIX'te veriye bakmanin yolu onu tuketmekten geciyor. Win32 ikisini
//! ayirmis. Sinav B bunu olcuyor: ayni bayti once gorup sonra okumak.
//!
//! ## 2. Dosya sonu bir hata mi?
//!
//! Burasi en keskin ayrisma:
//!
//! ```text
//!   POSIX  yazan uc kapali -> read() sifir doner, HATA DEGIL
//!   Win32  yazan uc kapali -> ReadFile FALSE doner + ERROR_BROKEN_PIPE
//! ```
//!
//! Ayni olay, birinde normal akis, otekinde hata. Sinav C bunu olcuyor.
//!
//! ## 3. Bloke olmamak kimin ozelligi
//!
//! POSIX'te `O_NONBLOCK` **acik dosya tanimina** ait ve `fcntl` ile
//! kuruluyor; Win32'de boru **tutamacinin** kipi ve
//! `SetNamedPipeHandleState` ile. Ikisi de varsayilan olarak bloke.
//!
//! ## Bes sinav
//!
//! ```text
//!   A  bekleme       -> bos borudan okumak kardes yazana kadar BEKLER
//!   B  PeekNamedPipe -> bakmak tuketmez, sonra ayni bayt okunur
//!   C  kirik boru    -> yazan kapaninca ERROR_BROKEN_PIPE (POSIX: 0)
//!   D  PIPE_NOWAIT   -> bos boruda FALSE + ERROR_NO_DATA, bekleme yok
//!   E  iki durum     -> NO_DATA ile BROKEN_PIPE ayni sey DEGIL
//! ```
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use tcmk::winapi::{self, Dword, Handle, Window};

tcmk::entry!(main);

const BG: u32 = 0x0018_1620;
const PANEL: u32 = 0x0026_2434;
const FG: u32 = 0x00E4_E0EE;
const DIM: u32 = 0x0088_84A0;
const ACCENT: u32 = 0x00A0_C8E8;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Kardesin yazacagi damga.
const MARK: &[u8] = b"gec";

/// Kardesin yazmadan once bekleyecegi sure.
const DELAY_MS: Dword = 150;

/// A sinavinin yazma ucu.
static WRITE_END: AtomicU32 = AtomicU32::new(u32::MAX);

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

/// A sinavinin kardesi: bekleyip yazar.
unsafe extern "system" fn late_writer(_param: *mut c_void) -> Dword {
    winapi::Sleep(DELAY_MS);
    let handle = WRITE_END.load(Ordering::SeqCst);
    if handle != u32::MAX {
        let mut written = 0u32;
        winapi::WriteFile(
            handle,
            MARK.as_ptr(),
            MARK.len() as u32,
            &mut written,
            core::ptr::null_mut(),
        );
    }
    0
}

/// Boru yaratir; basarisizsa `None`.
fn make_pipe() -> Option<(Handle, Handle)> {
    let mut read_end: Handle = 0;
    let mut write_end: Handle = 0;
    let ok = unsafe {
        winapi::CreatePipe(
            &mut read_end,
            &mut write_end,
            core::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    Some((read_end, write_end))
}

fn main() {
    let mut console = winapi::Console;
    let mut checks = [EMPTY; 5];

    // --- A: bekleme ---
    let mut waited_ms = 0u32;
    let a = match make_pipe() {
        Some((read_end, write_end)) => {
            WRITE_END.store(write_end, Ordering::SeqCst);
            let mut thread_id = 0u32;
            let helper = unsafe {
                winapi::CreateThread(
                    core::ptr::null_mut(),
                    0,
                    Some(late_writer),
                    core::ptr::null_mut(),
                    0,
                    &mut thread_id,
                )
            };
            let started = unsafe { winapi::GetTickCount() };
            let mut buf = [0u8; 8];
            let mut read = 0u32;
            let ok = unsafe {
                winapi::ReadFile(
                    read_end,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut read,
                    core::ptr::null_mut(),
                )
            };
            waited_ms = unsafe { winapi::GetTickCount() }.wrapping_sub(started);
            if helper != 0 {
                unsafe { winapi::CloseHandle(helper) };
            }
            unsafe { winapi::CloseHandle(read_end) };
            unsafe { winapi::CloseHandle(write_end) };
            ok != 0 && read == MARK.len() as u32 && &buf[..MARK.len()] == MARK && waited_ms >= 100
        }
        None => false,
    };
    checks[0] = Check {
        name: "A bekleme",
        detail: if a {
            "kardes yazana kadar beklendi"
        } else if waited_ms < 100 {
            "beklemeden dondu (bloke etmiyor)"
        } else {
            "okunan veri yanlis"
        },
        passed: a,
    };

    // --- B: PeekNamedPipe ---
    //
    // POSIX'te bunun karsiligi yok. Once bakiyoruz, sonra ayni bayti
    // okuyoruz: bakmak tuketmediyse ikisi de basarili olur.
    let mut peeked_total = 0u32;
    let b = match make_pipe() {
        Some((read_end, write_end)) => {
            let mut written = 0u32;
            unsafe {
                winapi::WriteFile(
                    write_end,
                    MARK.as_ptr(),
                    MARK.len() as u32,
                    &mut written,
                    core::ptr::null_mut(),
                )
            };

            let mut peek_buf = [0u8; 8];
            let mut peek_read = 0u32;
            let peeked = unsafe {
                winapi::PeekNamedPipe(
                    read_end,
                    peek_buf.as_mut_ptr(),
                    peek_buf.len() as u32,
                    &mut peek_read,
                    &mut peeked_total,
                    core::ptr::null_mut(),
                )
            };

            // Bakmak tuketmediyse ayni veri hala orada olmali.
            let mut buf = [0u8; 8];
            let mut read = 0u32;
            let got = unsafe {
                winapi::ReadFile(
                    read_end,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut read,
                    core::ptr::null_mut(),
                )
            };
            unsafe { winapi::CloseHandle(read_end) };
            unsafe { winapi::CloseHandle(write_end) };

            peeked != 0
                && peek_read == MARK.len() as u32
                && &peek_buf[..MARK.len()] == MARK
                && peeked_total == MARK.len() as u32
                && got != 0
                && read == MARK.len() as u32
                && &buf[..MARK.len()] == MARK
        }
        None => false,
    };
    checks[1] = Check {
        name: "B PeekNamedPipe",
        detail: if b {
            "bakmak tuketmedi, ayni bayt sonra okundu"
        } else {
            "bakma veriyi TUKETTI ya da gormedi"
        },
        passed: b,
    };

    // --- C: kirik boru ---
    //
    // En keskin ayrisma. POSIX ayni durumda `read`i sifirla dondurur ve
    // bunu hata saymaz; Windows bunu bir hata sayar.
    let mut broken_error = 0u32;
    let c = match make_pipe() {
        Some((read_end, write_end)) => {
            unsafe { winapi::CloseHandle(write_end) };
            let mut buf = [0u8; 8];
            let mut read = 0u32;
            let ok = unsafe {
                winapi::ReadFile(
                    read_end,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut read,
                    core::ptr::null_mut(),
                )
            };
            broken_error = unsafe { winapi::GetLastError() };
            unsafe { winapi::CloseHandle(read_end) };
            ok == 0 && broken_error == winapi::ERROR_BROKEN_PIPE
        }
        None => false,
    };
    checks[2] = Check {
        name: "C kirik boru",
        detail: if c {
            "FALSE + ERROR_BROKEN_PIPE (POSIX 0 der)"
        } else if broken_error == 0 {
            "basarili dondu (POSIX gibi davrandi)"
        } else {
            "hata kodu yanlis"
        },
        passed: c,
    };

    // --- D: PIPE_NOWAIT ---
    let mut nowait_error = 0u32;
    let d = match make_pipe() {
        Some((read_end, write_end)) => {
            let mode = winapi::PIPE_NOWAIT;
            let set = unsafe {
                winapi::SetNamedPipeHandleState(
                    read_end,
                    &mode,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                )
            };
            let started = unsafe { winapi::GetTickCount() };
            let mut buf = [0u8; 8];
            let mut read = 0u32;
            let ok = unsafe {
                winapi::ReadFile(
                    read_end,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut read,
                    core::ptr::null_mut(),
                )
            };
            nowait_error = unsafe { winapi::GetLastError() };
            let instant = unsafe { winapi::GetTickCount() }.wrapping_sub(started) < 50;
            unsafe { winapi::CloseHandle(read_end) };
            unsafe { winapi::CloseHandle(write_end) };
            set != 0 && ok == 0 && nowait_error == winapi::ERROR_NO_DATA && instant
        }
        None => false,
    };
    checks[3] = Check {
        name: "D PIPE_NOWAIT",
        detail: if d {
            "bos boruda ERROR_NO_DATA, bekleme yok"
        } else {
            "kip kurulamadi ya da kod yanlis"
        },
        passed: d,
    };

    // --- E: iki durum ayri ---
    //
    // Ayni bos boru, iki farkli cevap: yazan uc acikken "simdilik yok",
    // kapandiktan sonra "bir daha gelmeyecek". Ayni kodu dondurselerdi
    // okuyan taraf ikisini ayirt edemezdi.
    let e = match make_pipe() {
        Some((read_end, write_end)) => {
            let mode = winapi::PIPE_NOWAIT;
            unsafe {
                winapi::SetNamedPipeHandleState(
                    read_end,
                    &mode,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                )
            };
            let mut buf = [0u8; 8];
            let mut read = 0u32;
            unsafe {
                winapi::ReadFile(
                    read_end,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut read,
                    core::ptr::null_mut(),
                )
            };
            let while_open = unsafe { winapi::GetLastError() };

            unsafe { winapi::CloseHandle(write_end) };
            unsafe {
                winapi::ReadFile(
                    read_end,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut read,
                    core::ptr::null_mut(),
                )
            };
            let after_close = unsafe { winapi::GetLastError() };
            unsafe { winapi::CloseHandle(read_end) };

            while_open == winapi::ERROR_NO_DATA && after_close == winapi::ERROR_BROKEN_PIPE
        }
        None => false,
    };
    checks[4] = Check {
        name: "E iki durum ayri",
        detail: if e {
            "yazan varken NO_DATA, kapaninca BROKEN_PIPE"
        } else {
            "iki durum AYIRT EDILEMIYOR"
        },
        passed: e,
    };

    for check in &checks {
        let _ = core::fmt::Write::write_str(&mut console, "[winpipe] ");
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

    let mut win = match Window::create("winpipe -- bakmak ile almak", 330, 215, 470, 170) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, &checks, waited_ms as usize);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 5], waited: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "bakmak tuketmez", ACCENT);

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
    win.text(6, h - 30, "beklenen ms:", DIM);
    win.number(130, h - 30, waited, FG);
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
