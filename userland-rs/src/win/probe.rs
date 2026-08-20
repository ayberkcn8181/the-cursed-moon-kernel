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
//!   F  GetCurrentProcessId  -> ThreadId ile AYNI olmali
//!   G  SetEndOfFile         -> dosya IMLECIN oldugu yerde bitmeli
//!   H  CREATE_ALWAYS        -> var olan dosyayi BOSALTMALI
//!   I  TEB                  -> fs:[0x18] / gs:[0x30] kendi adresini
//!                             vermeli, kimlik ProcessId ile ayni olmali
//!   J  TEB'deki son hata     -> basarisiz bir cagridan sonra
//!                             GetLastError ile AYNI degeri tasimali
//! ```
//!
//! I ve J POSIX'te karsiligi olmayan bir seyi olcuyor. Bir Windows
//! programi kimligini ve son hata kodunu cekirdege **sormaz**: bir
//! bellek yapisindan (TEB) okur, ve o yapinin adresi bir segment
//! tabanindadir. `GetLastError` gercek Windows'ta tek satirdir --
//! `return NtCurrentTeb()->LastErrorValue;`
//!
//! J bu yuzden onemli: cekirdegin tuttugu deger ile TEB'deki deger
//! ayrisirsa, TEB'i dogrudan okuyan derlenmis bir kod **yanlis** hata
//! gorur.
//!
//! G, POSIX ikizinden yapisal olarak ayrilan bir yer: `ftruncate`
//! uzunlugu **parametre** alir, `SetEndOfFile` **dosya imlecini**
//! kullanir. Yani Win32'de once konumlanilir, sonra "buraya kadar"
//! denir.
//!
//! G ve H **disk ister** (RAMFS salt okunur); disk yoksa "atlandi".
//!
//! F bir eksikligi degil bir **gercegi** olcuyor: POSIX'te `getpid` ve
//! `gettid` ayri sayilar dondururler cunku bir surecte cok is parcacigi
//! olur. TCMK'de is parcacigi yok -- bir gorev = bir surec = bir akis --
//! yani ayni sayiyi dondurmek dogru cevap. Ayri sayilar uydurmak, is
//! parcacigi varmis gibi gorunmek olurdu.
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
    /// Kosullari saglanmadigi icin calistirilmadi (bkz. POSIX ikizi).
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
    let mut checks = [EMPTY; 10];

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
    skipped: false,
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
    skipped: false,
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
    skipped: false,
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
    skipped: false,
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
    skipped: false,
    };

    // --- F: surec ve is parcacigi kimligi ---
    let process = unsafe { winapi::GetCurrentProcessId() };
    let thread = unsafe { winapi::GetCurrentThreadId() };
    checks[5] = Check {
        name: "F surec kimligi",
        detail: if process == thread {
            "ProcessId == ThreadId (tek akis)"
        } else {
            "iki kimlik AYRISTI -- is parcacigi yok"
        },
        passed: process == thread,
    skipped: false,
    };

    // --- G ve H: kesme (disk gerektirir) ---
    let disk = unsafe { winapi::GetFileAttributesA(b"C:\\tmp\0".as_ptr()) };
    let writable = disk != winapi::INVALID_FILE_ATTRIBUTES
        && disk & winapi::FILE_ATTRIBUTE_DIRECTORY != 0
        && disk & winapi::FILE_ATTRIBUTE_READONLY == 0;
    if !writable {
        checks[6] = Check {
            name: "G SetEndOfFile",
            detail: "disk bagli degil",
            passed: false,
            skipped: true,
        };
        checks[7] = Check {
            name: "H CREATE_ALWAYS",
            detail: "disk bagli degil",
            passed: false,
            skipped: true,
        };
    } else {
        let name = b"C:\\tmp\\wintrunc.txt\0";
        let mut cut = false;
        let handle = unsafe {
            winapi::CreateFileA(
                name.as_ptr(),
                winapi::GENERIC_WRITE,
                0,
                core::ptr::null_mut(),
                winapi::CREATE_ALWAYS,
                0,
                0,
            )
        };
        if handle != winapi::INVALID_HANDLE_VALUE {
            let text = b"bu metin kirpilacak";
            let mut written = 0u32;
            unsafe {
                winapi::WriteFile(
                    handle,
                    text.as_ptr(),
                    text.len() as winapi::Dword,
                    &mut written,
                    core::ptr::null_mut(),
                );
                // Once KONUMLAN, sonra "buraya kadar" de: Win32'nin
                // kalibi bu, POSIX'te uzunluk parametreyle gelir.
                winapi::SetFilePointer(handle, 6, core::ptr::null_mut(), 0);
                cut = winapi::SetEndOfFile(handle) != 0;
                winapi::CloseHandle(handle);
            }
        }
        let size = unsafe { winapi::GetFileAttributesA(name.as_ptr()) };
        let exists = size != winapi::INVALID_FILE_ATTRIBUTES;
        checks[6] = Check {
            name: "G SetEndOfFile",
            detail: if !exists {
                "dosya olusturulamadi"
            } else if cut {
                "imlecin oldugu yerde bitirildi"
            } else {
                "cagri BASARISIZ"
            },
            passed: cut && exists,
            skipped: false,
        };

        // H: CREATE_ALWAYS **var olan** dosyayi bosaltmali.
        //
        // On kosul acikca sinaniyor: dosya yoksa "bosalmis" gorunurdu ve
        // sinav hicbir sey olcmeden gecerdi. G'den sonra dosya alti
        // bayt olmali.
        let before = unsafe {
            let h = winapi::CreateFileA(
                name.as_ptr(),
                winapi::GENERIC_READ,
                0,
                core::ptr::null_mut(),
                winapi::OPEN_EXISTING,
                0,
                0,
            );
            if h == winapi::INVALID_HANDLE_VALUE {
                0
            } else {
                let mut high = 0u32;
                let size = winapi::GetFileSize(h, &mut high);
                winapi::CloseHandle(h);
                size
            }
        };
        let handle = unsafe {
            winapi::CreateFileA(
                name.as_ptr(),
                winapi::GENERIC_WRITE,
                0,
                core::ptr::null_mut(),
                winapi::CREATE_ALWAYS,
                0,
                0,
            )
        };
        let mut emptied = false;
        if handle != winapi::INVALID_HANDLE_VALUE {
            let mut high = 0u32;
            emptied = unsafe { winapi::GetFileSize(handle, &mut high) } == 0;
            unsafe { winapi::CloseHandle(handle) };
        }
        checks[7] = Check {
            name: "H CREATE_ALWAYS",
            detail: if before == 0 {
                "on kosul yok: dosya bos ya da acilamadi"
            } else if emptied {
                "dolu dosya acilista bosaltildi"
            } else {
                "dosya BOSALMADI"
            },
            passed: before > 0 && emptied,
            skipped: false,
        };
    }

    // --- I: TEB gercekten kurulu mu? ---
    //
    // `Self` alani TEB'in kendi adresini tasir; bir Windows programi
    // TEB'e erisirken once bunu okur, cunku segment tabanini dogrudan
    // ogrenmenin baska yolu yoktur. Sifir gelmesi "TEB yok" demek.
    let teb = tcmk::teb::current();
    let teb_pid = tcmk::teb::read(tcmk::teb::UNIQUE_PROCESS_OFFSET);
    let i = teb != 0 && teb_pid == process as usize;
    checks[8] = Check {
        name: "I TEB",
        detail: if teb == 0 {
            "TEB YOK -- segment tabani sifir"
        } else if i {
            "Self dolu, kimlik ProcessId ile ayni"
        } else {
            "kimlik UYUSMUYOR"
        },
        passed: i,
        skipped: false,
    };

    // --- J: son hata TEB'de de duruyor mu? ---
    //
    // Bilerek basarisiz bir cagri yapiliyor; sonra iki kaynak
    // karsilastiriliyor. Ayrisirlarsa TEB'i dogrudan okuyan derlenmis
    // bir kod yanlis hata gorur.
    unsafe { winapi::GetFileAttributesA(b"C:\\hicyok\0".as_ptr()) };
    let from_call = unsafe { winapi::GetLastError() };
    let from_teb = tcmk::teb::read32(tcmk::teb::LAST_ERROR_OFFSET);
    let j = teb != 0 && from_call == winapi::ERROR_FILE_NOT_FOUND && from_teb == from_call;
    checks[9] = Check {
        name: "J TEB'de son hata",
        detail: if teb == 0 {
            "TEB yok"
        } else if from_call != winapi::ERROR_FILE_NOT_FOUND {
            "cagri beklenen hatayi vermedi"
        } else if j {
            "TEB ve GetLastError ayni degeri veriyor"
        } else {
            "iki kaynak AYRISTI"
        },
        passed: j,
        skipped: false,
    };

    for check in &checks {
        let _ = core::fmt::Write::write_str(&mut console, "[winprobe] ");
        let _ = core::fmt::Write::write_str(&mut console, check.name);
        let _ = core::fmt::Write::write_str(&mut console, ": ");
        let _ = core::fmt::Write::write_str(&mut console, result(check));
        let _ = core::fmt::Write::write_str(&mut console, " (");
        let _ = core::fmt::Write::write_str(&mut console, check.detail);
        let _ = core::fmt::Write::write_str(&mut console, ")\n");
    }

    let mut win = match Window::create("winprobe -- Win32 yuzeyi", 290, 150, 460, 280) {
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

fn draw(win: &mut Window, checks: &[Check; 10], now: &SystemTime, version: &OsVersionInfoA) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Win32: ayni cekirdek, baska sozlesme", ACCENT);

    // Ozet pencerede, ayrinti seri gunlukte (bkz. POSIX ikizi).
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
            "hepsi gecti   q cik"
        } else {
            "BIR SINAV KALDI   q cik"
        },
        if passed == checks.len() { OK } else { WARN },
    );
}
