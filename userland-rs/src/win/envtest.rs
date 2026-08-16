//! `winenv.exe` -- Win32'nin ortam sozlesmesi, kendi tarafindan.
//!
//! POSIX ikizi (`bequest`) mirasi olcuyor: `setenv` kendi surecte
//! goruluyor mu, `fork` devrediyor mu, `execve` koruyor mu. Bu program
//! **baska bir sey** olcuyor, cunku Win32'nin sozlesmesi baska:
//!
//! ```text
//!   POSIX   getenv  -> isaretci ya da NULL. Baska bilgi yok.
//!   Win32   GetEnvironmentVariableA -> DWORD, ve sayi UC ANLAM tasir:
//!             0                    -> degisken yok (GetLastError = 203)
//!             <= nSize             -> yazildi, deger bu kadar uzun
//!             >  nSize             -> yazilmadi, GEREKEN boy bu
//! ```
//!
//! Ucuncu satir Win32'ye ozgu ve gercek Windows programlarinin bagli
//! oldugu bir kalip: once kucuk bir tamponla sorulur, donen sayi
//! tampondan buyukse yer acilip yeniden sorulur. TCMK bu sozlesmeyi
//! taklit etmek zorunda -- yoksa o kalibi kullanan bir ikili, degeri
//! sessizce kirpilmis alirdi.
//!
//! ## Dort sinav
//!
//! ```text
//!   A  oturumdan miras: HOME okunabiliyor mu?
//!   B  SetEnvironmentVariableA yazdigini geri veriyor mu?
//!   C  tampon kucukse GEREKEN boy mu donuyor (ve tampon bozulmuyor mu)?
//!   D  NULL deger silmek mi demek (ve sonra GetLastError = 203 mu)?
//! ```
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::winapi::{self, Window};

tcmk::entry!(main);

const BG: u32 = 0x0014_1020;
const PANEL: u32 = 0x0024_1C38;
const FG: u32 = 0x00E0_DCF0;
const DIM: u32 = 0x0088_80A0;
const ACCENT: u32 = 0x00C0_98FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Sinavlarin uzerinde calistigi degisken.
const NAME: &[u8] = b"TCMK_MIRAS\0";

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

/// `GetEnvironmentVariableA`nin ham donusu -- yorumlamadan.
fn query(name: &[u8], buffer: &mut [u8]) -> u32 {
    unsafe {
        winapi::GetEnvironmentVariableA(
            name.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as winapi::Dword,
        )
    }
}

fn main() {
    let mut console = winapi::Console;
    let mut checks = [EMPTY; 4];

    // --- A: oturumdan miras alinan HOME ---
    let mut home = [0u8; 64];
    let home_len = query(b"HOME\0", &mut home);
    checks[0] = Check {
        name: "A oturum mirasi",
        detail: if home_len > 0 && (home_len as usize) < home.len() {
            "HOME okundu"
        } else {
            "HOME YOK -- ortam bos geldi"
        },
        passed: home_len > 0 && (home_len as usize) < home.len(),
    };

    // --- B: yazilan deger geri okunuyor mu? ---
    let wrote = unsafe { winapi::SetEnvironmentVariableA(NAME.as_ptr(), b"win32\0".as_ptr()) };
    let mut value = [0u8; 32];
    let value_len = query(NAME, &mut value);
    let matches = value_len == 5 && &value[..5] == b"win32";
    checks[1] = Check {
        name: "B SetEnvironmentVariableA",
        detail: if wrote == 0 {
            "cagri basarisiz"
        } else if matches {
            "yazilan deger geri okundu"
        } else {
            "deger DEGISMEDI"
        },
        passed: wrote != 0 && matches,
    };

    // --- C: tampon kucuk -- gereken boy donmeli, tampon bozulmamali ---
    //
    // "win32" bes harf; uc baytlik bir tampon yetmez. Windows bu durumda
    // **NUL dahil** gereken boyu (6) doner ve tampona dokunmaz.
    let mut small = [0xAAu8; 3];
    let needed = query(NAME, &mut small);
    let untouched = small == [0xAA; 3];
    checks[2] = Check {
        name: "C gereken boy",
        detail: if needed == 6 && untouched {
            "6 dondu, tampon bozulmadi"
        } else if needed == 6 {
            "6 dondu ama TAMPON YAZILDI"
        } else {
            "gereken boy DONMEDI"
        },
        passed: needed == 6 && untouched,
    };

    // --- D: NULL deger = silme ---
    let removed =
        unsafe { winapi::SetEnvironmentVariableA(NAME.as_ptr(), core::ptr::null()) };
    let mut after = [0u8; 32];
    let after_len = query(NAME, &mut after);
    let last = unsafe { winapi::GetLastError() };
    checks[3] = Check {
        name: "D NULL ile silme",
        detail: if removed == 0 {
            "silme cagrisi basarisiz"
        } else if after_len == 0 && last == winapi::ERROR_ENVVAR_NOT_FOUND {
            "silindi, GetLastError = 203"
        } else if after_len == 0 {
            "silindi ama HATA KODU yanlis"
        } else {
            "degisken DURUYOR"
        },
        passed: removed != 0 && after_len == 0 && last == winapi::ERROR_ENVVAR_NOT_FOUND,
    };

    for check in &checks {
        let _ = core::fmt::Write::write_str(&mut console, "[winenv] ");
        let _ = core::fmt::Write::write_str(&mut console, check.name);
        let _ = core::fmt::Write::write_str(
            &mut console,
            if check.passed { ": gecti (" } else { ": KALDI (" },
        );
        let _ = core::fmt::Write::write_str(&mut console, check.detail);
        let _ = core::fmt::Write::write_str(&mut console, ")\n");
    }

    let home_text = core::str::from_utf8(&home[..home_len.min(63) as usize]).unwrap_or("?");

    let mut win = match Window::create("winenv -- Win32 ortam sozlesmesi", 290, 170, 460, 210) {
        Some(w) => w,
        None => return,
    };

    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, &checks, home_text);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 4], home: &str) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Win32: Get/SetEnvironmentVariableA", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            210,
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
    win.text(12, h - 39, "HOME=", DIM);
    win.text(60, h - 39, home, FG);
    win.text(
        6,
        h - 14,
        if passed == checks.len() {
            "dort sinav da gecti   q cik"
        } else {
            "BIR SINAV KALDI   q cik"
        },
        if passed == checks.len() { OK } else { WARN },
    );
}
