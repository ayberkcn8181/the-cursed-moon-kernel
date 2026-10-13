//! `winprio.exe` -- iki katmanli oncelik: surec sinifi + akis duzeyi.
//!
//! POSIX'te oncelik **tek sayidir**: gorev basina bir `nice`. Windows'ta
//! iki katmandan olusur:
//!
//! ```text
//!   surec SINIFI   (SetPriorityClass)   -> taban: 4, 6, 8, 10, 13
//!   akis DUZEYI    (SetThreadPriority)  -> taban + (-2 .. +2)
//! ```
//!
//! Etkin oncelik ikisinin toplami, ve akis duzeyi **goreli**: bir akisi
//! `ABOVE_NORMAL` yapip surecin sinifini degistirirsen akis hala
//! `ABOVE_NORMAL`dir. TCMK uzun sure duzeyi dogrudan `nice`e ceviriyordu;
//! sinif olmadigi surece fark gorunmuyordu.
//!
//! ## Sekiz sinav
//!
//! ```text
//!   A  varsayilan NORMAL  -> yeni surecin sinifi NORMAL
//!   B  gidip geliyor      -> bes sinifin her biri yazilip geri okunuyor
//!   C  REALTIME -> HIGH   -> ayricaliksiz surec: cagri basarili, sinif HIGH
//!   D  gecersiz reddedilir-> bilinmeyen sinif, gecersiz duzey ve iki
//!                            sinif bayragi ERROR_INVALID_PARAMETER
//!   E  duzey GORELI       -> sinif degisince akisin duzeyi ayni kaliyor
//!   F  sinif CPU PAYINI   -> ayni isi HIGH surec IDLE surecten belirgin
//!      degistiriyor          bicimde once bitiriyor
//!   G  olusturma bayragi  -> CreateProcess'in sinif bayragi cocuga geciyor
//!   H  miras kurali       -> IDLE ebeveynin bayraksiz cocugu IDLE, HIGH
//!                            ebeveyninki NORMAL
//!   I  tutamac kapaninca  -> cikmis cocugun tutamaci kapaninca yuvasi
//!      yuva doner            geri veriliyor (yuva tavanindan fazla cocuk)
//!   J  yeni akis sinifta  -> CreateThread ile yaratilan akis da surecin
//!                            sinifindan basliyor (F'nin akis hali)
//! ```
//!
//! I bu sinavin buldugu bir hatayi olcuyor. J'nin ilk kosumunda
//! `CreateThread` basarisiz oldu ve sebep oncelikle ilgisizdi: gorev
//! tablosu doluydu. `CreateProcess` ile yaratilan cocuklar cikis kodu
//! `GetExitCodeProcess` ile yoklanip tutamaclari kapatilsa bile **hic
//! toplanmiyordu** -- yalnizca bloke eden `WaitForSingleObject` yuvayi
//! geri veriyordu. Windows'ta surec nesnesi son tutamac kapaninca yok
//! olur.
//!
//! J ayri bir sinav cunku ayri bir yol: yeni akis `nice` sifirla dogar
//! ve sinif ona `CreateThread` sirasinda **ayrica** uygulanmali. F'de isi
//! ana akis yaptigi icin o yol hic olculmuyordu -- bir bozma sinavinda
//! kaldirilsa F yine gecerdi.
//!
//! F bu sinavin olcusu: B'den E'ye kadarki her sey bir **kayit** -- yazilan
//! deger geri okunuyor. Kaydin zamanlayiciya ulasip ulasmadigini ancak
//! harcanan sure soyler. Bos bir kayit (yazilan saklanir ama hicbir sey
//! degismez) B..E'yi gecerdi; F'yi gecemez.
//!
//! H Windows'un en az bilinen kurallarindan biri ve asimetrik: dusuk
//! oncelikli bir arka plan isinin cocuklari da arka planda kalir, ama
//! yuksek oncelikli bir surec bu ayricaligi cocuklarina kendiliginden
//! gecirmez.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::winapi::{self, Window};

tcmk::entry!(main);

const BG: u32 = 0x0014_1A20;
const PANEL: u32 = 0x0022_2A34;
const FG: u32 = 0x00E6_ECF2;
const DIM: u32 = 0x0090_9AA8;
const ACCENT: u32 = 0x00FF_C870;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

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

