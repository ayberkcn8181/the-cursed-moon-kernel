//! Windows istisna dagitimi -- SEH ve VEH.
//!
//! TEB kurulduktan sonra (bkz. `teb.rs`) bir PE ikilisi `fs:[0]`da bir
//! istisna zinciri **gorebiliyordu**, ama zincir hicbir zaman
//! **yurutulmuyordu**: bir sayfa hatasi surecin sonu demekti. Bu modul o
//! boslugu kapatir.
//!
//! ## Windows'un iki mekanizmasi
//!
//! | | SEH (zincir) | VEH (vektorlu) |
//! |---|---|---|
//! | kayit yeri | `fs:[0]` -- **yiginda** | surec genelinde bir liste |
//! | kim kurar | derleyici (`__try`) | `AddVectoredExceptionHandler` |
//! | isleyici imzasi | `(record, frame, context, dispatcher)` | `(EXCEPTION_POINTERS*)` |
//! | "devam et" | `0` | `-1` |
//! | "sirakine gec" | `1` | `0` |
//! | mimari | yalnizca 32-bit | 32 ve 64-bit |
//!
//! Son iki satir onemli. Ayni anlami tasiyan iki donus degeri **farkli
//! sayilardir** -- bu Windows'un kendi tuhafligidir, TCMK'nin sadelestirmesi
//! degil. Ve x86_64'te zincir yoktur: Microsoft 64-bit'te tablo tabanli
//! (`.pdata`/`.xdata`) cozume gecmistir. TCMK de bu ayrimi aynen tasir:
//! zincir yalnizca i386'da yurutulur, VEH iki mimaride de calisir.
//!
//! ## Dagitim nasil calisiyor
//!
//! Gercek Windows'ta donguyu `ntdll!KiUserExceptionDispatcher` **Ring
//! 3'te** dondurur. TCMK'de dongunun kendisi cekirdektedir; Ring 3'e
//! yalnizca *isleyiciler* girer:
//!
//! ```text
//!   istisna  ->  cekirdek yigina EXCEPTION_RECORD + CONTEXT yazar
//!            ->  cerceveyi isleyiciye cevirir, donus adresi = tramplen
//!            ->  isleyici calisir (Ring 3), EAX/RAX ile karar doner
//!            ->  tramplen int 0x2E ile cekirdege doner
//!            ->  cekirdek ya devam eder ya siradaki isleyiciye gecer
//! ```
//!
//! Tramplen TEB'in ayrilmis alanina yazilan **13 baytlik** bir koddur.
//! Linux'un eski sinyal tramplenleri de aynen boyleydi (yigina yazilan
//! `sigreturn` stub'i): cekirdegin kullanici adres uzayina donus yolu
//! birakmasi disinda bir secenek yok.
//!
//! ## POSIX tarafiyla iliskisi
//!
//! Bu, `level0b1::signal`in yaptigi isin Windows'casidir. Iki yol da
//! "cekirdek kullanici yiginina bir cerceve kurar ve baglami cevirir"
//! desenini kullanir; hatta ayni `UserContext` tipini paylasirlar. Fark
//! cercevenin **bicimi**: POSIX bir sinyal numarasi verir, Windows bir
//! kayit ciftinin adresini.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::arch::cpu::regs::UserContext;
use crate::level0a::core::{mmu, scheduler};

#[cfg(target_arch = "x86_64")]
use super::pdata;
use super::teb;

// --- Windows istisna kodlari (ntstatus.h) -----------------------------
pub const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const STATUS_ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
pub const STATUS_INTEGER_DIVIDE_BY_ZERO: u32 = 0xC000_0094;
pub const STATUS_INTEGER_OVERFLOW: u32 = 0xC000_0095;
pub const STATUS_PRIVILEGED_INSTRUCTION: u32 = 0xC000_0096;
pub const STATUS_ARRAY_BOUNDS_EXCEEDED: u32 = 0xC000_008C;
pub const STATUS_FLOAT_DIVIDE_BY_ZERO: u32 = 0xC000_008E;
pub const STATUS_STACK_OVERFLOW: u32 = 0xC000_00FD;
pub const STATUS_BREAKPOINT: u32 = 0x8000_0003;
pub const STATUS_SINGLE_STEP: u32 = 0x8000_0004;
pub const STATUS_DATATYPE_MISALIGNMENT: u32 = 0x8000_0002;

/// `EXCEPTION_NONCONTINUABLE` -- isleyici "devam et" diyemez.
pub const EXCEPTION_NONCONTINUABLE: u32 = 0x1;

/// **Geri sarma** cagrisi: isleyici bu bayrakla ikinci kez cagriliyor.
///
/// Windows'ta bir isleyicinin iki isi vardir ve ayni fonksiyon ikisini
/// de yapar; hangisinin istendigini yalnizca bu bayrak soyler:
///
/// ```text
///   bayrak yok  ->  "bu istisnayi sahipleniyor musun?"   (__except filtresi)
///   bayrak var  ->  "cerceven yikiliyor, temizligini yap" (__finally)
/// ```
///
/// Derleyicinin `__finally` icin urettigi kod tam olarak bu dalda
/// calisir. Bayrak olmadan `__try`/`__finally` diye bir sey olamaz --
/// yikilan cercevelerin temizligi hic yapilmazdi.
#[cfg(target_arch = "x86")]
pub const EXCEPTION_UNWINDING: u32 = 0x2;

/// Hedefsiz geri sarma: zincirin **tamami** cozuluyor.
#[cfg(target_arch = "x86")]
pub const EXCEPTION_EXIT_UNWIND: u32 = 0x4;

/// Bir isleyicinin **kendisi** cokerse olusan istisna bu bayrakla gelir.
///
/// Windows'un bu bayragi tasimasinin sebebi somut: siradaki isleyici,
/// bakacagi kaydin "asil hata" mi yoksa "hatayi inceleyen kodun kendi
/// hatasi" mi oldugunu bilmek zorunda. Ikisi ayni sekilde ele
/// alinamaz -- ikincisinde zaten bir dagitim suruyor.
pub const EXCEPTION_NESTED_CALL: u32 = 0x10;

/// Geri sarmanin kendi istisna kodu (`STATUS_UNWIND`).
#[cfg(target_arch = "x86")]
const STATUS_UNWIND: u32 = 0xC000_0027;

// --- Isleyici donus degerleri -----------------------------------------
//
// Ikisi ayni anlami tasiyip farkli sayilar kullanir; bkz. modul basligi.
//
// Karar 32 bitlik bir `LONG` olarak doner; x86_64'te bile `mov eax, -1`
// ust yariyi sifirladigi icin karsilastirma **32 bit uzerinden** yapilir.
// Aksi halde 64-bit'te -1 hicbir zaman eslesmezdi.
const EXCEPTION_CONTINUE_EXECUTION: u32 = 0xFFFF_FFFF; // VEH: -1
#[allow(dead_code)]
const EXCEPTION_CONTINUE_SEARCH: u32 = 0; // VEH: 0
const DISPOSITION_CONTINUE_EXECUTION: u32 = 0; // SEH zinciri
#[allow(dead_code)]
const DISPOSITION_CONTINUE_SEARCH: u32 = 1; // SEH zinciri

/// Surec basina en fazla kac vektorlu isleyici tutulur.
///
/// Gercek Windows'ta sinir yok (baglantili liste). Burada sabit dizi
/// kullaniliyor cunku cekirdekte surec basina dinamik tahsis yapmamak
/// TCMK'nin genel tercihi.
pub const MAX_VEH: usize = 4;

/// Zincirde en fazla kac kayit yurunur -- bozuk/donguyle bagli bir
/// zincirin cekirdegi sonsuz donguye sokmasini engeller.
const MAX_CHAIN: usize = 16;

/// Zincirin **sonundaki** son savunma hatti.
///
/// Windows'ta hicbir isleyici sahiplenmezse `UnhandledExceptionFilter`
/// calisir; programlar oraya bir cokme raporlayicisi takar
/// (`SetUnhandledExceptionFilter`). Donus degeri ne yapilacagini soyler:
///
/// ```text
///   EXCEPTION_EXECUTE_HANDLER    (1)  -> surec sonlansin
///   EXCEPTION_CONTINUE_SEARCH    (0)  -> varsayilan davranis (yine sonlanma)
///   EXCEPTION_CONTINUE_EXECUTION (-1) -> yurutme surdurulsun
/// ```
///
/// Ucuncusu, filtrenin CONTEXT'i duzeltip sureci kurtarabilmesi demek --
/// yani filtre siradan bir isleyici gibi de davranabilir.
const EXCEPTION_EXECUTE_HANDLER: u32 = 1;

// --- Kullanici cercevesinin olculeri ----------------------------------
#[cfg(target_arch = "x86")]
mod sizes {
    /// `EXCEPTION_RECORD`: 0x14 sabit alan + 15 * 4 parametre.
    pub const RECORD: usize = 0x50;
    /// `CONTEXT` (x86): 716 bayt. Bu sayi Windows ABI'sinin parcasidir.
    pub const CONTEXT: usize = 0x2CC;
    /// Isleyiciye kurulan yigin cercevesi icin ayrilan yer.
    pub const FRAME: usize = RECORD + CONTEXT + 0x80;
}

