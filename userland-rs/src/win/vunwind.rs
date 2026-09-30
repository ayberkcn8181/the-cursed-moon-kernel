//! `winvunw.exe` -- prologu geri almak: sanal geri sarma.
//!
//! Bir onceki bati tablo tabanli SEH'in **birinci** yarisini getirdi:
//! hata adresini iceren fonksiyonu bul, isleyicisini cagir. Orada
//! yarim kalan soru suydu: o isleyici "sahiplenmiyorum" derse ne olur?
//!
//! i386'da soru yok. Zincir yiginda duruyor ve her kayit bir oncekini
//! gosteriyor -- **siradaki cerceve bir isaretci okumakla** bulunuyor.
//! x64'te zincir yok. Cagiranin yigin isaretcisini bulmanin tek yolu,
//! callee'nin prologunu **geri almak**:
//!
//! ```text
//!   push rbx          ->  geri alirken: rbx = [rsp]; rsp += 8
//!   sub  rsp, 0x20    ->  geri alirken: rsp += 0x20
//!   mov  rbp, rsp     ->  geri alirken: rsp = rbp - offset*16
//! ```
//!
//! Derleyici bunlari bir kod dizisi olarak ikiliye yaziyor ve cekirdek
//! o diziyi **yurutuyor** -- ters yonde. Maliyetin uygulamadan
//! cekirdege gecmesinin en somut hali burasi.
//!
//! ## Iki ayri olcum
//!
//! Sinav iki yoldan bakiyor ve ikisi de gerekli:
//!
//!   * **Dogrudan** (A, B, C, F, G): `RtlVirtualUnwind` elle kurulmus
//!     bir yigin uzerinde cagriliyor. Yorumlayicinin her kuralini tek
//!     tek olcmenin tek yolu bu -- gercek bir cokme, hangi kuralin
//!     bozuldugunu soylemez.
//!   * **Dagitim uzerinden** (D, E): gercek bir hata, gercek bir yigin.
//!     Yorumlayici dogru olsa bile yurumenin **baglanmis** olmasi ayri
//!     bir sey.
//!
//! ## Yedi sinav
//!
//! ```text
//!   A  bir cerceve geri sarildi -> RIP cagirana dondu
//!   B  register geri yuklendi   -> itilen RBX bulundu
//!   C  EstablisherFrame         -> govdedeki yigin tabani, geri
//!                                  sarilmis hali degil
//!   D  CAGIRANIN isleyicisi     -> callee patladi, cagiran yakaladi
//!   E  isleyicisiz cerceve      -> ara cerceve atlandi, iki ust
//!      atlandi                     yakaladi
//!   F  prolog ortasi            -> henuz itilmemis register geri
//!                                  ALINMADI
//!   G  SET_FPREG                -> cerceve registerinden RSP dogru
//! ```
//!
//! D bu batinin sebebi: bir `__try`nin asil isi **cagirdigi** kodun
//! hatasini yakalamaktir, kendi satirininkini degil.
//!
//! F kolayca atlanabilecek olani. Hata prologun ortasinda olustuysa
//! kodlarin bir kismi henuz yurutulmemistir; hepsini uygulamak, daha
//! itilmemis bir registeri yigindan "geri almak" demek olur ve sonuc
//! sessizce yanlis cikar.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::seh::{self, ExceptionRecord};
use tcmk::winapi::{self, RuntimeFunction, Window};

tcmk::entry!(main);

const BG: u32 = 0x0014_1824;
const PANEL: u32 = 0x0020_2838;
const FG: u32 = 0x00E4_EAF4;
const DIM: u32 = 0x008A_96A8;
const ACCENT: u32 = 0x0080_C8FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

// --- Elle kurulmus yigin: yorumlayiciyi dogrudan olcmek --------------
//
// Gercek bir cokme uzerinden olcmek, hangi kuralin bozuldugunu
// soylemez: yanlis RSP de yanlis register de ayni sekilde "surec oldu"
// diye gorunur. Sentetik yigin her kurali ayri ayri gorunur kiliyor.

/// Sahte yiginin kelime sayisi.
const STACK_WORDS: usize = 16;

#[repr(C, align(16))]
struct FakeStack([u64; STACK_WORDS]);

static mut FAKE: FakeStack = FakeStack([0; STACK_WORDS]);