const NAMES: [&str; 10] = [
    "A varsayilan NORMAL",
    "B gidip geliyor",
    "C REALTIME -> HIGH",
    "D gecersiz reddedilir",
    "E duzey GORELI",
    "F sinif CPU PAYINI degistiriyor",
    "G olusturma bayragi",
    "H miras kurali",
    "I tutamac kapaninca yuva doner",
    "J yeni akis sinifta",
];

fn say(check: &Check) {
    use core::fmt::Write;
    let _ = writeln!(
        winapi::Console,
        "[winprio] {}: {} ({})",
        check.name,
        if check.passed { "gecti" } else { "KALDI" },
        check.detail
    );
}

fn me() -> winapi::Handle {
    unsafe { winapi::GetCurrentProcess() }
}

fn class() -> u32 {
    unsafe { winapi::GetPriorityClass(me()) }
}

fn set_class(c: u32) -> bool {
    unsafe { winapi::SetPriorityClass(me(), c) != 0 }
}

/// Basit ondalik ayristirici (argumanlar icin).
fn parse(text: &str) -> u32 {
    let mut value = 0u32;
    for b in text.bytes() {
        if b.is_ascii_digit() {
            value = value.wrapping_mul(10).wrapping_add((b - b'0') as u32);
        }
    }
    value
}

/// Sayiyi ondalik olarak tampona yazar; uzunlugu doner.
fn put_num(out: &mut [u8], mut value: u32) -> usize {
    let mut digits = [0u8; 10];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (value % 10) as u8;
        n += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for i in 0..n {
        out[i] = digits[n - 1 - i];
    }
    n
}

/// Ayni ikiliyi verilen komut satiri kuyruguyla ve sinif bayragiyla
/// baslatir. Doner: tutamac (0 = olmadi).
fn spawn(tail: &[u8], flags: u32) -> usize {
    let mut command = [0u8; 64];
    let head = b"winprio.exe ";
    command[..head.len()].copy_from_slice(head);
    let n = tail.len().min(command.len() - head.len() - 1);
    command[head.len()..head.len() + n].copy_from_slice(&tail[..n]);
    let mut info = winapi::ProcessInformation::new();
    let created = unsafe {
        winapi::CreateProcessA(
            b"C:\\bin\\winprio.exe\0".as_ptr(),
            command.as_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            0,
            flags,
            core::ptr::null_mut(),
            core::ptr::null(),
            core::ptr::null_mut(),
            &mut info,
        )
    };
    if created == 0 {
        0
    } else {
        // Gercek programlarin kalibi: akis tutamaci hemen kapatilir,
        // surec tutamaci beklemek icin tutulur. TCMK'de ikisi ayni
        // deger; cekirdek onlari **sayarak** ayiriyor ve bu kapatma
        // sureci birakmamali.
        unsafe { winapi::CloseHandle(info.thread) };
        info.process as usize
    }
}

/// Cocugun cikis kodunu **sinirli** sure bekler; donmezse `None`.
fn reap(child: usize, ms: u32) -> Option<u32> {
    if child == 0 {
        return None;
    }
    let handle = child as winapi::Handle;
    for _ in 0..(ms / 50).max(1) {
        let mut code = 0u32;
        if unsafe { winapi::GetExitCodeProcess(handle, &mut code) } != 0
            && code != winapi::STILL_ACTIVE
        {
            unsafe { winapi::CloseHandle(handle) };
            return Some(code);
        }
        unsafe { winapi::Sleep(50) };
    }
    // Zaman asimi: cocuk gercekten mi kosuyor, yoksa **bitti ama kodu
    // kayboldu** mu? Ikincisinde bekleme hemen doner ama kod hala
    // `STILL_ACTIVE`. Bu ayrim bir bozma sinavindan geldi: tutamaclar
    // tek sayilinca `CloseHandle(pi.hThread)` sureci birakiyordu ve
    // sinav bunu yalnizca "cevap vermedi" diye, cok gec raporluyordu.
    if unsafe { winapi::WaitForSingleObject(handle, 0) } == winapi::WAIT_OBJECT_0 {
        LOST_EXIT.store(true, core::sync::atomic::Ordering::SeqCst);
    }
    unsafe { winapi::CloseHandle(handle) };
    None
}

/// Bir cocuk bitti ama cikis kodu okunamadi.
static LOST_EXIT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Belirli sayida yineleme: optimize edilemesin diye `black_box`.
fn burn(iterations: u32) {
    let mut acc = 0u32;
    for i in 0..iterations {
        acc = core::hint::black_box(acc.wrapping_mul(31).wrapping_add(i));
    }
    core::hint::black_box(acc);
}

