//! `winnest.exe` -- ya hatayi inceleyen kodun **kendisi** cokerse?
//!
//! `winseh` istisna dagitiminin calistigini gosterdi, `winunwind` onun
//! ikinci yarisini. Geriye dagiticinin en tuhaf sorusu kalmisti: bir
//! isleyici **kendi** patlarsa ne olur?
//!
//! Uzun sure TCMK'nin cevabi "surec biter" idi -- muhafazakar ama
//! pahali bir cevap: tek bir hatali isleyici butun sureci goturuyordu.
//! Windows'un cevabi baska ve daha kullanisli: dagitim **siradaki**
//! isleyiciyle surer.
//!
//! ```text
//!   h1 cagrilir  ->  h1'in kendisi coker
//!                      -> IC ICE dagitim baslar
//!                      -> h2 cagrilir, bayraginda EXCEPTION_NESTED_CALL
//!                      -> h2 h1'in hatasini duzeltir, "devam et" der
//!                    h1 kaldigi yerden surer ve doner
//!   h3 cagrilir  ->  ama ASIL hatayi gorur, h1'inkini degil
//! ```
//!
//! ## Iki kural, ikisi de zorunlu
//!
//! **Yurume bastan baslamaz.** Baslasaydi coken isleyici (h1) yeniden
//! cagrilir, yeniden coker ve dagitim sonsuz donguye girerdi. Bu yuzden
//! ic dagitim, dis dagitimin kaldigi yerden -- h2'den -- devam ediyor.
//!
//! **Ic dagitim cozulunce dis kayit geri gelir.** Gelmeseydi kalan
//! isleyiciler (h3) h1'in hatasini "asil hata" sanardi. Kayit iki ayri
//! olay anlatiyor ve karistirilmalari, bir cokme raporlayicisinin
//! yanlis adresi gunluge yazmasi demek olurdu.
//!
//! ## `ExceptionRecord` alani nicin var
//!
//! `EXCEPTION_RECORD`un icinde bir `ExceptionRecord` isaretcisi vardir
//! ve cogu zaman `NULL`dur. Dolu oldugu tek yer burasi: ic ice bir
//! kayitta o alan **dis** kaydi gosterir. Yani "bu hata su hatayi
//! incelerken olustu" zinciri, veri yapisinin kendisinde duruyor.
//!
//! ## POSIX'in ayni soruya cevabi
//!
//! ```text
//!   Win32  isleyici icinde yeni istisna -> SIRADAKI isleyiciye gider
//!   POSIX  isleyici icinde ayni sinyal  -> ENGELLI -> surec oler
//! ```
//!
//! POSIX'te bir sinyal kendi isleyicisi suresince maskelidir
//! (`SA_NODEFER` yoksa), yani `SIGSEGV` isleyicisinin kendi urettigi bir
//! sayfa hatasi teslim edilemez ve varsayilan davranisa duser. TCMK'nin
//! POSIX yuzu de tam olarak boyle davraniyor (bkz. `sigfault`).
//!
//! Ikisi de savunulabilir: Windows hatali isleyiciye ikinci bir sans
//! veriyor, POSIX donguye girme ihtimalini bastan kesiyor. Ayni
//! donanim olayina iki ayri **sozlesme** -- projenin tezi.
//!
//! ## Alti sinav
//!
//! ```text
//!   A  isleyici coktu    -> ilk isleyici kendi patladi, surec YASIYOR
//!   B  siradaki kostu    -> ikinci isleyici cagrildi
//!   C  NESTED_CALL       -> ic kaydin bayraginda o bit var
//!   D  zincir isaretcisi -> ExceptionRecord alani DIS kaydi gosteriyor
//!   E  bastan baslamadi  -> coken isleyici ikinci kez CAGRILMADI
//!   F  dis kayit dondu   -> ucuncu isleyici ASIL hatayi gordu
//! ```
//!
//! A sinavin sebebi: eskiden bu programin varligi bile mumkun degildi,
//! surec ilk cokmede biterdi. E ile F ise dogru yapilmasini olcuyor --
//! A tek basina bir donguyle de "gecebilirdi".
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use tcmk::seh::{self, ExceptionPointers};
use tcmk::winapi::{self, Window};

tcmk::entry!(main);

const BG: u32 = 0x001A_1618;
const PANEL: u32 = 0x002C_2428;
const FG: u32 = 0x00EC_E6E8;
const DIM: u32 = 0x0098_9094;
const ACCENT: u32 = 0x00F0_A0A0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Asil hatanin dusecegi yer.
static mut SCRATCH: usize = 0;
/// Isleyicinin kendi hatasinin dusecegi yer -- ayri olmasi sart, yoksa
/// hangi yazmanin dogru yere dustugu karisirdi.
static mut INNER_SCRATCH: usize = 0;

