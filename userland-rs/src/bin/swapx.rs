//! `swapx` -- sayfa diske gidiyor ve geri geliyor mu?
//!
//! Cerceve havuzu 16 MiB ve uzun sure tukenmesinin tek cevabi
//! **reddetmekti**: `frames::alloc` `None` doner, `fork`/`execve`
//! basarisiz olur, talep uzerine sayfalama hatayi normal yoluna
//! birakirdi. Yani sistem, aylardir dokunulmamis sayfalar yuzunden yeni
//! bir surec acamayabilirdi.
//!
//! Takas o cevabi degistiriyor:
//!
//! ```text
//!   sayfa diske yazilir  ->  cerceve havuza doner
//!   PTE: PRESENT=0, PTE_SWAP=1, ust bitler = yuva numarasi
//!   sayfaya dokunulunca  ->  sayfa hatasi -> yuva geri okunur
//! ```
//!
//! ## Neden bir cagriyla tetikleniyor
//!
//! Gercek bir cekirdekte takas bellek baskisiyla kendiliginden olur.
//! Burada `swap_out` diye bir cagri var (`0x50D`) ve POSIX'te karsiligi
//! yok -- olmamasi da dogal. Var olma sebebi olcum: surec penceresi
//! 512 KiB, havuz 16 MiB, yani bir sinav programinin baskiyi
//! **belirlenimci** bicimde uretmesi mumkun degil. Tetigi disari acmak,
//! mekanizmanin kendisini (yaz, birak, hatada geri oku) sinamayi
//! mumkun kiliyor.
//!
//! Baski yolu ayrica var ve kodda duruyor: `handle_demand_fault`
//! icinde `frames::alloc` basarisiz olunca ayni adres uzayindan bir
//! sayfa diske atiliyor.
//!
//! ## Alti sinav
//!
//! ```text
//!   A  yuva var        -> bicimlendirme takas alani ayirdi
//!   B  sayfa atildi    -> istenen sayida sayfa diske gitti
//!   C  icerik ayni     -> geri okunan sayfalarda desen BOZULMADI
//!   D  gercekten disk  -> cekirdegin "iceri" sayaci artti
//!   E  yuva geri verildi-> geri okunan sayfanin yuvasi serbest kaldi
//!   F  fork da gorur   -> diskteki sayfa cocukta dogru geliyor
//! ```
//!
//! C ile D birlikte duruyor ve ayri olmalari sart. C tek basina, hicbir
//! sey yapmayan bir `swap_out` ile de gecerdi: sayfa zaten bellekteydi,
//! desen elbette dogru. D, cekirdegin sayfayi **gercekten** diskten
//! okudugunu soyluyor.
//!
//! F en ince olani: `fork` diskteki bir sayfayla karsilastiginda onu
//! once geri okumak zorunda. Okumasaydi cocuk bos ya da yanlis bir
//! sayfa gorurdu -- ve bu, ancak cocugun icerigi denetlenirse ortaya
//! cikardi.
//!
//! Disk yoksa takas da yok; o zaman "gecti" degil **"atlandi"**.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0018_1220;
const PANEL: u32 = 0x0028_2034;
const FG: u32 = 0x00E8_E0F0;
const DIM: u32 = 0x0094_88A4;
const ACCENT: u32 = 0x00C0_A0F0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;
const SKIP: u32 = 0x00C0_B060;

const PAGE: usize = 4096;
/// Kac sayfa ayrilip diske atilacak.
const PAGES: usize = 8;
const REGION: usize = PAGES * PAGE;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Copy)]
struct Check {
    name: &'static str,
    detail: &'static str,
    verdict: Verdict,
}

const EMPTY: Check = Check {
    name: "",
    detail: "",
    verdict: Verdict::Failed,
};

