//! `winmap.exe` -- Win32'nin dosya esleme sozlesmesi: iki adim, bir nesne.
//!
//! POSIX ikizi (`mapped`) ayni cekirdek yoluna iniyor; ayrisan **kac
//! adimda** yapildigi:
//!
//! ```text
//!   POSIX   mmap(NULL, len, PROT_READ, MAP_PRIVATE, fd, offset)
//!             -> tek cagri, dogrudan adres
//!
//!   Win32   CreateFileMappingA(hFile, ..) -> esleme NESNESI
//!           MapViewOfFile(nesne, ..)      -> adres
//!           UnmapViewOfFile(adres)
//!           CloseHandle(nesne)
//! ```
//!
//! Aradaki nesne bosuna degil: Windows'ta **adlandirilabilir** ve baska
//! surecler ayni adla acip ayni belleği paylasabilir. POSIX'te ayni is
//! `shm_open` ile ayri bir yoldan yapilir. TCMK adlandirmayi
//! desteklemiyor -- ve bunu sessizce yok saymak yerine sinav B ile
//! acikca olcuyor.
//!
//! ## Bir asimetri daha
//!
//! ```text
//!   munmap(addr, len)      -> uzunlugu CAGIRAN sayar
//!   UnmapViewOfFile(addr)  -> uzunlugu CEKIRDEK hatirlar
//! ```
//!
//! Ikincisi cekirdege gorunum basina bir kayit tutturur; birincisi
//! tutturmaz. Ayni isin iki sozlesmesi.
//!
//! ## Dort sinav
//!
//! ```text
//!   A  icerik           -> eslenen baytlar ReadFile ile AYNI
//!   B  adlandirilmis    -> lpName doluysa REDDEDILIR (paylasim yok)
//!   C  nesne kapandi    -> CloseHandle gorunumu KALDIRMAZ
//!   D  UnmapViewOfFile  -> ilk cagri basarili, ikincisi basarisiz
//! ```
//!
//! C, Windows'un acik bir kurali: esleme nesnesi kapansa bile bellek,
//! son gorunum kaldirilana kadar durur. Nesneyi kapatinca gorunumu de
//! kaldirmak, tutamacini duzgunce kapatan bir programi cokertirdi.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::winapi::{self, Window};

tcmk::entry!(main);

const BG: u32 = 0x0010_1A20;
const PANEL: u32 = 0x001C_2C36;
const FG: u32 = 0x00E0_ECF0;
const DIM: u32 = 0x0080_949C;
const ACCENT: u32 = 0x0060_D0C0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Eslenecek dosya -- RAMFS'te, yani disk gerekmiyor. Windows yoluyla
/// veriliyor; cekirdek onu `/boot/msg.txt`e ceviriyor.
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

