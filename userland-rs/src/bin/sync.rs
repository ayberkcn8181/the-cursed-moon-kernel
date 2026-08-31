//! `sync` -- `futex`: beklemenin ucuz yolu.
//!
//! Bir onceki bati iki akisi ayni bellege baktirdi. Ama gormek yetmez;
//! **sira** gerekir. Iki akis paylasilan bir sayaci ayni anda artirirsa
//! sonuc yanlis cikar, ve birinin otekini beklemesi gerekir.
//!
//! ## Neden `futex` diye bir sey var
//!
//! Beklemenin iki yolu var:
//!
//! ```text
//!   mesgul bekleme   while flag == 0 {}        -> CPU'yu yakar
//!   uyuyarak bekleme futex(flag, WAIT, 0)      -> siraya girer
//! ```
//!
//! Ikincisi cekirdege iniyor, yani pahali. `futex`in adindaki "fast"
//! tam da bunu cozuyor: **cekisme yoksa cekirdege hic inilmez**. Kilidi
//! kapmak tek bir atomik islem; cagri ancak kilit doluysa yapilir.
//! Sinav C bunu dogrudan olcuyor -- cekismesiz kilit alip birakmak
//! cekirdegin bekleme sayacini **artirmamali**.
//!
//! ## `pthread_join` diye bir sistem cagrisi yoktur
//!
//! Onceki bati "POSIX'te is parcacigini beklemenin yolu yok" diye
//! yazmisti. Dogru degildi -- yolu var, sadece adi baska. `clone`,
//! `CLONE_CHILD_CLEARTID` ile cagrilirsa cekirdek akis olurken verilen
//! adrese 0 yazip orada bekleyenleri uyandiracagina soz veriyor. Yani
//! "bitmesini bekle" ayri bir mekanizma degil; `futex`in ta kendisi.
//! Gercek glibc de tam olarak boyle yapiyor.
//!
//! ## Bes sinav
//!
//! ```text
//!   A  uyandirma      -> kardes uyandirana kadar UYUNUR, sonra devam
//!   B  zaman asimi    -> kimse uyandirmazsa ETIMEDOUT gelir
//!   C  cekismesiz     -> kilit bosken cekirdege HIC inilmez
//!   D  join           -> CLONE_CHILD_CLEARTID ile bitmesi beklenir
//!   E  sayac dogru    -> iki akis kilitle artirir, toplam TAM cikar
//! ```
//!
//! B bilerek burada: uyandirmayi olcmenin durust yolu, **uyandirma
//! olmayan** durumu da olcmek. Ikisi ayni cagriyi kullaniyor.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0012_1A18;
const PANEL: u32 = 0x001E_2C2A;
const FG: u32 = 0x00DE_ECE8;
const DIM: u32 = 0x0084_9C98;
const ACCENT: u32 = 0x0070_D8C0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// A sinavinin bayragi: kardes bunu 1 yapip uyandiracak.
static FLAG: AtomicU32 = AtomicU32::new(0);
/// Kardes gercekten uyandirma yapti mi?
static WOKE: AtomicUsize = AtomicUsize::new(0);

/// E sinavinin paylasilan sayaci ve onu koruyan kilit.
static COUNTER: AtomicUsize = AtomicUsize::new(0);
static LOCK: AtomicU32 = AtomicU32::new(0);

/// Her akisin artiracagi miktar. Tek tek artirmak sart: tek bir
/// `fetch_add` zaten bolunemez olurdu ve kilidi hic sinamazdi.
const ROUNDS: usize = 200;

/// D sinavinin `pthread_t` yuvasi -- cekirdek buraya 0 yazacak.
static mut JOIN_SLOT: u32 = 0;
static JOINED_MARK: AtomicUsize = AtomicUsize::new(0);

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

