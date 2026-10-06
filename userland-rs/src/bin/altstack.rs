//! `altstack` -- yigin tasmasini yakalamak: koruma sayfasi + ayri yigin.
//!
//! Bir onceki bati POSIX yuzunun bir sayfa hatasini yakalayip
//! duzeltebilmesini getirdi. Geriye o ailenin **en zor** uyesi kalmisti:
//! yigin tasmasi. Zor olmasinin sebebi tuhaf bir kisir dongu --
//!
//! ```text
//!   yigin tasti  ->  SIGSEGV teslim edilecek
//!   teslim icin  ->  cekirdek kullanici YIGININA cerceve kurmali
//!   ama yigin    ->  tasmis; yazacak yer yok
//!   sonuc        ->  sinyal teslim edilemez, surec tanisiz oler
//! ```
//!
//! POSIX'in cozumu `sigaltstack`: isleyiciye **ayri** bir yigin ver, o
//! zaman kesilen yiginin durumu cerceveyi kurmaya engel olmaz.
//!
//! ## Once bir hata gorunur olmaliydi
//!
//! TCMK'de yigin tasmasi bu batiya kadar **hicbir hata uretmiyordu**.
//! Yigin asagi buyuyup program break bolgesine giriyor ve programin
//! kendi verisini sessizce eziyordu. Sessiz bozulma, coken bir surecten
//! cok daha kotudur: tanisi yoktur.
//!
//! Cozum bir **koruma sayfasi**: yigin ile brk arasinda, eslenmis ama
//! Ring 3'e kapali tek bir sayfa. Tasma oraya dokununca sayfa hatasi
//! olusuyor. Windows ayni isi ayni yolla yapar ve adi da odur
//! (`PAGE_GUARD`); ayrildiklari yer sonrasi:
//!
//! ```text
//!   Windows  koruma sayfasi -> yigin OTOMATIK buyur
//!   POSIX    koruma sayfasi -> SIGSEGV, isleyici ayri yiginda kosar
//! ```
//!
//! Windows'unki daha rahat, POSIX'inki daha acik: birinde program
//! tasmayi fark etmez bile, otekinde tasmayi **gorur** ve ne yapacagina
//! kendisi karar verir.
//!
//! ## Sonraki bati bu sayfayi hareketlendirdi
//!
//! `stackgrow` batisinda koruma sayfasi bir **duvar** olmaktan cikip
//! hareketli bir sinir oldu: ona dokunmak artik bir son degil, bir
//! istek, ve cekirdek yigina bir sayfa katip duvari bir asagi indiriyor.
//! Bu sinav yine gecerli, cunku buyumenin de bir tavani var
//! (`STACK_MAX`) ve orada duvar yerinde kaliyor -- tasma hala
//! **gorunur**. Degisen tek sey: tasma artik yiginin 16 KiB'lik ilk
//! olcusunde degil, 128 KiB'de duruyor. E sinavi bu yuzden duzeltildi
//! (bkz. `on_overflow`).
//!
//! ## Isleyici geri donemez
//!
//! Tasmayi yakalayan bir isleyici kaldigi yerden devam **edemez**:
//! donusteki ilk komut ayni yigin isaretcisiyle yine tasar. Gercek
//! programlar bu yuzden orada gunluge yazip cikar -- bu sinav da oyle
//! yapiyor. Yakalamanin degeri kurtarmak degil, **raporlamak**.
//!
//! ## Yedi sinav
//!
//! ```text
//!   A  kuruldu       -> sigaltstack kabul etti, geri okunan ayni
//!   B  ustunde kostu -> SA_ONSTACK isleyicisinin sp'si ALT yiginda
//!   C  SS_ONSTACK    -> isleyici icinde sorulunca "ustundeyim" diyor
//!   D  YIGIN TASMASI -> tasma yakalandi ve isleyici rapor edebildi
//!   E  koruma sayfasi-> tasma KORUMA SAYFASININ uzerinde durdu
//!   F  bayraksiz     -> SA_ONSTACK olmayan isleyici normal yiginda
//!   G  dar yigin     -> MINSIGSTKSZ altinda reddediliyor
//! ```
//!
//! D bu sinavin sebebi. E onun tamamlayicisi ve ayri bir sey olcuyor:
//! tasmanin **nerede** durdugu. "Surec oldu mu" diye sormak koruma
//! sayfasini olcmez -- koruma olmasa da surec olurdu, yalnizca once
//! .bss'i ezerek. Olcen soru, hata adresinin koruma sayfasinin
//! **icinde** olmasi.
//!
//! Ikisi de cocuk surecte kosuyor: tasmayi yakalayan bir isleyici
//! kaldigi yerden devam edemez, yani surec her halukarda biter.
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

