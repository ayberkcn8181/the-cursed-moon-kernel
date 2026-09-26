//! `heap` -- cekirdek heap'i geri aliniyor mu?
//!
//! `kmalloc` uzun sure bir **bump** ayiriciydi: isaretci ilerler, geri
//! donus yoktu. Gerekcesi de yaziliydi -- "tek tuketici scheduler'in
//! gorev yiginlari, onlar da yuvayla birlikte yeniden kullaniliyor".
//!
//! Gerekce bir dogruydu ve zamanla yanlis oldu. Iki tuketici daha geldi
//! ve ikisi de **her cagride yeniden** tahsis ediyordu:
//!
//! ```text
//!   pencere tamponu      width * height * 4 bayt, her pencere acilista
//!   surec cekirdek yigini  16 KiB, her Ring 3 baslatmada
//! ```
//!
//! Belirti yoktu. Bir pencere acip kapatmak 250 KiB'i kalici olarak
//! yiyordu, ama heap dolana kadar her sey calisiyordu -- ve dolunca da
//! hata "pencere acilamadi" diye gorunuyordu, "bellek bitti" diye degil.
//!
//! ## Sayac olmadan olculemezdi
//!
//! Bu sinav ancak cekirdek uc sayaci Ring 3'e actiktan sonra yazilabildi
//! (`kstat`):
//!
//! ```text
//!   heap_used          kullanilan bayt
//!   heap_largest_free  TEK PARCA halindeki en buyuk bos blok
//!   heap_blocks        blok sayisi (dolu + bos)
//! ```
//!
//! Ucu birden gerekiyor ve sebebi B ile C'de gorunuyor: yalnizca
//! `heap_used`a bakan bir sinav, **bloklari geri veren ama
//! birlestirmeyen** bir ayiriciyi gecirirdi. Kullanilan bayt geri
//! gelirdi, heap ise yavasca kullanilamaz parcalara bolunurdu --
//! sizintinin agir cekimi.
//!
//! ## Bes sinav
//!
//! ```text
//!   A  tampon geri geliyor -> pencere ac/kapat turlarindan sonra heap ayni
//!   B  bloklar birlesiyor  -> en buyuk bos blok da ayni
//!   C  blok sayisi ayni    -> heap parcalara bolunmedi
//!   D  surec sizdirmiyor   -> fork + execve turlari heap'i buyutmuyor
//!   E  buyuk tahsis olur   -> turlardan sonra 512 KiB'lik pencere aciliyor
//! ```
//!
//! Pencere yalnizca sahibi **cikinca** kapaniyor (kapatma cagrisi yok),
//! o yuzden her tur bir `fork` cocugu: cocuk pencereyi acar ve hemen
//! cikar, cekirdek tamponu cikista geri verir.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0014_1420;
const PANEL: u32 = 0x0022_2232;
const FG: u32 = 0x00E4_E4F0;
const DIM: u32 = 0x008C_8CA0;
const ACCENT: u32 = 0x0090_B0F0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Kac tur pencere acilip kapatilacak.
///
/// Alti tur, sizan bir cekirdekte 1,5 MiB demek: 4 MiB'lik heap'te
/// gorulmemesi imkansiz, ama tavana carpacak kadar da degil. Sinav
/// "tukendi mi" degil "geri geldi mi" diye soruyor.
const ROUNDS: usize = 6;
const W: usize = 320;
const H: usize = 200;
/// Bir turun sizmasi halinde kaybedilecek bayt.
const LEAK_PER_ROUND: usize = W * H * 4;

/// E sinavindaki buyuk pencere: 512x256x4 = 512 KiB tek parca.
const BIG_W: usize = 512;
const BIG_H: usize = 256;

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

/// Bir tur: cocuk pencere acar ve cikar.
///
/// Cocuk hicbir kosulda geri donmemeli -- donerse ebeveynin kodunu
/// ikinci kez kosturur.
fn window_round() -> bool {
    match sys::fork() {
        0 => {
            // Pencere acilmasa bile cikmak zorundayiz.
            let _ = Window::open("heap turu", 60, 60, W, H);
            sys::exit(0);
        }
        id if id > 0 => {
            let mut status = 0u32;
            sys::waitpid(id as usize, &mut status, 0) >= 0
        }
        _ => false,
    }
}

