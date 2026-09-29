//! `winpdata.exe` -- x86_64'un SEH'i: zincir degil **tablo**.
//!
//! i386'da bir `__try` calisma zamaninda para oder: derleyici her
//! girisde yigina iki kelimelik bir kayit iter ve `fs:[0]`i ona cevirir.
//! Zincir yigin uzerinde yasar, istisna aninda gezilir.
//!
//! x86_64'te Microsoft bu maliyeti tamamen kaldirdi. `__try` **hicbir
//! komut uretmez**; derleyici bunun yerine ikili dosyaya her fonksiyon
//! icin bir kayit yazar.
//!
//! ```text
//!   i386   zincir  ->  yiginda, CALISMA ZAMANINDA kurulur
//!                      bedeli: her __try girisinde iki yazma
//!                      cekirdek: iki kelime oku, izle
//!
//!   x64    tablo   ->  ikilide, DERLEME ZAMANINDA kurulur
//!                      bedeli: sifir komut
//!                      cekirdek: bir tablo cozumleyicisi
//! ```
//!
//! Maliyet kaybolmadi, **yer degistirdi**: uygulamadan cekirdege.
//!
//! ## Ayrilan sey yalnizca "nasil bulunur"
//!
//! Isleyicinin imzasi ve karar kumesi iki mimaride **aynidir**: dort
//! arguman, `EXCEPTION_CONTINUE_EXECUTION` (0) ya da `..._SEARCH` (1).
//! Bu sinavin belki en sasirtici sonucu bu -- ayrilan sey yalnizca
//! isleyiciye nasil ulasildigi.
//!
//! ## Tabloyu neden elle kuruyoruz
//!
//! Userland Rust ile yaziliyor ve Rust'in COFF cikisi `.pdata` degil
//! `.eh_frame` uretiyor (DWARF). Yani bu ikilide okunacak bir `.pdata`
//! yok. Kayit `RtlAddFunctionTable` ile calisma zamaninda ekleniyor --
//! gercek Windows'ta JIT'lerin yaptigi sey. `winseh.exe`nin zincir
//! kaydini elle kurmasiyla ayni durum: Rust'ta `__try` yok, o yuzden
//! derleyicinin yapacagi sey elle yapiliyor.
//!
//! ## Yedi sinav
//!
//! ```text
//!   A  tablo kabul edildi  -> RtlAddFunctionTable sifirdan farkli dondu
//!   B  kayit bulunuyor     -> RtlLookupFunctionEntry dogru girdiyi verdi
//!   C  ISLEYICI KOSTU      -> hata adresi tabloda, isleyici cagrildi
//!   D  dispatcher dolu     -> ImageBase ve FunctionEntry dogru geldi
//!   E  devam et yurudu     -> isleyici CONTEXT'i duzeltti, akis surdu
//!   F  EHANDLER sart       -> bayraksiz kayit isleyiciyi KOSTURMUYOR
//!   G  silinince bitiyor   -> RtlDeleteFunctionTable sonrasi bulunmuyor
//! ```
//!
//! C bu sinavin sebebi: bu batiya kadar x86_64'te SEH **yalnizca**
//! vektorlu isleyicilerdi (VEH), yani `__try`nin karsiligi yoktu.
//!
//! F ayri bir sey olcuyor ve kolayca atlanabilirdi: kayit bulunmasi
//! isleyici cagrilmasi demek **degil**. Kayitlarin cogunda isleyici
//! yoktur -- yalnizca geri sarma bilgisi tasirlar. Bayragi okumayan bir
//! cekirdek, kayittaki ilk kelimeyi kod adresi sanip oraya dallanirdi.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::seh::{self, ExceptionPointers, ExceptionRecord};
use tcmk::winapi::{self, RuntimeFunction, Window};

tcmk::entry!(main);