/// Sahte donus adresi -- geri sarma sonrasi RIP bu olmali.
const FAKE_RETURN: u64 = 0x0000_0000_DEAD_BE00;
/// Yigina konan RBX imi.
const RBX_MARK: u64 = 0x0000_0000_1234_5678;
/// Yigina konan RBP imi.
const RBP_MARK: u64 = 0x0000_0000_8765_4321;
/// Prolog ortasi sinavinda **okunmamasi gereken** deger.
const TRAP_MARK: u64 = 0x0000_0000_BAD0_BAD0;

/// `CONTEXT` tamponu.
#[repr(C, align(16))]
struct ContextBuf([u8; winapi::CONTEXT_SIZE]);

static mut CTX: ContextBuf = ContextBuf([0; winapi::CONTEXT_SIZE]);

/// Elle kurulan `UNWIND_INFO`.
///
/// Kod dizisi **azalan prolog ofseti** sirasinda: ters yurutme sirasi
/// budur ve cekirdek onlari bu sirada uyguluyor.
#[repr(C, align(4))]
struct Unwind2 {
    version_flags: u8,
    size_of_prolog: u8,
    count_of_codes: u8,
    frame: u8,
    codes: [u16; 2],
    handler_rva: u32,
    scope: u32,
}

/// A/B/C/F icin: `push rbx` + `sub rsp, 0x20`.
static mut UNWIND_PLAIN: Unwind2 = Unwind2 {
    version_flags: 1,
    size_of_prolog: 8,
    count_of_codes: 2,
    frame: 0,
    codes: [0, 0],
    handler_rva: 0,
    scope: 0,
};

/// G icin: `push rbp` + `mov rbp, rsp`.
static mut UNWIND_FP: Unwind2 = Unwind2 {
    version_flags: 1,
    size_of_prolog: 4,
    count_of_codes: 2,
    // `FrameRegister:4 | FrameOffset:4` -- RBP (5), ofset 0.
    frame: 5,
    codes: [0, 0],
    handler_rva: 0,
    scope: 0,
};

static mut PROBE_FN: [RuntimeFunction; 1] = [RuntimeFunction {
    begin: 0,
    end: 0,
    unwind: 0,
}];

// --- Gercek dagitim: elle yazilmis prologlar -------------------------
//
// Rust'ta `__try` yok ve derleyici `.pdata` uretmiyor, o yuzden hem
// fonksiyonlar hem tablolari elle yaziliyor. `winseh.exe`nin zincir
// kaydini elle kurmasiyla ayni durum.

core::arch::global_asm!(
    // Yaprak: hata burada olusuyor. Prologu yok, yani geri sarmasi
    // yalnizca donus adresini okumak.
    ".globl tcmk_leaf",
    "tcmk_leaf:",
    "xor rax, rax",
    "mov qword ptr [rax], 1",
    "ret",
    ".globl tcmk_leaf_end",
    "tcmk_leaf_end:",

    // Ara cerceve: tabloda kaydi var ama **isleyicisi yok**. E sinavi
    // bunun atlandigini olcuyor.
    ".globl tcmk_mid",
    "tcmk_mid:",
    "push rbx",
    "sub rsp, 0x20",
    "call tcmk_leaf",
    "add rsp, 0x20",
    "pop rbx",
    "ret",
    ".globl tcmk_mid_end",
    "tcmk_mid_end:",

    // D: dogrudan yapragi cagiran, isleyicisi olan cerceve.
    ".globl tcmk_outer1",
    "tcmk_outer1:",
    "push rbx",
    "sub rsp, 0x20",
    "call tcmk_leaf",
    ".globl tcmk_outer1_resume",
    "tcmk_outer1_resume:",
    "add rsp, 0x20",
    "pop rbx",
    "mov eax, 0x11",
    "ret",
    ".globl tcmk_outer1_end",
    "tcmk_outer1_end:",

    // E: arada isleyicisiz bir cerceve var.
    ".globl tcmk_outer2",
    "tcmk_outer2:",
    "push rbx",
    "sub rsp, 0x20",
    "call tcmk_mid",
    ".globl tcmk_outer2_resume",
    "tcmk_outer2_resume:",
    "add rsp, 0x20",
    "pop rbx",
    "mov eax, 0x22",
    "ret",
    ".globl tcmk_outer2_end",
    "tcmk_outer2_end:",
);

