//! `stackgrow` -- koruma sayfasi artik bir duvar degil, **hareketli bir
//! sinir**.
//!
//! Bir onceki bati (`altstack`) yigin tasmasini **gorunur** kildi: yigin
//! ile program break arasina Ring 3'e kapali bir sayfa konuldu ve tasma
//! oraya dokununca sayfa hatasi olustu. Ama o sayfa bir **duvardi** --
//! tasan program oluyordu, yalnizca bu kez tanisiyla birlikte.
//!
//! Gercek sistemlerde yigin **buyur**. Duvara dokunmak bir son degil,
//! bir **istek**:
//!
//! ```text
//!   once:   [ brk ... ][ DUVAR ][ yigin ]
//!   sonra:  [ brk ... ][ DUVAR ][ yigin + 1 sayfa ]
//!                       ^ bir sayfa asagi kaydi
//! ```
//!
//! ## Iki yandan sinirli
//!
//! ```text
//!   yukaridan  STACK_MAX      -- bir surec sinirsiz yigin alamaz
//!   asagidan   program break  -- heap'in ustune buyuyemez
//! ```
//!
//! Ikisi de gercek sistemlerde var. Linux'ta yigin asagi, heap yukari
//! buyur ve aralarinda bir **bosluk** olmak zorundadir; bosluk bitince
//! buyume durur.
//!
//! ## Iki ABI, iki sozlesme
//!
//! Buyumenin mekanizmasi iki yuzde de ayni, ama sozlesmesi degil:
//!
//! ```text
//!   POSIX    cekirdek SESSIZCE buyutur; sinirda SIGSEGV gelir.
//!   Windows  ilk dokunusta STATUS_GUARD_PAGE_VIOLATION atilir --
//!            program onu GORUR ve korumayi kendi yeniden kurabilir.
//! ```
//!
//! TCMK su an POSIX'in sessiz bicimini uyguluyor.
//!
//! ## Yedi sinav
//!
//! ```text
//!   A  yigin BUYUDU      -> derin ozyineleme sonrasi olcu artti
//!   B  sayfa sayfa       -> artis sayfa katlarinda, tek seferde degil
//!   C  duvar asagi indi  -> koruma sayfasinin adresi dustu
//!   D  hala bir duvar var-> brk tavani da duvarla birlikte indi
//!   E  SINIRDA SIGSEGV   -> sonsuz ozyineleme TAM STACK_MAX'te duruyor
//!   F  si_addr duvarda   -> hata adresi yiginin hemen altinda
//!   G  cocuk da buyuyor  -> fork sonrasi cocugun yigini da buyuyebiliyor
//! ```
//!
//! E bu sinavin en onemli yarisi ve kolayca kaybedilecek olani: buyume
//! eklenirken `altstack` batisinin kazandigi sey -- tasmanin **gorunur**
//! olmasi -- elden gitmemeli. Sinirsiz buyuyen bir yigin, sonsuz
//! ozyinelemeyi bir hata olmaktan cikarip butun sistemi tuketen bir
//! olaya cevirirdi.
//!
//! E'nin ilk yazilisti zayifti ve bunu bozma sinavi gosterdi: cekirdekten
//! `STACK_MAX` denetimi kaldirildiginda sinav yine 7/7 verdi. Sebebi,
//! buyumenin **iki** sinir olmasi -- tavan gidince asagidaki heap
//! carpismasi ozyinelemeyi durdurdu ve E "yakalandi" dedi. Yani E "bir
//! sinir var" olcuyordu, "tavan var" degil. Simdi `STACK_MAX`'i
//! cekirdekten okuyup yiginin **tam nerede** durdugunu soruyor.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::signal::{self, AltStack, SigInfo, UContext};
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0012_1A14;
const PANEL: u32 = 0x001E_2A22;
const FG: u32 = 0x00E2_EEE6;
const DIM: u32 = 0x0086_9C90;
const ACCENT: u32 = 0x0070_E0A0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

const PAGE: usize = 4096;