/// Isci cocuk: baslangic anini bekler, isi yapar, gecen sureyi doner.
///
/// Iki isci **ayni anda** baslamali; aksi halde once baslayan hakli
/// olarak once biterdi ve olculen sey oncelik degil sira olurdu. Ortak
/// bir saat ani komut satirinda geliyor.
fn worker(iterations: u32, start: u32) -> u32 {
    while unsafe { winapi::GetTickCount() } < start {
        unsafe { winapi::Sleep(10) };
    }
    let before = unsafe { winapi::GetTickCount() };
    burn(iterations);
    unsafe { winapi::GetTickCount() }.wrapping_sub(before)
}

static THREAD_WORK: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static THREAD_START: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static THREAD_MS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// I'nin cocugu: `CreateThread` basarisiz.
const NO_THREAD: u32 = 0x00F0_0000;
/// I'nin cocugu: bekleme akis bitmeden dondu (alt bayt: donus degeri).
const EARLY_WAIT: u32 = 0x00E0_0000;

/// I'nin akisi: isi yapar, sureyi birakir.
unsafe extern "system" fn thread_worker(_param: *mut core::ffi::c_void) -> u32 {
    let n = THREAD_WORK.load(core::sync::atomic::Ordering::SeqCst);
    let start = THREAD_START.load(core::sync::atomic::Ordering::SeqCst);
    let ms = worker(n, start);
    THREAD_MS.store(ms, core::sync::atomic::Ordering::SeqCst);
    0
}

