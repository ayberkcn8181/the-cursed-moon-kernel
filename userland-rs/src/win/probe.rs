//! `winprobe.exe` -- Win32'nin "sorma" cagrilari, kendi sozlesmeleriyle.
//!
//! POSIX ikizi (`probe`) ayni cekirdek cagrilarina iniyor; ayrisan
//! **cevabin bicimi**, ve her satirda ayri bir sebep var:
//!
//! ```text
//!   stat(yol, buf)          -> 0/-errno, bilgi TAMPONA
//!   GetFileAttributesA(yol) -> bilgi DONUS DEGERINDE (bayrak kumesi),
//!                              hata 0 degil 0xFFFFFFFF
//!
//!   time()                  -> 1970'ten beri SANIYE
//!   GetSystemTimeAsFileTime -> 1601'den beri 100 NANOSANIYE
//!
//!   uname(buf)              -> alti DIZE
//!   GetVersionExA(buf)      -> uc SAYI + servis paketi dizesi
//! ```
//!
//! Sonuncusu pratikte fark yaratiyor: bir Windows programi
//! `dwMajorVersion >= 5` diye **sayi** karsilastirir, ayni sey POSIX'te
//! dizeyi ayristirmakla yapilir.
//!
//! `GetFileAttributesA`in hata degerinin `0` olmamasi da rastlanti
//! degil: sifir "hicbir ozellik yok" demek olurdu ve o gecerli bir durum
//! sayilabilirdi.
//!
//! ## Bes sinav
//!
//! ```text
//!   A  dosya ozellikleri    -> READONLY var, DIRECTORY yok
//!   B  dizin ozellikleri    -> DIRECTORY var
//!   C  olmayan yol          -> 0xFFFFFFFF + GetLastError = 2
//!   D  GetVersionExA        -> platform NT(2); boyut alani bos gelirse
//!                              cagri BASARISIZ olmali
//!   E  GetSystemTime        -> makul takvim + FILETIME sifir degil
//! ```
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::winapi::{self, FileTime, OsVersionInfoA, SystemTime, Window};

tcmk::entry!(main);

const BG: u32 = 0x0018_1424;
const PANEL: u32 = 0x0028_2038;
const FG: u32 = 0x00E4_DEF0;
const DIM: u32 = 0x008C_84A0;
const ACCENT: u32 = 0x00B0_A0FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

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