#[cfg(target_arch = "x86_64")]
mod sizes {
    /// `EXCEPTION_RECORD` (x64): isaretciler 8 bayt oldugu icin daha genis.
    pub const RECORD: usize = 0x98;
    /// `CONTEXT` (x64): 1232 bayt.
    pub const CONTEXT: usize = 0x4D0;
    /// `DISPATCHER_CONTEXT`: yalnizca x64'te var ve **kayittir**.
    ///
    /// i386'da isleyicinin dorduncu argumani cekirdege aitti ve TCMK
    /// sifir geciyordu. x64'te tablo tabanli cozumun kendisi oradan
    /// okunuyor: goruntu tabani, fonksiyonun `RUNTIME_FUNCTION`u ve dil
    /// verisi. Bos gecmek, MSVC'nin urettigi bir isleyiciyi ilk
    /// adiminda cop okumaya gondermek olurdu.
    pub const DISPATCHER: usize = super::pdata::DISPATCHER_SIZE;
    pub const FRAME: usize = RECORD + CONTEXT + DISPATCHER + 0x100;
}

// --- EXCEPTION_RECORD alan ofsetleri ----------------------------------
#[cfg(target_arch = "x86")]
mod rec {
    pub const CODE: usize = 0x00;
    pub const FLAGS: usize = 0x04;
    pub const NESTED: usize = 0x08;
    pub const ADDRESS: usize = 0x0C;
    pub const PARAM_COUNT: usize = 0x10;
    pub const PARAMS: usize = 0x14;
}

#[cfg(target_arch = "x86_64")]
mod rec {
    pub const CODE: usize = 0x00;
    pub const FLAGS: usize = 0x04;
    pub const NESTED: usize = 0x08;
    pub const ADDRESS: usize = 0x10;
    pub const PARAM_COUNT: usize = 0x18;
    pub const PARAMS: usize = 0x20;
}

// --- CONTEXT alan ofsetleri -------------------------------------------
//
// Bu sayilar da derlenmis Windows kodunun icine gomuludur: bir isleyici
// `context->Eip`i duzeltmek istediginde tam olarak bu ofsete yazar.
#[cfg(target_arch = "x86")]
mod ctx {
    pub const FLAGS: usize = 0x00;
    pub const SEG_GS: usize = 0x8C;
    pub const SEG_FS: usize = 0x90;
    pub const SEG_ES: usize = 0x94;
    pub const SEG_DS: usize = 0x98;
    pub const EDI: usize = 0x9C;
    pub const ESI: usize = 0xA0;
    pub const EBX: usize = 0xA4;
    pub const EDX: usize = 0xA8;
    pub const ECX: usize = 0xAC;
    pub const EAX: usize = 0xB0;
    pub const EBP: usize = 0xB4;
    pub const EIP: usize = 0xB8;
    pub const SEG_CS: usize = 0xBC;
    pub const EFLAGS: usize = 0xC0;
    pub const ESP: usize = 0xC4;
    pub const SEG_SS: usize = 0xC8;
    /// `CONTEXT_i386 | CONTROL | INTEGER | SEGMENTS`
    pub const FULL: u32 = 0x0001_0007;
    /// Bir baglami **yazmak** icin en az bu kume istenir.
    ///
    /// `ContextFlags` "bu kayittaki hangi bolumler gecerli" demek. TCMK
    /// bolum bolum uygulamiyor; o yuzden eksik bir kume kabul edilseydi
    /// cagiranin hic doldurmadigi registerlar sifirla ezilirdi. Kumeyi
    /// istemek, o sessiz hasarin yerine bir hata koyuyor.
    pub const REQUIRED: u32 = 0x0001_0003;
}

#[cfg(target_arch = "x86_64")]
mod ctx {
    pub const FLAGS: usize = 0x30;
    pub const EFLAGS: usize = 0x44;
    pub const RAX: usize = 0x78;
    pub const RCX: usize = 0x80;
    pub const RDX: usize = 0x88;
    pub const RBX: usize = 0x90;
    pub const RSP: usize = 0x98;
    pub const RBP: usize = 0xA0;
    pub const RSI: usize = 0xA8;
    pub const RDI: usize = 0xB0;
    pub const R8: usize = 0xB8;
    pub const R9: usize = 0xC0;
    pub const R10: usize = 0xC8;
    pub const R11: usize = 0xD0;
    pub const R12: usize = 0xD8;
    pub const R13: usize = 0xE0;
    pub const R14: usize = 0xE8;
    pub const R15: usize = 0xF0;
    pub const RIP: usize = 0xF8;
    /// `CONTEXT_AMD64 | CONTROL | INTEGER | SEGMENTS`
    pub const FULL: u32 = 0x0010_0007;
    /// i386'daki ikiziyle ayni gerekce (bkz. orada).
    pub const REQUIRED: u32 = 0x0010_0003;
}

// --- Gorev basina dagitim durumu --------------------------------------

/// Sahipsiz istisna filtresi (`SetUnhandledExceptionFilter`).
static FILTER: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Filtre **calisti mi**? Iki kez cagirmamak icin: filtre de sahiplenmezse
/// surec sonlanmali, yoksa dongu olusurdu.
static FILTER_RAN: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Vektorlu isleyiciler; sifir = bos yuva.
static VEH: [[AtomicUsize; MAX_VEH]; scheduler::MAX_TASKS] =
    [const { [const { AtomicUsize::new(0) }; MAX_VEH] }; scheduler::MAX_TASKS];

/// Su an bir istisna dagitiliyor mu (0 = hayir).
static ACTIVE: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

// --- Ic ice dagitim --------------------------------------------------
//
// Bir istisna dagitilirken **isleyicinin kendisi** cokerse ne olur?
// Uzun sure TCMK'nin cevabi "surec biter" idi ve bu, muhafazakar ama
// pahali bir cevapti: tek bir hatali isleyici butun sureci goturuyordu.
// Windows'un cevabi baska -- dagitim siradaki isleyiciyle **surer**.
//
// Iki kural ayrimi tasiyor:
//
//   * Yurume bastan baslamaz. Baslasaydi coken isleyici yeniden
//     cagrilir ve sonsuz donguye girilirdi.
//   * Ic dagitim cozuldugunde **dis** kayit geri gelir: kalan
//     isleyiciler asil hatayi gormeli, isleyicinin hatasini degil.

/// En fazla kac katman ic ice dagitim.
///
/// Ikiden derini pratikte hatali isleyicilerin zinciri demek; sinir
/// olmasaydi her katman yiginda bir cerceve daha tuketir ve sonunda
/// yigin tasardi. Sinira dayanilinca surec sonlaniyor (eski davranis).
const MAX_NESTED_DISPATCH: usize = 2;

/// Kac katman ic ice (0 = ic ice degil).
static NESTED_DEPTH: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Ic ice girilirken saklanan **dis** dagitimin kayitlari.
static OUTER_RECORD: [[AtomicUsize; MAX_NESTED_DISPATCH]; scheduler::MAX_TASKS] =
    [const { [const { AtomicUsize::new(0) }; MAX_NESTED_DISPATCH] }; scheduler::MAX_TASKS];
static OUTER_CONTEXT: [[AtomicUsize; MAX_NESTED_DISPATCH]; scheduler::MAX_TASKS] =
    [const { [const { AtomicUsize::new(0) }; MAX_NESTED_DISPATCH] }; scheduler::MAX_TASKS];
static OUTER_POINTERS: [[AtomicUsize; MAX_NESTED_DISPATCH]; scheduler::MAX_TASKS] =
    [const { [const { AtomicUsize::new(0) }; MAX_NESTED_DISPATCH] }; scheduler::MAX_TASKS];
static OUTER_FLAGS: [[AtomicUsize; MAX_NESTED_DISPATCH]; scheduler::MAX_TASKS] =
    [const { [const { AtomicUsize::new(0) }; MAX_NESTED_DISPATCH] }; scheduler::MAX_TASKS];

/// Olcum: kac ic ice dagitim oldu.
static NESTED_DISPATCHES: AtomicUsize = AtomicUsize::new(0);

pub fn nested_dispatches() -> usize {
    NESTED_DISPATCHES.load(Ordering::Relaxed)
}

// --- Geri sarma (`RtlUnwind`) durumu ----------------------------------
//
// Dagitimdan **ayri** bir evre ve ayri tutulmasi sart: geri sarma bir
// dagitimin *icinden* baslatilabiliyor (`__except`in yaptigi tam olarak
// budur) ve bittiginde dagitim kaldigi yerden surmeli.