fn main() {
    // Cocuk rolleri.
    if tcmk::args::count() > 1 {
        match tcmk::args::get(1) {
            Some("c") => unsafe { winapi::ExitProcess(class()) },
            Some("w") => {
                let n = tcmk::args::get(2).map(parse).unwrap_or(0);
                let start = tcmk::args::get(3).map(parse).unwrap_or(0);
                unsafe { winapi::ExitProcess(worker(n, start)) };
            }
            // Ayni is, ama `CreateThread` ile yaratilan bir akista.
            Some("t") => {
                let n = tcmk::args::get(2).map(parse).unwrap_or(0);
                let start = tcmk::args::get(3).map(parse).unwrap_or(0);
                THREAD_WORK.store(n, core::sync::atomic::Ordering::SeqCst);
                THREAD_START.store(start, core::sync::atomic::Ordering::SeqCst);
                let handle = unsafe {
                    winapi::CreateThread(
                        core::ptr::null_mut(),
                        0,
                        Some(thread_worker),
                        core::ptr::null_mut(),
                        0,
                        core::ptr::null_mut(),
                    )
                };
                // Iki ariza ayri kodla: ilk kosumda ikisi de `0` donuyordu
                // ve sinav hangisi oldugunu soyleyemiyordu.
                if handle == 0 {
                    unsafe { winapi::ExitProcess(NO_THREAD) };
                }
                let waited = unsafe { winapi::WaitForSingleObject(handle, winapi::INFINITE) };
                let ms = THREAD_MS.load(core::sync::atomic::Ordering::SeqCst);
                if ms == 0 {
                    unsafe { winapi::ExitProcess(EARLY_WAIT | (waited & 0xFF)) };
                }
                unsafe { winapi::ExitProcess(ms) };
            }
            _ => {}
        }
    }

    use core::fmt::Write;
    let _ = writeln!(winapi::Console, "[winprio] sinav basliyor");
    let mut checks = [EMPTY; 10];

    // --- A ------------------------------------------------------------
    let initial = class();
    let a = initial == winapi::NORMAL_PRIORITY_CLASS;
    checks[0] = Check {
        name: NAMES[0],
        detail: if a {
            "yeni surec NORMAL sinifinda"
        } else if initial == 0 {
            "GetPriorityClass hata dondu"
        } else {
            "varsayilan sinif NORMAL degil"
        },
        passed: a,
    };
    say(&checks[0]);

    // --- B ------------------------------------------------------------
    let classes = [
        winapi::IDLE_PRIORITY_CLASS,
        winapi::BELOW_NORMAL_PRIORITY_CLASS,
        winapi::ABOVE_NORMAL_PRIORITY_CLASS,
        winapi::HIGH_PRIORITY_CLASS,
        winapi::NORMAL_PRIORITY_CLASS,
    ];
    let mut b = true;
    for &c in classes.iter() {
        if !set_class(c) || class() != c {
            b = false;
        }
    }
    checks[1] = Check {
        name: NAMES[1],
        detail: if b {
            "bes sinif yazildi ve ayni geri okundu"
        } else {
            "bir sinif yazilamadi ya da farkli okundu"
        },
        passed: b,
    };
    say(&checks[1]);

    // --- C ------------------------------------------------------------
    let rt_ok = set_class(winapi::REALTIME_PRIORITY_CLASS);
    let rt_read = class();
    set_class(winapi::NORMAL_PRIORITY_CLASS);
    let c = rt_ok && rt_read == winapi::HIGH_PRIORITY_CLASS;
    checks[2] = Check {
        name: NAMES[2],
        detail: if c {
            "cagri basarili, sinif sessizce HIGH oldu"
        } else if !rt_ok {
            "REALTIME REDDEDILDI: Windows ayricaliksizda basarili doner"
        } else if rt_read == winapi::REALTIME_PRIORITY_CLASS {
            "REALTIME VERILDI: ayricalik olmadan gercek zamanli sinif"
        } else {
            "beklenmedik sinif"
        },
        passed: c,
    };
    say(&checks[2]);

    // --- D ------------------------------------------------------------
    //
    // Uc ayri red, ve ucunde de **hicbir sey degismemeli**.
    let bad_class = !set_class(0x1234)
        && unsafe { winapi::GetLastError() } == winapi::ERROR_INVALID_PARAMETER
        && class() == winapi::NORMAL_PRIORITY_CLASS;
    let thread = unsafe { winapi::GetCurrentThread() };
    let level_before = unsafe { winapi::GetThreadPriority(thread) };
    let bad_level = unsafe { winapi::SetThreadPriority(thread, 7) } == 0
        && unsafe { winapi::GetLastError() } == winapi::ERROR_INVALID_PARAMETER
        && unsafe { winapi::GetThreadPriority(thread) } == level_before;
    // Iki sinif biti birden: surec **yaratilmamali**.
    let both = winapi::IDLE_PRIORITY_CLASS | winapi::HIGH_PRIORITY_CLASS;
    let bad_flags_handle = spawn(b"c", both);
    let bad_flags = bad_flags_handle == 0
        && unsafe { winapi::GetLastError() } == winapi::ERROR_INVALID_PARAMETER;
    if bad_flags_handle != 0 {
        let _ = reap(bad_flags_handle, 3000);
    }
    let d = bad_class && bad_level && bad_flags;
    checks[3] = Check {
        name: NAMES[3],
        detail: if d {
            "bilinmeyen sinif, duzey 7 ve iki sinif bayragi reddedildi"
        } else if !bad_class {
            "bilinmeyen sinif KABUL EDILDI"
        } else if !bad_level {
            "gecersiz duzey (7) kabul edildi ya da kirpildi"
        } else {
            "iki sinif bayragiyla surec YARATILDI"
        },
        passed: d,
    };
    say(&checks[3]);

    // --- E ------------------------------------------------------------
    //
    // Duzey goreli: sinif degisince ayni kalmali. Eski hal duzeyi
    // `nice`e cevirip ondan geri hesapliyordu; o zaman `HIGH` sinifinda
    // `ABOVE_NORMAL` bir akis `TIME_CRITICAL` gibi okunurdu.
    let set_above =
        unsafe { winapi::SetThreadPriority(thread, winapi::THREAD_PRIORITY_ABOVE_NORMAL) } != 0;
    let mut kept = true;
    for &c in [
        winapi::HIGH_PRIORITY_CLASS,
        winapi::IDLE_PRIORITY_CLASS,
        winapi::NORMAL_PRIORITY_CLASS,
    ]
    .iter()
    {
        set_class(c);
        if unsafe { winapi::GetThreadPriority(thread) } != winapi::THREAD_PRIORITY_ABOVE_NORMAL {
            kept = false;
        }
    }
    unsafe { winapi::SetThreadPriority(thread, winapi::THREAD_PRIORITY_NORMAL) };
    let e = set_above && kept;
    checks[4] = Check {
        name: NAMES[4],
        detail: if e {
            "uc sinifta da akis ABOVE_NORMAL kaldi"
        } else if !set_above {
            "SetThreadPriority(GetCurrentThread()) reddedildi"
        } else {
            "sinif degisince akisin duzeyi DEGISTI"
        },
        passed: e,
    };
    say(&checks[4]);

    // G ve H F'den **once** kosuyor: ikisi de hizli (cocuk hemen cikiyor)
    // ve tutamac mekanizmasini olcuyorlar. Yavas yaris (F) once kossaydi,
    // tutamac yolu bozukken sinav onun uzun beklemelerinde tukenir ve
    // asil taniyi hic yazmadan kesilirdi -- bir bozma sinavinda tam
    // olarak bu oldu.
    // --- G: olusturma bayragi -----------------------------------------
    let g_code = reap(spawn(b"c", winapi::BELOW_NORMAL_PRIORITY_CLASS), 5000);
    let g = g_code == Some(winapi::BELOW_NORMAL_PRIORITY_CLASS);
    checks[6] = Check {
        name: NAMES[6],
        detail: if g {
            "BELOW_NORMAL bayragiyla yaratilan cocuk BELOW_NORMAL"
        } else if g_code.is_none() && LOST_EXIT.load(core::sync::atomic::Ordering::SeqCst) {
            "cocuk bitti ama cikis kodu KAYBOLDU: hThread'i kapatmak sureci birakti"
        } else if g_code.is_none() {
            "cocuk cevap vermedi"
        } else {
            "bayrak cocuga GECMEDI"
        },
        passed: g,
    };
    say(&checks[6]);

    // --- H: miras kurali ----------------------------------------------
    set_class(winapi::IDLE_PRIORITY_CLASS);
    let from_idle = reap(spawn(b"c", 0), 8000);
    set_class(winapi::HIGH_PRIORITY_CLASS);
    let from_high = reap(spawn(b"c", 0), 5000);
    set_class(winapi::NORMAL_PRIORITY_CLASS);
    let h = from_idle == Some(winapi::IDLE_PRIORITY_CLASS)
        && from_high == Some(winapi::NORMAL_PRIORITY_CLASS);
    checks[7] = Check {
        name: NAMES[7],
        detail: if h {
            "IDLE ebeveynin cocugu IDLE, HIGH ebeveyninki NORMAL"
        } else if from_idle == Some(winapi::NORMAL_PRIORITY_CLASS) {
            "IDLE ebeveynin cocugu sinifi DEVRALMADI"
        } else if from_high == Some(winapi::HIGH_PRIORITY_CLASS) {
            "HIGH ebeveyn sinifini cocuga GECIRDI"
        } else {
            "cocuk cevap vermedi ya da beklenmedik sinif"
        },
        passed: h,
    };
    say(&checks[7]);

    // --- F: sinif zamanlayiciya ulasiyor mu ---------------------------
    //
    // Is miktari once **olculuyor**: QEMU'nun hizi bilinmiyor ve sabit bir
    // sayi ya cok kisa (fark olculemez) ya cok uzun (sinav susar) olurdu.
    // 150 ms'de kac yineleme yapildigi sayiliyor; isci bunun iki katini
    // yapiyor.
    let calib_start = unsafe { winapi::GetTickCount() };
    let mut rounds = 0u32;
    while unsafe { winapi::GetTickCount() }.wrapping_sub(calib_start) < 150 {
        burn(4096);
        rounds += 1;
    }
    let iterations = rounds.saturating_mul(4096).saturating_mul(2).max(4096);
    let start = unsafe { winapi::GetTickCount() }.wrapping_add(400);

    let mut tail = [0u8; 40];
    let mut n = 0;
    tail[n] = b'w';
    n += 1;
    tail[n] = b' ';
    n += 1;
    n += put_num(&mut tail[n..], iterations);
    tail[n] = b' ';
    n += 1;
    n += put_num(&mut tail[n..], start);

    let low = spawn(&tail[..n], winapi::IDLE_PRIORITY_CLASS);
    let high = spawn(&tail[..n], winapi::HIGH_PRIORITY_CLASS);
    let low_ms = reap(low, 8000);
    let high_ms = reap(high, 8000);
    let f = match (low_ms, high_ms) {
        // HIGH en az %25 daha kisa surmeli. Beklenen fark cok daha buyuk
        // (dilim butcesi 8'e 1); esik genis tutuldu ki olcu gurultuye
        // degil davranisa baksin.
        (Some(l), Some(h)) => h.saturating_mul(4) < l.saturating_mul(3),
        _ => false,
    };
    checks[5] = Check {
        name: NAMES[5],
        detail: if f {
            "HIGH surec ayni isi IDLE surecten belirgin once bitirdi"
        } else if low_ms.is_none() || high_ms.is_none() {
            "isci cevap vermedi"
        } else {
            "iki sinif AYNI hizda: sinif zamanlayiciya ulasmiyor"
        },
        passed: f,
    };
    say(&checks[5]);

    // --- I: tutamac kapaninca yuva geri geliyor -----------------------
    //
    // Yuva tavanindan (12) fazla cocuk **sirayla** yaratiliyor: her biri
    // hemen cikiyor, ebeveyn kodunu yokluyor ve tutamacini kapatiyor.
    // Kapatma yuvayi geri vermiyorsa tablo dolar ve bir noktada
    // `CreateProcess` basarisiz olur.
    const CHILDREN: u32 = 16;
    let mut made = 0u32;
    for _ in 0..CHILDREN {
        let child = spawn(b"c", 0);
        if child == 0 {
            break;
        }
        if reap(child, 5000).is_none() {
            break;
        }
        made += 1;
    }
    let i_leak = made == CHILDREN;
    checks[8] = Check {
        name: NAMES[8],
        detail: if i_leak {
            "16 cocuk sirayla yaratilip kapatildi; tablo dolmadi"
        } else if made < 8 {
            "tablo DOLDU: kapatilan tutamaclar yuvalari geri vermiyor"
        } else {
            "bir cocuk yaratilamadi ya da cevap vermedi"
        },
        passed: i_leak,
    };
    say(&checks[8]);

    // --- J: ayni yaris, isi CreateThread akisi yapiyor ----------------
    let start_i = unsafe { winapi::GetTickCount() }.wrapping_add(400);
    let mut n_i = 0;
    tail[n_i] = b't';
    n_i += 1;
    tail[n_i] = b' ';
    n_i += 1;
    n_i += put_num(&mut tail[n_i..], iterations);
    tail[n_i] = b' ';
    n_i += 1;
    n_i += put_num(&mut tail[n_i..], start_i);
    let low_t = spawn(&tail[..n_i], winapi::IDLE_PRIORITY_CLASS);
    let high_t = spawn(&tail[..n_i], winapi::HIGH_PRIORITY_CLASS);
    let low_t_ms = reap(low_t, 8000);
    let high_t_ms = reap(high_t, 8000);
    let i = match (low_t_ms, high_t_ms) {
        (Some(l), Some(h)) if l < EARLY_WAIT && h < EARLY_WAIT => {
            h.saturating_mul(4) < l.saturating_mul(3)
        }
        _ => false,
    };
    checks[9] = Check {
        name: NAMES[9],
        detail: if i {
            "HIGH surecin YENI akisi IDLE surecinkinden belirgin once bitirdi"
        } else if low_t_ms.is_none() || high_t_ms.is_none() {
            "isci cevap vermedi"
        } else if low_t_ms == Some(NO_THREAD) || high_t_ms == Some(NO_THREAD) {
            "CreateThread basarisiz (gorev tablosu dolu olabilir: I'ya bakin)"
        } else if matches!(low_t_ms, Some(v) if v & 0xFFFF_0000 == EARLY_WAIT)
            || matches!(high_t_ms, Some(v) if v & 0xFFFF_0000 == EARLY_WAIT)
        {
            "WaitForSingleObject akis BITMEDEN dondu"
        } else {
            "yeni akislar AYNI hizda: sinif yeni akisa uygulanmiyor"
        },
        passed: i,
    };
    say(&checks[9]);

    let passed = checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(winapi::Console, "[winprio] sonuc: {}/10 gecti", passed);
    let _ = writeln!(
        winapi::Console,
        "[winprio] isci: {} yineleme  IDLE {:?} ms  HIGH {:?} ms  akis: IDLE {:?} ms  HIGH {:?} ms",
        iterations,
        low_ms,
        high_ms,
        low_t_ms,
        high_t_ms
    );

    show(&checks);
}

fn show(checks: &[Check; 10]) {
    let mut win = match Window::create("winprio -- iki katmanli oncelik", 250, 130, 480, 230) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, checks);
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 10]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Sinif surecte, duzey akista", ACCENT);
    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            400,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }
    win.text(6, h - 30, "etkin oncelik = sinif tabani + akis duzeyi", DIM);
    let passed = checks.iter().filter(|c| c.passed).count();
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