const BG: u32 = 0x0018_1220;
const PANEL: u32 = 0x0026_1C34;
const FG: u32 = 0x00E8_E2F0;
const DIM: u32 = 0x0092_88A2;
const ACCENT: u32 = 0x00C0_A0FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// `UNWIND_INFO` -- elle kurulmus, en kucuk gecerli hali.
///
/// Gercek derleyici buraya prologun geri sarma kodlarini da yazar;
/// bizim olctugumuz sey dagitim oldugu icin kod sayisi sifir. Duzen
/// yine de ABI'ye uygun olmak zorunda: isleyici RVA'si **kodlardan
/// sonra** gelir ve kod sayisi cifte yuvarlanir.
///
/// Hizalama 4: kayit `DWORD` hizali olmali, yoksa isleyici RVA'si
/// hizasiz okunur.
#[repr(C, align(4))]
struct UnwindInfo {
    /// `Version:3 | Flags:5` -- surum 1, bayraklar ustte.
    version_flags: u8,
    size_of_prolog: u8,
    /// Kac geri sarma kodu var. **Tek** sayi veriliyor ve bu bilincli:
    /// ABI kod sayisini cifte yuvarlar (kayit `DWORD` hizali kalsin
    /// diye), yani tek sayida kodda arada bir **dolgu** slotu kalir.
    /// Cift sayi verseydik yuvarlamayi yapan ile yapmayan cekirdek ayni
    /// yeri okurdu -- yani sinav o kurali hic olcmezdi.
    count_of_codes: u8,
    /// `FrameRegister:4 | FrameOffset:4` -- cerceve registeri yok.
    frame: u8,
    /// Tek geri sarma kodu: `UWOP_PUSH_NONVOL` (RBX).
    ///
    /// Icerigi dagitimi ilgilendirmiyor -- kodlar yalnizca geri sarmada
    /// yurutulur. Burada olmasinin sebebi yerlesim: isleyici RVA'sini
    /// iki bayt oteye itmek.
    code: u16,
    /// Yuvarlamanin biraktigi dolgu. Kernel yuvarlamayi yapmazsa
    /// isleyici RVA'si diye **burayi** okur ve sifir gorur.
    pad: u16,
    handler_rva: u32,
    /// `ExceptionData` -- dil'e ozel. MSVC burada "scope table" tutar;
    /// cekirdek icerigini yorumlamaz, yalnizca adresini gecirir.
    scope: u32,
}

/// Isleyicisi **olan** kayit.
static mut UNWIND_WITH: UnwindInfo = UnwindInfo {
    version_flags: 1 | (winapi::UNW_FLAG_EHANDLER << 3),
    size_of_prolog: 1,
    count_of_codes: 1,
    frame: 0,
    code: 0x3001,
    pad: 0,
    handler_rva: 0,
    scope: 0xABCD_1234,
};

/// Isleyicisi **olmayan** kayit -- yalnizca geri sarma bilgisi.
///
/// F sinavinin butun meselesi: bu kayit da bulunur, ama isleyici
/// cagrilmamali. Bayragi okumayan bir cekirdek `handler_rva` alanindaki
/// degeri kod adresi sanardi.
///
/// Alan kasten **calisan bir isleyicinin** adresiyle dolduruluyor
/// (`wrong_handler`, kurulum sirasinda yaziliyor). Ilk yazilista buraya
/// uydurma bir RVA konmustu ve bozma sinavi F'yi yakalamadi: uydurma
/// deger `boom`un kendisine denk gelmis, "isleyici" yeniden patlamis ve
/// cocuk yine olmustu -- yani F dogru sonucu **yanlis sebeple**
/// veriyordu. Calisan bir isleyici, bayragin gercekten okundugunu
/// olcmenin tek yolu: okunmazsa cocuk hayatta kalir ve bunu soyler.
static mut UNWIND_WITHOUT: UnwindInfo = UnwindInfo {
    version_flags: 1,
    size_of_prolog: 1,
    count_of_codes: 1,
    frame: 0,
    code: 0x3001,
    pad: 0,
    handler_rva: 0,
    scope: 0,
};

static mut TABLE: [RuntimeFunction; 1] = [RuntimeFunction {
    begin: 0,
    end: 0,
    unwind: 0,
}];