fn main() {
    let mut console = winapi::Console;
    let mut checks = [EMPTY; 4];

    // Once siradan yoldan oku: karsilastirmanin dogru tarafi bu.
    let file = unsafe {
        winapi::CreateFileA(
            PATH.as_ptr(),
            winapi::GENERIC_READ,
            0,
            core::ptr::null_mut(),
            winapi::OPEN_EXISTING,
            0,
            0,
        )
    };
    let mut expected = [0u8; 128];
    let mut expected_len = 0u32;
    if file != winapi::INVALID_HANDLE_VALUE {
        let mut read = 0u32;
        let ok = unsafe {
            winapi::ReadFile(
                file,
                expected.as_mut_ptr(),
                expected.len() as u32,
                &mut read,
                core::ptr::null_mut(),
            )
        };
        if ok != 0 {
            expected_len = read;
        }
        // Imleci basa al: esleme onu bozmamali ama okuma bozdu.
        unsafe { winapi::SetFilePointer(file, 0, core::ptr::null_mut(), 0) };
    }

    // --- A: icerik ---
    let mapping = if file != winapi::INVALID_HANDLE_VALUE {
        unsafe {
            winapi::CreateFileMappingA(
                file,
                core::ptr::null_mut(),
                winapi::PAGE_READONLY,
                0,
                0,
                core::ptr::null(),
            )
        }
    } else {
        0
    };
    let view = if mapping != 0 {
        unsafe { winapi::MapViewOfFile(mapping, winapi::FILE_MAP_READ, 0, 0, 0) }
    } else {
        core::ptr::null_mut()
    };
    let a = !view.is_null()
        && expected_len > 0
        && (0..expected_len as usize).all(|i| unsafe { *view.add(i) } == expected[i]);
    checks[0] = Check {
        name: "A eslenen icerik",
        detail: if file == winapi::INVALID_HANDLE_VALUE {
            "dosya acilamadi"
        } else if mapping == 0 {
            "esleme nesnesi yaratilamadi"
        } else if view.is_null() {
            "gorunum kurulamadi"
        } else if a {
            "eslenen baytlar ReadFile ile ayni"
        } else {
            "icerik AYRISIYOR"
        },
        passed: a,
    };

    // --- B: adlandirilmis esleme ---
    //
    // Windows'ta ad, eslemeyi surecler arasinda paylasilir kilar. TCMK'de
    // paylasimli bellek yok; sessizce adsiz gibi davranmak, iki surecin
    // ayni adla **ayri** bellek gormesi olurdu -- yani sessiz bir veri
    // hatasi. Cagri bu yuzden acikca reddediliyor.
    let named = if file != winapi::INVALID_HANDLE_VALUE {
        unsafe {
            winapi::CreateFileMappingA(
                file,
                core::ptr::null_mut(),
                winapi::PAGE_READONLY,
                0,
                0,
                b"PaylasilanBlok\0".as_ptr(),
            )
        }
    } else {
        0
    };
    let named_error = unsafe { winapi::GetLastError() };
    let b = named == 0 && named_error == ERROR_NOT_SUPPORTED;
    checks[1] = Check {
        name: "B adlandirilmis esleme",
        detail: if named != 0 {
            "ad KABUL edildi (paylasim yok oysa)"
        } else if named_error != ERROR_NOT_SUPPORTED {
            "reddedildi ama hata kodu yanlis"
        } else {
            "reddedildi, ERROR_NOT_SUPPORTED"
        },
        passed: b,
    };

    // --- C: nesneyi kapatmak gorunumu kaldirmaz ---
    let closed = if mapping != 0 {
        unsafe { winapi::CloseHandle(mapping) != 0 }
    } else {
        false
    };
    let still_readable = !view.is_null()
        && expected_len > 0
        && unsafe { *view } == expected[0];
    let c = closed && still_readable;
    checks[2] = Check {
        name: "C nesne kapandi",
        detail: if !closed {
            "CloseHandle basarisiz"
        } else if !still_readable {
            "nesne kapaninca gorunum de GITTI"
        } else {
            "nesne kapandi, gorunum yasiyor"
        },
        passed: c,
    };

    // --- D: gorunumu kaldirmak ---
    //
    // Ikinci cagri basarisiz olmali: adres artik bir gorunum degil.
    // Basarili donmek, cekirdegin ayni bolgeyi iki kez birakmasi
    // demek olurdu.
    let unmapped = !view.is_null() && unsafe { winapi::UnmapViewOfFile(view) != 0 };
    let twice = !view.is_null() && unsafe { winapi::UnmapViewOfFile(view) != 0 };
    let d = unmapped && !twice;
    checks[3] = Check {
        name: "D UnmapViewOfFile",
        detail: if !unmapped {
            "kaldirma basarisiz"
        } else if twice {
            "ayni adres IKI kez kaldirildi"
        } else {
            "bir kez kaldirildi, ikincisi reddedildi"
        },
        passed: d,
    };

    if file != winapi::INVALID_HANDLE_VALUE {
        unsafe { winapi::CloseHandle(file) };
    }

    for check in &checks {
        let _ = core::fmt::Write::write_str(&mut console, "[winmap] ");
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

    let mut win = match Window::create("winmap -- iki adim, bir nesne", 320, 220, 440, 150) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, &checks, expected_len as usize);
        win.frame(60);
    }
}

/// `ERROR_NOT_SUPPORTED` -- istenen ozellik TCMK'de yok.
const ERROR_NOT_SUPPORTED: winapi::Dword = 50;

fn draw(win: &mut Window, checks: &[Check; 4], size: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "iki adim, bir nesne", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            320,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, "dosya boyu:", DIM);
    win.number(110, h - 30, size, FG);
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
