//! `winctx.exe` -- bir akisin **nereden** baslayacagini degistirmek.
//!
//! Bir onceki bati askiya almayi getirdi: `CREATE_SUSPENDED` ile dogan
//! akis giris noktasina hic girmiyor. O ara, tek basina bir duraklama
//! degil -- Windows'un asil verdigi soz, o arada akisin **her seyinin**
//! degistirilebilir olmasi:
//!
//! ```text
//!   CreateThread(.., CREATE_SUSPENDED, ..)
//!   GetThreadContext(h, &c)
//!   c.Eip = baska_bir_yer
//!   SetThreadContext(h, &c)
//!   ResumeThread(h)              -> akis BASKA yerden baslar
//! ```
//!
//! POSIX'te bu dizinin karsiligi **yoktur**. En yakini `ptrace`:
//!
//! ```text
//!   POSIX  ptrace(PTRACE_ATTACH, pid)     -> iliski KUR
//!          waitpid(pid, ..)               -> durmasini BEKLE
//!          ptrace(PTRACE_GETREGS, ..)     -> oku
//!          ptrace(PTRACE_DETACH, pid)     -> iliskiyi BOZ
//!
//!   Win32  GetThreadContext(h, &c)        -> oku
//! ```
//!
//! Ayrim, isin ne oldugundan cok **kimin yapabildigi**: POSIX'te register
//! okumak hata ayiklamaya ozel bir iliski gerektirir ve o iliski
//! tekildir (bir surece tek izleyen). Windows'ta yalnizca bir tutamac
//! yetkisidir -- bir profilleyici, bir kurtarma kodu ya da bir paketleyici
//! bunu siradan bir cagri gibi kullanir. Kalibi yayginlastiran sey bu.
//!
//! ## TCMK ne kadarini soyluyor
//!
//! Baglam okumak, "o akis su an nerede" sorusudur ve cevabin **bugune
//! ait** olmasi gerekir. TCMK iki durumda bunu gercekten bilir:
//!
//! ```text
//!   hedef                  GetThreadContext     SetThreadContext
//!   cagiranin kendisi      GERCEK (canli)       reddedilir
//!   hic kosmamis akis      GERCEK (giris)       GERCEK
//!   kosmaya baslamis akis  REDDEDILIR           REDDEDILIR
//! ```
//!
//! Ucuncu satir eksik bir yetenek ve oyle yazildi. Kosan bir akisin
//! registerlari cekirdekte saklanmiyor; o akisin cekirdek yiginindaki bir
//! kesme cercevesinde duruyorlar. Orayi okumak mumkun olabilirdi ama
//! **son syscall anina** ait bir goruntu verirdi -- yani cagiran onu
//! "simdi" sanip yanilirdi. Reddetmek daha az yetenek, daha fazla dogru
//! bilgi.
//!
//! Ikinci satirin sagi da bilerek dar: yazma yalnizca **askida** bir akis
//! icin kabul ediliyor. Kosmayi bekleyen bir akis her an zamanlanabilir,
//! yani "henuz kosmadi" cevabi bir sonraki komuta kadar gecerli olurdu.
//! Windows da ayni sarti koyuyor.
//!
//! ## Alti sinav
//!
//! ```text
//!   A  giris baglami   -> askida dogan akisin Eip'i giris fonksiyonu
//!   B  kendi baglami   -> okunan Esp yerel bir degiskene KOMSU
//!   C  YAZILAN YURUDU  -> Eip degistirildi, akis OTEKI yerden basladi
//!   D  kosani reddet   -> kosan akis icin FALSE + ERROR_NOT_SUPPORTED
//!   E  iki ret         -> eksik bayrak ve kendine yazma reddedildi
//!   F  gecersiz tutamac-> FALSE + ERROR_INVALID_HANDLE
//!   G  segment tabani  -> CreateThread CAGIRANIN TEB'ini bozmuyor
//! ```
//!
//! G otekilerden baska bir soru soruyor ve buraya, bu bati yuzunden
//! geldi. Baglamin dogmadan okunabilmesi icin cekirdek yeni akisin TEB'ini
//! artik `CreateThread` icinde, **cagiranin** baglaminda kuruyor. TEB
//! kurmak segment tabanini hemen etkinlestirdigi icin bu, cagirani
//! cocugunun TEB'ine bakmaya birakabilirdi: son hata kodu ve SEH zinciri
//! yanlis bloktan okunurdu. Cekirdek tabani geri koyuyor; G tam olarak
//! onu olcuyor -- `CreateThread`den hemen sonra, arada tek bir cagri bile
//! olmadan, segmentin gosterdigi TEB'e bakiyor.
//!
//! C bu sinavin sebebi. A tek basina "bir sayi okundu" demek olurdu; C
//! ise okunanin **gercekten** o akisin yazgisi oldugunu gosteriyor:
//! degistirilen deger yurudu.
//!
//! B, A'nin baska turden bir dogrulamasi. Cekirdek "kendisi" durumunda
//! canli cerceveyi okuyor; olcu, o cercevenin gercekten bu cagrinin
//! cercevesi olmasi. Yerel bir degiskenin adresi ile okunan `Esp`
//! arasindaki mesafeye bakmak bunu kanitliyor -- uydurulmus ya da
//! bayatlamis bir deger o pencereye dusmez.
//!
//! D, E ve F birer **ret** sinavi: gecmeleri, cekirdegin bilmedigi ya da
//! emin olmadigi bir isi yapmayi reddetmesiyle oluyor. Hata kodu da
//! sinava dahil -- "FALSE" tek basina sebebi soylemez, ve cagiran
//! sebebe gore farkli davranir.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use tcmk::winapi::{self, Context, Dword, Window};