fn main() {
    let mut console = winapi::Console;
    let mut checks = [EMPTY; 5];

    // --- A: RAMFS dosyasi ---
    let file = unsafe { winapi::GetFileAttributesA(b"C:\\bin\\browse\0".as_ptr()) };
    let a = file != winapi::INVALID_FILE_ATTRIBUTES
        && file & winapi::FILE_ATTRIBUTE_READONLY != 0
        && file & winapi::FILE_ATTRIBUTE_DIRECTORY == 0;
    checks[0] = Check {
        name: "A dosya ozellikleri",
        detail: if file == winapi::INVALID_FILE_ATTRIBUTES {
            "browse BULUNAMADI"
        } else if a {
            "READONLY var, DIRECTORY yok"
        } else {
            "bayraklar beklenen gibi degil"
        },
        passed: a,
    };

    // --- B: dizin ---
    let dir = unsafe { winapi::GetFileAttributesA(b"C:\\bin\0".as_ptr()) };
    let b = dir != winapi::INVALID_FILE_ATTRIBUTES
        && dir & winapi::FILE_ATTRIBUTE_DIRECTORY != 0;
    checks[1] = Check {
        name: "B dizin ozellikleri",
        detail: if b {
            "DIRECTORY bayragi var"
        } else {
            "dizin GORULMEDI"
        },
        passed: b,
    };

    // --- C: olmayan yol -- hata degeri 0 DEGIL ---
    let missing = unsafe { winapi::GetFileAttributesA(b"C:\\yokboyle\0".as_ptr()) };
    let last = unsafe { winapi::GetLastError() };
    let c = missing == winapi::INVALID_FILE_ATTRIBUTES && last == winapi::ERROR_FILE_NOT_FOUND;
    checks[2] = Check {
        name: "C olmayan yol",
        detail: if missing != winapi::INVALID_FILE_ATTRIBUTES {
            "OLMAYAN yol icin ozellik dondu"
        } else if c {
            "0xFFFFFFFF + GetLastError = 2"
        } else {
            "hata kodu yanlis"
        },
        passed: c,
    };

    // --- D: surum, ve boyut alaninin dogrulanmasi ---
    //
    // Ikinci cagri bilerek **bos** bir yapiyla yapiliyor: Windows
    // `dwOSVersionInfoSize`i dogrular ve doldurulmamis bir yapiyi
    // reddeder. Kabul etseydi, cagiranin hangi surumu bekledigi
    // bilinemezdi.
    let mut version = OsVersionInfoA::new();
    let filled = unsafe { winapi::GetVersionExA(&mut version) };
    let mut unset = OsVersionInfoA::new();
    unset.os_version_info_size = 0;
    let rejected = unsafe { winapi::GetVersionExA(&mut unset) };
    let d = filled != 0 && version.platform_id == 2 && rejected == 0;
    checks[3] = Check {
        name: "D GetVersionExA",
        detail: if filled == 0 {
            "cagri BASARISIZ"
        } else if version.platform_id != 2 {
            "platform NT degil"
        } else if rejected != 0 {
            "bos boyut alani KABUL edildi"
        } else {
            "platform NT(2), bos boyut reddedildi"
        },
        passed: d,
    };

    // --- E: takvim ve FILETIME ---
    let mut now = SystemTime::default();
    let got = unsafe { winapi::GetSystemTime(&mut now) };
    let mut stamp = FileTime::ZERO;
    unsafe { winapi::GetSystemTimeAsFileTime(&mut stamp) };
    let filetime = ((stamp.high as u64) << 32) | stamp.low as u64;
    let e = got != 0
        && now.year >= 2020
        && (1..=12).contains(&now.month)
        && now.day_of_week < 7
        && filetime > 0;
    checks[4] = Check {
        name: "E saat",
        detail: if got == 0 {
            "GetSystemTime BASARISIZ"
        } else if now.year < 2020 {
            "yil makul degil"
        } else if filetime == 0 {
            "FILETIME sifir"
        } else {
            "takvim ve FILETIME dolu"
        },
        passed: e,
    };

    for check in &checks {
        let _ = core::fmt::Write::write_str(&mut console, "[winprobe] ");
        let _ = core::fmt::Write::write_str(&mut console, check.name);
        let _ = core::fmt::Write::write_str(
            &mut console,
            if check.passed { ": gecti (" } else { ": KALDI (" },
        );
        let _ = core::fmt::Write::write_str(&mut console, check.detail);
        let _ = core::fmt::Write::write_str(&mut console, ")\n");
    }

    let mut win = match Window::create("winprobe -- Win32 sorma cagrilari", 300, 180, 470, 230) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, &checks, &now, &version);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 5], now: &SystemTime, version: &OsVersionInfoA) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Win32: ayni cekirdek, baska sozlesme", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            200,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 13;
        win.text(16, y, check.detail, DIM);
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.fill(6, h - 42, w - 12, 20, PANEL);
    win.text(12, h - 39, version.csd(), FG);
    win.text(70, h - 39, "yil:", DIM);
    win.number(110, h - 39, now.year as usize, DIM);
    win.text(160, h - 39, "ay:", DIM);
    win.number(195, h - 39, now.month as usize, DIM);
    win.text(
        6,
        h - 14,
        if passed == checks.len() {
            "bes sinav da gecti   q cik"
        } else {
            "BIR SINAV KALDI   q cik"
        },
        if passed == checks.len() { OK } else { WARN },
    );
}