/// Isleyici kac kez kostu.
static HANDLER_RAN: AtomicUsize = AtomicUsize::new(0);
/// Sahipsiz istisna filtresi kac kez kostu -- **ag**.
static NET_HITS: AtomicUsize = AtomicUsize::new(0);
/// Isleyicinin `DISPATCHER_CONTEXT`ten okudugu alanlar.
static SEEN_BASE: AtomicUsize = AtomicUsize::new(0);
static SEEN_ENTRY: AtomicUsize = AtomicUsize::new(0);
static SEEN_HANDLER_DATA: AtomicUsize = AtomicUsize::new(0);
/// Hata adresini iceren fonksiyonun kurtarma noktasi.
static RESUME_AT: AtomicUsize = AtomicUsize::new(0);
/// Fonksiyonun tablo ile kapsanan uzunlugu.
///
/// Rust'ta bir fonksiyonun **bittigi** yeri ogrenmenin tasinabilir bir
/// yolu yok. Comert bir aralik veriyoruz: olculen sey zaten "hata adresi
/// bu araliga dusuyor mu", ve aralik genis olsa da hata yalnizca
/// `boom`un icinde uretiliyor.
const SPAN: u32 = 0x200;

/// x64 dil isleyicisi.
///
/// Imza i386'daki zincir isleyicisiyle birebir ayni. Fark dorduncu
/// argumanda: i386'da kullanilmiyordu, burada **kayittir**.
unsafe extern "system" fn language_handler(
    record: *mut ExceptionRecord,
    _establisher: usize,
    context: *mut c_void,
    dispatcher: *mut c_void,
) -> i32 {
    HANDLER_RAN.fetch_add(1, Ordering::SeqCst);
    // Bos isaretci **denetleniyor** ve bu bir uslup tercihi degil.
    // Bozma sinavinda dorduncu arguman sifir gecildiginde isleyici
    // burayi okuyup coktu, dagitim ic ice girdi ve surec oldu -- yani
    // sinav "DISPATCHER_CONTEXT bos geldi" diyecegi yerde **hicbir sey**
    // demedi. Denetim, o sessizligi bir olcuye ceviriyor.
    if dispatcher.is_null() {
        SEEN_BASE.store(usize::MAX, Ordering::SeqCst);
        SEEN_ENTRY.store(usize::MAX, Ordering::SeqCst);
        SEEN_HANDLER_DATA.store(0, Ordering::SeqCst);
    } else {
        SEEN_BASE.store(
            winapi::dispatcher_field(dispatcher, winapi::disp::IMAGE_BASE),
            Ordering::SeqCst,
        );
        SEEN_ENTRY.store(
            winapi::dispatcher_field(dispatcher, winapi::disp::FUNCTION_ENTRY),
            Ordering::SeqCst,
        );
        SEEN_HANDLER_DATA.store(
            winapi::dispatcher_field(dispatcher, winapi::disp::HANDLER_DATA),
            Ordering::SeqCst,
        );
    }
    let _ = record;

    // "Devam et" demenin tek anlamli yolu CONTEXT'i degistirmek: aksi
    // halde ayni komut yeniden kosar ve ayni hatayi verir. Akisi
    // kurtarma noktasina tasiyoruz.
    let resume = RESUME_AT.load(Ordering::SeqCst);
    if resume != 0 {
        seh::set_reg(context, seh::Reg::Ip, resume);
        return seh::EXCEPTION_CONTINUE_EXECUTION_SEH;
    }
    seh::EXCEPTION_CONTINUE_SEARCH_SEH
}