const MARK: usize = 0x1111_2222;
const INNER_MARK: usize = 0x3333_4444;

/// Cagri sirasini veren sayac.
static SEQUENCE: AtomicU32 = AtomicU32::new(1);

/// h1 kac kez cagrildi (bir kez olmali -- bkz. E).
static H1_CALLS: AtomicU32 = AtomicU32::new(0);
/// h2 hangi sirada cagrildi (0 = hic).
static H2_ORDER: AtomicU32 = AtomicU32::new(0);
/// h2 kac kez cagrildi -- karar bunun uzerinden veriliyor.
static H2_CALLS: AtomicU32 = AtomicU32::new(0);
/// h2'nin gordugu bayraklar ve `nested` isaretcisi.
static H2_FLAGS: AtomicU32 = AtomicU32::new(0);
static H2_NESTED: AtomicUsize = AtomicUsize::new(0);
/// h2'ye gelen ic kaydin gosterdigi **dis** kaydin kodu.
static H2_OUTER_CODE: AtomicU32 = AtomicU32::new(0);
/// h3'un gordugu kod ve bayraklar.
static H3_ORDER: AtomicU32 = AtomicU32::new(0);
static H3_CODE: AtomicU32 = AtomicU32::new(0);
static H3_FLAGS: AtomicU32 = AtomicU32::new(0);
/// Dis kaydin adresi -- h1 onu goruyor, h2 ile karsilastiriliyor.
static OUTER_RECORD: AtomicUsize = AtomicUsize::new(0);

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

const NAMES: [&str; 6] = [
    "A isleyici coktu",
    "B siradaki kostu",
    "C NESTED_CALL",
    "D zincir isaretcisi",
    "E bastan baslamadi",
    "F dis kayit dondu",
];

/// Gecersiz bir adrese yazar; hedef adres `ecx`/`rcx`te durur.
///
/// `winseh::write_through_null` ile **ayni komut** -- duzeltilecek sey
/// tam olarak o register.
#[inline(never)]
unsafe fn write_through_null(value: usize) {
    #[cfg(target_arch = "x86")]
    core::arch::asm!("mov [ecx], edx", inout("ecx") 0usize => _, inout("edx") value => _);
    #[cfg(target_arch = "x86_64")]
    core::arch::asm!("mov [rcx], rdx", inout("rcx") 0usize => _, inout("rdx") value => _);
}

/// Birinci isleyici: asil hatayi gorur ve **kendisi coker**.
///
/// Cokme kasitli ve tek seferlik. Ikinci kez cagrilsaydi yeniden
/// cokerdi; E sinavinin olctugu sey tam olarak cagrilmamasi.
unsafe extern "system" fn h1(info: *mut ExceptionPointers) -> i32 {
    H1_CALLS.fetch_add(1, Ordering::SeqCst);
    OUTER_RECORD.store((*info).exception_record as usize, Ordering::SeqCst);

    // Isleyicinin kendi hatasi. Buradan sonrasi ic ice dagitim: h2
    // cagrilacak, hatayi duzeltecek ve yurutme **bu satira** donecek.
    write_through_null(INNER_MARK);

    // Buraya gelinmesi, ic dagitimin cozuldugu ve yurutmenin coken
    // isleyicinin icinde surdugu anlamina geliyor.
    seh::EXCEPTION_CONTINUE_SEARCH
}

/// Ikinci isleyici: h1'in hatasini duzeltir.
unsafe extern "system" fn h2(info: *mut ExceptionPointers) -> i32 {
    let record = (*info).exception_record;
    H2_ORDER.store(SEQUENCE.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
    H2_FLAGS.store((*record).flags, Ordering::SeqCst);

    let outer = (*record).nested;
    H2_NESTED.store(outer as usize, Ordering::SeqCst);
    if !outer.is_null() {
        H2_OUTER_CODE.store((*outer).code, Ordering::SeqCst);
    }

    // Karar **bayraga bakmadan** veriliyor: h2'nin ilk cagrisi h1'in
    // hatasidir, ikincisi olursa asil hata. Boyle olmasi kasitli --
    // akis bayraga baglansaydi, bayragi kaldiran bir cekirdekte butun
    // sinavlar birden duserdi ve hangisinin bozuldugu anlasilmazdi.
    // Bayrak burada yalnizca **gozlemleniyor** (C sinavi).
    if H2_CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
        // h1'in hatali isaretcisini gecerli bir adrese cevir; komut
        // tekrarlanacak ve yazma `INNER_SCRATCH`e dusecek.
        seh::set_reg(
            (*info).context_record,
            seh::Reg::C,
            core::ptr::addr_of_mut!(INNER_SCRATCH) as usize,
        );
        return seh::EXCEPTION_CONTINUE_EXECUTION;
    }
    seh::EXCEPTION_CONTINUE_SEARCH
}

