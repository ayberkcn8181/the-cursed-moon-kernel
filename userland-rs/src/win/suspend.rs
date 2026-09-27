//! `winsusp.exe` -- askiya almak: ayni mekanizma, **sayilan** bir soz.
//!
//! Bir onceki bati POSIX'in is denetimini getirdi: `SIGSTOP` durduruyor,
//! `SIGCONT` kaldiriyor. Windows ayni seyi yapiyor gibi gorunuyor ve
//! yapmiyor -- cunku onun askisi **sayiliyor**:
//!
//! ```text
//!   POSIX  SIGSTOP x2 + SIGCONT x1  ->  KOSUYOR   (sayilmaz)
//!   Win32  Suspend x2 + Resume x1   ->  DURUYOR   (sayilir)
//! ```
//!
//! Fark keyfi degil. Windows'un sayaci, ayni akisi birbirinden habersiz
//! **iki kutuphanenin** askiya alabilmesi icin: biri devam
//! ettirdiginde otekinin askisi bozulmamali. Bir hata ayiklayici ile
//! bir profilleyici ayni anda calisiyorsa bu sart. POSIX'in yaklasimi
//! daha yalin -- ve daha kaba: son `SIGCONT` kazanir.
//!
//! TCMK ikisini **ayni** cekirdek mekanizmasinin (`TaskState::Stopped`)
//! uzerine kuruyor ama sozlesmeleri ayri tutuyor: sayac NT tarafinda
//! duruyor, POSIX tarafi onu hic gormuyor. Bir `SIGCONT` gelirse sayac
//! sifirlaniyor, yoksa ayni gorev icin iki ayri gercek olusurdu.
//!
//! ## `CREATE_SUSPENDED`: POSIX'te karsiligi olmayan bir dogum
//!
//! ```text
//!   Win32  CreateThread(.., CREATE_SUSPENDED, ..)
//!            -> akis dogar, giris noktasina HIC girmez
//!   POSIX  clone(..)
//!            -> akis dogar ve KOSAR; durdurmak icin once kosmasi gerekir
//! ```
//!
//! Ayrim gorunurden fazlasi: `CREATE_SUSPENDED` ile bir akis
//! yaratilip, kosmadan **once** onceligi ayarlanabilir ya da baglami
//! degistirilebilir. POSIX'te arada birkac komut mutlaka yurur.
//!
//! ## Alti sinav
//!
//! ```text
//!   A  askida dogdu   -> CREATE_SUSPENDED ile yaratilan akis ilerlemiyor
//!   B  Resume         -> devam edince ilerliyor
//!   C  SAYILIYOR      -> iki Suspend + bir Resume sonrasi HALA duruyor
//!   D  sayac sifir    -> ikinci Resume ile yeniden ilerliyor
//!   E  donus degeri   -> Suspend/Resume ONCEKI sayiyi donduruyor
//!   F  oncelik        -> SetThreadPriority/GetThreadPriority gidip geliyor
//! ```
//!
//! C bu sinavin sebebi. A, B ve D olmadan da bir "durdur/devam ettir"
//! gosterilebilirdi; ayirt edici olan, **bir** `Resume`un yetmemesi.
//! POSIX ikizi (`jobs`) ayni yerde zit cevabi veriyor.
//!
//! E ayri duruyor cunku donus degeri Windows'ta bilgi tasiyor: cagiran,
//! askinin kendisinden **once** kac kez alinmis oldugunu ogreniyor.
//! POSIX'in `kill`i yalnizca "gonderildi" der.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use tcmk::winapi::{self, Dword, Window};

tcmk::entry!(main);

const BG: u32 = 0x0018_1626;
const PANEL: u32 = 0x0026_2238;
const FG: u32 = 0x00E6_E2F4;
const DIM: u32 = 0x0092_8CA8;
const ACCENT: u32 = 0x00A0_B0F0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Akisin iki artirma arasinda bekledigi sure.
const TICK_MS: Dword = 20;
/// Ilerlemenin olculdugu pencere. Akis kosuyor olsaydi bu surede en az
/// yedi artirma olurdu; durmussa **hic** olmaz.
const WATCH_MS: Dword = 160;

/// Akisin ilerlemesi. Paylasilan bellek -- is parcaciklari ayni adres
/// uzayinda.
static PROGRESS: AtomicU32 = AtomicU32::new(0);

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

/// Surekli ilerleyen akis. Olculen sey bu sayacin **artip artmadigi**.
unsafe extern "system" fn ticker(_parameter: *mut c_void) -> Dword {
    loop {
        PROGRESS.fetch_add(1, Ordering::SeqCst);
        winapi::Sleep(TICK_MS);
    }
}

/// Verilen sure boyunca ilerleme oldu mu.
fn advances() -> bool {
    let before = PROGRESS.load(Ordering::SeqCst);
    unsafe { winapi::Sleep(WATCH_MS) };
    PROGRESS.load(Ordering::SeqCst) != before
}