tcmk::entry!(main);

const BG: u32 = 0x0014_1A28;
const PANEL: u32 = 0x0022_2C40;
const FG: u32 = 0x00E4_E8F4;
const DIM: u32 = 0x008C_94A8;
const ACCENT: u32 = 0x0098_C8F0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Iki artirma arasindaki bekleme.
const TICK_MS: Dword = 20;
/// Ilerlemenin olculdugu pencere.
const WATCH_MS: Dword = 160;

/// `Esp` ile yerel bir degiskenin adresi arasinda kabul edilen en buyuk
/// mesafe.
///
/// Sifir beklenemez: cagri zinciri (thunk + syscall) arada birkac
/// cerceve kuruyor. Bir sayfa, "ayni yiginin ayni bolgesi" demek icin
/// yeterince dar ve derleyicinin cerceve duzenine bagli olmayacak kadar
/// genis.
const STACK_WINDOW: usize = 4096;

/// Akisin **yaratildigi** giris noktasinin sayaci.
static PROGRESS_A: AtomicU32 = AtomicU32::new(0);
/// Baglam degistirildikten sonra kosmasi beklenen giris noktasinin
/// sayaci.
static PROGRESS_B: AtomicU32 = AtomicU32::new(0);

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

/// `CreateThread`e verilen giris noktasi. Baglam degistirilirse buraya
/// **hic** girilmemeli.
unsafe extern "system" fn ticker_a(_parameter: *mut c_void) -> Dword {
    loop {
        PROGRESS_A.fetch_add(1, Ordering::SeqCst);
        winapi::Sleep(TICK_MS);
    }
}

/// `SetThreadContext` ile yoneltilen giris noktasi.
unsafe extern "system" fn ticker_b(_parameter: *mut c_void) -> Dword {
    loop {
        PROGRESS_B.fetch_add(1, Ordering::SeqCst);
        winapi::Sleep(TICK_MS);
    }
}