/// Sahipsiz istisna agi.
///
/// Tablo isleyicisi bulunamazsa hata sahipsiz kalir ve surec **oler** --
/// yani sinav rapor vermek yerine susardi. Bu filtre akisi kurtarip
/// sinavin konusmasini sagliyor.
///
/// Dagitim sirasindaki yeri belirleyici: filtre **en sonda** kosuyor,
/// yani tablo isleyicisi varken hic cagrilmiyor. Bir VEH kurmak ayni
/// isi yapmazdi -- o **en basta** kosar ve tablo yolunu hic
/// denenmeden kapatirdi.
unsafe extern "system" fn net(info: *mut ExceptionPointers) -> i32 {
    NET_HITS.fetch_add(1, Ordering::SeqCst);
    let resume = RESUME_AT.load(Ordering::SeqCst);
    if resume != 0 {
        seh::set_reg((*info).context_record, seh::Reg::Ip, resume);
        return seh::EXCEPTION_CONTINUE_EXECUTION;
    }
    winapi::EXCEPTION_EXECUTE_HANDLER
}

/// Cagrilmamasi gereken isleyici.
///
/// `EHANDLER` bayragi olmayan bir kaydin isleyici alaninda duruyor.
/// Cekirdek bayragi okuyorsa buraya hic gelinmez; okumuyorsa burasi
/// akisi kurtarir ve cocuk hayatta kalarak bunu bildirir.
unsafe extern "system" fn wrong_handler(
    _record: *mut ExceptionRecord,
    _establisher: usize,
    context: *mut c_void,
    _dispatcher: *mut c_void,
) -> i32 {
    let resume = RESUME_AT.load(Ordering::SeqCst);
    if resume != 0 {
        seh::set_reg(context, seh::Reg::Ip, resume);
        return seh::EXCEPTION_CONTINUE_EXECUTION_SEH;
    }
    seh::EXCEPTION_CONTINUE_SEARCH_SEH
}

// Hatayi ureten ve kurtarma noktasini tasiyan fonksiyon.
//
// `global_asm!` ile yaziliyor, cunku iki seye birden ihtiyac var:
// **kesin** bir giris adresi ve **kesin** bir kurtarma adresi. Rust
// fonksiyonunun icinde bir etikete adres almak mumkun degil.
core::arch::global_asm!(
    ".globl tcmk_boom",
    "tcmk_boom:",
    // Sifir isaretciye yazma: erisim ihlali.
    "xor rax, rax",
    "mov qword ptr [rax], 1",
    ".globl tcmk_boom_resume",
    "tcmk_boom_resume:",
    "mov eax, 1",
    "ret",
);