/// Ayri sinyal yigini: sinirdaki `SIGSEGV`i yakalamanin tek yolu.
#[repr(align(16))]
struct AltArea(#[allow(dead_code)] [u8; ALT_SIZE]);
const ALT_SIZE: usize = 8 * 1024;
static mut ALT_AREA: AltArea = AltArea([0; ALT_SIZE]);

/// Ne kadar derine inildi (ozyineleme sayaci).
static DEPTH: AtomicUsize = AtomicUsize::new(0);

/// E sinavinin isleyicisi: tasma yakalandi, ama **nerede**.
///
/// Ilk yazilista bu isleyici yalnizca `CHILD_CAUGHT` donuyordu ve sinav
/// "sinirsiz ozyineleme sinirda yakalandi" diyordu. Sonra cekirdekten
/// `STACK_MAX` denetimi **kaldirildi** ve sinav yine 7/7 verdi: cunku
/// buyumenin iki sinir var ve asagidaki (heap carpismasi) tavan olmadan
/// da ozyinelemeyi durduruyor. Yani olculen sey "bir sinir var"di --
/// "tavan var" degil.
///
/// Olcu artik tam: tavan calisiyorsa yigin **tam** `STACK_MAX`'te durur.
/// Tavan yoksa heap'e kadar buyur ve olcu cok daha buyuk cikar.
extern "C" fn on_overflow(_signo: u32, _info: *const SigInfo, _ctx: *mut UContext) {
    let size = sys::stack_size();
    let max = sys::stack_max();
    // Tasmayi yakalayan bir isleyici kaldigi yerden devam **edemez**:
    // donusteki ilk komut ayni yigin isaretcisiyle yine tasar. Degeri
    // kurtarmak degil, **raporlamak**.
    sys::exit(if max == 0 {
        CHILD_FLAT
    } else if size == 0 {
        CHILD_NOREC
    } else if size == max {
        CHILD_CAUGHT
    } else if size > max {
        CHILD_PAST
    } else {
        CHILD_SHORT
    } as i32);
}

/// Yigini belirli bir dereceye kadar tuketen ozyineleme.
///
/// `black_box` sart: LLVM'in biriktirici kuyruk cagri eliminasyonu
/// `f(n+1) + c` bicimli ozyinelemeyi sessizce bir donguye ceviriyor ve
/// o zaman yigin **hic** tuketilmiyor. Ayni tuzak `altstack` batisinda
/// da iki kez yakalanmisti.
#[inline(never)]
fn devour(depth: usize, limit: usize) -> usize {
    let mut pad = [0u8; 512];
    pad[0] = depth as u8;
    pad[511] = depth as u8;
    DEPTH.store(depth, Ordering::SeqCst);
    core::hint::black_box(&pad);
    if depth >= limit {
        return core::hint::black_box(pad[0] as usize);
    }
    let deeper = devour(depth + 1, limit);
    core::hint::black_box(pad[511] as usize) + deeper
}

/// Cocugun "tasmayi yakaladim" cevabi.
const CHILD_CAUGHT: u32 = 0x61;
/// Cocugun "yigin buyudu" cevabi (G sinavi).
const CHILD_GREW: u32 = 0x62;
/// Cocugun "yigin buyumedi" cevabi.
const CHILD_FLAT: u32 = 0x63;
/// Cocugun "tasma yakalandi ama tavanin **otesinde**" cevabi.
///
/// Bu kodun varlik sebebi bir hata: tavan kaldirildiginda sinav yine
/// gecmisti, cunku asagidaki sinir (heap carpismasi) ozyinelemeyi
/// durduruyordu. Bu kod o iki sinirin ayirt edilmesini sagliyor.
const CHILD_PAST: u32 = 0x64;
/// Cocugun "tasma tavandan **once** durdu" cevabi.
///
/// Yerlesim hatasi: yigin ile heap arasinda `STACK_MAX` kadar yer yok,
/// yani tavan hic sinanamiyor.
const CHILD_SHORT: u32 = 0x65;
/// Cocugun "yigin kaydim **yok**" cevabi.
///
/// Ayri bir kod olmasinin sebebi bir bozma sinavi: `fork` yigin
/// yerlesimini devretmeyi birakinca cocuk sifir olcu goruyordu ve sinav
/// "yerlesimde yer yok" diyordu -- dogru sonuc, yanlis teshis. Kaydin
/// hic olmamasi, yerin yetmemesinden baska bir ariza.
const CHILD_NOREC: u32 = 0x66;

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

const NAMES: [&str; 7] = [
    "A yigin BUYUDU",
    "B sayfa sayfa",
    "C duvar asagi indi",
    "D brk tavani da indi",
    "E SINIRDA SIGSEGV",
    "F si_addr duvarda",
    "G cocuk da buyuyor",
];

fn say(check: &Check) {
    use core::fmt::Write;
    let mut console = Stdout;
    let _ = writeln!(
        console,
        "[stackgrow] {}: {} ({})",
        check.name,
        if check.passed { "gecti" } else { "KALDI" },
        check.detail
    );
}

/// Cocugu **sinirli** sure bekler; donmezse 0 doner.
fn reap_bounded(child: isize) -> u32 {
    if child <= 0 {
        return 0;
    }
    let mut status = 0u32;
    for _ in 0..60 {
        if sys::waitpid(child as usize, &mut status, sys::WNOHANG) > 0 {
            return if sys::exited(status) {
                sys::exit_status(status)
            } else {
                // Sinyalle oldu: cikis kodu degil olum sebebi var.
                0xDEAD
            };
        }
        sys::sleep_ms(50);
    }
    0
}

fn main() {
    use core::fmt::Write;
    let _ = writeln!(Stdout, "[stackgrow] sinav basliyor");
    let mut checks = [EMPTY; 7];

    let size0 = sys::stack_size();
    let guard0 = sys::stack_guard();
    let grown0 = sys::stack_grown();
    let brk0 = sys::brk(0);

    // --- A, B, C, D: olculu bir derinlige inmek ----------------------
    //
    // Her cerceve ~512 bayt; 64 kademe en az 32 KiB, yani baslangictaki
    // 16 KiB'lik yigini birkac sayfa asiyor. Buyume olmasaydi burada
    // olunurdu -- `altstack` batisinin davranisi tam olarak buydu.
    let sum = devour(0, 64);
    core::hint::black_box(sum);

    let size1 = sys::stack_size();
    let guard1 = sys::stack_guard();
    let grown1 = sys::stack_grown();

    let a_ok = size1 > size0;
    checks[0] = Check {
        name: NAMES[0],
        detail: if a_ok {
            "derin ozyineleme sonrasi yigin buyudu"
        } else if size0 == 0 {
            "yigin olcusu okunamadi"
        } else {
            "yigin BUYUMEDI: duvar hala sabit"
        },
        passed: a_ok,
    };
    say(&checks[0]);

    // B: artis sayfa katlarinda mi, ve eklenen sayfa sayisiyla tutarli mi.
    //
    // "Buyudu" tek basina yetmez: tek hamlede sinirsiz yer veren bir
    // cekirdek de buyurdu. Olculen sey artisin **sayfa sayfa** oldugu.
    let delta = size1 - size0;
    let pages = grown1 - grown0;
    let b_ok = a_ok && delta % PAGE == 0 && pages > 0 && delta == pages * PAGE;
    checks[1] = Check {
        name: NAMES[1],
        detail: if b_ok {
            "artis sayfa katlarinda ve sayacla tutarli"
        } else if !a_ok {
            "yigin buyumedi, olculemedi"
        } else if delta % PAGE != 0 {
            "artis sayfa kati degil"
        } else {
            "artis eklenen sayfa sayisiyla uyusmuyor"
        },
        passed: b_ok,
    };
    say(&checks[1]);

    // C: duvarin kendisi asagi indi mi.
    let c_ok = a_ok && guard1 < guard0 && guard0 - guard1 == delta;
    checks[2] = Check {
        name: NAMES[2],
        detail: if c_ok {
            "koruma sayfasi buyume kadar asagi indi"
        } else if guard1 >= guard0 {
            "duvar yerinde kaldi: sayfa yigina katilmadi"
        } else {
            "duvarin indigi mesafe buyume ile uyusmuyor"
        },
        passed: c_ok,
    };
    say(&checks[2]);

    // D: brk'nin **tavani** da indi mi.
    //
    // Kolayca atlanabilecek olani. Tavan birakilsaydi `brk` artik yigina
    // ait olan bir adrese kadar buyuyebilir ve iki bolge sessizce
    // birbirinin verisini ezerdi. Olcu: brk'yi eski duvara kadar
    // buyutmeyi denemek -- reddedilmeli.
    let brk_try = sys::brk(guard0);
    let d_ok = a_ok && brk_try != guard0 && brk_try == brk0;
    checks[3] = Check {
        name: NAMES[3],
        detail: if d_ok {
            "brk eski duvara kadar buyuyemedi"
        } else if !a_ok {
            "yigin buyumedi, olculemedi"
        } else {
            "brk yigina ait bir adrese kadar BUYUDU"
        },
        passed: d_ok,
    };
    say(&checks[3]);

    // --- E ve F: sinirda hala SIGSEGV ---------------------------------
    //
    // Olcum cocukta: tasmayi yakalayan isleyici kaldigi yerden devam
    // edemez, yani surec her halukarda biter.
    //
    // E yalnizca "yakalandi mi" diye sormuyor; **nerede** yakalandigini
    // soruyor ve sebebi somut: tavan ile heap carpismasi iki ayri sinir,
    // ve biri digerinin yoklugunu gizleyebiliyor (bkz. modul basligi).
    let stack_max = sys::stack_max();
    let child = sys::fork();
    if child == 0 {
        let area = AltStack {
            sp: core::ptr::addr_of!(ALT_AREA) as usize,
            flags: 0,
            size: ALT_SIZE,
        };
        signal::sigaltstack(Some(&area), None);
        signal::action_info(signal::SIGSEGV, on_overflow, signal::SA_ONSTACK, 0);
        // Sinirsiz derinlik: yigin STACK_MAX'e dayanana kadar buyur,
        // sonra duvar yerinde kalir ve tasma yakalanir.
        let eaten = devour(0, usize::MAX);
        core::hint::black_box(eaten);
        sys::exit(0);
    }
    let e_code = reap_bounded(child);
    let e_ok = e_code == CHILD_CAUGHT;
    checks[4] = Check {
        name: NAMES[4],
        detail: if child < 0 {
            "cocuk surec acilamadi"
        } else if e_ok {
            "ozyineleme TAM STACK_MAX'te durdu"
        } else if e_code == CHILD_PAST {
            "yigin tavani ASTI: sinir STACK_MAX degil, heap carpismasi"
        } else if e_code == CHILD_SHORT {
            "tavandan once duruldu: yerlesimde STACK_MAX kadar yer yok"
        } else if e_code == CHILD_NOREC {
            "cocugun yigin kaydi YOK: yerlesim devredilmemis"
        } else if e_code == CHILD_FLAT {
            "cekirdek tavani bildirmedi"
        } else if e_code == 0 {
            "cocuk cevap vermedi: yigin SINIRSIZ buyuyor"
        } else if e_code == 0xDEAD {
            "cocuk sinyalle oldu: isleyici kosamadi"
        } else {
            "cocuk beklenmedik bir kodla cikti"
        },
        passed: e_ok,
    };
    say(&checks[4]);

    // F ayri bir cocuk istiyor ve sebebi somut: hata **cocukta**
    // olustu, yani adresi de orada kaldi. Ebeveyn onu okuyamaz.
    //
    // Olcuyu cocuk kendisi yapiyor: yakaladigi adres koruma sayfasinin
    // **icinde** mi. Disinda olsaydi tasma duvari asmis, once .bss'i
    // ezmis ve cok daha asagida patlamis olurdu -- yani E gecerken F
    // kalabilir, ve ikisi gercekten ayri seyler olcuyor.
    let probe = sys::fork();
    if probe == 0 {
        let area = AltStack {
            sp: core::ptr::addr_of!(ALT_AREA) as usize,
            flags: 0,
            size: ALT_SIZE,
        };
        signal::sigaltstack(Some(&area), None);
        signal::action_info(signal::SIGSEGV, on_near_guard, signal::SA_ONSTACK, 0);
        let eaten = devour(0, usize::MAX);
        core::hint::black_box(eaten);
        sys::exit(0);
    }
    let f_code = reap_bounded(probe);
    let f_ok = f_code == CHILD_CAUGHT;
    checks[5] = Check {
        name: NAMES[5],
        detail: if probe < 0 {
            "cocuk surec acilamadi"
        } else if f_ok {
            "hata adresi koruma sayfasinin icinde"
        } else if f_code == CHILD_FLAT {
            "hata adresi duvarin disinda: tasma duvari asti"
        } else if f_code == CHILD_NOREC {
            "cocugun duvar kaydi YOK: yerlesim devredilmemis"
        } else if f_code == 0 {
            "cocuk cevap vermedi"
        } else {
            "cocuk beklenmedik bir kodla cikti"
        },
        passed: f_ok,
    };
    say(&checks[5]);

    // --- G: cocugun yigini da buyuyebiliyor mu ------------------------
    //
    // `fork` adres uzayini kopyaliyor ama yigin yerlesimi **ayri bir
    // kayit**: devredilmezse cocugun yigini hic buyumez ve bu yalnizca
    // derin ozyineleme yapan bir cocukta ortaya cikar.
    let heir = sys::fork();
    if heir == 0 {
        let before = sys::stack_size();
        // Derinlik ebeveynin **buyuttugu** yigindan daha fazlasini
        // istemeli. Ilk yazilista 48 kademe vardi ve cocuk miras
        // aldigi 36 KiB'ye rahatca siginca sinav "buyumedi" diyordu --
        // oysa olculen sey buyumenin devredilip devredilmedigiydi,
        // cocugun ne kadar yer kullandigi degil.
        let eaten = devour(0, 160);
        core::hint::black_box(eaten);
        let after = sys::stack_size();
        sys::exit(if after > before { CHILD_GREW } else { CHILD_FLAT } as i32);
    }
    let g_code = reap_bounded(heir);
    let g_ok = g_code == CHILD_GREW;
    checks[6] = Check {
        name: NAMES[6],
        detail: if heir < 0 {
            "cocuk surec acilamadi"
        } else if g_ok {
            "fork cocugunun yigini da buyudu"
        } else if g_code == CHILD_FLAT {
            "cocugun yigini BUYUMEDI: yerlesim devredilmemis"
        } else if g_code == 0xDEAD {
            "cocuk coktu: buyume yok"
        } else {
            "cocuk cevap vermedi"
        },
        passed: g_ok,
    };
    say(&checks[6]);

    let passed = checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(Stdout, "[stackgrow] sonuc: {}/7 gecti", passed);
    let _ = writeln!(
        Stdout,
        "[stackgrow] yigin {} -> {} bayt  duvar 0x{:x} -> 0x{:x}  eklenen sayfa {}  tavan {} KiB",
        size0,
        size1,
        guard0,
        guard1,
        pages,
        stack_max / 1024
    );
    show(&checks, size0, size1);
}

/// F sinavinin isleyicisi: hata adresi koruma sayfasinin **icinde** mi.
///
/// Duvar bir sayfa genis. Tasma onu asip daha asagida patlasaydi adres
/// o sayfanin disinda olurdu -- ve o, korumanin delindigi anlamina
/// gelirdi.
extern "C" fn on_near_guard(_signo: u32, info: *const SigInfo, _ctx: *mut UContext) {
    // SAFETY: cekirdek gecerli bir kayit verir.
    let addr = unsafe { (*info).addr() };
    let guard = sys::stack_guard();
    // Kaydin hic olmamasi (guard == 0) ile adresin duvar disinda olmasi
    // ayri arizalar; ikisine ayni kodu dondurmek dogru sonucu yanlis
    // teshisle vermek olurdu.
    sys::exit(if guard == 0 {
        CHILD_NOREC
    } else if addr >= guard && addr < guard + PAGE {
        CHILD_CAUGHT
    } else {
        CHILD_FLAT
    } as i32);
}

fn show(checks: &[Check; 7], size0: usize, size1: usize) {
    let mut win = match Window::open("stackgrow -- duvar asagi iniyor", 260, 130, 490, 250) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, checks, size0, size1);
        win.frame(60);
    }
}

fn draw(win: &mut Window, checks: &[Check; 7], size0: usize, size1: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Duvara dokunmak bir son degil, bir istek", ACCENT);

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

    win.text(6, h - 46, "yigin:", DIM);
    win.number(60, h - 46, size0 / 1024, FG);
    win.text(90, h - 46, "KiB ->", DIM);
    win.number(150, h - 46, size1 / 1024, ACCENT);
    win.text(180, h - 46, "KiB", DIM);
    win.text(6, h - 30, "derinlik:", DIM);
    win.number(80, h - 30, DEPTH.load(Ordering::SeqCst), FG);

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
