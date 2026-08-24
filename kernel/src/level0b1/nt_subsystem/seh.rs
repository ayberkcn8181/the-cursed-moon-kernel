//! Windows istisna dagitimi -- SEH ve VEH.
//!
//! TEB kurulduktan sonra (bkz. `teb.rs`) bir PE ikilisi `fs:[0]`da bir
//! istisna zinciri **gorebiliyordu**, ama zincir hicbir zaman
//! **yurutulmuyordu**: bir sayfa hatasi surecin sonu demekti. Bu modul o
//! bosluğu kapatir.
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
//! Bu, `level0b1::signal`in yaptigi isin Windows'çasidir. Iki yol da
//! "cekirdek kullanici yiginina bir cerceve kurar ve baglami cevirir"
//! desenini kullanir; hatta ayni `UserContext` tipini paylasirlar. Fark
//! cercevenin **bicimi**: POSIX bir sinyal numarasi verir, Windows bir
//! kayit ciftinin adresini.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::arch::cpu::regs::UserContext;
use crate::level0a::core::{mmu, scheduler};

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
    pub const FRAME: usize = RECORD + CONTEXT + 0x100;
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

/// Ring 3'teki EXCEPTION_RECORD / CONTEXT adresleri.
static RECORD_AT: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
static CONTEXT_AT: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
static POINTERS_AT: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Yurumede kalinan yer: once vektorlu liste, sonra (i386'da) zincir.
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
    PHASE[task].store(PHASE_VECTORED, Ordering::Relaxed);
    FILTER[task].store(0, Ordering::Relaxed);
    FILTER_RAN[task].store(0, Ordering::Relaxed);
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
        if !mmu::is_user_accessible(page) {
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
    // Dagitim sirasinda cikan istisna, dagiticinin kendisini bozar.
    // Windows bunu `ExceptionNestedException` ile ele alir; TCMK daha
    // muhafazakar davranip sureci sonlandirir.
    if ACTIVE[task].load(Ordering::Relaxed) != 0 {
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

    core::ptr::write_bytes(record_at as *mut u8, 0, sizes::RECORD);
    ((record_at + rec::CODE) as *mut u32).write_unaligned(code);
    ((record_at + rec::FLAGS) as *mut u32).write_unaligned(flags);
    ((record_at + rec::NESTED) as *mut usize).write_unaligned(0);
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

    RECORD_AT[task].store(record_at, Ordering::Relaxed);
    CONTEXT_AT[task].store(context_at, Ordering::Relaxed);
    POINTERS_AT[task].store(pointers_at, Ordering::Relaxed);
    PHASE[task].store(PHASE_VECTORED, Ordering::Relaxed);
    FILTER_RAN[task].store(0, Ordering::Relaxed);
    FLAGS[task].store(flags as usize, Ordering::Relaxed);
    NEXT_VEH[task].store(0, Ordering::Relaxed);
    NEXT_RECORD[task].store(chain_head(teb_at), Ordering::Relaxed);
    CHAIN_STEPS[task].store(0, Ordering::Relaxed);
    ACTIVE[task].store(1, Ordering::Relaxed);

    match advance(task, base) {
        Some(next) => {
            DISPATCHED.fetch_add(1, Ordering::Relaxed);
            Some(next)
        }
        None => {
            ACTIVE[task].store(0, Ordering::Relaxed);
            None
        }
    }
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

    // --- 2. SEH zinciri (yalnizca i386) ---
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
    let _ = task;
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
    if task >= scheduler::MAX_TASKS || ACTIVE[task].load(Ordering::Relaxed) == 0 {
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
        UNHANDLED.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    if continue_execution {
        // Isleyici CONTEXT'i **degistirmis** olabilir -- zaten butun
        // mesele bu: hatali registeri duzeltip komutu tekrarlatmak ya da
        // yurutmeyi baska bir noktaya tasimak.
        let resumed = read_context(context_at);
        ACTIVE[task].store(0, Ordering::Relaxed);
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