fn main() {
    let mut checks = [EMPTY; 6];

    let mut thread_id = 0u32;
    let handle = unsafe {
        winapi::CreateThread(
            core::ptr::null_mut(),
            0,
            Some(ticker),
            core::ptr::null_mut(),
            winapi::CREATE_SUSPENDED,
            &mut thread_id,
        )
    };

    if handle == 0 {
        for (i, name) in [
            "A askida dogdu",
            "B Resume",
            "C SAYILIYOR",
            "D sayac sifir",
            "E donus degeri",
            "F oncelik",
        ]
        .iter()
        .enumerate()
        {
            checks[i] = Check {
                name,
                detail: "akis yaratilamadi",
                passed: false,
            };
        }
        report(&checks, 0, 0);
        show(&checks, 0);
        return;
    }

    // --- A: askida dogdu ---
    //
    // Olcu "ilerlemiyor". Yalnizca bir bayraga bakmak yetmezdi: akis
    // bir kez kosup sonra durmus olsaydi bayrak yine kurulu olurdu.
    let born_quiet = !advances();
    checks[0] = Check {
        name: "A askida dogdu",
        detail: if born_quiet {
            "CREATE_SUSPENDED akisi hic kosmadi"
        } else {
            "akis askida DOGMADI"
        },
        passed: born_quiet,
    };

    // --- B: Resume ---
    let first_resume = unsafe { winapi::ResumeThread(handle) };
    let running = advances();
    checks[1] = Check {
        name: "B Resume",
        detail: if running {
            "devam edince ilerledi"
        } else {
            "ResumeThread akisi KALDIRMADI"
        },
        passed: running,
    };

    // --- C: aski SAYILIYOR ---
    //
    // Sinavin sebebi burasi. POSIX'te iki `SIGSTOP` + bir `SIGCONT`
    // kosan bir surec verir; Windows'ta ayni dizi **hala askida** olan
    // bir akis verir.
    let suspend_a = unsafe { winapi::SuspendThread(handle) };
    let suspend_b = unsafe { winapi::SuspendThread(handle) };
    let resume_a = unsafe { winapi::ResumeThread(handle) };
    let still_stopped = !advances();
    checks[2] = Check {
        name: "C SAYILIYOR",
        detail: if still_stopped {
            "iki Suspend + bir Resume: hala duruyor"
        } else {
            "bir Resume yetti (POSIX gibi davrandi)"
        },
        passed: still_stopped,
    };

    // --- D: sayac sifira dusunce ---
    let resume_b = unsafe { winapi::ResumeThread(handle) };
    let running_again = advances();
    checks[3] = Check {
        name: "D sayac sifir",
        detail: if running_again {
            "ikinci Resume ile yeniden ilerledi"
        } else {
            "sayac sifirlandi ama akis KALKMADI"
        },
        passed: running_again,
    };

    // --- E: donus degerleri ---
    //
    // Windows her iki cagride de **onceki** sayiyi donduruyor:
    //
    //   Resume  (askida dogmus)  -> 1
    //   Suspend (kosuyor)        -> 0
    //   Suspend (bir kez askida) -> 1
    //   Resume  (iki kez askida) -> 2
    //   Resume  (bir kez askida) -> 1
    let counts_ok = first_resume == 1
        && suspend_a == 0
        && suspend_b == 1
        && resume_a == 2
        && resume_b == 1;
    checks[4] = Check {
        name: "E donus degeri",
        detail: if counts_ok {
            "her cagri onceki sayiyi dondurdu"
        } else if first_resume != 1 {
            "askida dogan akisin sayisi 1 degildi"
        } else {
            "aski sayisi yanlis ilerledi"
        },
        passed: counts_ok,
    };

    // --- F: oncelik ---
    //
    // Olcek POSIX'in `nice`inin tersi: buyuk sayi daha oncelikli.
    // Cekirdek ikisini eslestiriyor, sinav gidip gelmeyi olcuyor.
    let set_high =
        unsafe { winapi::SetThreadPriority(handle, winapi::THREAD_PRIORITY_HIGHEST) } != 0;
    let read_high = unsafe { winapi::GetThreadPriority(handle) };
    let set_normal =
        unsafe { winapi::SetThreadPriority(handle, winapi::THREAD_PRIORITY_NORMAL) } != 0;
    let read_normal = unsafe { winapi::GetThreadPriority(handle) };
    let priority_ok = set_high
        && read_high == winapi::THREAD_PRIORITY_HIGHEST
        && set_normal
        && read_normal == winapi::THREAD_PRIORITY_NORMAL;
    checks[5] = Check {
        name: "F oncelik",
        detail: if !set_high || !set_normal {
            "SetThreadPriority reddedildi"
        } else if read_high == winapi::THREAD_PRIORITY_ERROR_RETURN {
            "GetThreadPriority hata dondurdu"
        } else if priority_ok {
            "HIGHEST ve NORMAL gidip geldi"
        } else {
            "okunan oncelik yazilanla AYNI DEGIL"
        },
        passed: priority_ok,
    };

    // Akisi askiya alip birak: pencere dongusu boyunca bosuna
    // kosmasinin anlami yok.
    unsafe { winapi::SuspendThread(handle) };
    let progress = PROGRESS.load(Ordering::SeqCst);
    report(&checks, progress, thread_id);
    unsafe { winapi::CloseHandle(handle) };
    show(&checks, progress);
}

fn report(checks: &[Check; 6], progress: u32, thread_id: u32) {
    use core::fmt::Write;
    let mut console = winapi::Console;
    for check in checks {
        let _ = writeln!(
            console,
            "[winsusp] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let _ = writeln!(
        console,
        "[winsusp] ilerleme: {}  akis kimligi: {}",
        progress, thread_id
    );
}

fn show(checks: &[Check; 6], progress: u32) {
    let mut win = match Window::create("winsusp -- sayilan aski", 285, 185, 470, 190) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, checks, progress);
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 6], progress: u32) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Win32'de aski SAYILIR, POSIX'te sayilmaz", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            365,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, "ilerleme:", DIM);
    win.number(100, h - 30, progress as usize, FG);
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
