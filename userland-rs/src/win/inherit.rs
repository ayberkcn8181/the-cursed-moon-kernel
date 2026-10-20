//! `winherit.exe` -- tutamac mirasi: `bInheritHandles` ve `STARTUPINFO`.
//!
//! POSIX'te cocuk her seyi devralir: `fork` tabloyu kopyalar, `execve`
//! `FD_CLOEXEC`siz her tanimlayiciyi yeni programa gecirir. Kapatmak
//! icin bayrak gerekir.
//!
//! Windows tersini secti: `CreateProcess` hicbir tutamaci kendiliginden
//! gecirmez. Iki kosul birlikte gerekir:
//!
//! ```text
//!   tutamac miras alinabilir  (SECURITY_ATTRIBUTES.bInheritHandle ya da
//!                              SetHandleInformation(HANDLE_FLAG_INHERIT))
//!   CreateProcess(..., bInheritHandles = TRUE, ...)
//! ```
//!
//! Gecen tutamac cocukta **ayni degere** sahiptir ve ayni nesneye -- ayni
//! acik dosya tanimina -- bakar. Yonlendirme `STARTUPINFO` ile anlatilir:
//! `STARTF_USESTDHANDLES` cocugun stdin/stdout/stderr'ini kurar.
//!
//! ## Dokuz sinav
//!
//! ```text
//!   A  varsayilan GECMEZ     -> nitelik vermeden acilan dosya cocukta yok
//!   B  miras alinabilir      -> bInheritHandle'li dosya cocukta AYNI
//!      gecer                    numarayla okunuyor
//!   C  bInheritHandles=FALSE -> ayni tutamac bu kez gecmedi
//!   D  ayni konum            -> cocuk ebeveynin kaldigi yerden okudu,
//!                               ebeveyn de cocugun biraktigi yerden
//!   E  SetHandleInformation  -> bayrak kapaninca tutamac gecmedi
//!   F  stdout boruya         -> STARTF_USESTDHANDLES: cocugun GetStdHandle
//!                               ciktisi boruda, boru sonra KAPANDI
//!   G  stdin borudan         -> cocuk stdin'den ebeveynin yazdigini okudu
//!   H  sizan uc              -> yazma ucunu devralan uyuyan cocuk boruyu
//!                               kendi omru boyunca ACIK tuttu
//!   I  miras alinamayan std  -> bayraksiz boru STARTUPINFO'da verildi:
//!      gecmez                   cocugun ciktisi boruya GITMEDI
//! ```
//!
//! A ile B ayni kalip, tek fark bayrak; C ayni bayrak, tek fark
//! `bInheritHandles`. F ile H de bir cift: F'de ebeveyn kendi ucunu
//! dogru kapatiyor ve boru cocuk bitince kapaniyor; H'de yazma ucunu
//! devralan ilgisiz bir cocuk boruyu "acik" tutuyor. Gercek Windows
//! programlarinin en bilinen tuzagi bu -- `SetHandleInformation`
//! cagrisinin varlik sebebi.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::winapi::{self, Dword, Handle, Window};

tcmk::entry!(main);

const BG: u32 = 0x0012_1622;
const PANEL: u32 = 0x001E_2636;
const FG: u32 = 0x00E2_E8F2;
const DIM: u32 = 0x0086_94A6;
const ACCENT: u32 = 0x00F0_C070;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

const MSG: &[u8] = b"C:\\boot\\msg.txt\0";
/// Cocuk rolunun gercekten kostugunu soyleyen bit.
const DONE: Dword = 0x40;

fn parse(s: &str) -> usize {
    s.bytes().fold(0usize, |n, b| n * 10 + b.wrapping_sub(b'0') as usize)
}

fn open_msg(inherit: bool) -> Handle {
    let mut sa = winapi::SecurityAttributes::inheritable();
    let attrs = if inherit {
        &mut sa as *mut _ as *mut core::ffi::c_void
    } else {
        core::ptr::null_mut()
    };
    unsafe {
        winapi::CreateFileA(
            MSG.as_ptr(),
            winapi::GENERIC_READ,
            0,
            attrs,
            winapi::OPEN_EXISTING,
            0,
            0,
        )
    }
}

fn read4(h: Handle) -> Option<[u8; 4]> {
    let mut buf = [0u8; 4];
    let mut got: Dword = 0;
    let ok = unsafe { winapi::ReadFile(h, buf.as_mut_ptr(), 4, &mut got, core::ptr::null_mut()) };
    (ok != 0 && got == 4).then_some(buf)
}