/// Konuma bagli desen.
///
/// Hem sayfa icinde hem sayfalar arasinda degisiyor: sabit bir bayt
/// kullanilsaydi yanlis yuvadan okunan sayfa da dogru gorunurdu.
fn pattern(at: usize) -> u8 {
    ((at >> 9) ^ at ^ 0xA5) as u8
}

unsafe fn fill(base: *mut u8) {
    for i in 0..REGION {
        base.add(i).write_volatile(pattern(i));
    }
}

unsafe fn verify(base: *mut u8) -> bool {
    for i in 0..REGION {
        if base.add(i).read_volatile() != pattern(i) {
            return false;
        }
    }
    true
}

fn main() {
    let mut out = Stdout;
    let mut checks = [EMPTY; 6];
    let names = [
        "A yuva var",
        "B sayfa atildi",
        "C icerik ayni",
        "D gercekten disk",
        "E yuva geri verildi",
        "F fork da gorur",
    ];

    let slots = sys::swap_slots();
    if slots == 0 {
        for (i, name) in names.iter().enumerate() {
            checks[i] = Check {
                name,
                detail: "takas alani yok (disk bagli degil)",
                verdict: Verdict::Skipped,
            };
        }
        report(&mut out, &checks, 0, 0);
        show(&checks, 0);
        return;
    }

    checks[0] = Check {
        name: names[0],
        detail: "bicimlendirme takas alani ayirdi",
        verdict: Verdict::Passed,
    };

    let base = match sys::mmap(REGION) {
        Some(p) => p,
        None => {
            for (i, name) in names.iter().enumerate().skip(1) {
                checks[i] = Check {
                    name,
                    detail: "bellek ayrilamadi",
                    verdict: Verdict::Failed,
                };
            }
            report(&mut out, &checks, slots, 0);
            show(&checks, slots);
            return;
        }
    };

    unsafe { fill(base) };

    // --- B: diske at ---
    let out_before = sys::swap_pages_out();
    let used_before = sys::swap_used();
    let thrown = sys::swap_out(PAGES);
    let out_after = sys::swap_pages_out();
    let used_after = sys::swap_used();

    let b = thrown == PAGES && out_after == out_before + PAGES;
    checks[1] = Check {
        name: names[1],
        detail: if thrown == 0 {
            "hicbir sayfa atilamadi"
        } else if thrown != PAGES {
            "istenenden az sayfa atildi"
        } else if out_after != out_before + PAGES {
            "sayfa atildi ama sayac artmadi"
        } else {
            "sekiz sayfa diske gitti"
        },
        verdict: verdict(b),
    };

    // --- C + D: geri oku ---
    //
    // Okuma sayfa hatasi uretiyor; cekirdek yuvayi geri okuyup girdiyi
    // tazeliyor. Desen bozulmadan gelmeli.
    let in_before = sys::swap_pages_in();
    let intact = unsafe { verify(base) };
    let in_after = sys::swap_pages_in();

    let c = b && intact;
    checks[2] = Check {
        name: names[2],
        detail: if !b {
            "B gectikten sonra anlamli"
        } else if intact {
            "geri okunan sayfalarda desen ayni"
        } else {
            "icerik BOZULDU"
        },
        verdict: verdict(c),
    };

    let d = c && in_after >= in_before + PAGES;
    checks[3] = Check {
        name: names[3],
        detail: if !c {
            "C gectikten sonra anlamli"
        } else if in_after == in_before {
            "hic disk okumasi olmadi (sayfa hic gitmemis)"
        } else if in_after < in_before + PAGES {
            "beklenenden az sayfa geri okundu"
        } else {
            "sekiz sayfa diskten geldi"
        },
        verdict: verdict(d),
    };

    // --- E: yuvalar geri verildi ---
    let used_end = sys::swap_used();
    let e = d && used_after == used_before + PAGES && used_end == used_before;
    checks[4] = Check {
        name: names[4],
        detail: if !d {
            "D gectikten sonra anlamli"
        } else if used_after != used_before + PAGES {
            "atarken yuva sayisi beklendigi gibi artmadi"
        } else if used_end > used_before {
            "yuva geri verilmedi (takas alani SIZIYOR)"
        } else {
            "geri okunan sayfanin yuvasi serbest kaldi"
        },
        verdict: verdict(e),
    };

    // --- F: fork diskteki sayfayi gorur mu ---
    //
    // Sayfalar yeniden diske atiliyor, sonra `fork`. Cocuk icerigi
    // denetliyor ve sonucu cikis koduyla bildiriyor.
    let refilled = sys::swap_out(PAGES);
    let f = refilled == PAGES
        && match sys::fork() {
            0 => {
                let ok = unsafe { verify(base) };
                sys::exit(if ok { 0 } else { 1 });
            }
            id if id > 0 => {
                let mut status = 0u32;
                // `exited` denetimi sart ve bunu olcum ogretti: sinyalle
                // olen bir surecin durum kelimesinde `WEXITSTATUS`
                // **sifir** okunur (kod `(x & 0xFF) << 8` ile
                // paketleniyor, olum sinyali ise alt baytta). Yalnizca
                // cikis koduna bakan ilk hal, coken bir cocugu
                // "basariyla bitti" sayiyordu -- ve bilerek bozulmus
                // cekirdekte sinav tam bu yuzden yanlis "gecti" dedi.
                sys::waitpid(id as usize, &mut status, 0) >= 0
                    && sys::exited(status)
                    && sys::exit_status(status) == 0
            }
            _ => false,
        };
    checks[5] = Check {
        name: names[5],
        detail: if refilled != PAGES {
            "ikinci turda sayfa atilamadi"
        } else if f {
            "diskteki sayfa cocukta dogru geldi"
        } else {
            "cocuk YANLIS icerik gordu"
        },
        verdict: verdict(f),
    };

    sys::munmap(base, REGION);
    report(&mut out, &checks, slots, thrown);
    show(&checks, slots);
}