/// Bir tur: cocuk kendini baska bir imajla degistirir ve cikar.
///
/// `execve` yolu ayri sinaniyor cunku sizinti orada baska bir yerdeydi:
/// her Ring 3 baslatmasi kendine yeni bir cekirdek yigini ayiriyordu.
fn exec_round() -> bool {
    match sys::fork() {
        0 => {
            sys::execve("/bin/hello");
            // Buraya ulasilirsa `execve` basarisiz olmustur.
            sys::exit(1);
        }
        id if id > 0 => {
            let mut status = 0u32;
            sys::waitpid(id as usize, &mut status, 0) >= 0
        }
        _ => false,
    }
}

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;
    let mut checks = [EMPTY; 5];

    // Temel cizgi: hicbir sey yapilmadan once.
    let used0 = sys::heap_used();
    let largest0 = sys::heap_largest_free();
    let blocks0 = sys::heap_blocks();

    // --- A/B/C: pencere ac-kapat turlari ---
    let mut rounds_ok = 0usize;
    for _ in 0..ROUNDS {
        if window_round() {
            rounds_ok += 1;
        }
    }
    let used1 = sys::heap_used();
    let largest1 = sys::heap_largest_free();
    let blocks1 = sys::heap_blocks();

    let a = rounds_ok == ROUNDS && used1 == used0;
    checks[0] = Check {
        name: "A tampon geri geliyor",
        detail: if rounds_ok != ROUNDS {
            "turlar tamamlanamadi"
        } else if used1 > used0 + LEAK_PER_ROUND {
            "her tur bir tampon SIZDIRDI"
        } else if used1 != used0 {
            "heap kullanimi degisti"
        } else {
            "alti tur sonrasi heap ayni"
        },
        passed: a,
    };

    // B: sayilar geri gelmis olabilir ama bloklar birlesmemis olabilir.
    let b = largest1 == largest0;
    checks[1] = Check {
        name: "B bloklar birlesiyor",
        detail: if b {
            "en buyuk bos blok da ayni"
        } else if largest1 < largest0 {
            "en buyuk blok KUCULDU (birlesme yok)"
        } else {
            "en buyuk blok buyudu"
        },
        passed: b,
    };

    let c = blocks1 == blocks0;
    checks[2] = Check {
        name: "C blok sayisi ayni",
        detail: if c {
            "heap parcalara bolunmedi"
        } else if blocks1 > blocks0 {
            "heap parcalandi"
        } else {
            "blok sayisi azaldi"
        },
        passed: c,
    };

    // --- D: execve turlari ---
    let used2 = sys::heap_used();
    let mut exec_ok = 0usize;
    for _ in 0..ROUNDS {
        if exec_round() {
            exec_ok += 1;
        }
    }
    let used3 = sys::heap_used();
    let d = exec_ok == ROUNDS && used3 == used2;
    checks[3] = Check {
        name: "D surec sizdirmiyor",
        detail: if exec_ok != ROUNDS {
            "execve turlari tamamlanamadi"
        } else if used3 > used2 {
            "her baslatma cekirdek yigini SIZDIRDI"
        } else {
            "alti execve sonrasi heap ayni"
        },
        passed: d,
    };

    // --- E: buyuk tahsis hala mumkun ---
    //
    // A-D sayilarla olcuyor; bu, ayni seyi **sonucla** olcuyor. Heap
    // kullanilamaz parcalara bolunmus olsaydi sayilar yine geri gelmis
    // gorunebilirdi ama 512 KiB'lik tek parca bulunamazdi.
    let e = match sys::fork() {
        0 => {
            let opened = Window::open("heap buyuk", 40, 40, BIG_W, BIG_H).is_some();
            sys::exit(if opened { 0 } else { 1 });
        }
        id if id > 0 => {
            let mut status = 0u32;
            // `exited` olmadan: sinyalle olen bir cocuk da "kod 0" gibi
            // gorunur, cunku cikis kodu durum kelimesinin ust baytinda
            // duruyor ve sinyalle olumde orasi sifirdir.
            sys::waitpid(id as usize, &mut status, 0) >= 0
                && sys::exited(status)
                && sys::exit_status(status) == 0
        }
        _ => false,
    };
    checks[4] = Check {
        name: "E buyuk tahsis olur",
        detail: if e {
            "512 KiB'lik pencere aciliyor"
        } else {
            "buyuk tampon AYRILAMADI"
        },
        passed: e,
    };

    for check in &checks {
        let _ = writeln!(
            out,
            "[heap] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let _ = writeln!(
        out,
        "[heap] kullanilan: {} -> {} bayt, en buyuk bos: {} -> {} bayt, blok: {} -> {}",
        used0, used1, largest0, largest1, blocks0, blocks1
    );

    let mut win = match Window::open("heap -- geri alinan bellek", 250, 150, 470, 190) {
        Some(w) => w,
        None => return,
    };
    let delta = used1.saturating_sub(used0);
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks, delta);
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 5], delta: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "bump degil: kfree + birlestirme", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            370,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, "alti turda buyume (bayt):", DIM);
    win.number(215, h - 30, delta, FG);
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