extern "C" {
    fn tcmk_leaf();
    fn tcmk_leaf_end();
    fn tcmk_mid();
    fn tcmk_mid_end();
    fn tcmk_outer1() -> u32;
    fn tcmk_outer1_resume();
    fn tcmk_outer1_end();
    fn tcmk_outer2() -> u32;
    fn tcmk_outer2_resume();
    fn tcmk_outer2_end();
}

/// Dagitim sinavlarinin tablosu: yaprak, ara ve iki dis cerceve.
static mut DISPATCH_TABLE: [RuntimeFunction; 4] = [RuntimeFunction {
    begin: 0,
    end: 0,
    unwind: 0,
}; 4];

/// Isleyicisiz kayit (yaprak ve ara cerceve icin): kod yok.
#[repr(C, align(4))]
struct Unwind0 {
    version_flags: u8,
    size_of_prolog: u8,
    count_of_codes: u8,
    frame: u8,
}

static mut UNWIND_LEAF: Unwind0 = Unwind0 {
    version_flags: 1,
    size_of_prolog: 0,
    count_of_codes: 0,
    frame: 0,
};

/// Ara/dis cercevelerin prologu: `push rbx` + `sub rsp, 0x20`.
static mut UNWIND_FRAME_NO_HANDLER: Unwind2 = Unwind2 {
    version_flags: 1,
    size_of_prolog: 8,
    count_of_codes: 2,
    frame: 0,
    codes: [0, 0],
    handler_rva: 0,
    scope: 0,
};

static mut UNWIND_FRAME_WITH_HANDLER: Unwind2 = Unwind2 {
    version_flags: 1 | (winapi::UNW_FLAG_EHANDLER << 3),
    size_of_prolog: 8,
    count_of_codes: 2,
    frame: 0,
    codes: [0, 0],
    handler_rva: 0,
    scope: 0,
};

/// Isleyici kac kez kostu ve hangi cerceveden.
static HANDLER_RAN: AtomicUsize = AtomicUsize::new(0);
static SEEN_ESTABLISHER: AtomicUsize = AtomicUsize::new(0);
/// Isleyicinin akisi tasiyacagi adres (sinava gore degisiyor).
static RESUME_AT: AtomicUsize = AtomicUsize::new(0);

/// Cagiran cercevenin dil isleyicisi.
///
/// "Devam et" demek icin **iki** register duzeltiliyor ve ikincisi
/// batinin tam kalbinde: RIP'i tasimak yetmez, cunku yigin isaretcisi
/// hala **callee'nin** cercevesini gosteriyor. Dogru deger
/// `EstablisherFrame` -- yani cekirdegin prologu geri alarak buldugu,
/// bu fonksiyonun govdesindeki yigin tabani.
unsafe extern "system" fn frame_handler(
    _record: *mut ExceptionRecord,
    establisher: usize,
    context: *mut c_void,
    _dispatcher: *mut c_void,
) -> i32 {
    HANDLER_RAN.fetch_add(1, Ordering::SeqCst);
    SEEN_ESTABLISHER.store(establisher, Ordering::SeqCst);
    let resume = RESUME_AT.load(Ordering::SeqCst);
    if resume != 0 {
        seh::set_reg(context, seh::Reg::Ip, resume);
        seh::set_reg(context, seh::Reg::Sp, establisher);
        return seh::EXCEPTION_CONTINUE_EXECUTION_SEH;
    }
    seh::EXCEPTION_CONTINUE_SEARCH_SEH
}

/// F sinavinin ham degerleri (tani icin).
static F_IP: AtomicUsize = AtomicUsize::new(0);
static F_BX: AtomicUsize = AtomicUsize::new(0);

// --- Dagitim sinavlari cocukta kosuyor -------------------------------
//
// D ve E gercek bir hata uretiyor ve yurume kirilirsa o hata **sahipsiz
// kalir**: surec oler. Ilk yazilista ikisi de ebeveynde kosuyordu ve
// bozma sinavi bunu hemen gosterdi -- yurume bir cerceveden oteye
// gecmeyince sinav C'den sonra sustu.
//
// Cevap cikis kodunda. Isaret biti ayri tutuluyor: cocuk hic cevap
// veremezse kod sifir olur ve "her sey basarisiz" ile "cocuk oldu"
// birbirinden ayirt edilemezdi.
const BIT_DONE: u32 = 0x8000;
const BIT_HANDLER: u32 = 0x1;
const BIT_RESUMED: u32 = 0x2;