extern "C" {
    fn tcmk_boom() -> u32;
    fn tcmk_boom_resume();
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

const NAMES: [&str; 7] = [
    "A tablo kabul edildi",
    "B kayit bulunuyor",
    "C ISLEYICI KOSTU",
    "D dispatcher dolu",
    "E devam et yurudu",
    "F EHANDLER sart",
    "G silinince bitiyor",
];

/// Bir sinavin sonucunu hesaplandigi anda yazar.
fn say(check: &Check) {
    use core::fmt::Write;
    let mut console = winapi::Console;
    let _ = writeln!(
        console,
        "[winpdata] {}: {} ({})",
        check.name,
        if check.passed { "gecti" } else { "KALDI" },
        check.detail
    );
}

fn main() {
    use core::fmt::Write;
    // Cocuk yolu: bayraksiz kayitla ayni hatayi uretir (bkz. F).
    if tcmk::args::count() > 1 && tcmk::args::get(1) == Some(CHILD_ARG) {
        unsafe { winapi::ExitProcess(no_handler_child()) };
    }

    let _ = writeln!(winapi::Console, "[winpdata] sinav basliyor");
    let mut checks = [EMPTY; 7];

    let base = unsafe { winapi::GetModuleHandleA(core::ptr::null()) } as usize;
    let boom_at = tcmk_boom as usize;
    let resume_at = tcmk_boom_resume as usize;
    RESUME_AT.store(resume_at, Ordering::SeqCst);

    // Tabloyu kur: RVA'lar goruntu tabanina gore.
    let table_at = unsafe {
        (*core::ptr::addr_of_mut!(UNWIND_WITH)).handler_rva =
            (language_handler as usize - base) as u32;
        let unwind_rva = (core::ptr::addr_of!(UNWIND_WITH) as usize - base) as u32;
        let table = core::ptr::addr_of_mut!(TABLE) as *mut RuntimeFunction;
        (*table).begin = (boom_at - base) as u32;
        (*table).end = (boom_at - base) as u32 + SPAN;
        (*table).unwind = unwind_rva;
        table as *const RuntimeFunction
    };

    // --- A: tablo kabul edildi ---------------------------------------
    // Ag once kuruluyor: tablo isleyicisi bulunamazsa surec olmesin.
    unsafe { winapi::SetUnhandledExceptionFilter(Some(net)) };

    let added = unsafe { winapi::RtlAddFunctionTable(table_at, 1, base) } != 0;
    checks[0] = Check {
        name: NAMES[0],
        detail: if added {
            "RtlAddFunctionTable kabul etti"
        } else {
            "reddedildi (32-bit mi, yoksa destek yok mu?)"
        },
        passed: added,
    };
    say(&checks[0]);

    // --- B: kayit bulunuyor ------------------------------------------
    //
    // Aramanin kendisi ayri bir yuzey: cekirdek hata aninda ayni isi
    // yapacak, ama burada onu **hatasiz** sorabiliyoruz.
    let mut found_base = 0u64;
    let found = unsafe {
        winapi::RtlLookupFunctionEntry(boom_at + 4, &mut found_base, core::ptr::null_mut())
    };
    let right_entry = found == table_at && found_base as usize == base;
    checks[1] = Check {
        name: NAMES[1],
        detail: if right_entry {
            "dogru girdi ve dogru goruntu tabani"
        } else if found.is_null() {
            "adres hicbir tabloda bulunamadi"
        } else {
            "baska bir girdi ya da taban dondu"
        },
        passed: right_entry,
    };
    say(&checks[1]);

    // Kapsam disi bir adres bulunmamali: aramanin **aralik** baktiginin
    // kaniti. Bulunsaydi arama "her adrese ayni cevap" demek olurdu.
    let outside = unsafe {
        winapi::RtlLookupFunctionEntry(
            boom_at + SPAN as usize + 0x100,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };

    // --- C, D, E: isleyici kostu, dolu bir dispatcher ile ------------
    //
    // `boom` erisim ihlali uretiyor; isleyici CONTEXT'in RIP'ini
    // kurtarma noktasina tasiyip "devam et" diyor. Donus degeri 1
    // olarak geliyorsa akis gercekten oradan surmus demektir.
    let returned = unsafe { tcmk_boom() };
    let ran = HANDLER_RAN.load(Ordering::SeqCst);

    checks[2] = Check {
        name: NAMES[2],
        detail: if ran == 1 {
            "hata adresi tabloda, isleyici cagrildi"
        } else if ran == 0 && NET_HITS.load(Ordering::SeqCst) > 0 {
            "isleyici KOSMADI: hata sahipsiz kaldi, agi buldu"
        } else if ran == 0 {
            "isleyici KOSMADI: tablo dagitima baglanmamis"
        } else {
            "isleyici birden fazla kez kostu"
        },
        passed: ran == 1,
    };
    say(&checks[2]);

    let seen_base = SEEN_BASE.load(Ordering::SeqCst);
    let seen_entry = SEEN_ENTRY.load(Ordering::SeqCst);
    let seen_data = SEEN_HANDLER_DATA.load(Ordering::SeqCst);
    // `HandlerData` isleyici RVA'sinin hemen ardini gostermeli: orada
    // bizim `scope` alanimiz duruyor ve icerigi taninabilir.
    let data_ok = seen_data != 0 && unsafe { (seen_data as *const u32).read_unaligned() } == 0xABCD_1234;
    let disp_ok = seen_base == base && seen_entry == table_at as usize && data_ok;
    checks[3] = Check {
        name: NAMES[3],
        detail: if disp_ok {
            "ImageBase, FunctionEntry ve HandlerData dogru"
        } else if ran == 0 {
            "isleyici kosmadi, okunamadi"
        } else if seen_base == usize::MAX {
            "DISPATCHER_CONTEXT bos geldi"
        } else if seen_base != base {
            "ImageBase yanlis"
        } else if seen_entry != table_at as usize {
            "FunctionEntry kaydin adresini gostermiyor"
        } else {
            "HandlerData dil verisini gostermiyor"
        },
        passed: disp_ok,
    };
    say(&checks[3]);

    checks[4] = Check {
        name: NAMES[4],
        detail: if returned == 1 && ran == 1 {
            "CONTEXT duzeltildi, akis kurtarma noktasindan surdu"
        } else if ran == 0 {
            "akisi kurtaran tablo isleyicisi degildi"
        } else {
            "isleyici kostu ama akis kurtarilamadi"
        },
        passed: returned == 1 && ran == 1,
    };
    say(&checks[4]);

    // --- F: EHANDLER bayragi olmayan kayit ---------------------------
    //
    // Kayit **bulunuyor** ama isleyici cagrilmamali. Kayitlarin cogunda
    // isleyici yoktur; bayragi okumayan bir cekirdek `handler_rva`
    // alanindaki degeri kod adresi sanip oraya dallanirdi.
    // Once: kayit hala **bulunuyor** mu? Bulunmuyorsa F'nin olcusu bos
    // olurdu -- "isleyici kosmadi" ile "kayit yoktu" ayni gorunurdu.
    unsafe {
        let table = core::ptr::addr_of_mut!(TABLE) as *mut RuntimeFunction;
        (*table).unwind = (core::ptr::addr_of!(UNWIND_WITHOUT) as usize - base) as u32;
    }
    let still_found = unsafe {
        winapi::RtlLookupFunctionEntry(boom_at + 4, core::ptr::null_mut(), core::ptr::null_mut())
    } == table_at;
    unsafe {
        let table = core::ptr::addr_of_mut!(TABLE) as *mut RuntimeFunction;
        (*table).unwind = (core::ptr::addr_of!(UNWIND_WITH) as usize - base) as u32;
    }

    // Asil olcum cocukta: bayraksiz kayitla `boom` yine patlayacak ve
    // bu kez kimse sahiplenmeyecek, yani surec olecek. Ebeveynde
    // kosturmak sinavin kendisini bitirirdi.
    let f_code = reap_bounded(spawn_child());
    let f_ok = still_found && f_code == CHILD_DIED;
    checks[5] = Check {
        name: NAMES[5],
        detail: if !still_found {
            "bayraksiz kayit hic bulunamadi, olcu bos"
        } else if f_code == 0 {
            "cocuk cevap vermedi"
        } else if f_ok {
            "kayit bulundu ama isleyici kosmadi"
        } else {
            "bayraksiz kayitta isleyici KOSTU"
        },
        passed: f_ok,
    };
    say(&checks[5]);

    // --- G: silinince arama bitiyor ----------------------------------
    let deleted = unsafe { winapi::RtlDeleteFunctionTable(table_at) } != 0;
    let gone = unsafe {
        winapi::RtlLookupFunctionEntry(boom_at + 4, core::ptr::null_mut(), core::ptr::null_mut())
    }
    .is_null();
    checks[6] = Check {
        name: NAMES[6],
        detail: if deleted && gone && outside.is_null() {
            "silindi, aranmiyor; kapsam disi da bulunmuyor"
        } else if !deleted {
            "RtlDeleteFunctionTable reddetti"
        } else if !gone {
            "silindigi halde hala bulunuyor"
        } else {
            "kapsam disi bir adres de bulunuyor"
        },
        passed: deleted && gone && outside.is_null(),
    };
    say(&checks[6]);

    let passed = checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(winapi::Console, "[winpdata] sonuc: {}/7 gecti", passed);
    let _ = writeln!(
        winapi::Console,
        "[winpdata] taban 0x{:x}  boom 0x{:x}  isleyici kosma: {}  ag: {}",
        base,
        boom_at,
        HANDLER_RAN.load(Ordering::SeqCst),
        NET_HITS.load(Ordering::SeqCst)
    );
    show(&checks);
}

/// Cocugu ayiran arguman.
const CHILD_ARG: &str = "nohandler";

/// Cocugun "sahiplenilmemis istisnayla oldum" cevabi.
///
/// Sahiplenilmeyen bir erisim ihlalinde cikis kodu `STATUS_ACCESS_
/// VIOLATION`dir; cocuk hayatta kalirsa bambaska bir deger doner.
const CHILD_DIED: u32 = 0xC000_0005;

/// Cocuk hayatta kaldi: isleyici kosmus demek -- yani **hata**.
const CHILD_SURVIVED: u32 = 0x77;

/// Cocuk govdesi: bayraksiz bir kayitla ayni hatayi uretir.
///
/// Donerse isleyici kosmus demektir ve bu bir hatadir; donmezse surec
/// zaten olur ve cikis kodu `CHILD_DIED` olur. Yani iki sonuc da
/// **konusuyor**.
fn no_handler_child() -> u32 {
    let base = unsafe { winapi::GetModuleHandleA(core::ptr::null()) } as usize;
    let boom_at = tcmk_boom as usize;
    RESUME_AT.store(tcmk_boom_resume as usize, Ordering::SeqCst);
    let table_at = unsafe {
        // Bayraksiz kaydin isleyici alani **calisan** bir isleyiciyi
        // gosteriyor: cekirdek bayragi okumuyorsa bu kosar, akisi
        // kurtarir ve cocuk hayatta kalir.
        (*core::ptr::addr_of_mut!(UNWIND_WITHOUT)).handler_rva =
            (wrong_handler as usize - base) as u32;
        let table = core::ptr::addr_of_mut!(TABLE) as *mut RuntimeFunction;
        (*table).begin = (boom_at - base) as u32;
        (*table).end = (boom_at - base) as u32 + SPAN;
        (*table).unwind = (core::ptr::addr_of!(UNWIND_WITHOUT) as usize - base) as u32;
        table as *const RuntimeFunction
    };
    if unsafe { winapi::RtlAddFunctionTable(table_at, 1, base) } == 0 {
        return CHILD_SURVIVED;
    }
    unsafe { tcmk_boom() };
    CHILD_SURVIVED
}

/// Ayni ikiliyi `CHILD_ARG` ile baslatir; tutamaci doner (0 = olmadi).
fn spawn_child() -> usize {
    let mut info = winapi::ProcessInformation::new();
    let created = unsafe {
        winapi::CreateProcessA(
            b"C:\\bin\\winpdata.exe\0".as_ptr(),
            b"winpdata.exe nohandler\0".as_ptr(),
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
    if created == 0 {
        0
    } else {
        info.process as usize
    }
}

/// Cocugu **sinirli** sure bekler; donmezse 0 doner.
///
/// Suresiz beklemek burada kullanilamaz: cocuk hayatta kalirsa kendi
/// penceresini acip beklemeye girebilir ve ebeveyn de onunla birlikte
/// asili kalirdi.
fn reap_bounded(child: usize) -> u32 {
    if child == 0 {
        return 0;
    }
    let handle = child as winapi::Handle;
    for _ in 0..60 {
        let mut code = 0u32;
        if unsafe { winapi::GetExitCodeProcess(handle, &mut code) } != 0
            && code != winapi::STILL_ACTIVE
        {
            unsafe { winapi::CloseHandle(handle) };
            return code;
        }
        unsafe { winapi::Sleep(50) };
    }
    unsafe { winapi::CloseHandle(handle) };
    0
}

fn show(checks: &[Check; 7]) {
    let mut win = match Window::create("winpdata -- zincir degil tablo", 250, 140, 500, 250) {
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

fn draw(win: &mut Window, checks: &[Check; 7]) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "x64'te __try hicbir komut uretmez", ACCENT);

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

    win.text(6, h - 46, "i386: zincir yiginda, calisma zamaninda", DIM);
    win.text(6, h - 30, "x64:  tablo ikilide, derleme zamaninda", DIM);

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