fn verdict(ok: bool) -> Verdict {
    if ok {
        Verdict::Passed
    } else {
        Verdict::Failed
    }
}

fn report(out: &mut Stdout, checks: &[Check; 6], slots: usize, thrown: usize) {
    use core::fmt::Write;
    for check in checks {
        let _ = writeln!(
            out,
            "[swapx] {}: {} ({})",
            check.name,
            match check.verdict {
                Verdict::Passed => "gecti",
                Verdict::Failed => "KALDI",
                Verdict::Skipped => "atlandi",
            },
            check.detail
        );
    }
    let _ = writeln!(
        out,
        "[swapx] yuva: {} ({} KiB), atilan: {}, disari: {}, iceri: {}",
        slots,
        slots * 4,
        thrown,
        sys::swap_pages_out(),
        sys::swap_pages_in()
    );
}

fn show(checks: &[Check; 6], slots: usize) {
    let mut win = match Window::open("swapx -- diske giden sayfa", 270, 175, 470, 190) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, checks, slots);
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 6], slots: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "cerceve bitince sayfa diske gider", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        let (word, color) = match check.verdict {
            Verdict::Passed => ("gecti", OK),
            Verdict::Failed => ("KALDI", WARN),
            Verdict::Skipped => ("atlandi", SKIP),
        };
        win.text(360, y, word, color);
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.verdict == Verdict::Passed).count();
    let skipped = checks.iter().filter(|c| c.verdict == Verdict::Skipped).count();
    win.text(6, h - 30, "takas alani (KiB):", DIM);
    win.number(170, h - 30, slots * 4, FG);
    win.text(
        6,
        h - 14,
        if skipped == checks.len() {
            "takas yok -- hepsi atlandi   q cik"
        } else if passed == checks.len() {
            "hepsi gecti   q cik"
        } else {
            "BIR SINAV KALDI   q cik"
        },
        if skipped == checks.len() {
            SKIP
        } else if passed == checks.len() {
            OK
        } else {
            WARN
        },
    );
}