const BG: u32 = 0x0012_1E1A;
const PANEL: u32 = 0x0020_2E2A;
const FG: u32 = 0x00E4_EEE8;
const DIM: u32 = 0x008C_9C96;
const ACCENT: u32 = 0x0080_E0C0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Ayri sinyal yigini. `.bss`te duruyor, yani imajla birlikte eslenmis
/// -- tasan yiginin durumundan bagimsiz.
///
/// Hizalama bilincli: cekirdek tepeyi 16'ya yuvarliyor, dizinin kendisi
/// de hizali olmazsa kullanilabilir alan sessizce kisalirdi.
#[repr(align(16))]
// Alana yalnizca ham adres uzerinden dokunuluyor (cekirdek yazar),
// bu yuzden derleyici "hic okunmadi" saniyor.
struct AltArea(#[allow(dead_code)] [u8; ALT_SIZE]);

const ALT_SIZE: usize = 8 * 1024;
static mut ALT_AREA: AltArea = AltArea([0; ALT_SIZE]);

/// Isleyicinin gordugu yigin isaretcisi.
static HANDLER_SP: AtomicUsize = AtomicUsize::new(0);
/// Isleyici icinde sorulan `ss_flags`.
static HANDLER_FLAGS: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Bayraksiz isleyicinin gordugu yigin isaretcisi.
static PLAIN_SP: AtomicUsize = AtomicUsize::new(0);

/// Cocugun cevabi cikis kodunda: taban "isleyici kostu", +1 "hata
/// adresi koruma sayfasinin icinde".
const CAUGHT_BASE: i32 = 70;

/// Sayfa olcusu -- koruma sayfasi bir sayfa genis.
const PAGE: usize = 4096;

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

const NAMES: [&str; 7] = [
    "A kuruldu",
    "B ustunde kostu",
    "C SS_ONSTACK",
    "D YIGIN TASMASI",
    "E koruma sayfasi",
    "F bayraksiz",
    "G dar yigin",
];

/// Ayri yiginda kosmasi beklenen isleyici.
extern "C" fn on_alt(_signo: u32, _info: *const SigInfo, context: *mut UContext) {
    // Olcu `ucontext_t`den degil, isleyicinin **kendi** yiginindan
    // aliniyor: `uc_mcontext` kesilen baglami tasir, yani oradaki `sp`
    // eski yigindir. Bizi ilgilendiren su an nerede oldugumuz.
    let here = 0u8;
    HANDLER_SP.store(&here as *const u8 as usize, Ordering::SeqCst);

    let mut current = AltStack::NONE;
    signal::sigaltstack(None, Some(&mut current));
    HANDLER_FLAGS.store(current.flags as usize, Ordering::SeqCst);
    let _ = context;
}

/// `SA_ONSTACK` **olmayan** isleyici -- normal yiginda kosmali.
extern "C" fn on_plain(_signo: u32, _info: *const SigInfo, _context: *mut UContext) {
    let here = 0u8;
    PLAIN_SP.store(&here as *const u8 as usize, Ordering::SeqCst);
}

/// Yigini tuketen ozyineleme.
///
/// `#[inline(never)]` ve yerel dizi bilincli: derleyici cagriyi acarsa
/// ya da cerceveyi sifirlarsa yigin hic tukenmez ve sinav olcmesi
/// gereken seyi kacirir. Dizi ayrica `read_volatile` ile okunuyor --
/// yoksa "kullanilmiyor" diye elenebilir.
#[inline(never)]
fn devour(depth: usize) -> usize {
    let mut block = [0u8; 512];
    // Cerceveyi optimize ediciden **gizle**. Gizlenmezse LLVM
    // ozyinelemeyi bir donguye ceviriyor (`f(n+1) + c` kalibinda
    // birikecli kuyruk cagrisi eliminasyonu) ve yigin hic tukenmiyor --
    // yani sinav olcmesi gereken seyi kaciriyor. Ilk yazilista tam
    // olarak bu oldu; i386'da cagridan sonra diziye dokunmak yetti,
    // x86_64'te yetmedi.
    let frame = core::hint::black_box(block.as_mut_ptr()) as usize;
    unsafe { core::ptr::write_volatile(block.as_mut_ptr(), depth as u8) };
    if depth > 1_000_000 {
        return frame;
    }
    let deeper = devour(core::hint::black_box(depth + 1));
    // Cagridan **sonra** yerel diziye dokunmak, cerceveyi cagri boyunca
    // canli tutuyor.
    let again = unsafe { core::ptr::read_volatile(block.as_ptr()) };
    core::hint::black_box(deeper.wrapping_add(frame).wrapping_add(again as usize))
}

/// Tasmayi yakalayan isleyici: kaldigi yerden devam **edemez**.
///
/// Donusteki ilk komut ayni yigin isaretcisiyle yine tasardi. Gercek
/// programlar da burada gunluge yazip cikar; yakalamanin degeri
/// kurtarmak degil, raporlamak.
extern "C" fn on_overflow(_signo: u32, info: *const SigInfo, _context: *mut UContext) {
    // Hata **adresi** koruma sayfasinin calisip calismadigini soyluyor.
    // Calisiyorsa tasma duvarin uzerinde durmus; calismiyorsa .bss'i
    // ezerek ilerlemis ve cok daha asagida patlamistir.
    //
    // Ilk yazilista olcu bir **vekildi**: hata adresi, tasmadan once
    // kaydedilen bir yigin adresinin 24 KiB altinda mi. Yigin 16 KiB'de
    // sabitken bu dogruydu. Yigin otomatik buyumeye baslayinca vekil
    // bozuldu -- tasma artik `STACK_MAX`'e kadar iniyor, yani duvara
    // tam ustunde carpmasina ragmen kayittan 100 KiB'den fazla uzakta.
    // Sinav o zaman "tasma .bss'i ezerek ILERLEDI" dedi: dogru bir
    // olcunun yanlis bir vekille verdigi yanlis cevap.
    //
    // Duvarin yeri artik Ring 3'ten okunabiliyor, yani vekile gerek yok.
    let addr = unsafe { (*info).addr() };
    let guard = sys::stack_guard();
    let near = guard != 0 && addr >= guard && addr < guard + PAGE;
    sys::exit(CAUGHT_BASE + i32::from(near));
}

fn main() {
    let mut checks = [EMPTY; 7];

    let alt_base = core::ptr::addr_of_mut!(ALT_AREA) as usize;
    let alt_top = alt_base + ALT_SIZE;

    // --- A: kurulum ve geri okuma ---
    let wanted = AltStack {
        sp: alt_base,
        flags: 0,
        size: ALT_SIZE,
    };
    let installed = signal::sigaltstack(Some(&wanted), None) == 0;
    let mut read_back = AltStack::NONE;
    signal::sigaltstack(None, Some(&mut read_back));
    let install_ok = installed && read_back.sp == alt_base && read_back.size == ALT_SIZE;
    checks[0] = Check {
        name: NAMES[0],
        detail: if !installed {
            "sigaltstack cagrisi REDDEDILDI"
        } else if install_ok {
            "kuruldu ve geri okunan ayni"
        } else {
            "kuruldu ama geri okunan FARKLI"
        },
        passed: install_ok,
    };

    // --- B / C: isleyici gercekten ayri yiginda mi ---
    //
    // Tasma beklemeden olculuyor: siradan bir sinyal de `SA_ONSTACK` ile
    // teslim edilir. Tasmayla olculseydi, gecen bir sinav "ayri yigin"
    // ile "tasma yakalandi"yi ayirt edemezdi.
    signal::action_info(signal::SIGUSR1, on_alt, signal::SA_ONSTACK, 0);
    signal::kill(sys::getpid(), signal::SIGUSR1);
    let handler_sp = HANDLER_SP.load(Ordering::SeqCst);
    let on_alt_stack = handler_sp >= alt_base && handler_sp < alt_top;
    checks[1] = Check {
        name: NAMES[1],
        detail: if handler_sp == 0 {
            "isleyici HIC cagrilmadi"
        } else if on_alt_stack {
            "isleyicinin yigini ayri bolgede"
        } else {
            "isleyici NORMAL yiginda kostu"
        },
        passed: on_alt_stack,
    };

    let seen_flags = HANDLER_FLAGS.load(Ordering::SeqCst);
    let onstack_ok = seen_flags == signal::SS_ONSTACK as usize;
    checks[2] = Check {
        name: NAMES[2],
        detail: if seen_flags == usize::MAX {
            "isleyici cagrilmadi"
        } else if onstack_ok {
            "isleyici icinde SS_ONSTACK gorundu"
        } else if seen_flags == signal::SS_DISABLE as usize {
            "isleyici icinde 'kurulu degil' dedi"
        } else {
            "ss_flags SS_ONSTACK degil"
        },
        passed: onstack_ok,
    };

    // --- F: bayraksiz isleyici normal yiginda ---
    //
    // Ayrimin olculmesi sart: `SA_ONSTACK` bir **secim**. Her isleyici
    // ayri yigina gitseydi bayragin anlami kalmazdi.
    let anchor = 0u8;
    let anchor_at = &anchor as *const u8 as usize;
    signal::action_info(signal::SIGUSR2, on_plain, 0, 0);
    signal::kill(sys::getpid(), signal::SIGUSR2);
    let plain_sp = PLAIN_SP.load(Ordering::SeqCst);
    let off_alt = plain_sp != 0 && !(plain_sp >= alt_base && plain_sp < alt_top);
    checks[5] = Check {
        name: NAMES[5],
        detail: if plain_sp == 0 {
            "isleyici cagrilmadi"
        } else if !off_alt {
            "bayraksiz isleyici AYRI yigina gitti"
        } else if plain_sp < anchor_at {
            "bayraksiz isleyici normal yiginda kostu"
        } else {
            "normal yiginda ama beklenen yerde degil"
        },
        passed: off_alt,
    };

    // --- G: dar yigin reddediliyor ---
    //
    // Bir **ret** sinavi. Kabul edilseydi isleyiciye girer girmez tasan
    // bir yigin verilmis olurdu -- yani sorun cozulmez, tasinirdi.
    let narrow = AltStack {
        sp: alt_base,
        flags: 0,
        size: signal::MINSIGSTKSZ - 1,
    };
    let refused = signal::sigaltstack(Some(&narrow), None) < 0;
    let mut after = AltStack::NONE;
    signal::sigaltstack(None, Some(&mut after));
    let narrow_ok = refused && after.size == ALT_SIZE;
    checks[6] = Check {
        name: NAMES[6],
        detail: if !refused {
            "MINSIGSTKSZ altindaki yigin KABUL edildi"
        } else if narrow_ok {
            "reddedildi ve kurulu yigin bozulmadi"
        } else {
            "reddedildi ama kurulu yigin degisti"
        },
        passed: narrow_ok,
    };

    // --- D / E: tasma ---
    //
    // Cocukta kosuyor cunku surec her halukarda oluyor: ya isleyici
    // rapor edip cikiyor (basari), ya da teslim edilemeyen sinyal onu
    // olduruyor (basarisizlik). Ebeveyn ikisini cikis koduyla ayiriyor.
    let child = sys::fork();
    if child == 0 {
        signal::action_info(signal::SIGSEGV, on_overflow, signal::SA_ONSTACK, 0);
        devour(0);
        sys::exit(1);
    }
    let mut status = 0u32;
    sys::waitpid(child as usize, &mut status, 0);
    let code = if sys::exited(status) {
        sys::exit_status(status) as i32
    } else {
        -1
    };
    let caught = code == CAUGHT_BASE || code == CAUGHT_BASE + 1;
    checks[3] = Check {
        name: NAMES[3],
        detail: if caught {
            "tasma yakalandi, isleyici rapor edebildi"
        } else if sys::signalled(status) {
            "tasmada sinyal TESLIM EDILEMEDI"
        } else if sys::exited(status) {
            "ozyineleme yigini hic tuketmedi"
        } else {
            "cocugun sonu okunamadi"
        },
        passed: caught,
    };

    // E: tasma **nerede** durdu.
    //
    // D'nin olcemedigi sey. Koruma sayfasi olmasaydi tasma hicbir hata
    // uretmeden .bss'e girer, programin kendi verisini ezer ve ancak cok
    // asagida, eslenmemis bir yerde patlardi. Surec yine olurdu -- yani
    // "oldu mu" diye sormak koruma sayfasini olcmez. Olcen soru: hata
    // adresi koruma sayfasinin **icinde** mi.
    let stopped_early = code == CAUGHT_BASE + 1;
    checks[4] = Check {
        name: NAMES[4],
        detail: if !caught {
            "isleyici kosmadi, adres okunamadi"
        } else if stopped_early {
            "tasma koruma sayfasinin uzerinde durdu"
        } else {
            "tasma .bss'i ezerek ILERLEDI"
        },
        passed: stopped_early,
    };

    report(&checks, handler_sp, plain_sp, alt_base);
    show(&checks, handler_sp, alt_base);
}

fn report(checks: &[Check; 7], handler_sp: usize, plain_sp: usize, alt_base: usize) {
    use core::fmt::Write;
    let mut console = Stdout;
    let _ = writeln!(console, "[altstack] sinav basliyor");
    for check in checks {
        let _ = writeln!(
            console,
            "[altstack] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let _ = writeln!(
        console,
        "[altstack] ayri yigin: 0x{:x}..0x{:x}  isleyici sp: 0x{:x}  bayraksiz sp: 0x{:x}",
        alt_base,
        alt_base + ALT_SIZE,
        handler_sp,
        plain_sp
    );
}

fn show(checks: &[Check; 7], handler_sp: usize, alt_base: usize) {
    let mut win = match Window::open("altstack -- tasmayi yakalamak", 250, 160, 500, 216) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, checks, handler_sp, alt_base);
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 7], handler_sp: usize, alt_base: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Tasan yigina cerceve kurulamaz -- ayri yigin sart", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            395,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    let inside = handler_sp >= alt_base && handler_sp < alt_base + ALT_SIZE;
    win.text(6, h - 30, "isleyici sp ayri yiginda:", DIM);
    win.text(
        230,
        h - 30,
        if inside { "evet" } else { "HAYIR" },
        if inside { OK } else { WARN },
    );
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