/// Cocugu ayiran argumanlar.
const ARG_D: &str = "d";
const ARG_E: &str = "e";

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
    "A cerceve geri sarildi",
    "B register geri yuklendi",
    "C EstablisherFrame",
    "D CAGIRANIN isleyicisi",
    "E isleyicisiz cerceve atlandi",
    "F prolog ortasi",
    "G SET_FPREG",
];

fn say(check: &Check) {
    use core::fmt::Write;
    let mut console = winapi::Console;
    let _ = writeln!(
        console,
        "[winvunw] {}: {} ({})",
        check.name,
        if check.passed { "gecti" } else { "KALDI" },
        check.detail
    );
}

/// `CONTEXT` tamponuna bir register yazar.
unsafe fn set_ctx(reg: seh::Reg, value: usize) {
    seh::set_reg(core::ptr::addr_of_mut!(CTX) as *mut c_void, reg, value);
}

/// `CONTEXT` tamponundan bir register okur.
unsafe fn get_ctx(reg: seh::Reg) -> usize {
    seh::get_reg(core::ptr::addr_of_mut!(CTX) as *mut c_void, reg)
}

/// Sentetik bir geri sarma kosar.
///
/// Doner: `(EstablisherFrame, isleyici)`.
unsafe fn probe(base: usize, rsp: usize, pc: usize) -> (u64, usize) {
    core::ptr::write_bytes(core::ptr::addr_of_mut!(CTX) as *mut u8, 0, winapi::CONTEXT_SIZE);
    set_ctx(seh::Reg::Sp, rsp);
    set_ctx(seh::Reg::Ip, pc);
    let mut establisher = 0u64;
    let mut data = 0usize;
    let handler = winapi::RtlVirtualUnwind(
        winapi::UNW_FLAG_NHANDLER,
        base,
        pc,
        core::ptr::addr_of!(PROBE_FN) as *const RuntimeFunction,
        core::ptr::addr_of_mut!(CTX) as *mut c_void,
        &mut data,
        &mut establisher,
        core::ptr::null_mut(),
    );
    (establisher, handler)
}