/// Geri sarma suruyor mu (0 = hayir).
static UNWINDING: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
/// Hedef kayit: yurume buraya gelince durur ve `fs:[0]` buna cekilir.
static UNWIND_TARGET: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
/// Yurumede siradaki kayit.
static UNWIND_NEXT: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
/// Geri sarma kayitlarinin kullanici yiginindaki tabani.
#[cfg(target_arch = "x86")]
static UNWIND_BASE: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
/// Geri sarmaya ozel `EXCEPTION_RECORD`in adresi.
#[cfg(target_arch = "x86")]
static UNWIND_RECORD_AT: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
/// Kac adim atildi (bozuk zincire karsi).
static UNWIND_STEPS: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Geri sarma bittiginde donulecek baglam.
///
/// `RtlUnwind`in sozlesmesi burada: yurutme **cagiranin** baglamiyla
/// surer. Hedef adres verilmisse yalnizca `EIP` degisir, verilmemisse o
/// bile degismez -- yani cagri siradan bir cagri gibi doner.
#[cfg(target_arch = "x86")]
static mut UNWIND_RESUME: [UserContext; scheduler::MAX_TASKS] =
    [UserContext::ZERO; scheduler::MAX_TASKS];

/// Kac geri sarma yapildi, kac isleyici `EXCEPTION_UNWINDING` ile
/// cagrildi -- kabuktaki `faults` raporu.
static UNWINDS: AtomicUsize = AtomicUsize::new(0);
static FINALLY_CALLS: AtomicUsize = AtomicUsize::new(0);

/// Ring 3'teki EXCEPTION_RECORD / CONTEXT adresleri.
static RECORD_AT: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
static CONTEXT_AT: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
static POINTERS_AT: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Yurumede kalinan yer: once vektorlu liste, sonra zincir (i386) ya da
/// islev tablosu (x86_64).
const PHASE_VECTORED: usize = 0;
const PHASE_CHAIN: usize = 1;
/// Sahipsiz istisna filtresi calisiyor.
const PHASE_FILTER: usize = 2;
static PHASE: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
static NEXT_VEH: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
static NEXT_RECORD: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
/// Zincirde kac adim atildi (dongu koruyucusu).
static CHAIN_STEPS: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// x86_64 yurumesinin **sanal olarak geri sarilmis** durumu.
///
/// i386'da bu diziye gerek yok: zincir yiginda duruyor ve "siradaki"
/// bir isaretci okumakla bulunuyor. x64'te siradaki cerceveyi bulmak
/// bir hesap gerektiriyor, ve o hesap isleyici cagrilari **arasinda**
/// korunmak zorunda -- her isleyici cekirdege geri donuyor ve yurume
/// oradan devam ediyor.
#[cfg(target_arch = "x86_64")]
static mut UNWIND_STATE: [pdata::UnwindState; scheduler::MAX_TASKS] =
    [pdata::UnwindState {
        rip: 0,
        rsp: 0,
        regs: [0; 16],
    }; scheduler::MAX_TASKS];
/// Yurume durumu kuruldu mu (0 = henuz baslamadi).
#[cfg(target_arch = "x86_64")]
static UNWIND_READY: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
/// Kac cerceve **sanal olarak** geri sarildi (olcum).
#[cfg(target_arch = "x86_64")]
static FRAMES_UNWOUND: AtomicUsize = AtomicUsize::new(0);

/// Sanal geri sarilan cerceve sayisi -- kabuk raporu.
#[cfg(target_arch = "x86_64")]
pub fn frames_unwound() -> usize {
    FRAMES_UNWOUND.load(Ordering::Relaxed)
}
/// Dagitilan istisnanin bayraklari -- `EXCEPTION_NONCONTINUABLE` burada.
static FLAGS: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Olcum sayaclari -- kabuktaki `faults` komutu bunlari gosterir.
static DISPATCHED: AtomicUsize = AtomicUsize::new(0);
static CONTINUED: AtomicUsize = AtomicUsize::new(0);
static UNHANDLED: AtomicUsize = AtomicUsize::new(0);

pub fn dispatched() -> usize {
    DISPATCHED.load(Ordering::Relaxed)
}

pub fn continued() -> usize {
    CONTINUED.load(Ordering::Relaxed)
}

pub fn unhandled() -> usize {
    UNHANDLED.load(Ordering::Relaxed)
}

/// Gorevin butun istisna durumunu sifirlar (yeni imaj, `fork` sonrasi
/// cocuk, gorev sonlanmasi).
pub fn reset(task: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    for slot in &VEH[task] {
        slot.store(0, Ordering::Relaxed);
    }
    ACTIVE[task].store(0, Ordering::Relaxed);
    NESTED_DEPTH[task].store(0, Ordering::Relaxed);
    PHASE[task].store(PHASE_VECTORED, Ordering::Relaxed);
    FILTER[task].store(0, Ordering::Relaxed);
    FILTER_RAN[task].store(0, Ordering::Relaxed);
    // Geri sarma durumu da yuvaya bagli: yarida kalmis bir yurume yeni
    // imajin zincirine devam ediyormus gibi gorunurdu.
    UNWINDING[task].store(0, Ordering::Relaxed);
    UNWIND_TARGET[task].store(0, Ordering::Relaxed);
    UNWIND_NEXT[task].store(0, Ordering::Relaxed);
    UNWIND_STEPS[task].store(0, Ordering::Relaxed);
    #[cfg(target_arch = "x86_64")]
    UNWIND_READY[task].store(0, Ordering::Relaxed);
}

/// `SetUnhandledExceptionFilter`. Doner: **onceki** filtre (Windows'un
/// sozlesmesi; programlar zincirlemek icin onu saklar).
pub fn set_filter(task: usize, handler: usize) -> usize {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    FILTER[task].swap(handler, Ordering::Relaxed)
}

/// `AddVectoredExceptionHandler`. `first` sifirdan farkliysa isleyici
/// listenin **basina** eklenir -- Windows'un sozlesmesi bu.
///
/// Doner: isleyici tanitici (basitce isleyicinin kendi adresi; gercek
/// Windows da opak bir isaretci dondurur) ya da yer yoksa sifir.
pub fn add_vectored(task: usize, first: bool, handler: usize) -> usize {
    if task >= scheduler::MAX_TASKS || handler == 0 {
        return 0;
    }
    let table = &VEH[task];
    if first {
        // Basa ekleme: dolu yuvalari bir saga kaydir.
        if table[MAX_VEH - 1].load(Ordering::Relaxed) != 0 {
            return 0;
        }
        for i in (1..MAX_VEH).rev() {
            let prev = table[i - 1].load(Ordering::Relaxed);
            table[i].store(prev, Ordering::Relaxed);
        }
        table[0].store(handler, Ordering::Relaxed);
        return handler;
    }
    for slot in table {
        if slot.load(Ordering::Relaxed) == 0 {
            slot.store(handler, Ordering::Relaxed);
            return handler;
        }
    }
    0
}

/// `RemoveVectoredExceptionHandler`. Doner: kaldirildi mi.
pub fn remove_vectored(task: usize, handle: usize) -> bool {
    if task >= scheduler::MAX_TASKS || handle == 0 {
        return false;
    }
    let table = &VEH[task];
    let mut found = None;
    for (i, slot) in table.iter().enumerate() {
        if slot.load(Ordering::Relaxed) == handle {
            found = Some(i);
            break;
        }
    }
    let Some(index) = found else { return false };
    // Bosluk birakmadan kaydir: sira **anlamlidir**, isleyiciler ekleme
    // sirasiyla cagrilir.
    for i in index..MAX_VEH - 1 {
        let next = table[i + 1].load(Ordering::Relaxed);
        table[i].store(next, Ordering::Relaxed);
    }
    table[MAX_VEH - 1].store(0, Ordering::Relaxed);
    true
}

/// CPU istisna vektorunu Windows istisna koduna cevirir.
///
/// Esleme gercek Windows'un `KiTrap*` tablosuyla ayni: ornegin bir genel
/// koruma hatasi da erisim ihlali olarak raporlanir, cunku Win32
/// programlari `0xC0000005` bekler.
fn code_for(vector: usize) -> u32 {
    match vector {
        0 => STATUS_INTEGER_DIVIDE_BY_ZERO,
        1 => STATUS_SINGLE_STEP,
        3 => STATUS_BREAKPOINT,
        4 => STATUS_INTEGER_OVERFLOW,
        5 => STATUS_ARRAY_BOUNDS_EXCEEDED,
        6 => STATUS_ILLEGAL_INSTRUCTION,
        8 | 12 => STATUS_STACK_OVERFLOW,
        13 => STATUS_PRIVILEGED_INSTRUCTION,
        14 => STATUS_ACCESS_VIOLATION,
        16 | 19 => STATUS_FLOAT_DIVIDE_BY_ZERO,
        17 => STATUS_DATATYPE_MISALIGNMENT,
        _ => STATUS_ILLEGAL_INSTRUCTION,
    }
}

/// Bir bellek araliginin tamami Ring 3'e acik mi?
fn writable(from: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let mut page = from & !0xFFF;
    let last = (from + len - 1) & !0xFFF;
    loop {
        if !mmu::is_user_or_demand(page) {
            return false;
        }
        if page == last {
            return true;
        }
        page += 0x1000;
    }
}

