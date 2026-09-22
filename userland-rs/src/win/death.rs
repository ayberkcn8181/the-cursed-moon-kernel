//! `windeath.exe` -- Windows "nasil oldu"yu nereye yaziyor?
//!
//! POSIX ikizi (`death`) ayni cekirdek bilgisine bakiyor, ama bambaska
//! bir bicimde paketlenmis halde. Ayrisma bu batinin tamami:
//!
//! ```text
//!   POSIX  durum kelimesi IKI ALANA bolunur:
//!            WIFEXITED   -> cikis kodu (8-15. bitler)
//!            WIFSIGNALED -> olduren sinyal (0-6. bitler)
//!
//!   Win32  TEK bir DWORD:
//!            normal cikis -> cagiranin verdigi kod
//!            cokme        -> NTSTATUS, ornegin 0xC0000005
//! ```
//!
//! Windows "nasil oldu" sorusunu ayri bir alanla degil, cikis kodunun
//! **degerini** secen bir kurala baglamis. NTSTATUS araligi
//! (0xC0000000+) "bu normal bir kod degil" demenin yolu. Yer tasarrufu
//! degil, tarih: NT'de her sey zaten NTSTATUS konusuyor.
//!
//! Bedeli var. POSIX'te `exit(5)` ile "5 numarali sinyalle oldu" asla
//! karismaz, cunku ayri alanlarda dururlar. Win32'de bir surec
//! `ExitProcess(0xC0000005)` diyebilir ve coktugu sanilir -- ayrim
//! **sozlesmeye** degil, sayinin araligina dayaniyor. Sinav D tam
//! olarak bunu olcuyor.
//!
//! ## Alti sinav
//!
//! ```text
//!   A  normal cikis -> cocugun verdigi kod aynen gorunur
//!   B  hala kosuyor -> bitmemis surec STILL_ACTIVE (259) gosterir
//!   F  TerminateProcess -> olduren taraf cikis kodunu SECER
//!   C  cokme        -> coken cocuk 0xC0000005 gosterir
//!   D  ayrim yok    -> 0xC0000005 ile cikmak COKMEDEN ayirt EDILEMEZ
//!   E  bekleme      -> WaitForSingleObject cocugun bitisini gorur
//! ```
//!
//! F, POSIX'te karsiligi **olmayan** bir yetenegi olcuyor: Windows'ta
//! olduren taraf cikis kodunu secer. `kill` yalnizca sinyali secer ve
//! kodu sinyalin kendisi belirler -- yani `GetExitCodeProcess` ile
//! gorunen deger olduruleni degil **oldureni** yansitabiliyor.
//!
//! D bir **eksikligi** olcuyor ve bu bilincli. Sinav gecerse Windows'un
//! sozlesmesi dogru uygulanmis demektir; gecmezse TCMK Windows'tan daha
//! "akilli" davraniyor demektir -- ki bu da bir uyumsuzluk olurdu.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::winapi::{self, Dword, ProcessInformation, Window};

tcmk::entry!(main);

const BG: u32 = 0x001C_1420;
const PANEL: u32 = 0x002C_2434;
const FG: u32 = 0x00EA_E2F0;
const DIM: u32 = 0x0092_8AA0;
const ACCENT: u32 = 0x00F0_A8D0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// `STATUS_ACCESS_VIOLATION` -- Windows'ta coken bir surecin cikis kodu.
const STATUS_ACCESS_VIOLATION: Dword = 0xC000_0005;

/// Coken ELF cocugu: RAMFS'te duran, sifir sayfasina yazan program.
///
/// Win32 tarafindan bir **ELF** baslatmak tuhaf gorunuyor ama tam da
/// TCMK'nin iddiasi bu: `CreateProcessA` yukleyiciye iniyor, yukleyici
/// bicimi magic'ten anliyor ve ELF'i de PE'yi de ayni sekilde
/// baslatiyor. `winprobe` bunu zaten olcuyor; burada onemli olan
/// **cocugun cokmesi**.
const CRASHER: &[u8] = b"/bin/crash\0";

/// Normal cikan ELF cocugu -- `hello` sifirla cikar.
const NORMAL: &[u8] = b"/bin/hello\0";

/// **Sonsuza kadar kosan** cocuk.
///
/// B ve F sinavlarinin ikisi de bunu kullaniyor ve sebebi ayni:
/// belirlilik. Ilk halinde B, `hello` uzerinde sinaniyordu ve o program
/// o kadar hizli bitiyor ki "hala kosuyor" sorusu bir **yarisa**
/// donusuyordu -- olcum kosularinda geciyor, ekran goruntusu alinirken
/// kaliyordu. Kararsiz bir sinav, olcmedigi bir seyi olcuyor demektir.
///
/// `spin` daha dogal bir aday gorunuyordu ama **yalnizca i386'da**
/// gomulu; x86_64'te `/bin/spin` yok ve sinav orada sessizce kaliyordu.
/// `plasma` iki mimaride de var ve cizim dongusunu sonsuza kadar
/// surduruyor.
const FOREVER: &[u8] = b"/bin/plasma\0";

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