/// Ucuncu isleyici: asil hatayi duzeltir.
///
/// Gordugu kaydin **dis** kayit olmasi F sinavinin kendisi.
unsafe extern "system" fn h3(info: *mut ExceptionPointers) -> i32 {
    let record = (*info).exception_record;
    H3_ORDER.store(SEQUENCE.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
    H3_CODE.store((*record).code, Ordering::SeqCst);
    H3_FLAGS.store((*record).flags, Ordering::SeqCst);

    seh::set_reg(
        (*info).context_record,
        seh::Reg::C,
        core::ptr::addr_of_mut!(SCRATCH) as usize,
    );
    seh::EXCEPTION_CONTINUE_EXECUTION
}

/// Cocugun cevabi: her sinav bir bit.
///
/// Cikis koduna sigmasi kasitli. Coken kismin **ayri bir surecte**
/// olmasi sart: ic ice dagitim calismiyorsa o surec olur ve bu sinavin
/// hic ciktisi olmazdi. Daha once iki kez ogrenilmis bir ders (bkz.
/// `bigfile`, `jobs`, `sigfault`); burada dorduncu kez cikti.
const BIT_CRASHED: u32 = 1;
const BIT_NEXT: u32 = 2;
const BIT_FLAG: u32 = 4;
const BIT_CHAIN: u32 = 8;
const BIT_ONCE: u32 = 16;
const BIT_OUTER: u32 = 32;

/// Cocuk govdesini **bitirdigini** soyleyen isaret biti.
///
/// Gerekli, cunku "cocuk coktu"yu cikis kodunun buyuklugunden
/// anlamak kirilgandi: coken bir cocugun kodu her yolda ayni degil.
/// Isaret biti, cevabi sonucun kendisinden okumayi birakip **cocugun
/// oraya varmis olmasindan** okumayi sagliyor.
const BIT_DONE: u32 = 0x8000;

/// Cocugu ayiran arguman.
const CHILD_ARG: &str = "child";

fn main() {
    if tcmk::args::count() > 1 && tcmk::args::get(1) == Some(CHILD_ARG) {
        unsafe { winapi::ExitProcess(crashing_child()) };
    }

    let mut checks = [EMPTY; 6];

    // Cocuk: ayni ikili, "child" argumaniyla.
    let mut info = winapi::ProcessInformation::new();
    let created = unsafe {
        winapi::CreateProcessA(
            b"C:\\bin\\winnest.exe\0".as_ptr(),
            b"winnest.exe child\0".as_ptr(),
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

    let mut code = 0u32;
    if created != 0 {
        unsafe {
            winapi::WaitForSingleObject(info.process, 5_000);
            winapi::GetExitCodeProcess(info.process, &mut code);
            winapi::CloseHandle(info.process);
        }
    }

    // Cocuk govdesinin sonuna vardiysa isaret biti kurulu olur; her
    // baska deger "oraya varamadi" demektir.
    let finished = code & BIT_DONE != 0;
    let crashed = created != 0 && !finished;
    let bits = if finished { code } else { 0 };

    let reason = |ok: bool, good: &'static str, bad: &'static str| -> &'static str {
        if created == 0 {
            "cocuk baslatilamadi"
        } else if crashed {
            "cocuk COKTU -- ic ice dagitim yok"
        } else if ok {
            good
        } else {
            bad
        }
    };

    let set = |bit: u32| finished && bits & bit != 0;

    checks[0] = Check {
        name: NAMES[0],
        detail: reason(
            set(BIT_CRASHED),
            "isleyici coktu, surec yasadi, ikisi de duzeldi",
            "isleyici coktu ama kurtarma eksik",
        ),
        passed: set(BIT_CRASHED),
    };
    checks[1] = Check {
        name: NAMES[1],
        detail: reason(
            set(BIT_NEXT),
            "ikinci isleyici cagrildi",
            "ikinci isleyici CAGRILMADI",
        ),
        passed: set(BIT_NEXT),
    };
    checks[2] = Check {
        name: NAMES[2],
        detail: reason(
            set(BIT_FLAG),
            "ic kayitta EXCEPTION_NESTED_CALL var",
            "bayrak yok -- kayit asil hata gibi gorunuyor",
        ),
        passed: set(BIT_FLAG),
    };
    checks[3] = Check {
        name: NAMES[3],
        detail: reason(
            set(BIT_CHAIN),
            "alan dis kaydi gosteriyor, kodu da dogru",
            "ExceptionRecord alani dis kaydi GOSTERMIYOR",
        ),
        passed: set(BIT_CHAIN),
    };
    checks[4] = Check {
        name: NAMES[4],
        detail: reason(
            set(BIT_ONCE),
            "coken isleyici bir kez cagrildi",
            "coken isleyici YENIDEN cagrildi (dongu)",
        ),
        passed: set(BIT_ONCE),
    };
    checks[5] = Check {
        name: NAMES[5],
        detail: reason(
            set(BIT_OUTER),
            "ucuncu isleyici asil hatayi gordu",
            "ucuncu isleyici asil hatayi GORMEDI",
        ),
        passed: set(BIT_OUTER),
    };

    report(&checks, code);
    show(&checks, code);
}

/// Cocuk govdesi: isleyicileri kurar, coker, sonucu bit maskesi doner.
fn crashing_child() -> u32 {
    unsafe {
        winapi::AddVectoredExceptionHandler(0, Some(h1));
        winapi::AddVectoredExceptionHandler(0, Some(h2));
        winapi::AddVectoredExceptionHandler(0, Some(h3));
        SCRATCH = 0;
        INNER_SCRATCH = 0;
    }

    // Asil hata. Dagitim h1 -> (h1 coker) -> h2 -> h1 surer -> h3
    // seklinde yurumeli.
    unsafe { write_through_null(MARK) };

    let outer_landed = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(SCRATCH)) };
    let inner_landed = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(INNER_SCRATCH)) };
    let h1_calls = H1_CALLS.load(Ordering::SeqCst);
    let h2_order = H2_ORDER.load(Ordering::SeqCst);
    let h3_order = H3_ORDER.load(Ordering::SeqCst);

    let mut bits = 0u32;

    // A: "bu satira gelindi" tek basina yetmez -- h1 hic cokmemis de
    // olabilirdi. Olcu, **ic** yazmanin da dogru yere dusmesi.
    if h1_calls > 0 && inner_landed == INNER_MARK && outer_landed == MARK {
        bits |= BIT_CRASHED;
    }
    if h2_order != 0 {
        bits |= BIT_NEXT;
    }
    if h2_order != 0 && H2_FLAGS.load(Ordering::SeqCst) & seh::EXCEPTION_NESTED_CALL != 0 {
        bits |= BIT_FLAG;
    }

    // D: iki yonden olculuyor -- adres dis kaydin adresiyle ayni mi, ve
    // o adresteki kod asil hatanin kodu mu. "Sifir degil" demek zayif
    // olurdu.
    let nested_ptr = H2_NESTED.load(Ordering::SeqCst);
    if nested_ptr != 0
        && nested_ptr == OUTER_RECORD.load(Ordering::SeqCst)
        && H2_OUTER_CODE.load(Ordering::SeqCst) == seh::STATUS_ACCESS_VIOLATION
    {
        bits |= BIT_CHAIN;
    }

    // E: bastan baslasaydi h1 yeniden cagrilir, yeniden coker ve
    // dagitim donguye girerdi.
    if h1_calls == 1 {
        bits |= BIT_ONCE;
    }

    // F: ic dagitim cozuldukten sonra kalanlar **asil** hatayi gormeli.
    let h3_flags = H3_FLAGS.load(Ordering::SeqCst);
    if h3_order != 0
        && H3_CODE.load(Ordering::SeqCst) == seh::STATUS_ACCESS_VIOLATION
        && h3_flags & seh::EXCEPTION_NESTED_CALL == 0
        && h3_order > h2_order
    {
        bits |= BIT_OUTER;
    }

    // Cocuk ham sayilarini da birakiyor: bit maskesi "neyin gectigini"
    // soyluyor ama "neden" sorusuna ancak bunlar cevap veriyor.
    {
        use core::fmt::Write;
        let mut console = winapi::Console;
        let _ = writeln!(
            console,
            "[winnest/cocuk] h1={} h2={} h3={} bayrak={:#x} zincir={:#x} ic={:#x} dis={:#x}",
            h1_calls,
            h2_order,
            h3_order,
            H2_FLAGS.load(Ordering::SeqCst),
            nested_ptr,
            inner_landed,
            outer_landed
        );
    }

    bits | BIT_DONE
}

fn report(checks: &[Check; 6], code: u32) {
    use core::fmt::Write;
    let mut console = winapi::Console;
    for check in checks {
        let _ = writeln!(
            console,
            "[winnest] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let _ = writeln!(console, "[winnest] cocugun cevabi: {:#06x}", code);
}

fn show(checks: &[Check; 6], code: u32) {
    let mut win = match Window::create("winnest -- isleyicinin kendi hatasi", 255, 170, 500, 190) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, checks, code);
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 6], code: u32) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Coken isleyici sureci goturmuyor", ACCENT);

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
    win.text(6, h - 30, "cocugun cevabi:", DIM);
    win.number(
        160,
        h - 30,
        code as usize,
        if code == BIT_DONE | 0x3F { OK } else { WARN },
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