/// Bir CPU istisnasini Windows'a devretmeyi dener.
///
/// Doner: `true` ise cerceve bir isleyiciye cevrildi ve cagiran donmeli;
/// `false` ise dagitilacak kimse yok, istisna olumcul.
///
/// # Safety
/// Cagiran gorevin adres uzayi etkin olmali ve `frame` Ring 3'ten gelen
/// gecerli bir istisna cercevesi olmalidir.
pub unsafe fn dispatch(
    frame: &mut crate::arch::cpu::regs::ExceptionFrame,
    vector: usize,
    error_code: usize,
    fault_addr: usize,
) -> bool {
    let task = scheduler::current_id();
    // Erisim ihlalinde Windows iki parametre verir: [0] = erisim turu
    // (0 okuma, 1 yazma), [1] = hedef adres. Hata kodunun 1. biti tam
    // olarak bu ayrimi tasiyor.
    let params: [usize; 2] = if vector == 14 {
        [(error_code >> 1) & 1, fault_addr]
    } else {
        [0, 0]
    };
    let count = if vector == 14 { 2 } else { 0 };

    let context = frame.user_context();
    match begin(task, &context, code_for(vector), 0, context.instruction_pointer(), &params[..count])
    {
        Some(redirected) => {
            frame.set_user_context(&redirected);
            true
        }
        None => false,
    }
}

/// `RaiseException` -- yazilim kaynakli istisna.
///
/// Donanim istisnasindan tek farki kodun **programdan** gelmesidir;
/// dagitim yolu birebir aynidir. Windows'ta da oyle: `RaiseException`
/// `RtlRaiseException`e, o da ayni dagiticiya gider.
///
/// # Safety
/// `frame` Ring 3'ten gelen gecerli bir syscall cercevesi olmalidir.
pub unsafe fn raise(
    frame: &mut crate::arch::cpu::regs::SyscallFrame,
    from_interrupt: bool,
    code: u32,
    flags: u32,
    params: &[usize],
) -> bool {
    let task = scheduler::current_id();
    let context = frame.user_context_via(from_interrupt);
    match begin(task, &context, code, flags, context.instruction_pointer(), params) {
        Some(redirected) => {
            frame.set_user_context_via(from_interrupt, &redirected);
            true
        }
        None => false,
    }
}

/// Dagitimi baslatir: kayitlari kullanici yigina yazar ve **ilk**
/// isleyiciye cevrilmis baglami doner.
unsafe fn begin(
    task: usize,
    context: &UserContext,
    code: u32,
    flags: u32,
    address: usize,
    params: &[usize],
) -> Option<UserContext> {
    if task >= scheduler::MAX_TASKS {
        return None;
    }
    // TEB yoksa bu bir PE degil -- POSIX sureclerinde istisna yolu
    // sinyaldir, buraya hic gelinmemeli.
    let teb_at = teb::address(task);
    if teb_at == 0 {
        return None;
    }
    // Dagitim sirasinda cikan istisna: **isleyicinin kendisi** cokmus.
    // Yurume bastan baslamiyor, siradaki isleyiciyle suruyor -- bastan
    // baslasaydi coken isleyici yeniden cagrilir ve dongu olusurdu.
    let nested = ACTIVE[task].load(Ordering::Relaxed) != 0;
    let depth = NESTED_DEPTH[task].load(Ordering::Relaxed);
    if nested && depth >= MAX_NESTED_DISPATCH {
        // Ust uste coken isleyiciler: her katman yiginda bir cerceve
        // daha tuketiyor. Burada durup sureci sonlandirmak, yigini
        // tuketip tanisiz colmekten iyidir.
        return None;
    }

    // --- Kullanici yiginina yer ac ---
    let sp = context.stack_pointer();
    if sp < sizes::FRAME {
        return None;
    }
    let base = (sp - sizes::FRAME) & !0xF;
    if !writable(base, sizes::FRAME) {
        return None;
    }

    let record_at = base;
    let context_at = (record_at + sizes::RECORD + 0xF) & !0xF;
    let pointers_at = context_at + sizes::CONTEXT;

    // Ic ice ise kayit iki sey daha tasiyor: bayrakta
    // `EXCEPTION_NESTED_CALL` ve `ExceptionRecord` alaninda **dis**
    // kaydin adresi. Ikincisi alanin varlik sebebi: siradaki isleyici
    // "asil hata neydi" sorusunu oradan cevapliyor.
    let outer_record = RECORD_AT[task].load(Ordering::Relaxed);
    let flags = if nested {
        flags | EXCEPTION_NESTED_CALL
    } else {
        flags
    };

    core::ptr::write_bytes(record_at as *mut u8, 0, sizes::RECORD);
    ((record_at + rec::CODE) as *mut u32).write_unaligned(code);
    ((record_at + rec::FLAGS) as *mut u32).write_unaligned(flags);
    ((record_at + rec::NESTED) as *mut usize)
        .write_unaligned(if nested { outer_record } else { 0 });
    ((record_at + rec::ADDRESS) as *mut usize).write_unaligned(address);
    let count = params.len().min(15);
    ((record_at + rec::PARAM_COUNT) as *mut u32).write_unaligned(count as u32);
    for (i, value) in params.iter().take(count).enumerate() {
        ((record_at + rec::PARAMS + i * core::mem::size_of::<usize>()) as *mut usize)
            .write_unaligned(*value);
    }

    write_context(context_at, context);

    // EXCEPTION_POINTERS: yalnizca iki isaretci. VEH isleyicisi
    // dogrudan bunu alir.
    (pointers_at as *mut usize).write_unaligned(record_at);
    ((pointers_at + core::mem::size_of::<usize>()) as *mut usize).write_unaligned(context_at);

    // Ic ice girilirken dis dagitimin kayitlari saklaniyor: ic dagitim
    // cozuldugunde geri gelecekler (bkz. `continue_dispatch`). Kalan
    // isleyicilerin asil hatayi gormesi buna bagli.
    if nested {
        OUTER_RECORD[task][depth].store(outer_record, Ordering::Relaxed);
        OUTER_CONTEXT[task][depth].store(CONTEXT_AT[task].load(Ordering::Relaxed), Ordering::Relaxed);
        OUTER_POINTERS[task][depth].store(POINTERS_AT[task].load(Ordering::Relaxed), Ordering::Relaxed);
        OUTER_FLAGS[task][depth].store(FLAGS[task].load(Ordering::Relaxed), Ordering::Relaxed);
        NESTED_DEPTH[task].store(depth + 1, Ordering::Relaxed);
        NESTED_DISPATCHES.fetch_add(1, Ordering::Relaxed);
    }

    RECORD_AT[task].store(record_at, Ordering::Relaxed);
    CONTEXT_AT[task].store(context_at, Ordering::Relaxed);
    POINTERS_AT[task].store(pointers_at, Ordering::Relaxed);
    FLAGS[task].store(flags as usize, Ordering::Relaxed);
    // Yurume durumu **ic ice degilse** sifirlaniyor. Ic ice ise oldugu
    // gibi kaliyor ve dagitim siradaki isleyiciyle suruyor.
    if !nested {
        PHASE[task].store(PHASE_VECTORED, Ordering::Relaxed);
        FILTER_RAN[task].store(0, Ordering::Relaxed);
        NEXT_VEH[task].store(0, Ordering::Relaxed);
        NEXT_RECORD[task].store(chain_head(teb_at), Ordering::Relaxed);
        CHAIN_STEPS[task].store(0, Ordering::Relaxed);
        // x64 yurumesi de bastan kuruluyor. Unutmak sessiz ve olumcul
        // bir hataydi: **ikinci** dagitim, birincinin biraktigi
        // cerceveden devam ediyor ve o cerceve coktan yok oluyor.
        // Sinav bunu ilk kosumda yakaladi -- D geciyor, E ondan sonra
        // geldigi icin bambaska bir adreste patliyordu.
        #[cfg(target_arch = "x86_64")]
        UNWIND_READY[task].store(0, Ordering::Relaxed);
    }
    ACTIVE[task].store(1, Ordering::Relaxed);

    match advance(task, base) {
        Some(next) => {
            DISPATCHED.fetch_add(1, Ordering::Relaxed);
            Some(next)
        }
        None => {
            ACTIVE[task].store(0, Ordering::Relaxed);
            NESTED_DEPTH[task].store(0, Ordering::Relaxed);
            None
        }
    }
}

/// Ic dagitim cozuldu: **dis** dagitimin kayitlari geri geliyor.
///
/// Doner: geri donulecek bir dis dagitim var miydi.
fn pop_nested(task: usize) -> bool {
    let depth = NESTED_DEPTH[task].load(Ordering::Relaxed);
    if depth == 0 {
        return false;
    }
    let depth = depth - 1;
    RECORD_AT[task].store(OUTER_RECORD[task][depth].load(Ordering::Relaxed), Ordering::Relaxed);
    CONTEXT_AT[task].store(OUTER_CONTEXT[task][depth].load(Ordering::Relaxed), Ordering::Relaxed);
    POINTERS_AT[task].store(OUTER_POINTERS[task][depth].load(Ordering::Relaxed), Ordering::Relaxed);
    FLAGS[task].store(OUTER_FLAGS[task][depth].load(Ordering::Relaxed), Ordering::Relaxed);
    NESTED_DEPTH[task].store(depth, Ordering::Relaxed);
    true
}