fn main() {
    use core::fmt::Write;
    if tcmk::args::count() > 1 {
        if tcmk::args::get(1) == Some(ARG_D) {
            unsafe { winapi::ExitProcess(dispatch_child(false)) };
        }
        if tcmk::args::get(1) == Some(ARG_E) {
            unsafe { winapi::ExitProcess(dispatch_child(true)) };
        }
    }
    let _ = writeln!(winapi::Console, "[winvunw] sinav basliyor");
    let mut checks = [EMPTY; 7];

    let base = unsafe { winapi::GetModuleHandleA(core::ptr::null()) } as usize;
    let rva = |at: usize| (at - base) as u32;

    // --- A, B, C: duz bir prologu geri almak --------------------------
    //
    // Yigin duzeni, `push rbx` + `sub rsp, 0x20` prologunun birakacagi
    // hali birebir taklit ediyor:
    //
    //   [0]..[3]  yerel alan (0x20)
    //   [4]       saklanan RBX
    //   [5]       donus adresi
    let (body_rsp, a_ok, b_ok, c_ok) = unsafe {
        let stack = core::ptr::addr_of_mut!(FAKE) as *mut u64;
        // `write_bytes` **eleman** sayisi alir, bayt degil: `*mut u64` icin
        // buradaki sayi kelime sayisi olmali. Ilk yazilista `* 8` vardi
        // ve sahte yigin 8 kat asiliyordu -- hemen ardindaki statikleri
        // (kayit tablosu ve `UNWIND_INFO`) sifirliyordu. Sinav bunu
        // F'nin acikladigi tuhaf sonucla gosterdi: teshis "prologun
        // tamami uygulandi" diyordu, oysa uygulanacak kod kalmamisti.
        core::ptr::write_bytes(stack, 0, STACK_WORDS);
        stack.add(4).write(RBX_MARK);
        stack.add(5).write(FAKE_RETURN);

        let codes = core::ptr::addr_of_mut!(UNWIND_PLAIN);
        (*codes).codes[0] = winapi::unwind_code(8, winapi::UWOP_ALLOC_SMALL, 3);
        (*codes).codes[1] = winapi::unwind_code(1, winapi::UWOP_PUSH_NONVOL, 3);

        let fn_entry = core::ptr::addr_of_mut!(PROBE_FN) as *mut RuntimeFunction;
        (*fn_entry).begin = rva(tcmk_leaf as usize);
        (*fn_entry).end = rva(tcmk_leaf_end as usize);
        (*fn_entry).unwind = rva(core::ptr::addr_of!(UNWIND_PLAIN) as usize);

        let body = stack as usize;
        // Prologun **bittigi** yerden sonrasi: butun kodlar yurutulmus
        // sayilir. Ilk yazilista burada 2 vardi ve `sub rsp, 0x20`
        // kodunun prolog ofseti 8 oldugu icin o kod **atlaniyordu** --
        // yani sinav yanlislikla F'nin (prolog ortasi) durumunu
        // kuruyor, sonra A/B'nin cevabini bekliyordu.
        let pc = tcmk_leaf as usize + 16;
        let (establisher, _) = probe(base, body, pc);
        (
            body,
            get_ctx(seh::Reg::Ip) as u64 == FAKE_RETURN,
            get_ctx(seh::Reg::B) as u64 == RBX_MARK,
            establisher as usize == body,
        )
    };

    checks[0] = Check {
        name: NAMES[0],
        detail: if a_ok {
            "RIP cagirana dondu"
        } else {
            "RIP geri sarilmadi"
        },
        passed: a_ok,
    };
    say(&checks[0]);
    checks[1] = Check {
        name: NAMES[1],
        detail: if b_ok {
            "itilen RBX yigindan geri alindi"
        } else {
            "RBX geri yuklenmedi"
        },
        passed: b_ok,
    };
    say(&checks[1]);
    checks[2] = Check {
        name: NAMES[2],
        detail: if c_ok {
            "govdedeki yigin tabani dondu"
        } else {
            "geri sarilmis RSP dondu: taban yanlis"
        },
        passed: c_ok,
    };
    say(&checks[2]);
    let _ = body_rsp;

    // --- F: prolog ortasinda ------------------------------------------
    //
    // `push rbx` yurutulmus (ofset 1), `sub rsp, 0x20` (ofset 8) henuz
    // degil. Yigin bu yuzden dogrudan saklanan RBX'i gosteriyor.
    // Kodlarin hepsini uygulayan bir cekirdek once 0x20 atlar ve
    // **tuzak** degeri RBX sanardi.
    let f_ok = unsafe {
        let stack = core::ptr::addr_of_mut!(FAKE) as *mut u64;
        core::ptr::write_bytes(stack, 0, STACK_WORDS);
        stack.add(0).write(RBX_MARK);
        stack.add(1).write(FAKE_RETURN);
        // 0x20 ilerisi: atlamayi yapmayan bir cekirdegin okuyacagi yer.
        stack.add(4).write(TRAP_MARK);
        stack.add(5).write(TRAP_MARK);

        let body = stack as usize;
        // Prologun **icinde** bir adres: 4. bayt.
        let pc = tcmk_leaf as usize + 4;
        let (_, _) = probe(base, body, pc);
        F_IP.store(get_ctx(seh::Reg::Ip), Ordering::SeqCst);
        F_BX.store(get_ctx(seh::Reg::B), Ordering::SeqCst);
        get_ctx(seh::Reg::Ip) as u64 == FAKE_RETURN && get_ctx(seh::Reg::B) as u64 == RBX_MARK
    };
    checks[5] = Check {
        name: NAMES[5],
        detail: if f_ok {
            "yurutulmemis kod atlandi"
        } else {
            "prologun tamami uygulandi: tuzak degeri okundu"
        },
        passed: f_ok,
    };

    // --- G: cerceve registeri -----------------------------------------
    //
    // `push rbp` + `mov rbp, rsp`. RSP artik nerede olursa olsun, dogru
    // taban RBP'den hesaplaniyor -- `SET_FPREG`in varlik sebebi bu:
    // degisken boyutlu yigin ayirmasi (`alloca`) yapan fonksiyonlarda
    // RSP'yi izlemek imkansizdir.
    let g_ok = unsafe {
        let stack = core::ptr::addr_of_mut!(FAKE) as *mut u64;
        core::ptr::write_bytes(stack, 0, STACK_WORDS);
        // [6] saklanan RBP, [7] donus adresi. RBP oraya isaret ediyor.
        stack.add(6).write(RBP_MARK);
        stack.add(7).write(FAKE_RETURN);

        let codes = core::ptr::addr_of_mut!(UNWIND_FP);
        (*codes).codes[0] = winapi::unwind_code(4, winapi::UWOP_SET_FPREG, 0);
        (*codes).codes[1] = winapi::unwind_code(1, winapi::UWOP_PUSH_NONVOL, 5);

        let fn_entry = core::ptr::addr_of_mut!(PROBE_FN) as *mut RuntimeFunction;
        (*fn_entry).unwind = rva(core::ptr::addr_of!(UNWIND_FP) as usize);

        let frame_base = stack.add(6) as usize;
        // RSP cok asagida: dogru cevap yalnizca RBP'den gelebilir.
        core::ptr::write_bytes(core::ptr::addr_of_mut!(CTX) as *mut u8, 0, winapi::CONTEXT_SIZE);
        set_ctx(seh::Reg::Sp, stack as usize);
        set_ctx(seh::Reg::Ip, tcmk_leaf as usize + 16);
        seh::set_reg(
            core::ptr::addr_of_mut!(CTX) as *mut c_void,
            seh::Reg::Bp,
            frame_base,
        );
        let mut establisher = 0u64;
        let mut data = 0usize;
        winapi::RtlVirtualUnwind(
            winapi::UNW_FLAG_NHANDLER,
            base,
            tcmk_leaf as usize + 16,
            core::ptr::addr_of!(PROBE_FN) as *const RuntimeFunction,
            core::ptr::addr_of_mut!(CTX) as *mut c_void,
            &mut data,
            &mut establisher,
            core::ptr::null_mut(),
        );
        get_ctx(seh::Reg::Ip) as u64 == FAKE_RETURN
            && establisher as usize == frame_base
            && seh::get_reg(core::ptr::addr_of_mut!(CTX) as *mut c_void, seh::Reg::Bp) as u64
                == RBP_MARK
    };
    checks[6] = Check {
        name: NAMES[6],
        detail: if g_ok {
            "RSP cerceve registerinden yeniden hesaplandi"
        } else {
            "SET_FPREG uygulanmadi: RSP izlenmeye calisildi"
        },
        passed: g_ok,
    };

    // --- D ve E: gercek dagitim, cocuk surecte ------------------------
    let d_bits = reap_bounded(spawn_child(ARG_D));
    let d_ok = d_bits & BIT_DONE != 0
        && d_bits & BIT_HANDLER != 0
        && d_bits & BIT_RESUMED != 0;
    checks[3] = Check {
        name: NAMES[3],
        detail: if d_ok {
            "callee patladi, cagiranin isleyicisi yakaladi"
        } else if d_bits & BIT_DONE == 0 {
            "cocuk cevap vermedi: hata sahipsiz kaldi"
        } else if d_bits & BIT_HANDLER == 0 {
            "cagiranin isleyicisi KOSMADI: yurume ilerlemedi"
        } else {
            "isleyici kostu ama akis kurtarilamadi"
        },
        passed: d_ok,
    };
    say(&checks[3]);

    let e_bits = reap_bounded(spawn_child(ARG_E));
    let e_ok = e_bits & BIT_DONE != 0
        && e_bits & BIT_HANDLER != 0
        && e_bits & BIT_RESUMED != 0;
    checks[4] = Check {
        name: NAMES[4],
        detail: if e_ok {
            "isleyicisiz ara cerceve atlandi, iki ust yakaladi"
        } else if e_bits & BIT_DONE == 0 {
            "cocuk cevap vermedi: yurume iki cerceve gidemedi"
        } else if e_bits & BIT_HANDLER == 0 {
            "iki cerceve ustteki isleyici KOSMADI"
        } else {
            "isleyici kostu ama akis kurtarilamadi"
        },
        passed: e_ok,
    };
    say(&checks[4]);

    say(&checks[5]);
    say(&checks[6]);

    let passed = checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(winapi::Console, "[winvunw] sonuc: {}/7 gecti", passed);
    let _ = writeln!(
        winapi::Console,
        "[winvunw] taban 0x{:x}  establisher 0x{:x}",
        base,
        SEEN_ESTABLISHER.load(Ordering::SeqCst)
    );
    // F'nin ham degerleri raporda kaliyor. Bir kez is gordu: "prologun
    // tamami uygulandi" teshisi dogruyken bile sayilar bambaska bir
    // sey soyluyordu (ikisi de sifir), ve asil hatanin sinavin kendi
    // tamponunda oldugunu ancak o gosterdi.
    let _ = writeln!(
        winapi::Console,
        "[winvunw] F ham: ip=0x{:x} bx=0x{:x} (beklenen 0x{:x} / 0x{:x})",
        F_IP.load(Ordering::SeqCst),
        F_BX.load(Ordering::SeqCst),
        FAKE_RETURN,
        RBX_MARK
    );
    show(&checks);
}