fn flags_of(h: Handle) -> Option<Dword> {
    let mut f: Dword = 0xFF;
    (unsafe { winapi::GetHandleInformation(h, &mut f) } != 0).then_some(f)
}

/// Cocuk rolleri. Cikis kodu `DONE` + rolun bitleri.
fn child(role: &str) -> ! {
    let arg = |i| tcmk::args::get(i).map(parse).unwrap_or(99);
    let code = match role {
        // "r h": h okunabiliyor mu (1), bayragi acik mi (2).
        "r" => {
            let h = arg(2) as Handle;
            let mut c = DONE;
            if read4(h).is_some() {
                c |= 1;
            }
            if flags_of(h) == Some(winapi::HANDLE_FLAG_INHERIT) {
                c |= 2;
            }
            c
        }
        // "p h": 4 bayt oku, donus = 4. bayttan sonraki ilk bayt (DONE ile).
        "p" => match read4(arg(2) as Handle) {
            Some(b) => DONE | (b[0] as Dword & 0x3F) << 8,
            None => DONE,
        },
        // "o": stdout'a yaz.
        "o" => {
            let out = unsafe { winapi::GetStdHandle(winapi::STD_OUTPUT) };
            let text = b"merhaba\n";
            let mut wrote: Dword = 0;
            unsafe {
                winapi::WriteFile(out, text.as_ptr(), text.len() as Dword, &mut wrote, core::ptr::null_mut())
            };
            DONE | (out & 0xF)
        }
        // "i": stdin'i sonuna kadar oku; donus = okunan bayt.
        "i" => {
            let input = unsafe { winapi::GetStdHandle(winapi::STD_INPUT) };
            let mut total: Dword = 0;
            for _ in 0..50 {
                let mut buf = [0u8; 8];
                let mut got: Dword = 0;
                let ok = unsafe {
                    winapi::ReadFile(input, buf.as_mut_ptr(), 8, &mut got, core::ptr::null_mut())
                };
                if ok == 0 {
                    break;
                }
                total += got;
            }
            DONE | (total & 0x3F) << 8
        }
        // "s": yalnizca bir sure yasa (H).
        "s" => {
            unsafe { winapi::Sleep(1000) };
            DONE
        }
        _ => 0,
    };
    unsafe { winapi::ExitProcess(code) }
}

/// Ayni ikiliyi bir rolle baslatir. Doner: surec tutamaci (0 = olmadi).
fn spawn(tail: &str, inherit: bool, startup: Option<&mut winapi::StartupInfo>) -> Handle {
    let mut command = [0u8; 48];
    let head = b"winherit.exe ";
    command[..head.len()].copy_from_slice(head);
    let n = tail.len().min(command.len() - head.len() - 1);
    command[head.len()..head.len() + n].copy_from_slice(&tail.as_bytes()[..n]);
    let mut info = winapi::ProcessInformation::new();
    let si = match startup {
        Some(s) => s as *mut _ as *mut core::ffi::c_void,
        None => core::ptr::null_mut(),
    };
    let ok = unsafe {
        winapi::CreateProcessA(
            b"C:\\bin\\winherit.exe\0".as_ptr(),
            command.as_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            inherit as winapi::Bool,
            0,
            core::ptr::null_mut(),
            core::ptr::null(),
            si,
            &mut info,
        )
    };
    if ok == 0 {
        return 0;
    }
    unsafe { winapi::CloseHandle(info.thread) };
    info.process
}

/// Cocugun cikis kodunu sinirli sure bekler.
fn reap(p: Handle, ms: Dword) -> Option<Dword> {
    if p == 0 {
        return None;
    }
    for _ in 0..(ms / 20).max(1) {
        let mut code = 0;
        if unsafe { winapi::GetExitCodeProcess(p, &mut code) } != 0 && code != winapi::STILL_ACTIVE {
            unsafe { winapi::CloseHandle(p) };
            return Some(code);
        }
        unsafe { winapi::Sleep(20) };
    }
    None
}