/// SEH zincirinin basi: `fs:[0]` (i386) -- x86_64'te zincir yok.
fn chain_head(teb_at: usize) -> usize {
    #[cfg(target_arch = "x86")]
    {
        unsafe { (teb_at as *const usize).read_unaligned() }
    }
    #[cfg(target_arch = "x86_64")]
    {
        // Windows x64'te `NtTib.ExceptionList` alani vardir ama
        // **kullanilmaz**: 64-bit'te cozum tablo tabanlidir. Ayni ayrimi
        // koruyoruz, yoksa 64-bit bir PE'nin o alanda tuttugu baska bir
        // veri kod adresi sanilirdi.
        let _ = teb_at;
        usize::MAX
    }
}

/// Siradaki isleyiciyi secer ve ona cevrilmis baglami doner.
///
/// `base` -- kullanici yigininda kayitlar icin ayrilan blogun tabani;
/// isleyici cercevesi bunun **altina** kurulur.
unsafe fn advance(task: usize, base: usize) -> Option<UserContext> {
    let pointers_at = POINTERS_AT[task].load(Ordering::Relaxed);
    let record_at = RECORD_AT[task].load(Ordering::Relaxed);
    let context_at = CONTEXT_AT[task].load(Ordering::Relaxed);

    // --- 1. Vektorlu isleyiciler ---
    if PHASE[task].load(Ordering::Relaxed) == PHASE_VECTORED {
        loop {
            let index = NEXT_VEH[task].load(Ordering::Relaxed);
            if index >= MAX_VEH {
                PHASE[task].store(PHASE_CHAIN, Ordering::Relaxed);
                break;
            }
            NEXT_VEH[task].store(index + 1, Ordering::Relaxed);
            let handler = VEH[task][index].load(Ordering::Relaxed);
            if handler == 0 {
                continue;
            }
            return build_frame(task, base, handler, &[pointers_at]);
        }
    }

    // --- 2a. Islev tablosu (yalnizca x86_64) ---
    //
    // i386'nin zincirinin yerini tutan sey. Aradaki fark yalnizca
    // **bulma** yolu: zincir yiginda gezilir, tablo ikilide aranir.
    // Bulunduktan sonrasi ortak -- ayni imza, ayni karar kumesi.
    #[cfg(target_arch = "x86_64")]
    {
        if let Some(next) = table_handler(task, base, record_at, context_at) {
            return Some(next);
        }
        return last_resort(task, base, pointers_at);
    }

    // --- 2b. SEH zinciri (yalnizca i386) ---
    #[cfg(target_arch = "x86")]
    loop {
        let record = NEXT_RECORD[task].load(Ordering::Relaxed);
        if record == usize::MAX || record == 0 {
            return last_resort(task, base, pointers_at);
        }
        let steps = CHAIN_STEPS[task].fetch_add(1, Ordering::Relaxed);
        if steps >= MAX_CHAIN {
            return last_resort(task, base, pointers_at);
        }
        // Kayit iki kelimedir: {Next, Handler}. Yiginda durur, yani
        // surec onu bozmus olabilir -- okumadan once dogrula.
        let word = core::mem::size_of::<usize>();
        if !writable(record, word * 2) {
            return last_resort(task, base, pointers_at);
        }
        let next = (record as *const usize).read_unaligned();
        let handler = ((record + word) as *const usize).read_unaligned();
        NEXT_RECORD[task].store(next, Ordering::Relaxed);
        if handler == 0 {
            continue;
        }
        // SEH imzasi: (ExceptionRecord, EstablisherFrame, ContextRecord,
        // DispatcherContext). `EstablisherFrame` kaydin kendi adresidir --
        // isleyici yerel degiskenlerine oradan ulasir.
        return build_frame(task, base, handler, &[record_at, record, context_at, 0]);
    }
}

// --- Geri sarma: `RtlUnwind` ------------------------------------------

/// `RtlUnwind(TargetFrame, TargetIp, ExceptionRecord, ReturnValue)`.
///
/// Dagitimin **ikinci yarisi** ve uzun sure eksik olan yari. Dagitim
/// "bu istisnayi kim sahipleniyor" sorusunu cevapliyordu; geri sarma
/// ondan sonra gelen soruyu cevapliyor: **aradaki cerceveler ne olacak?**
///
/// ```text
///   __try  {  __try { patlar }  __finally { A }  }  __except { B }
///
///   1. dagitim   ic isleyici  -> "sahiplenmiyorum"
///                dis isleyici -> "sahipleniyorum" -> RtlUnwind
///   2. GERI SARMA ic isleyici  -> EXCEPTION_UNWINDING ile CAGRILIR -> A kosar
///   3. hedef      fs:[0] dis kayda cekilir, yurutme B'ye gecer
/// ```
///
/// Ikinci satir olmadan `A` hic kosmaz. Derleyicinin `__finally` icin
/// urettigi kod tam olarak orada durur, yani TCMK bu satir olmadan
/// `__try`/`__finally` iceren **hicbir** Windows ikilisini dogru
/// calistiramazdi -- ve o yapi C++ yikicilarindan kaynak temizligine
/// kadar her yerde.
///
/// Hedef adres (`TargetIp`) sifirsa yalnizca geri sarma yapilir ve cagri
/// **normal doner**; sifirdan farkliysa yurutme oraya gecer. Ikisi de
/// Windows'un sozlesmesi: ilki `__finally`nin tek basina kullanimi,
/// ikincisi `__except`e atlama.
///
/// Doner: `true` ise cerceve guncellendi. `false` ise istek reddedildi
/// ve cagirana **hicbir sey yapilmadan** donulur.
///
/// # Safety
/// `frame` Ring 3'ten gelen gecerli bir syscall cercevesi olmalidir.
#[cfg(target_arch = "x86")]
pub unsafe fn unwind(
    frame: &mut crate::arch::cpu::regs::SyscallFrame,
    from_interrupt: bool,
    target_frame: usize,
    target_ip: usize,
    return_value: usize,
) -> bool {
    let task = scheduler::current_id();
    if task >= scheduler::MAX_TASKS {
        return false;
    }
    let teb_at = teb::address(task);
    if teb_at == 0 {
        return false;
    }
    // Ic ice geri sarma (`ExceptionCollidedUnwind`) desteklenmiyor: bir
    // geri sarma isleyicisi yeniden `RtlUnwind` cagirirsa reddediliyor.
    if UNWINDING[task].load(Ordering::Relaxed) != 0 {
        return false;
    }

    let head = chain_head(teb_at);

    // Hedef gercekten zincirde mi? Olmayan bir kayda "cekmek", `fs:[0]`i
    // rastgele bir adrese yazmak olurdu -- bir sonraki istisna cop veriye
    // dallanirdi. Windows bu durumda `STATUS_BAD_STACK` atar; TCMK daha
    // sade davranip istegi reddediyor.
    if target_frame != 0 && !chain_contains(head, target_frame) {
        crate::println!(
            "[LEVEL-0b1] SEH: RtlUnwind hedefi zincirde yok (0x{:08x}) -- reddedildi.",
            target_frame
        );
        return false;
    }

    let context = frame.user_context_via(from_interrupt);

    // Geri sarma kayitlari **mevcut yiginin altina** kuruluyor. Dagitim
    // sirasinda cagrildiginda bu, dagitimin kendi kayitlarinin da
    // altidir -- yani ikisi birbirini ezmiyor.
    let sp = context.stack_pointer();
    if sp < sizes::FRAME {
        return false;
    }
    let base = (sp - sizes::FRAME) & !0xF;
    if !writable(base, sizes::FRAME) {
        return false;
    }

    let record_at = base;
    core::ptr::write_bytes(record_at as *mut u8, 0, sizes::RECORD);
    let flags = if target_frame == 0 {
        EXCEPTION_UNWINDING | EXCEPTION_EXIT_UNWIND
    } else {
        EXCEPTION_UNWINDING
    };
    ((record_at + rec::CODE) as *mut u32).write_unaligned(STATUS_UNWIND);
    ((record_at + rec::FLAGS) as *mut u32).write_unaligned(flags);
    ((record_at + rec::ADDRESS) as *mut usize).write_unaligned(context.instruction_pointer());
    ((record_at + rec::PARAM_COUNT) as *mut u32).write_unaligned(0);

    // Donus baglami: cagiranin baglami. Hedef verildiyse yalnizca komut
    // isaretcisi degisiyor -- yigin oldugu gibi kaliyor, cunku hedef
    // kodun calisacagi cerceve zaten odur.
    let mut resume = context;
    if target_ip != 0 {
        resume.eip = target_ip as u32;
    }
    resume.eax = return_value as u32;

    (core::ptr::addr_of_mut!(UNWIND_RESUME) as *mut UserContext)
        .add(task)
        .write(resume);

    UNWIND_TARGET[task].store(target_frame, Ordering::Relaxed);
    UNWIND_NEXT[task].store(head, Ordering::Relaxed);
    UNWIND_BASE[task].store(base, Ordering::Relaxed);
    UNWIND_RECORD_AT[task].store(record_at, Ordering::Relaxed);
    UNWIND_STEPS[task].store(0, Ordering::Relaxed);
    UNWINDING[task].store(1, Ordering::Relaxed);
    UNWINDS.fetch_add(1, Ordering::Relaxed);

    let next = match unwind_step(task) {
        Some(handler_frame) => handler_frame,
        None => unwind_finish(task),
    };
    frame.set_user_context_via(from_interrupt, &next);
    true
}