/// A sinavinin kardesi: biraz bekleyip bayragi kaldirir ve uyandirir.
///
/// Gecikme bilincli -- ana akisin gercekten **uyumus** olmasi gerekiyor,
/// yoksa sinav uyandirmayi degil sirayi olcerdi.
extern "C" fn waker(_param: usize) -> usize {
    sys::sleep_ms(120);
    FLAG.store(1, Ordering::SeqCst);
    let woken = unsafe { sys::futex_wake(FLAG.as_ptr() as *const u32, 1) };
    WOKE.store(if woken > 0 { woken as usize } else { 0 }, Ordering::SeqCst);
    0
}

/// Kilidi alir. Hizli yol cekirdege inmez.
///
/// Kilit uc degerli: 0 bos, 1 alinmis, 2 alinmis **ve bekleyen var**.
/// Ucuncu deger `futex`in klasik hilesi: birakan taraf, bekleyen
/// olmadigini biliyorsa uyandirma cagrisini hic yapmaz.
fn lock_acquire() {
    // Hizli yol: bos ise kap ve cik. Cekirdek yok.
    if LOCK
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        return;
    }
    loop {
        // Bekleyen oldugunu isaretle; birakan taraf bunu gorup
        // uyandirma yapacak.
        let previous = LOCK.swap(2, Ordering::Acquire);
        if previous == 0 {
            return;
        }
        unsafe { sys::futex_wait(LOCK.as_ptr() as *const u32, 2, 0) };
    }
}

/// Kilidi birakir; yalnizca bekleyen varsa cekirdege iner.
fn lock_release() {
    if LOCK.swap(0, Ordering::Release) == 2 {
        unsafe { sys::futex_wake(LOCK.as_ptr() as *const u32, 1) };
    }
}

/// E sinavinin kardesi: kilitle sayaci artirir.
extern "C" fn adder(_param: usize) -> usize {
    for _ in 0..ROUNDS {
        lock_acquire();
        let value = COUNTER.load(Ordering::Relaxed);
        // Okuma ile yazma arasinda bilerek bir bosluk: kilit
        // calismiyorsa iki akis burada birbirini ezer ve toplam eksik
        // cikar. Kilidi olcmenin tek yolu, onu zorlayacak bir aralik
        // birakmak.
        sys::yield_now();
        COUNTER.store(value + 1, Ordering::Relaxed);
        lock_release();
    }
    0
}