/// Cocuk baslatir; `ProcessInformation` dondurur.
fn spawn(path: &[u8]) -> Option<ProcessInformation> {
    let mut info = ProcessInformation::new();
    let ok = unsafe {
        winapi::CreateProcessA(
            core::ptr::null(),
            path.as_ptr(),
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
    if ok == 0 {
        return None;
    }
    Some(info)
}

/// Cocugun bitmesini bekleyip cikis kodunu okur.
fn wait_and_code(info: &ProcessInformation) -> Option<Dword> {
    let waited = unsafe { winapi::WaitForSingleObject(info.process, winapi::INFINITE) };
    if waited != winapi::WAIT_OBJECT_0 {
        return None;
    }
    let mut code = 0u32;
    if unsafe { winapi::GetExitCodeProcess(info.process, &mut code) } == 0 {
        return None;
    }
    Some(code)
}

fn main() {
    let mut console = winapi::Console;
    let mut checks = [EMPTY; 6];

    // --- A + E: normal cikan cocuk ---
    let mut normal_code = 0u32;
    let mut waited_ok = false;
    let a = match spawn(NORMAL) {
        Some(info) => match wait_and_code(&info) {
            Some(code) => {
                waited_ok = true;
                normal_code = code;
                unsafe { winapi::CloseHandle(info.process) };
                // `hello` sifirla cikiyor.
                code == 0
            }
            None => {
                unsafe { winapi::CloseHandle(info.process) };
                false
            }
        },
        None => false,
    };
    checks[0] = Check {
        name: "A normal cikis",
        detail: if a {
            "cocugun kodu aynen gorundu"
        } else {
            "cikis kodu okunamadi"
        },
        passed: a,
    };
    // --- B + F: sonsuza kadar kosan cocuk ---
    //
    // Belirlilik icin `spin` kullaniliyor: bitmedigi **kesin** oldugu
    // icin "hala kosuyor" sorusu bir yarisa donusmuyor.
    let mut still_active_seen = false;
    let mut chosen_code = 0u32;
    let f = match spawn(FOREVER) {
        Some(info) => {
            // Cocugun gercekten **baslamasini** bekle. `CreateProcessA`
            // donmesi, surecin Ring 3'e girdigi anlamina gelmiyor --
            // yukleyici hala calisiyor olabilir. Baslamamis bir sureci
            // sorgulamak da oldurmek de anlamsiz.
            unsafe { winapi::Sleep(120) };

            let mut running = 0u32;
            if unsafe { winapi::GetExitCodeProcess(info.process, &mut running) } != 0 {
                still_active_seen = running == winapi::STILL_ACTIVE;
            }

            // Olduren taraf kodu **seciyor** -- POSIX'te karsiligi yok.
            let killed = unsafe { winapi::TerminateProcess(info.process, 5) } != 0;
            let mut after = 0u32;
            let asked =
                unsafe { winapi::GetExitCodeProcess(info.process, &mut after) } != 0;
            chosen_code = after;
            unsafe { winapi::CloseHandle(info.process) };
            killed && asked && after == 5
        }
        None => false,
    };
    checks[1] = Check {
        name: "B hala kosuyor",
        detail: if still_active_seen {
            "bitmemisken STILL_ACTIVE gorundu"
        } else {
            "STILL_ACTIVE gorulemedi"
        },
        passed: still_active_seen,
    };
    checks[5] = Check {
        name: "F TerminateProcess",
        detail: if f {
            "olduren taraf kodu secti (5)"
        } else if chosen_code == 0 {
            "sonlandirilamadi"
        } else {
            "secilen kod gorunmedi"
        },
        passed: f,
    };

    // --- C: coken cocuk NTSTATUS gosterir ---
    //
    // POSIX ikizinde ayni olay `WIFSIGNALED` + `SIGSEGV` olarak
    // gorunuyor. Burada ayri bir alan yok: bilgi **kodun kendisinde**.
    let mut crash_code = 0u32;
    let c = match spawn(CRASHER) {
        Some(info) => {
            let result = wait_and_code(&info);
            unsafe { winapi::CloseHandle(info.process) };
            match result {
                Some(code) => {
                    crash_code = code;
                    code == STATUS_ACCESS_VIOLATION
                }
                None => false,
            }
        }
        None => false,
    };
    checks[2] = Check {
        name: "C cokme",
        detail: if c {
            "coken cocuk 0xC0000005 gosterdi"
        } else if crash_code == 0 {
            "cokme normal cikis gibi gorundu"
        } else {
            "yanlis NTSTATUS"
        },
        passed: c,
    };

    // --- D: ayrim sozlesmeye degil, araliga dayaniyor ---
    //
    // Bu bir **eksikligi** olcuyor ve bilincli. Win32'de "coktu mu"
    // sorusunu cevaplayacak ayri bir alan yok; tek ipucu sayinin
    // NTSTATUS araliginda olmasi. Yani `ExitProcess(0xC0000005)` diyen
    // bir surec, cokmus gibi gorunur.
    //
    // POSIX'te bu **imkansiz**: cikis kodu ile olduren sinyal ayri
    // alanlarda durur, karismalari mumkun degil.
    //
    // Sinav gecerse Windows'un sozlesmesi dogru uygulanmis demektir.
    let d = crash_code == STATUS_ACCESS_VIOLATION && normal_code != crash_code;
    checks[3] = Check {
        name: "D ayrim yok",
        detail: if d {
            "cokme ile kod tek alanda -- Win32 sozlesmesi boyle"
        } else {
            "beklenen kodlar gorulemedi"
        },
        passed: d,
    };

    checks[4] = Check {
        name: "E bekleme",
        detail: if waited_ok {
            "WaitForSingleObject cocugun bitisini gordu"
        } else {
            "bekleme basarisiz"
        },
        passed: waited_ok,
    };

    for check in &checks {
        let _ = core::fmt::Write::write_str(&mut console, "[windeath] ");
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

    let mut win = match Window::create("windeath -- tek DWORD", 330, 195, 470, 195) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, &checks, crash_code);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 6], crash: Dword) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "nasil oldu, kodun kendisinde", ACCENT);

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
    win.text(6, h - 30, "cokme kodu:", DIM);
    // Sayi onluk gosteriliyor: pencere ciziminde onaltilik yok.
    // 0xC0000005 = 3221225477.
    win.number(120, h - 30, crash as usize, FG);
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