/// x86_64'te zincir yok, dolayisiyla geri sarilacak bir sey de yok.
///
/// Sessizce basarili donmek yanlis olurdu: `__finally` bloklari
/// kosmadigi halde kosmus sayilirdi. 64-bit Windows'un cozumu tablo
/// tabanlidir (`.pdata`) ve TCMK'de yok -- bkz. README.
#[cfg(target_arch = "x86_64")]
pub unsafe fn unwind(
    _frame: &mut crate::arch::cpu::regs::SyscallFrame,
    _from_interrupt: bool,
    _target_frame: usize,
    _target_ip: usize,
    _return_value: usize,
) -> bool {
    false
}

/// Hedef kayit zincirde duruyor mu?
#[cfg(target_arch = "x86")]
unsafe fn chain_contains(head: usize, target: usize) -> bool {
    let word = core::mem::size_of::<usize>();
    let mut record = head;
    for _ in 0..MAX_CHAIN {
        if record == target {
            return true;
        }
        if record == usize::MAX || record == 0 || !writable(record, word * 2) {
            return false;
        }
        record = (record as *const usize).read_unaligned();
    }
    false
}

/// Geri sarmada siradaki isleyiciyi secer.
///
/// Dagitimdaki `advance` ile ayni desen, iki farkla: kayitlar **hedefe
/// kadar** yurunuyor ve isleyiciye verilen kayitta `EXCEPTION_UNWINDING`
/// kurulu.
#[cfg(target_arch = "x86")]
unsafe fn unwind_step(task: usize) -> Option<UserContext> {
    let base = UNWIND_BASE[task].load(Ordering::Relaxed);
    let record_at = UNWIND_RECORD_AT[task].load(Ordering::Relaxed);
    let target = UNWIND_TARGET[task].load(Ordering::Relaxed);
    let word = core::mem::size_of::<usize>();

    loop {
        let record = UNWIND_NEXT[task].load(Ordering::Relaxed);
        if record == target || record == usize::MAX || record == 0 {
            return None;
        }
        let steps = UNWIND_STEPS[task].fetch_add(1, Ordering::Relaxed);
        if steps >= MAX_CHAIN {
            return None;
        }
        if !writable(record, word * 2) {
            return None;
        }
        let next = (record as *const usize).read_unaligned();
        let handler = ((record + word) as *const usize).read_unaligned();
        UNWIND_NEXT[task].store(next, Ordering::Relaxed);
        if handler == 0 {
            continue;
        }
        FINALLY_CALLS.fetch_add(1, Ordering::Relaxed);
        // Imza dagitimdakiyle ayni. `ContextRecord` geri sarmada
        // anlamsiz oldugu icin sifir veriliyor: Windows da oraya
        // guvenilecek bir kayit koymaz.
        return build_frame(task, base, handler, &[record_at, record, 0, 0]);
    }
}

/// Yurume bitti: zincir hedefe cekilir ve donus baglami hazirlanir.
#[cfg(target_arch = "x86")]
unsafe fn unwind_finish(task: usize) -> UserContext {
    let target = UNWIND_TARGET[task].load(Ordering::Relaxed);
    let teb_at = teb::address(task);

    // `fs:[0]` hedefe cekiliyor. Hedefsiz geri sarmada zincir tumden
    // bosaltilir -- ve sonu `0` degil `-1`dir: sifir "gecerli bir kayit"
    // gibi gorunur ve zinciri yuruyen kod oraya dallanirdi.
    if teb_at != 0 {
        let head = if target == 0 { usize::MAX } else { target };
        (teb_at as *mut usize).write_unaligned(head);
        // Dagitim suruyorsa onun yurume durumu artik eski zinciri
        // gosteriyor. Yeni basa cekilmezse, sahiplenen isleyici "devam
        // et" demeyip "sirakine gec" derse cozulmus kayitlara
        // dallanilirdi.
        NEXT_RECORD[task].store(head, Ordering::Relaxed);
    }

    UNWINDING[task].store(0, Ordering::Relaxed);
    (core::ptr::addr_of!(UNWIND_RESUME) as *const UserContext)
        .add(task)
        .read()
}

/// Bir geri sarma isleyicisi dondu: siradakine gec ya da bitir.
///
/// Donus degeri **yok sayiliyor**. Windows'ta yalnizca
/// `ExceptionCollidedUnwind` anlamlidir ve o da ic ice geri sarmayla
/// ilgilidir; TCMK onu zaten bastan reddediyor (bkz. `unwind`).
#[cfg(target_arch = "x86")]
unsafe fn unwind_continue(
    frame: &mut crate::arch::cpu::regs::SyscallFrame,
    from_interrupt: bool,
    task: usize,
) -> bool {
    let next = match unwind_step(task) {
        Some(handler_frame) => handler_frame,
        None => unwind_finish(task),
    };
    frame.set_user_context_via(from_interrupt, &next);
    true
}

pub fn unwinds() -> usize {
    UNWINDS.load(Ordering::Relaxed)
}

pub fn finally_calls() -> usize {
    FINALLY_CALLS.load(Ordering::Relaxed)
}

/// Zincir bitti, kimse sahiplenmedi: son savunma hatti.
///
/// Windows'ta bu noktada `UnhandledExceptionFilter` calisir. Programlar
/// oraya bir cokme raporlayicisi takar -- gunluge yazan, ekrana pencere
/// cikaran, ya da CONTEXT'i duzeltip sureci kurtaran bir kod.
///
/// Filtre yalnizca **bir kez** cagrilir: filtrenin kendisi de
/// sahiplenmezse surec sonlanmali, yoksa "sahipsiz -> filtre -> sahipsiz"
/// dongusu olusurdu.
unsafe fn last_resort(task: usize, base: usize, pointers_at: usize) -> Option<UserContext> {
    if FILTER_RAN[task].load(Ordering::Relaxed) != 0 {
        return None;
    }
    let filter = FILTER[task].load(Ordering::Relaxed);
    if filter == 0 {
        return None;
    }
    FILTER_RAN[task].store(1, Ordering::Relaxed);
    // Filtrenin imzasi VEH ile ayni: tek arguman, EXCEPTION_POINTERS*.
    // Donus degerleri ise farkli -- bkz. `continue_dispatch`.
    PHASE[task].store(PHASE_FILTER, Ordering::Relaxed);
    build_frame(task, base, filter, &[pointers_at])
}