/// Olcum penceresi boyunca iki sayacin artisi.
fn watch() -> (bool, bool) {
    let before = (PROGRESS_A.load(Ordering::SeqCst), PROGRESS_B.load(Ordering::SeqCst));
    unsafe { winapi::Sleep(WATCH_MS) };
    (
        PROGRESS_A.load(Ordering::SeqCst) != before.0,
        PROGRESS_B.load(Ordering::SeqCst) != before.1,
    )
}

fn main() {
    let mut checks = [EMPTY; 7];

    let mut thread_id = 0u32;
    let handle = unsafe {
        winapi::CreateThread(
            core::ptr::null_mut(),
            0,
            Some(ticker_a),
            core::ptr::null_mut(),
            winapi::CREATE_SUSPENDED,
            &mut thread_id,
        )
    };

    // G'nin olcusu buradan aliniyor ve sirasi **onemli**: arada bir cagri
    // olsaydi baglam degisimi segment tabanini kendiliginden geri koyar,
    // yani sinav olcmesi gereken seyi kacirirdi. Bu satir saf bir bellek
    // okumasidir -- segmentin gosterdigi yere bakiyor, cekirdege
    // sormuyor.
    let teb_owner = tcmk::teb::read(tcmk::teb::UNIQUE_PROCESS_OFFSET);

    if handle == 0 {
        for i in 0..checks.len() {
            checks[i] = Check {
                name: NAMES[i],
                detail: "akis yaratilamadi",
                passed: false,
            };
        }
        report(&checks, 0, 0);
        show(&checks, 0, 0);
        return;
    }

    // --- A: askida dogan akisin giris baglami ---
    //
    // Olcu bir bayrak degil, **sayinin kendisi**: okunan `Eip`
    // `CreateThread`e verilen fonksiyonun adresi olmali. "Cagri basarili
    // oldu" demek yetmezdi -- sifir dolu bir kayit da basarili gorunur.
    let mut context = Context::new();
    let read_ok = unsafe { winapi::GetThreadContext(handle, context.as_mut_ptr()) } != 0;
    let entry_a = ticker_a as *const () as usize;
    let entry_matches = read_ok && context.ip() == entry_a;
    checks[0] = Check {
        name: NAMES[0],
        detail: if !read_ok {
            "GetThreadContext reddedildi"
        } else if entry_matches {
            "Eip giris fonksiyonunu gosteriyor"
        } else {
            "okunan Eip giris noktasi DEGIL"
        },
        passed: entry_matches,
    };

    // --- B: cagiranin kendi baglami ---
    //
    // Cekirdek burada canli syscall cercevesini okuyor. Olcu, okunan
    // `Esp`nin gercekten **bu** yigina ait olmasi.
    let anchor = 0u8;
    let anchor_at = &anchor as *const u8 as usize;
    let mut own = Context::new();
    let own_ok = unsafe { winapi::GetThreadContext(winapi::CURRENT_THREAD, own.as_mut_ptr()) } != 0;
    let own_sp = own.sp();
    let near = own_ok
        && own_sp != 0
        && anchor_at.abs_diff(own_sp) < STACK_WINDOW
        && own.ip() != 0;
    checks[1] = Check {
        name: NAMES[1],
        detail: if !own_ok {
            "kendi baglami okunamadi"
        } else if own_sp == 0 {
            "okunan Esp sifir"
        } else if near {
            "Esp yerel degiskene komsu"
        } else {
            "okunan Esp BU yigina ait degil"
        },
        passed: near,
    };

    // --- E: iki ret ---
    //
    // C'den **once** yapiliyor, cunku ilkinin hedefi hala askida bekleyen
    // akis: eksik bayrakla gelen bir yazma reddedilmeli. `ContextFlags`
    // "bu kayitta hangi bolumler gecerli" demek; cekirdek bolum bolum
    // uygulamiyor, yani eksik bir kume kabul edilseydi cagiranin hic
    // doldurmadigi registerlar sifirla ezilirdi. Sessiz hasarin yerinde
    // bir hata olmasi gerekiyor.
    //
    // Ikinci ret cagiranin kendisi. Teknik olarak mumkun -- canli
    // cerceveye yazmak SEH'in `NtContinue` yolunun tam olarak yaptigi sey
    // -- ama bir cagrinin nereye donecegi tek anlamli olmali.
    //
    // Iki ret ayni sinavda, cunku ikisi de ayni soruyu soruyor: cekirdek
    // emin olmadigi bir yazmayi yapiyor mu? Ilki bu sinavi **dusurebilir**
    // olan yari: bayrak denetimi tek bir yerde duruyor. Ikincisi iki kez
    // korunuyor (kendisi zaten askida olmadigi icin ikinci kapidan da
    // gecemez), yani tek basina bir olcu degil -- sozlesmenin yazili
    // olmasi.
    //
    // Tampon yeniden kullaniliyor: `own`un B'deki isi bitti. Ucuncu bir
    // `CONTEXT` ayirmak x86_64'te yigin sondasini (`__chkstk`) tetikler
    // -- kayit orada 1232 bayt ve ucu birlikte dort kilobayti gecer.
    own.clear();
    let blank_write = unsafe { winapi::SetThreadContext(handle, own.as_ptr()) } != 0;
    let blank_error = unsafe { winapi::GetLastError() };
    let self_write = unsafe { winapi::SetThreadContext(winapi::CURRENT_THREAD, own.as_ptr()) } != 0;
    let self_error = unsafe { winapi::GetLastError() };
    let refused_both = !blank_write
        && blank_error == winapi::ERROR_INVALID_PARAMETER
        && !self_write
        && self_error == winapi::ERROR_NOT_SUPPORTED;
    checks[4] = Check {
        name: NAMES[4],
        detail: if blank_write {
            "eksik bayrakli kayit YAZILDI"
        } else if blank_error != winapi::ERROR_INVALID_PARAMETER {
            "eksik bayrak reddedildi ama hata kodu yanlis"
        } else if self_write {
            "kendi baglamini YAZDI"
        } else if refused_both {
            "eksik bayrak ve kendine yazma reddedildi"
        } else {
            "kendine yazma reddedildi ama hata kodu yanlis"
        },
        passed: refused_both,
    };

    // --- C: yazilan baglam yurudu mu ---
    //
    // Sinavin sebebi burasi. Yalnizca `Eip` degisiyor: yigin, donus
    // trampleni ve parametre oldugu gibi kaliyor -- yani akis, kendisi
    // icin kurulmus cerceveyle **baska** bir fonksiyona giriyor.
    let entry_b = ticker_b as *const () as usize;
    context.set_ip(entry_b);
    let write_ok = unsafe { winapi::SetThreadContext(handle, context.as_ptr()) } != 0;
    let resumed = unsafe { winapi::ResumeThread(handle) };
    let (moved_a, moved_b) = watch();
    let redirected = write_ok && moved_b && !moved_a;
    checks[2] = Check {
        name: NAMES[2],
        detail: if !write_ok {
            "SetThreadContext reddedildi"
        } else if resumed != 1 {
            "akis askidan kalkmadi"
        } else if moved_a {
            "akis ESKI giris noktasindan kostu"
        } else if redirected {
            "akis yazilan yerden basladi"
        } else {
            "akis hic ilerlemedi"
        },
        passed: redirected,
    };

    // --- D: kosan akis reddediliyor ---
    //
    // Gecmesi bir yetenek degil, bir **durustluk** olcusu: cekirdek
    // bilmedigi seyi bilmiyor diyor. Hata kodu da sinava dahil, cunku
    // "FALSE" tek basina sebebi soylemez.
    // `context`in isi bitti: C'de yazildi, artik bir sonraki okumanin
    // hedefi olabilir.
    let running_read =
        unsafe { winapi::GetThreadContext(handle, context.as_mut_ptr()) } != 0;
    let running_error = unsafe { winapi::GetLastError() };
    let refused_running = !running_read && running_error == winapi::ERROR_NOT_SUPPORTED;
    checks[3] = Check {
        name: NAMES[3],
        detail: if running_read {
            "kosan akis icin BIR SEY dondurdu"
        } else if refused_running {
            "kosan akis icin ERROR_NOT_SUPPORTED"
        } else {
            "reddetti ama hata kodu yanlis"
        },
        passed: refused_running,
    };

    // --- F: gecersiz tutamac ---
    let bad_read = unsafe { winapi::GetThreadContext(0, context.as_mut_ptr()) } != 0;
    let bad_error = unsafe { winapi::GetLastError() };
    let refused_bad = !bad_read && bad_error == winapi::ERROR_INVALID_HANDLE;
    checks[5] = Check {
        name: NAMES[5],
        detail: if bad_read {
            "gecersiz tutamac kabul edildi"
        } else if refused_bad {
            "ERROR_INVALID_HANDLE dondu"
        } else {
            "reddetti ama hata kodu yanlis"
        },
        passed: refused_bad,
    };

    // --- G: cagiranin segment tabani bozulmadi mi ---
    //
    // Olcu, `CreateThread`den hemen sonra okunan TEB'in **cagiranin**
    // TEB'i olmasi. Iki yonden bakiliyor: kendi kimligiyle ayni mi, ve
    // yeni dogan akisin kimliginden ayri mi. Ikincisi sart -- yalnizca
    // "sifir degil" demek, cocugun TEB'ini de gecerli sayardi.
    let own_pid = unsafe { winapi::GetCurrentProcessId() } as usize;
    let teb_intact = teb_owner != 0
        && teb_owner == own_pid
        && teb_owner != thread_id as usize;
    checks[6] = Check {
        name: NAMES[6],
        detail: if teb_owner == 0 {
            "TEB yok -- segment tabani sifir"
        } else if teb_owner == thread_id as usize {
            "segment COCUGUN TEB'ini gosteriyor"
        } else if teb_intact {
            "taban hala cagiranin TEB'inde"
        } else {
            "TEB'deki kimlik cagirana ait DEGIL"
        },
        passed: teb_intact,
    };

    // Akisi askiya al: pencere dongusu boyunca bosuna kosmasinin anlami
    // yok.
    unsafe { winapi::SuspendThread(handle) };
    let (a, b) = (
        PROGRESS_A.load(Ordering::SeqCst),
        PROGRESS_B.load(Ordering::SeqCst),
    );
    report(&checks, a, b);
    unsafe { winapi::CloseHandle(handle) };
    show(&checks, a, b);
}

const NAMES: [&str; 7] = [
    "A giris baglami",
    "B kendi baglami",
    "C YAZILAN YURUDU",
    "D kosani reddet",
    "E iki ret",
    "F gecersiz tutamac",
    "G segment tabani",
];

fn report(checks: &[Check; 7], a: u32, b: u32) {
    use core::fmt::Write;
    let mut console = winapi::Console;
    for check in checks {
        let _ = writeln!(
            console,
            "[winctx] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let _ = writeln!(console, "[winctx] eski giris: {}  yeni giris: {}", a, b);
}

fn show(checks: &[Check; 7], a: u32, b: u32) {
    let mut win = match Window::create("winctx -- baglam okuma/yazma", 270, 168, 490, 216) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, checks, a, b);
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 7], a: u32, b: u32) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Akis baska bir yerden baslatilabilir", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            385,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 44, "eski giris:", DIM);
    win.number(110, h - 44, a as usize, if a == 0 { OK } else { WARN });
    win.text(6, h - 30, "yeni giris:", DIM);
    win.number(110, h - 30, b as usize, if b != 0 { OK } else { WARN });
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