/// Boruyu `ms` boyunca **bloke olmadan** okur. Doner: (okunan,
/// okunan bayt, kirik boru goruldu mu, kirik borunun goruldugu ms).
fn drain(r: Handle, out: &mut [u8], ms: Dword) -> (usize, bool, Dword) {
    let mode = winapi::PIPE_NOWAIT;
    unsafe { winapi::SetNamedPipeHandleState(r, &mode, core::ptr::null_mut(), core::ptr::null_mut()) };
    let start = unsafe { winapi::GetTickCount() };
    let mut n = 0usize;
    loop {
        let elapsed = unsafe { winapi::GetTickCount() }.wrapping_sub(start);
        if elapsed > ms {
            return (n, false, elapsed);
        }
        let mut buf = [0u8; 16];
        let mut got: Dword = 0;
        let ok = unsafe { winapi::ReadFile(r, buf.as_mut_ptr(), 16, &mut got, core::ptr::null_mut()) };
        if ok != 0 {
            let take = (got as usize).min(out.len() - n);
            out[n..n + take].copy_from_slice(&buf[..take]);
            n += take;
            continue;
        }
        if unsafe { winapi::GetLastError() } == winapi::ERROR_BROKEN_PIPE {
            return (n, true, elapsed);
        }
        unsafe { winapi::Sleep(20) };
    }
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

const NAMES: [&str; 9] = [
    "A varsayilan GECMEZ",
    "B miras alinabilir gecer",
    "C bInheritHandles=FALSE",
    "D ayni konum",
    "E SetHandleInformation",
    "F stdout boruya",
    "G stdin borudan",
    "H sizan uc",
    "I miras alinamayan std",
];

fn num(buf: &mut [u8; 24], prefix: &str, v: usize) -> usize {
    let p = prefix.as_bytes();
    buf[..p.len()].copy_from_slice(p);
    let mut n = p.len();
    if v >= 10 {
        buf[n] = b'0' + (v / 10) as u8;
        n += 1;
    }
    buf[n] = b'0' + (v % 10) as u8;
    n + 1
}

fn role_with(role: &str, h: Handle) -> ([u8; 24], usize) {
    let mut buf = [0u8; 24];
    let mut prefix = [0u8; 4];
    prefix[0] = role.as_bytes()[0];
    prefix[1] = b' ';
    let n = num(&mut buf, core::str::from_utf8(&prefix[..2]).unwrap_or("r "), h as usize);
    (buf, n)
}

fn run_role(role: &str, h: Handle, inherit: bool) -> Option<Dword> {
    let (buf, n) = role_with(role, h);
    let tail = core::str::from_utf8(&buf[..n]).unwrap_or("r 0");
    reap(spawn(tail, inherit, None), 3000)
}

fn main() {
    if tcmk::args::count() > 1 {
        if let Some(role) = tcmk::args::get(1) {
            child(role);
        }
    }

    use core::fmt::Write;
    let mut console = winapi::Console;
    let _ = writeln!(console, "[winherit] sinav basliyor");
    let mut checks = [EMPTY; 9];

    // --- A: varsayilan gecmez -------------------------------------------
    let plain = open_msg(false);
    let a_flags = flags_of(plain);
    let a_code = run_role("r", plain, true);
    let a = a_flags == Some(0) && a_code == Some(DONE);
    checks[0] = Check {
        name: NAMES[0],
        detail: if a {
            "nitelik vermeden acilan dosya cocukta YOK"
        } else if a_code.map_or(false, |c| c & 1 != 0) {
            "bayraksiz tutamac cocuga GECTI"
        } else if a_code.is_none() {
            "cocuk cevap vermedi"
        } else {
            "bayrak beklenenden farkli"
        },
        passed: a,
    };
    unsafe { winapi::CloseHandle(plain) };

    // --- B, C: miras alinabilir tutamac, bInheritHandles iki yonde ------
    let shared = open_msg(true);
    let b_code = run_role("r", shared, true);
    let b = flags_of(shared) == Some(winapi::HANDLE_FLAG_INHERIT) && b_code == Some(DONE | 3);
    checks[1] = Check {
        name: NAMES[1],
        detail: if b {
            "cocuk ayni numarayla okudu, bayrak cocukta da acik"
        } else if b_code == Some(DONE) {
            "miras alinabilir tutamac GECMEDI"
        } else if b_code == Some(DONE | 1) {
            "gecti ama cocukta bayrak kapali"
        } else {
            "cocuk cevap vermedi ya da bayrak yanlis"
        },
        passed: b,
    };
    let c_code = run_role("r", shared, false);
    let c = c_code == Some(DONE);
    checks[2] = Check {
        name: NAMES[2],
        detail: if c {
            "bInheritHandles=FALSE ile hicbir tutamac gecmedi"
        } else if c_code.map_or(false, |x| x & 1 != 0) {
            "bInheritHandles=FALSE yok sayildi: tutamac GECTI"
        } else {
            "cocuk cevap vermedi"
        },
        passed: c,
    };

    // --- D: ayni konum ----------------------------------------------------
    unsafe { winapi::SetFilePointer(shared, 0, core::ptr::null_mut(), winapi::FILE_BEGIN) };
    let first = read4(shared);
    let d_code = run_role("p", shared, true);
    let after = read4(shared);
    // msg.txt: "/boot/msg.txt: ..." -- 0-4 "/boo", 4-8 "t/ms", 8-12 "g.tx".
    let child_saw = d_code.map(|c| ((c >> 8) & 0x3F) as u8);
    let d = first == Some(*b"/boo")
        && child_saw == Some(b't' & 0x3F)
        && after == Some(*b"g.tx");
    checks[3] = Check {
        name: NAMES[3],
        detail: if d {
            "cocuk 4. bayttan, ebeveyn sonra 8. bayttan devam etti"
        } else if child_saw == Some(b'/' & 0x3F) {
            "cocuk BASTAN okudu: konum paylasilmiyor"
        } else if after == Some(*b"t/ms") {
            "cocugun okumasi ebeveynin konumunu TASIMADI"
        } else {
            "okuma ya da cocuk basarisiz"
        },
        passed: d,
    };

    // --- E: SetHandleInformation bayragi kapatir --------------------------
    let cleared = unsafe { winapi::SetHandleInformation(shared, winapi::HANDLE_FLAG_INHERIT, 0) } != 0;
    let e_code = run_role("r", shared, true);
    let e = cleared && flags_of(shared) == Some(0) && e_code == Some(DONE);
    checks[4] = Check {
        name: NAMES[4],
        detail: if e {
            "bayrak kapaninca tutamac gecmedi"
        } else if !cleared {
            "SetHandleInformation basarisiz"
        } else if flags_of(shared) != Some(0) {
            "GetHandleInformation bayragi hala acik goruyor"
        } else {
            "bayrak kapali ama tutamac yine GECTI"
        },
        passed: e,
    };
    unsafe { winapi::CloseHandle(shared) };

    // --- F: STARTF_USESTDHANDLES ile stdout boruya ------------------------
    let mut sa = winapi::SecurityAttributes::inheritable();
    let (mut r, mut w): (Handle, Handle) = (0, 0);
    let piped = unsafe {
        winapi::CreatePipe(&mut r, &mut w, &mut sa as *mut _ as *mut core::ffi::c_void, 0)
    } != 0;
    let (f, f_detail) = if piped {
        // Kendi ucumuz gecmesin: cocuk okuma ucunu tasimamali.
        unsafe { winapi::SetHandleInformation(r, winapi::HANDLE_FLAG_INHERIT, 0) };
        let mut si = winapi::StartupInfo::with_std(0, w, w);
        let p = spawn("o", true, Some(&mut si));
        // Ebeveynin yazma kopyasi kapanmali; yoksa boru hic bitmez.
        unsafe { winapi::CloseHandle(w) };
        let mut out = [0u8; 32];
        let (n, broken, _) = drain(r, &mut out, 3000);
        let code = reap(p, 2000);
        unsafe { winapi::CloseHandle(r) };
        let ok = n == 8 && &out[..8] == b"merhaba\n" && broken && code == Some(DONE | 1);
        (
            ok,
            if ok {
                "cocugun stdout'u boruda: merhaba, sonra boru kapandi"
            } else if n == 0 && code.is_some() {
                "cocuk yazdi ama boruya GELMEDI: stdout kurulmadi"
            } else if !broken {
                "veri geldi ama boru KAPANMADI: yazma ucu sizdi"
            } else {
                "icerik ya da cocuk beklenenden farkli"
            },
        )
    } else {
        (false, "boru yaratilamadi")
    };
    checks[5] = Check {
        name: NAMES[5],
        detail: f_detail,
        passed: f,
    };

    // --- G: stdin borudan --------------------------------------------------
    let (mut r, mut w): (Handle, Handle) = (0, 0);
    let piped = unsafe {
        winapi::CreatePipe(&mut r, &mut w, &mut sa as *mut _ as *mut core::ffi::c_void, 0)
    } != 0;
    let (g, g_detail) = if piped {
        unsafe { winapi::SetHandleInformation(w, winapi::HANDLE_FLAG_INHERIT, 0) };
        let mut si = winapi::StartupInfo::with_std(r, 1, 2);
        let p = spawn("i", true, Some(&mut si));
        unsafe { winapi::CloseHandle(r) };
        let text = b"abcde";
        let mut wrote: Dword = 0;
        unsafe {
            winapi::WriteFile(w, text.as_ptr(), text.len() as Dword, &mut wrote, core::ptr::null_mut());
            winapi::CloseHandle(w);
        }
        let code = reap(p, 3000);
        let ok = code == Some(DONE | 5 << 8);
        (
            ok,
            if ok {
                "cocuk stdin'den 5 bayt okudu ve boru sonunu gordu"
            } else if code.is_none() {
                "cocuk BITMEDI: stdin borusu kapanmadi ya da kurulmadi"
            } else {
                "cocuk beklenenden farkli sayida bayt okudu"
            },
        )
    } else {
        (false, "boru yaratilamadi")
    };
    checks[6] = Check {
        name: NAMES[6],
        detail: g_detail,
        passed: g,
    };

    // --- H: sizan uc --------------------------------------------------------
    //
    // Iki uc da miras alinabilir ve ebeveyn hicbirini kapatmadan ilgisiz
    // bir cocuk baslatiyor. Cocuk yazma ucunu tasiyor; ebeveyn kendi
    // kopyasini kapatsa da boru cocuk olene kadar "acik".
    let (mut r, mut w): (Handle, Handle) = (0, 0);
    let piped = unsafe {
        winapi::CreatePipe(&mut r, &mut w, &mut sa as *mut _ as *mut core::ffi::c_void, 0)
    } != 0;
    let (h, h_detail, h_ms) = if piped {
        let p = spawn("s", true, None);
        unsafe { winapi::CloseHandle(w) };
        let mut out = [0u8; 4];
        let (_, broken, ms) = drain(r, &mut out, 3000);
        let code = reap(p, 2000);
        unsafe { winapi::CloseHandle(r) };
        let ok = p != 0 && broken && ms >= 700 && code == Some(DONE);
        (
            ok,
            if ok {
                "devralinan yazma ucu boruyu cocugun omru boyunca acik tuttu"
            } else if broken && ms < 700 {
                "boru HEMEN kapandi: cocuk yazma ucunu devralmadi"
            } else if !broken {
                "boru cocuk olduktan sonra da KAPANMADI"
            } else {
                "cocuk baslatilamadi"
            },
            ms,
        )
    } else {
        (false, "boru yaratilamadi", 0)
    };
    checks[7] = Check {
        name: NAMES[7],
        detail: h_detail,
        passed: h,
    };

    // --- I: miras alinamayan std tutamaci gecmez ---------------------------
    //
    // Windows'un kurali: `STARTF_USESTDHANDLES` ile verilen tutamaclar miras
    // alinabilir olmali. Bayraksiz bir boru verilince cocugun stdout'u o
    // boru olmamali -- TCMK'de yuva varsayilanda (konsol) kaliyor.
    let (mut r, mut w): (Handle, Handle) = (0, 0);
    let piped = unsafe { winapi::CreatePipe(&mut r, &mut w, core::ptr::null_mut(), 0) } != 0;
    let (i_ok, i_detail) = if piped {
        let mut si = winapi::StartupInfo::with_std(0, w, w);
        let p = spawn("o", true, Some(&mut si));
        let code = reap(p, 3000);
        unsafe { winapi::CloseHandle(w) };
        let mut out = [0u8; 16];
        let (n, broken, _) = drain(r, &mut out, 1500);
        unsafe { winapi::CloseHandle(r) };
        let ok = code == Some(DONE | 1) && n == 0 && broken;
        (
            ok,
            if ok {
                "bayraksiz boru cocugun stdout'u OLMADI"
            } else if n > 0 {
                "miras alinamayan boru yine de stdout oldu"
            } else if code.is_none() {
                "cocuk cevap vermedi"
            } else {
                "boru beklenenden farkli davrandi"
            },
        )
    } else {
        (false, "boru yaratilamadi")
    };
    checks[8] = Check {
        name: NAMES[8],
        detail: i_detail,
        passed: i_ok,
    };

    for check in &checks {
        let _ = writeln!(
            console,
            "[winherit] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let passed = checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(console, "[winherit] olcu: H boru {} ms acik kaldi", h_ms);
    let _ = writeln!(console, "[winherit] sonuc: {}/9 gecti", passed);
    show(&checks);
}

fn show(checks: &[Check; 9]) {
    let mut win = match Window::create("winherit -- tutamac mirasi", 250, 150, 500, 250) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        let (w, h) = (win.width(), win.height());
        win.clear(BG);
        win.fill(0, 0, w, 22, PANEL);
        win.text(6, 3, "Windows: miras bayrakla, POSIX: miras varsayilan", ACCENT);
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
        win.text(6, h - 14, "bayrak + bInheritHandles = cocukta ayni tutamac", DIM);
        win.frame(30);
    }
}