/// Hata adresini iceren fonksiyonun isleyicisini bulur ve cagirir.
///
/// x86_64'un zincir karsiligi -- ve neden ayri bir fonksiyon oldugu
/// bir satirla anlasiliyor: zincirde "siradaki" diye bir sey var, burada
/// yok. Tablo aramasi **tek** bir cevap verir (hata adresini iceren
/// fonksiyon), o cevabin isleyicisi de "sahiplenmiyorum" derse gidecek
/// baska yer yoktur -- cagiran cerceveye gecmek icin yigini **sanal
/// olarak geri sarmak** gerekir ve o, dagitimin ikinci yarisidir
/// (bkz. README). Su an tek cerceve derinliginde calisiyor.
///
/// `CHAIN_STEPS` burada da sayiliyor: ayni isleyici ic ice dagitimda
/// yeniden bulunabilir ve dongu koruyucusu ortak.
#[cfg(target_arch = "x86_64")]
unsafe fn table_handler(
    task: usize,
    base: usize,
    record_at: usize,
    context_at: usize,
) -> Option<UserContext> {
    // Yurume durumu ilk cagrida kuruluyor. Kaynak **kesilen baglam**:
    // hata hangi cercevede olustuysa yurume oradan basliyor.
    if UNWIND_READY[task].swap(1, Ordering::Relaxed) == 0 {
        let state = pdata::load_state(context_at);
        (core::ptr::addr_of_mut!(UNWIND_STATE) as *mut pdata::UnwindState)
            .add(task)
            .write(state);
    }
    let state_at = (core::ptr::addr_of_mut!(UNWIND_STATE) as *mut pdata::UnwindState).add(task);

    // Cerceveleri **sirayla** geziyoruz: isleyicisi olan ilk cerceve
    // dagitiliyor, olmayanlar atlaniyor.
    //
    // i386'da bu dongunun karsiligi zincirdeki bir sonraki kayda
    // gecmekti -- bir isaretci okumak. Burada her adim bir prolog
    // yorumlamak demek (bkz. `pdata::virtual_unwind`).
    loop {
        let steps = CHAIN_STEPS[task].fetch_add(1, Ordering::Relaxed);
        if steps >= MAX_CHAIN {
            return None;
        }
        let pc = (*state_at).rip as usize;
        // Yaprak fonksiyonlarin tabloda kaydi olmayabilir ve o zaman
        // yurume burada biter: kayitsiz bir cercevenin prologunu
        // yorumlamanin yolu yok.
        let (entry_at, function, image_base) = pdata::lookup(task, pc)?;

        let mut next = *state_at;
        let (establisher, found) =
            pdata::virtual_unwind(image_base, &function, pc, &mut next)?;
        // Durum **simdi** ilerletiliyor: bu isleyici "sahiplenmiyorum"
        // derse cekirdege geri donulecek ve yurume cagiranin
        // cercevesinden surecek.
        state_at.write(next);
        FRAMES_UNWOUND.fetch_add(1, Ordering::Relaxed);

        let Some((handler, handler_data)) = found else {
            // Bu cercevenin isleyicisi yok -- kayit yalnizca geri sarma
            // bilgisi tasiyor. Bir ust cerceveye gecilir.
            continue;
        };
        let pc = pc;

        // `DISPATCHER_CONTEXT` kayitlarin hemen ustune yaziliyor:
        // `begin` bloga onun icin de yer ayirdi.
        let dispatcher_at = (POINTERS_AT[task].load(Ordering::Relaxed)
            + core::mem::size_of::<usize>() * 2
            + 0xF)
            & !0xF;
        if !writable(dispatcher_at, sizes::DISPATCHER) {
            return None;
        }
        pdata::write_dispatcher(
            dispatcher_at,
            pc,
            image_base,
            entry_at,
            establisher,
            context_at,
            handler,
            handler_data,
        );

        // Imza i386 ile **birebir ayni**: (ExceptionRecord,
        // EstablisherFrame, ContextRecord, DispatcherContext). Ayrilan
        // sey yalnizca ilk ikisini nasil buldugumuz ve dorduncunun dolu
        // olmasi.
        return build_frame(
            task,
            base,
            handler,
            &[record_at, establisher, context_at, dispatcher_at],
        );
    }
}

/// Isleyiciye girilecek yigin cercevesini kurar.
///
/// Donus adresi TEB'deki tramplendir: isleyici `ret` ettiginde oraya
/// duser, tramplen de karari `int 0x2E` ile cekirdege getirir.
unsafe fn build_frame(
    task: usize,
    base: usize,
    handler: usize,
    args: &[usize],
) -> Option<UserContext> {
    let trampoline = teb::trampoline(task);
    if trampoline == 0 {
        return None;
    }
    build_call_frame(base, handler, trampoline, args)
}

/// Bir Ring 3 fonksiyonunu **Windows cagri geleneginde** cagiracak
/// baglami kurar.
///
/// SEH dagitimindan ayri bir fonksiyon olmasinin sebebi ikinci bir
/// musteri: APC teslimi de tam olarak ayni seyi yapiyor -- kullanici
/// yiginina bir cagri cercevesi kurmak ve donus adresine bir tramplen
/// koymak. Degisen yalnizca hangi tramplen (bkz. `apc.rs`).
///
/// Cerceve `base`in **altina** kuruluyor; `ret_addr` yordam `ret`
/// ettiginde dusecegi yerdir ve oradan geri donus yoktur.
pub(super) unsafe fn build_call_frame(
    base: usize,
    handler: usize,
    ret_addr: usize,
    args: &[usize],
) -> Option<UserContext> {
    let trampoline = ret_addr;
    let word = core::mem::size_of::<usize>();

    #[cfg(target_arch = "x86")]
    let sp = {
        // i386 cdecl: butun argumanlar yiginda, donus adresi en ustte.
        // Hizalama: girisde `esp + 4` 16'ya bolunmeli.
        let need = word * (args.len() + 1);
        let sp = ((base - need) & !0xF) - 4;
        if !writable(sp, need) {
            return None;
        }
        (sp as *mut usize).write_unaligned(trampoline);
        for (i, value) in args.iter().enumerate() {
            ((sp + word * (i + 1)) as *mut usize).write_unaligned(*value);
        }
        sp
    };

    #[cfg(target_arch = "x86_64")]
    let sp = {
        // Win64: ilk dort arguman registerda (RCX/RDX/R8/R9); yiginda
        // yalnizca donus adresi ve **golge alan** durur. Golge alani
        // ayirmak cagiranin gorevidir -- burada cagiran cekirdek.
        const SHADOW: usize = 32;
        let need = word + SHADOW;
        // Girisde RSP % 16 == 8 (cunku `call` donus adresini itmis olur).
        let sp = ((base - need) & !0xF) - word;
        if !writable(sp, need) {
            return None;
        }
        (sp as *mut usize).write_unaligned(trampoline);
        core::ptr::write_bytes((sp + word) as *mut u8, 0, SHADOW);
        sp
    };

    let mut next = UserContext::ZERO;
    next.redirect(handler, sp);
    #[cfg(target_arch = "x86_64")]
    {
        // Register argumanlari. Dorde kadar; SEH imzasi tam dort alir.
        let mut regs = [0usize; 4];
        for (i, value) in args.iter().take(4).enumerate() {
            regs[i] = *value;
        }
        next.rcx = regs[0] as u64;
        next.rdx = regs[1] as u64;
        next.r8 = regs[2] as u64;
        next.r9 = regs[3] as u64;
    }
    // Bayraklar temiz baslar: IF acik, yon bayragi kapali (Windows
    // cagri geleneginin sarti).
    #[cfg(target_arch = "x86")]
    {
        next.eflags = 0x202;
    }
    #[cfg(target_arch = "x86_64")]
    {
        next.rflags = 0x202;
    }
    Some(next)
}