/// D sinavinin kardesi: kisa surer, tek isi bitmek.
extern "C" fn joinable(_param: usize) -> usize {
    sys::sleep_ms(80);
    JOINED_MARK.store(1, Ordering::SeqCst);
    0
}

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;
    let mut checks = [EMPTY; 5];

    // --- A: uyandirma ---
    let tid = sys::clone_thread(waker, 0, 0);
    let started = sys::address_waits();
    // Deger hala 0 iken uyu. Kardes 120 ms sonra kaldiracak.
    let result = unsafe { sys::futex_wait(FLAG.as_ptr() as *const u32, 0, 0) };
    let slept = sys::address_waits() > started;
    let a = tid > 0 && result == 0 && FLAG.load(Ordering::SeqCst) == 1 && slept;
    checks[0] = Check {
        name: "A uyandirma",
        detail: if tid <= 0 {
            "kardes yaratilamadi"
        } else if !slept {
            "hic uyunmadi (mesgul bekleme)"
        } else if result != 0 {
            "bekleme hatayla dondu"
        } else if FLAG.load(Ordering::SeqCst) != 1 {
            "uyandi ama bayrak kalkmamis"
        } else {
            "uyundu ve kardes uyandirdi"
        },
        passed: a,
    };

    // --- B: zaman asimi ---
    //
    // Ayni cagri, kimse uyandirmiyor. Uyandirmayi olcmenin durust yolu
    // budur: uyandirma **olmayan** durum da ayni satirlarla olculmeli.
    let idle = AtomicU32::new(7);
    let started = sys::ticks();
    let timed = unsafe { sys::futex_wait(idle.as_ptr() as *const u32, 7, 200) };
    let waited = sys::ticks().wrapping_sub(started);
    let b = timed == -ETIMEDOUT && waited >= 15;
    checks[1] = Check {
        name: "B zaman asimi",
        detail: if timed == 0 {
            "uyandirilmadigi halde uyandi"
        } else if timed != -ETIMEDOUT {
            "yanlis hata kodu"
        } else if waited < 15 {
            "sure dolmadan dondu"
        } else {
            "kimse uyandirmadi, ETIMEDOUT geldi"
        },
        passed: b,
    };

    // --- C: cekismesiz kilit cekirdege inmez ---
    //
    // `futex`in adindaki "fast" tam olarak bu. Kilit bosken alip
    // birakmak, cekirdegin bekleme sayacini artirmamali.
    let before_waits = sys::address_waits();
    let before_wakes = sys::address_wakes();
    for _ in 0..64 {
        lock_acquire();
        lock_release();
    }
    let c = sys::address_waits() == before_waits && sys::address_wakes() == before_wakes;
    checks[2] = Check {
        name: "C cekismesiz kilit",
        detail: if !c {
            "bos kilit icin cekirdege inildi"
        } else {
            "64 kilit/birakma, sifir sistem cagrisi"
        },
        passed: c,
    };

    // --- D: join ---
    //
    // Ayri bir cagri yok: cekirdegin `CLONE_CHILD_CLEARTID` sozu ile
    // `futex` yetiyor. `pthread_join`in tamami bu.
    let slot = core::ptr::addr_of_mut!(JOIN_SLOT);
    let joinable_tid = unsafe { sys::clone_joinable(joinable, 0, slot) };
    let joined = joinable_tid > 0 && unsafe { sys::join_thread(slot, 2000) };
    let d = joined && JOINED_MARK.load(Ordering::SeqCst) == 1 && unsafe { slot.read_volatile() } == 0;
    checks[3] = Check {
        name: "D join",
        detail: if joinable_tid <= 0 {
            "kardes yaratilamadi"
        } else if !joined {
            "bitmesi beklenemedi"
        } else if JOINED_MARK.load(Ordering::SeqCst) != 1 {
            "dondu ama kardes bitmemis"
        } else if unsafe { slot.read_volatile() } != 0 {
            "cekirdek yuvayi sifirlamadi"
        } else {
            "cekirdek yuvayi sifirladi, bekleyen uyandi"
        },
        passed: d,
    };

    // --- E: kilit gercekten koruyor mu ---
    let helper = sys::clone_thread(adder, 0, 0);
    adder(0);
    // Kardesin de bitmesini bekle: sayac ancak ikisi de bitince tamdir.
    let mut spins = 0;
    while COUNTER.load(Ordering::Relaxed) < 2 * ROUNDS && spins < 400 {
        sys::sleep_ms(5);
        spins += 1;
    }
    let total = COUNTER.load(Ordering::Relaxed);
    let e = helper > 0 && total == 2 * ROUNDS;
    checks[4] = Check {
        name: "E sayac dogru",
        detail: if helper <= 0 {
            "kardes yaratilamadi"
        } else if total < 2 * ROUNDS {
            "sayac EKSIK (kilit korumadi)"
        } else if total > 2 * ROUNDS {
            "sayac FAZLA"
        } else {
            "iki akis, 400 artirma, toplam tam"
        },
        passed: e,
    };

    for check in &checks {
        let _ = writeln!(
            out,
            "[sync] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }

    let mut win = match Window::open("sync -- futex", 290, 200, 460, 170) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks, total, sys::address_waits());
        win.flush();
    }
}

/// `futex` zaman asiminin errno'su.
const ETIMEDOUT: isize = 110;

fn draw(win: &mut Window, checks: &[Check; 5], total: usize, waits: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "cekisme yoksa cekirdek yok", ACCENT);

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
    win.text(6, h - 30, "sayac:", DIM);
    win.number(70, h - 30, total, FG);
    win.text(140, h - 30, "uyku:", DIM);
    win.number(200, h - 30, waits, FG);
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