/// Cocuk govdesi: tablolari kurar, hatayi uretir, cevabi cikis kodunda
/// tasir.
///
/// `deep` -- arada **isleyicisiz** bir cerceve olsun mu (E sinavi).
fn dispatch_child(deep: bool) -> u32 {
    let base = unsafe { winapi::GetModuleHandleA(core::ptr::null()) } as usize;
    let rva = |at: usize| (at - base) as u32;

    let table_at = unsafe {
        let t = core::ptr::addr_of_mut!(DISPATCH_TABLE) as *mut RuntimeFunction;
        let leaf_unwind = rva(core::ptr::addr_of!(UNWIND_LEAF) as usize);
        let plain = core::ptr::addr_of_mut!(UNWIND_FRAME_NO_HANDLER);
        (*plain).codes[0] = winapi::unwind_code(8, winapi::UWOP_ALLOC_SMALL, 3);
        (*plain).codes[1] = winapi::unwind_code(1, winapi::UWOP_PUSH_NONVOL, 3);
        let with = core::ptr::addr_of_mut!(UNWIND_FRAME_WITH_HANDLER);
        (*with).codes[0] = winapi::unwind_code(8, winapi::UWOP_ALLOC_SMALL, 3);
        (*with).codes[1] = winapi::unwind_code(1, winapi::UWOP_PUSH_NONVOL, 3);
        (*with).handler_rva = rva(frame_handler as usize);
        let plain_rva = rva(plain as usize);
        let with_rva = rva(with as usize);

        (*t.add(0)) = RuntimeFunction {
            begin: rva(tcmk_leaf as usize),
            end: rva(tcmk_leaf_end as usize),
            unwind: leaf_unwind,
        };
        (*t.add(1)) = RuntimeFunction {
            begin: rva(tcmk_mid as usize),
            end: rva(tcmk_mid_end as usize),
            unwind: plain_rva,
        };
        (*t.add(2)) = RuntimeFunction {
            begin: rva(tcmk_outer1 as usize),
            end: rva(tcmk_outer1_end as usize),
            unwind: with_rva,
        };
        (*t.add(3)) = RuntimeFunction {
            begin: rva(tcmk_outer2 as usize),
            end: rva(tcmk_outer2_end as usize),
            unwind: with_rva,
        };
        t as *const RuntimeFunction
    };
    if unsafe { winapi::RtlAddFunctionTable(table_at, 4, base) } == 0 {
        return BIT_DONE;
    }

    let (resume, expected) = if deep {
        (tcmk_outer2_resume as usize, 0x22u32)
    } else {
        (tcmk_outer1_resume as usize, 0x11u32)
    };
    RESUME_AT.store(resume, Ordering::SeqCst);
    let returned = if deep {
        unsafe { tcmk_outer2() }
    } else {
        unsafe { tcmk_outer1() }
    };

    let mut bits = BIT_DONE;
    if HANDLER_RAN.load(Ordering::SeqCst) == 1 {
        bits |= BIT_HANDLER;
    }
    if returned == expected {
        bits |= BIT_RESUMED;
    }
    bits
}

/// Ayni ikiliyi verilen argumanla baslatir; tutamaci doner (0 = olmadi).
fn spawn_child(arg: &str) -> usize {
    let mut command = [0u8; 32];
    let head = b"winvunw.exe ";
    command[..head.len()].copy_from_slice(head);
    command[head.len()..head.len() + arg.len()].copy_from_slice(arg.as_bytes());

    let mut info = winapi::ProcessInformation::new();
    let created = unsafe {
        winapi::CreateProcessA(
            b"C:\\bin\\winvunw.exe\0".as_ptr(),
            command.as_ptr(),
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
    let mut win = match Window::create("winvunw -- prologu geri almak", 250, 140, 510, 250) {
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
    win.text(6, 3, "Cagiranin RSP'si ancak prolog geri alinarak bulunur", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            420,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    win.text(6, h - 46, "i386: siradaki cerceve = bir isaretci oku", DIM);
    win.text(6, h - 30, "x64:  siradaki cerceve = prologu yurut (ters)", DIM);

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