/// Tramplenin cekirdege dondugu nokta: isleyicinin karari elde.
///
/// Doner: `true` ise cerceve guncellendi ve Ring 3 devam edebilir.
/// `false` ise dagitilacak isleyici kalmadi -- cagiran sureci
/// sonlandirmalidir.
///
/// # Safety
/// `frame` Ring 3'ten gelen gecerli bir syscall cercevesi olmalidir.
pub unsafe fn continue_dispatch(
    frame: &mut crate::arch::cpu::regs::SyscallFrame,
    from_interrupt: bool,
    disposition: usize,
) -> bool {
    let task = scheduler::current_id();
    if task >= scheduler::MAX_TASKS {
        return false;
    }

    // Geri sarma once bakilir ve `ACTIVE` denetiminden **once**: geri
    // sarma bir dagitim olmadan da baslatilabiliyor (`RtlUnwind`
    // siradan koddan da cagrilabilir), ve bir dagitimin icinden
    // baslatildiginda da o dagitim hala acik duruyor.
    #[cfg(target_arch = "x86")]
    if UNWINDING[task].load(Ordering::Relaxed) != 0 {
        let _ = disposition;
        return unwind_continue(frame, from_interrupt, task);
    }

    if ACTIVE[task].load(Ordering::Relaxed) == 0 {
        return false;
    }

    let context_at = CONTEXT_AT[task].load(Ordering::Relaxed);
    let phase = PHASE[task].load(Ordering::Relaxed);

    // "Devam et" karari iki mekanizmada **farkli sayidir**: VEH -1,
    // zincir 0. Ayni sayiyi ikisinde de kabul etmek, zincirdeki bir
    // "sirakine gec" (1) yanitini yanlis okumak olurdu.
    let decision = disposition as u32;
    // Uc mekanizma, uc ayri sayi kumesi. Filtrenin "devam et"i VEH ile
    // ayni (-1), ama "sonlandir" icin ayri bir degeri var (1) -- ve o
    // deger zincirde "sirakine gec" anlamina geliyor. Ayni cagriyi tek
    // kumeyle okumak, uc yerden birini yanlis yorumlamak olurdu.
    let continue_execution = match phase {
        PHASE_VECTORED | PHASE_FILTER => decision == EXCEPTION_CONTINUE_EXECUTION,
        _ => decision == DISPOSITION_CONTINUE_EXECUTION,
    };
    // Filtre "isleyiciyi calistir" derse (ya da bir sey sahiplenmezse)
    // surec sonlanir; filtreden sonra gidilecek baska yer yok.
    if phase == PHASE_FILTER && !continue_execution {
        let _ = EXCEPTION_EXECUTE_HANDLER;
        ACTIVE[task].store(0, Ordering::Relaxed);
        NESTED_DEPTH[task].store(0, Ordering::Relaxed);
        UNHANDLED.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    // `EXCEPTION_NONCONTINUABLE`: istisnayi ureten taraf "bu noktadan
    // devam edilemez" demis. Windows'ta bir isleyici yine de "devam et"
    // derse `STATUS_NONCONTINUABLE_EXCEPTION` ile surec biter. Ayni kural
    // burada da gecerli, cunku aksi halde donusu olmayan bir noktaya
    // donulurdu.
    let noncontinuable =
        FLAGS[task].load(Ordering::Relaxed) as u32 & EXCEPTION_NONCONTINUABLE != 0;
    if continue_execution && noncontinuable {
        crate::println!(
            "[LEVEL-0b1] SEH: NONCONTINUABLE istisnada 'devam et' istendi -- reddedildi."
        );
        ACTIVE[task].store(0, Ordering::Relaxed);
        NESTED_DEPTH[task].store(0, Ordering::Relaxed);
        UNHANDLED.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    if continue_execution {
        // Isleyici CONTEXT'i **degistirmis** olabilir -- zaten butun
        // mesele bu: hatali registeri duzeltip komutu tekrarlatmak ya da
        // yurutmeyi baska bir noktaya tasimak.
        let resumed = read_context(context_at);
        // Ic ice bir dagitim cozuldu: yurutme coken **isleyicinin**
        // icinde surecek, ama dis dagitim hala acik. Kayitlar geri
        // geliyor, `ACTIVE` dusmuyor -- isleyici dondugunde kalanlar
        // asil hatayi gormeli.
        if !pop_nested(task) {
            ACTIVE[task].store(0, Ordering::Relaxed);
        }
        CONTINUED.fetch_add(1, Ordering::Relaxed);
        frame.set_user_context_via(from_interrupt, &resumed);
        return true;
    }

    let base = RECORD_AT[task].load(Ordering::Relaxed);
    match advance(task, base) {
        Some(next) => {
            frame.set_user_context_via(from_interrupt, &next);
            true
        }
        None => {
            ACTIVE[task].store(0, Ordering::Relaxed);
            NESTED_DEPTH[task].store(0, Ordering::Relaxed);
            UNHANDLED.fetch_add(1, Ordering::Relaxed);
            false
        }
    }
}

/// Dagitim suruyor mu -- `NtContinueDispatch` disindaki yollar icin.
pub fn active(task: usize) -> bool {
    task < scheduler::MAX_TASKS && ACTIVE[task].load(Ordering::Relaxed) != 0
}

/// Istisna anindaki kodun adresi (raporlama icin).
pub fn fault_address(task: usize) -> usize {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    let record = RECORD_AT[task].load(Ordering::Relaxed);
    if record == 0 {
        return 0;
    }
    unsafe { ((record + rec::ADDRESS) as *const usize).read_unaligned() }
}

// --- CONTEXT kaydinin dis yuzu ----------------------------------------
//
// Asagidaki cevirici istisna dagitimi icin yazildi ama sozlesmesi ona
// ozgu degil: Win32'de bir Ring 3 baglami **her zaman** bu kayitla
// konusulur. `GetThreadContext`/`SetThreadContext` (bkz.
// `nt_syscalls`) ayni ceviriciyi kullaniyor -- ikinci bir kopya
// yazmak, iki yerde ayrisabilen bir ABI birakmak olurdu.

/// Win32 `CONTEXT` kaydinin bayt olcusu.
///
/// Sayi ABI'nin parcasidir: cagiran tamponu bu kadar ayirir ve cekirdek
/// tam bu kadarini yazar.
pub const CONTEXT_SIZE: usize = sizes::CONTEXT;

/// Cagiranin bildirdigi `ContextFlags`.
///
/// # Safety
/// `at` en az `CONTEXT_SIZE` bayt okunabilir olmalidir.
pub unsafe fn context_flags(at: usize) -> u32 {
    ((at + ctx::FLAGS) as *const u32).read_unaligned()
}

/// Verilen bayrak kumesi bir baglami **yazmaya** yetiyor mu?
pub fn context_flags_enough(flags: u32) -> bool {
    flags & ctx::REQUIRED == ctx::REQUIRED
}

/// Bir cekirdek baglamini Ring 3'teki `CONTEXT` kaydina doker.
///
/// # Safety
/// `at` Ring 3'e acik ve en az `CONTEXT_SIZE` bayt yazilabilir olmalidir.
pub unsafe fn store_context(at: usize, context: &UserContext) {
    write_context(at, context)
}

/// Tersi: Ring 3'teki `CONTEXT` kaydini cekirdek baglamina cevirir.
///
/// Bayraklarin sistem bitleri **alinmaz** (bkz. `read_context`): bir
/// program kendi IOPL'unu ya da kesme bayragini bu yolla degistirememeli.
///
/// # Safety
/// `at` en az `CONTEXT_SIZE` bayt okunabilir olmalidir.
pub unsafe fn load_context(at: usize) -> UserContext {
    read_context(at)
}

// --- CONTEXT okuma/yazma ----------------------------------------------

#[cfg(target_arch = "x86")]
unsafe fn write_context(at: usize, c: &UserContext) {
    core::ptr::write_bytes(at as *mut u8, 0, sizes::CONTEXT);
    let put = |offset: usize, value: u32| ((at + offset) as *mut u32).write_unaligned(value);
    put(ctx::FLAGS, ctx::FULL);
    put(ctx::EDI, c.edi);
    put(ctx::ESI, c.esi);
    put(ctx::EBX, c.ebx);
    put(ctx::EDX, c.edx);
    put(ctx::ECX, c.ecx);
    put(ctx::EAX, c.eax);
    put(ctx::EBP, c.ebp);
    put(ctx::EIP, c.eip);
    put(ctx::EFLAGS, c.eflags);
    put(ctx::ESP, c.esp);
    // Segment secicileri: Ring 3 degerleri (bkz. `gdt::i386`). Windows
    // kodu bunlari nadiren okur ama CONTEXT_SEGMENTS bayragini
    // koydugumuz icin dolu olmalari gerekir.
    put(ctx::SEG_CS, 0x1B);
    put(ctx::SEG_SS, 0x23);
    put(ctx::SEG_DS, 0x23);
    put(ctx::SEG_ES, 0x23);
    put(ctx::SEG_FS, 0x33);
    put(ctx::SEG_GS, 0x3B);
}

#[cfg(target_arch = "x86")]
unsafe fn read_context(at: usize) -> UserContext {
    let get = |offset: usize| ((at + offset) as *const u32).read_unaligned();
    UserContext {
        edi: get(ctx::EDI),
        esi: get(ctx::ESI),
        ebp: get(ctx::EBP),
        ebx: get(ctx::EBX),
        edx: get(ctx::EDX),
        ecx: get(ctx::ECX),
        eax: get(ctx::EAX),
        eip: get(ctx::EIP),
        esp: get(ctx::ESP),
        // Bayraklarin **tamamini** kullanicidan almak tehlikeli olurdu
        // (ornegin IOPL ya da IF'i degistirebilirdi). Yalnizca durum
        // bitleri alinir, sistem bitleri cekirdegin degeriyle kalir.
        eflags: (get(ctx::EFLAGS) & 0x0000_0CD5) | 0x202,
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn write_context(at: usize, c: &UserContext) {
    core::ptr::write_bytes(at as *mut u8, 0, sizes::CONTEXT);
    ((at + ctx::FLAGS) as *mut u32).write_unaligned(ctx::FULL);
    ((at + ctx::EFLAGS) as *mut u32).write_unaligned(c.rflags as u32);
    let put = |offset: usize, value: u64| ((at + offset) as *mut u64).write_unaligned(value);
    put(ctx::RAX, c.rax);
    put(ctx::RCX, c.rcx);
    put(ctx::RDX, c.rdx);
    put(ctx::RBX, c.rbx);
    put(ctx::RSP, c.rsp);
    put(ctx::RBP, c.rbp);
    put(ctx::RSI, c.rsi);
    put(ctx::RDI, c.rdi);
    put(ctx::R8, c.r8);
    put(ctx::R9, c.r9);
    put(ctx::R10, c.r10);
    put(ctx::R11, c.r11);
    put(ctx::R12, c.r12);
    put(ctx::R13, c.r13);
    put(ctx::R14, c.r14);
    put(ctx::R15, c.r15);
    put(ctx::RIP, c.rip);
}

#[cfg(target_arch = "x86_64")]
unsafe fn read_context(at: usize) -> UserContext {
    let get = |offset: usize| ((at + offset) as *const u64).read_unaligned();
    let eflags = ((at + ctx::EFLAGS) as *const u32).read_unaligned();
    UserContext {
        rax: get(ctx::RAX),
        rbx: get(ctx::RBX),
        rcx: get(ctx::RCX),
        rdx: get(ctx::RDX),
        rsi: get(ctx::RSI),
        rdi: get(ctx::RDI),
        rbp: get(ctx::RBP),
        r8: get(ctx::R8),
        r9: get(ctx::R9),
        r10: get(ctx::R10),
        r11: get(ctx::R11),
        r12: get(ctx::R12),
        r13: get(ctx::R13),
        r14: get(ctx::R14),
        r15: get(ctx::R15),
        rip: get(ctx::RIP),
        rsp: get(ctx::RSP),
        // i386'daki ile ayni gerekce: yalnizca durum bitleri.
        rflags: ((eflags as u64) & 0x0000_0CD5) | 0x202,
    }
}
