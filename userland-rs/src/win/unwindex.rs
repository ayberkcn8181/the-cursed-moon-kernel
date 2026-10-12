//! `winunwex.exe` -- geri sarmanin **ucuncu** yarisi: cerceveleri cozmek.
//!
//! Tablo tabanli SEH uc parcadan olusuyor ve ucu de ayri sorular:
//!
//! ```text
//!   1. arama             hata adresini iceren fonksiyon hangisi?
//!   2. sanal geri sarma  cagiran cerceve nerede?
//!   3. GERI SARMA        aradaki cerceveler NASIL COZULUR, ve
//!                        yoldaki temizlik kodu nasil kosar?
//! ```
//!
//! Ilk ikisi onceki iki batida geldi. Orada yarim kalan sey suydu:
//! cekirdek cagiran cerceveyi **bulabiliyordu** ama yigini
//! **cozmuyordu**. Yani `__try`/`__finally` iceren hicbir x64 ikilisi
//! dogru calisamazdi -- derleyicinin `__finally` icin urettigi kod
//! yalnizca bu yolda kosar, ve o yapi C++ yikicilarindan kaynak
//! temizligine kadar her yerde.
//!
//! ## i386 ile ayni sozlesme, bambaska mekanizma
//!
//! ```text
//!   i386   zincirdeki her kaydi hedefe kadar yuru.
//!          "Siradaki" = bir isaretci okumak.
//!          Hedef      = bir zincir KAYDININ ADRESI.
//!
//!   x64    her cerceveyi prologunu geri alarak coz.
//!          "Siradaki" = kucuk bir yorumlayici kosturmak.
//!          Hedef      = bir EstablisherFrame.
//! ```
//!
//! Hedefin kimliginin degismesi bir ayrinti degil: x64'te gosterilecek
//! bir kayit yoktur, cunku zincir yoktur. Hedef ancak "hangi
//! cercevenin govdesinde duracagiz" diye tarif edilebilir.
//!
//! ## `UHANDLER`, `EHANDLER` degil
//!
//! Dagitim `EHANDLER` ariyor ("bu istisnayi kim sahipleniyor"), geri
//! sarma `UHANDLER` ("yoldaki hangi cercevenin temizlenecek isi var").
//! Ayni alanda duran iki bayrak, iki ayri soru. Ikisini tek maskeyle
//! aramak, yalnizca `__except` filtresi olan bir cerceveyi geri
//! sarmada da cagirmak olurdu: derleyicinin o is icin **uretmedigi**
//! kodu kosturmak.
//!
//! ## Yedi sinav
//!
//! ```text
//!   A  hedefe atladi      -> yurutme TargetIp'te surdu, RAX = ReturnValue
//!   B  __finally KOSTU    -> yoldaki iki UHANDLER de cagrildi
//!   C  EXCEPTION_UNWINDING-> isleyiciye verilen kayitta bayrak kurulu
//!   D  hedefte DURDU      -> HEDEF cercevenin isleyicisi cagrilmadi
//!   E  UHANDLER sart      -> yalnizca EHANDLER'i olan cerceve atlandi
//!   F  sira: icten disa   -> once ic cerceve, sonra dis
//!   G  atlamasiz geri sarma -> TargetIp=0: yalnizca temizlik, cagri
//!                              NORMAL donuyor
//! ```
//!
//! D ile E birbirinin yerine gecmez ve ikisi de gerekli. D "yurume
//! nerede **duruyor**" diye soruyor, E "yolda hangi cerceve
//! **atlaniyor**". Bir cekirdek hedefi hic gozetmeden butun
//! cerceveleri kosturabilir (D kalir, E gecer) ya da bayragi hic
//! okumadan yoldaki herkesi cagirabilir (E kalir, D gecer).
//!
//! Ikisi de "su isleyici **cagrilmadi**" bicimindedir ve bu bicimin
//! bilinen bir zayifligi var: hicbir sey kosmazsa cevap bos yere
//! dogru cikar. Bozma sinavi bunu hemen gosterdi -- thunk cercevesini
//! cozen adim kaldirilinca yurume hic baslamadi ve D ile E yine gecti.
//! Ikisi artik yurumenin gerceklesmis olmasini **sart** kosuyor
//! (`BIT_WALKED`); kosmamissa sinav "olculemedi" diyor.
//!
//! A'nin ayni zayifligi **yok**, cunku olctugu sey ayri: yurutmenin
//! hedefte surup surmedigi. Yurume hic olmasa bile cekirdek hedefe
//! atlayabilir, ve A o zaman hakli olarak geciyor -- "atladi ama
//! hicbir seyi temizlemedi" tanisini veren B'dir.
//!
//! ## Cagri zinciri
//!
//! ```text
//!   outer  (HEDEF, UHANDLER'i var -- kosmamali)
//!     mid  (yalnizca EHANDLER -- atlanmali)
//!       in2 (UHANDLER -- ikinci kosmali)
//!         in1 (UHANDLER -- once kosmali)
//!           leaf -> RtlUnwindEx
//! ```
//!
//! Rust'ta `__try` yok ve derleyici `.pdata` uretmiyor, o yuzden hem
//! fonksiyonlar hem tablolari elle yaziliyor -- `winvunw` ve `winseh`
//! ile ayni durum.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::seh::{self, ExceptionRecord};
use tcmk::winapi::{self, RuntimeFunction, Window};

tcmk::entry!(main);

const BG: u32 = 0x0018_1420;
const PANEL: u32 = 0x0026_2034;
const FG: u32 = 0x00E8_E4F2;
const DIM: u32 = 0x0094_8CA4;
const ACCENT: u32 = 0x00C0_A0FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// `RtlUnwindEx`e verilen deger; hedefte RAX bu olmali.
const RETURN_MARK: usize = 0x5A5A_1234;
/// Geri sarma **olmazsa** `outer`in dondurecegi deger.
const NO_UNWIND: u32 = 0x0BAD;

// --- Elle yazilmis cerceveler ----------------------------------------
//
// Hepsinin prologu ayni kalipta (`push rbx` + `sub rsp, N`) cunku
// olculen sey prolog cesitliligi degil -- onu `winvunw` olcuyor.
// Burada olculen, cerceveler **arasinda** ne olup bittigi.
//
// `params` blogunun duzeni (her yuva 8 bayt):
//
//   [0x00] TargetFrame     (outer kendi govdesindeki RSP'yi yaziyor)
//   [0x08] TargetIp        (0 = atlamasiz: yalnizca temizlik)
//   [0x10] RtlUnwindEx     (cagrilacak adres)
//   [0x18] ReturnValue
core::arch::global_asm!(
    // Yaprak: geri sarmayi **baslatan** cerceve. Isleyicisi yok.
    ".globl tcmk_ux_leaf",
    "tcmk_ux_leaf:",
    "push rbx",
    "sub rsp, 0x30",
    "mov rbx, rcx",
    "mov rax, [rbx + 0x10]",
    "mov rcx, [rbx + 0x00]",
    "mov rdx, [rbx + 0x08]",
    "xor r8, r8",
    "mov r9, [rbx + 0x18]",
    "mov qword ptr [rsp + 0x20], 0",
    "mov qword ptr [rsp + 0x28], 0",
    "call rax",
    // Hedefli geri sarmada buraya **hic donulmez**; hedefsizde donulur.
    "add rsp, 0x30",
    "pop rbx",
    "ret",
    ".globl tcmk_ux_leaf_end",
    "tcmk_ux_leaf_end:",

    // Ic cerceve 1: `UHANDLER`i var, once kosmali.
    ".globl tcmk_ux_in1",
    "tcmk_ux_in1:",
    "push rbx",
    "sub rsp, 0x20",
    "call tcmk_ux_leaf",
    "add rsp, 0x20",
    "pop rbx",
    "ret",
    ".globl tcmk_ux_in1_end",
    "tcmk_ux_in1_end:",

    // Ic cerceve 2: `UHANDLER`i var, ikinci kosmali.
    ".globl tcmk_ux_in2",
    "tcmk_ux_in2:",
    "push rbx",
    "sub rsp, 0x20",
    "call tcmk_ux_in1",
    "add rsp, 0x20",
    "pop rbx",
    "ret",
    ".globl tcmk_ux_in2_end",
    "tcmk_ux_in2_end:",

    // Ara cerceve: yalnizca `EHANDLER`. Geri sarmada **atlanmali**.
    ".globl tcmk_ux_mid",
    "tcmk_ux_mid:",
    "push rbx",
    "sub rsp, 0x20",
    "call tcmk_ux_in2",
    "add rsp, 0x20",
    "pop rbx",
    "ret",
    ".globl tcmk_ux_mid_end",
    "tcmk_ux_mid_end:",

    // Hedef cerceve. `UHANDLER`i var ama **cagrilmamali**: yurume
    // burada duruyor, cunku hedef kod bu cercevenin govdesinde kosacak.
    ".globl tcmk_ux_outer",
    "tcmk_ux_outer:",
    "push rbx",
    "sub rsp, 0x20",
    // Govdedeki RSP = bu cercevenin EstablisherFrame'i, yani geri
    // sarmanin hedefi. Rust tarafindan hesaplanamazdi: prologun
    // birakacagi degeri ancak fonksiyonun kendisi bilir.
    "mov [rcx + 0x00], rsp",
    "call tcmk_ux_mid",
    // Buraya ancak geri sarma hedefe atlamazsa donulur.
    "mov eax, 0xBAD",
    "add rsp, 0x20",
    "pop rbx",
    "ret",
    ".globl tcmk_ux_outer_resume",
    "tcmk_ux_outer_resume:",
    // `RtlUnwindEx` buraya atlar: RSP hedef cerceveye cekilmis,
    // RAX `ReturnValue` tasiyor. RAX'e dokunulmuyor -- A sinavi onu
    // okuyor.
    "add rsp, 0x20",
    "pop rbx",
    "ret",
    ".globl tcmk_ux_outer_end",
    "tcmk_ux_outer_end:",
);

extern "C" {
    fn tcmk_ux_leaf();
    fn tcmk_ux_leaf_end();
    fn tcmk_ux_in1();
    fn tcmk_ux_in1_end();
    fn tcmk_ux_in2();
    fn tcmk_ux_in2_end();
    fn tcmk_ux_mid();
    fn tcmk_ux_mid_end();
    fn tcmk_ux_outer(params: *mut usize) -> u32;
    fn tcmk_ux_outer_resume();
    fn tcmk_ux_outer_end();
}

/// `UNWIND_INFO`: iki kod, isleyici alani.
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

const fn unwind2(flags: u8, prolog: u8) -> Unwind2 {
    Unwind2 {
        version_flags: 1 | (flags << 3),
        size_of_prolog: prolog,
        count_of_codes: 2,
        frame: 0,
        codes: [0, 0],
        handler_rva: 0,
        scope: 0,
    }
}

/// Yaprak: `push rbx` + `sub rsp, 0x30`, isleyicisiz.
static mut UW_LEAF: Unwind2 = unwind2(0, 9);
/// Ic cerceve 1: `UHANDLER`.
static mut UW_IN1: Unwind2 = unwind2(winapi::UNW_FLAG_UHANDLER, 5);
/// Ic cerceve 2: `UHANDLER`.
static mut UW_IN2: Unwind2 = unwind2(winapi::UNW_FLAG_UHANDLER, 5);
/// Ara cerceve: yalnizca `EHANDLER` -- geri sarmada atlanmali.
static mut UW_MID: Unwind2 = unwind2(winapi::UNW_FLAG_EHANDLER, 5);
/// Hedef cerceve: `UHANDLER`i var ama cagrilmamali.
static mut UW_OUTER: Unwind2 = unwind2(winapi::UNW_FLAG_UHANDLER, 5);

static mut TABLE: [RuntimeFunction; 5] = [RuntimeFunction {
    begin: 0,
    end: 0,
    unwind: 0,
}; 5];

/// `RtlUnwindEx`e verilecek arguman blogu (bkz. yukaridaki duzen).
static mut PARAMS: [usize; 4] = [0; 4];

// --- Isleyicilerin gozlemleri ----------------------------------------

/// Kac `__finally` kostu.
static FINALLY_RAN: AtomicUsize = AtomicUsize::new(0);
/// Cagrilma sirasi: her isleyici kendi imini kaydiriyor.
static ORDER: AtomicUsize = AtomicUsize::new(0);
/// Isleyiciye verilen kayitin bayraklari.
static SEEN_FLAGS: AtomicUsize = AtomicUsize::new(0);
/// Yalnizca `EHANDLER`i olan cerceve cagrildi mi (cagrilmamali).
static MID_RAN: AtomicUsize = AtomicUsize::new(0);
/// Hedef cercevenin isleyicisi cagrildi mi (cagrilmamali).
static TARGET_RAN: AtomicUsize = AtomicUsize::new(0);

/// Siraya bir im ekler: dort bit kaydirip altina yazar.
fn note_order(mark: usize) {
    let now = ORDER.load(Ordering::SeqCst);
    ORDER.store((now << 4) | mark, Ordering::SeqCst);
}

/// Ic cerceve 1'in `__finally`si.
unsafe extern "system" fn finally_in1(
    record: *mut ExceptionRecord,
    _establisher: usize,
    _context: *mut c_void,
    _dispatcher: *mut c_void,
) -> i32 {
    FINALLY_RAN.fetch_add(1, Ordering::SeqCst);
    note_order(1);
    // Bayraklar **ilk** isleyiciden okunuyor; ikincisi de ayni kaydi
    // gorur, cunku cekirdek geri sarma icin tek bir kayit yaziyor.
    SEEN_FLAGS.store((*record).flags as usize, Ordering::SeqCst);
    seh::EXCEPTION_CONTINUE_SEARCH_SEH
}

/// Ic cerceve 2'nin `__finally`si.
unsafe extern "system" fn finally_in2(
    _record: *mut ExceptionRecord,
    _establisher: usize,
    _context: *mut c_void,
    _dispatcher: *mut c_void,
) -> i32 {
    FINALLY_RAN.fetch_add(1, Ordering::SeqCst);
    note_order(2);
    seh::EXCEPTION_CONTINUE_SEARCH_SEH
}

/// Ara cercevenin isleyicisi -- yalnizca `EHANDLER` bayragiyla kayitli.
///
/// Geri sarmada cagrilirsa bu, cekirdegin bayragi hic okumadigini
/// soyler: derleyicinin `__finally` icin uretmedigi kod kosturulmus
/// olur.
unsafe extern "system" fn handler_mid(
    _record: *mut ExceptionRecord,
    _establisher: usize,
    _context: *mut c_void,
    _dispatcher: *mut c_void,
) -> i32 {
    MID_RAN.fetch_add(1, Ordering::SeqCst);
    note_order(3);
    seh::EXCEPTION_CONTINUE_SEARCH_SEH
}

/// Hedef cercevenin isleyicisi -- yurume buraya varmadan durmali.
unsafe extern "system" fn finally_target(
    _record: *mut ExceptionRecord,
    _establisher: usize,
    _context: *mut c_void,
    _dispatcher: *mut c_void,
) -> i32 {
    TARGET_RAN.fetch_add(1, Ordering::SeqCst);
    note_order(4);
    seh::EXCEPTION_CONTINUE_SEARCH_SEH
}

// --- Cocugun cevabi --------------------------------------------------
//
// Butun geri sarma cocukta kosuyor ve sebebi somut: yurume kirilirsa
// yurutme cozulmus bir yigina atlar ve surec oler. Ebeveynde kossaydi
// sinav tek bir satir bile yazmadan susardi.
//
// Isaret biti ayri tutuluyor; yoksa "her sey basarisiz" ile "cocuk
// oldu" ayirt edilemezdi.
const BIT_DONE: u32 = 0x8000;
const BIT_LANDED: u32 = 0x01;
const BIT_FINALLY: u32 = 0x02;
const BIT_FLAGS: u32 = 0x04;
const BIT_TARGET_QUIET: u32 = 0x08;
const BIT_MID_SKIPPED: u32 = 0x10;
const BIT_ORDER: u32 = 0x20;
const BIT_EXIT_UNWIND: u32 = 0x40;
/// Yurume **hic basladi mi**: yolda en az bir isleyici cagrildi.
///
/// Bu bit bir bozma sinavindan dogdu. Thunk cercevesini cozen adim
/// cekirdekten kaldirilinca yurume hic baslamadi -- ve D ile E yine
/// gecti, cunku ikisi de "su isleyici **cagrilmadi**" diye soruyordu
/// ve hicbir sey kosmayinca iki cevap da dogru cikiyordu. Bos yere
/// dogru: olculmek istenen sey yurumenin nerede durdugu ve neyi
/// atladigiydi, yurumenin hic olmamasi degil.
///
/// Olcusu de bir kez duzeltildi. Once "`__finally` kostu mu" diye
/// bakiyordu; ikinci bozma sinavinda (geri sarmada `EHANDLER`
/// aranmasi) yurume yurudu ve **ara cerceveyi** cagirdi, ama hicbir
/// `__finally` kosmadigi icin sinav "yurume hic baslamadi" dedi --
/// oysa elindeki tani tam tersiydi. Dogru olcu: yolda **herhangi**
/// bir isleyici cagrildi mi, yani sira sozcugu bos degil mi.
const BIT_WALKED: u32 = 0x80;

const ARG_RUN: &str = "run";

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
    "A hedefe atladi",
    "B __finally KOSTU",
    "C EXCEPTION_UNWINDING",
    "D hedefte DURDU",
    "E UHANDLER sart",
    "F sira: icten disa",
    "G atlamasiz geri sarma",
];

fn say(check: &Check) {
    use core::fmt::Write;
    let mut console = winapi::Console;
    let _ = writeln!(
        console,
        "[winunwex] {}: {} ({})",
        check.name,
        if check.passed { "gecti" } else { "KALDI" },
        check.detail
    );
}

/// Tabloyu kurar ve kaydeder. Doner: oldu mu.
unsafe fn install_table(base: usize) -> bool {
    let rva = |at: usize| (at - base) as u32;

    // Prolog kodlari **azalan prolog ofseti** sirasinda: ters yurutme
    // sirasi budur ve cekirdek onlari bu sirada uyguluyor.
    //
    // Yaprak `sub rsp, 0x30` yapiyor (0x30/8 - 1 = 5), otekiler
    // `sub rsp, 0x20` (0x20/8 - 1 = 3). `push rbx` hepsinde ofset 1.
    let leaf = core::ptr::addr_of_mut!(UW_LEAF);
    (*leaf).codes[0] = winapi::unwind_code(5, winapi::UWOP_ALLOC_SMALL, 5);
    (*leaf).codes[1] = winapi::unwind_code(1, winapi::UWOP_PUSH_NONVOL, 3);

    for info in [
        core::ptr::addr_of_mut!(UW_IN1),
        core::ptr::addr_of_mut!(UW_IN2),
        core::ptr::addr_of_mut!(UW_MID),
        core::ptr::addr_of_mut!(UW_OUTER),
    ] {
        (*info).codes[0] = winapi::unwind_code(5, winapi::UWOP_ALLOC_SMALL, 3);
        (*info).codes[1] = winapi::unwind_code(1, winapi::UWOP_PUSH_NONVOL, 3);
    }

    (*core::ptr::addr_of_mut!(UW_IN1)).handler_rva = rva(finally_in1 as *const () as usize);
    (*core::ptr::addr_of_mut!(UW_IN2)).handler_rva = rva(finally_in2 as *const () as usize);
    (*core::ptr::addr_of_mut!(UW_MID)).handler_rva = rva(handler_mid as *const () as usize);
    (*core::ptr::addr_of_mut!(UW_OUTER)).handler_rva = rva(finally_target as *const () as usize);

    let t = core::ptr::addr_of_mut!(TABLE) as *mut RuntimeFunction;
    let entries: [(usize, usize, usize); 5] = [
        (
            tcmk_ux_leaf as *const () as usize,
            tcmk_ux_leaf_end as *const () as usize,
            core::ptr::addr_of!(UW_LEAF) as usize,
        ),
        (
            tcmk_ux_in1 as *const () as usize,
            tcmk_ux_in1_end as *const () as usize,
            core::ptr::addr_of!(UW_IN1) as usize,
        ),
        (
            tcmk_ux_in2 as *const () as usize,
            tcmk_ux_in2_end as *const () as usize,
            core::ptr::addr_of!(UW_IN2) as usize,
        ),
        (
            tcmk_ux_mid as *const () as usize,
            tcmk_ux_mid_end as *const () as usize,
            core::ptr::addr_of!(UW_MID) as usize,
        ),
        (
            tcmk_ux_outer as *const () as usize,
            tcmk_ux_outer_end as *const () as usize,
            core::ptr::addr_of!(UW_OUTER) as usize,
        ),
    ];
    for (i, (begin, end, info)) in entries.iter().enumerate() {
        (*t.add(i)) = RuntimeFunction {
            begin: rva(*begin),
            end: rva(*end),
            unwind: rva(*info),
        };
    }
    winapi::RtlAddFunctionTable(t as *const RuntimeFunction, 5, base) != 0
}

/// Geri sarmayi kosan cocuk: cevabi cikis kodunda.
fn unwind_child() -> u32 {
    use core::fmt::Write;
    let mut console = winapi::Console;
    let base = unsafe { winapi::GetModuleHandleA(core::ptr::null()) } as usize;

    if !unsafe { install_table(base) } {
        let _ = writeln!(console, "[winunwex/cocuk] tablo kaydedilemedi");
        return BIT_DONE;
    }

    // `RtlUnwindEx`in adresi ithal tablosundan degil **adla** aliniyor:
    // elle yazilmis yaprak onu bir registerdan cagiriyor.
    let kernel32 = unsafe { winapi::GetModuleHandleA(b"KERNEL32.dll\0".as_ptr()) };
    let unwind_ex =
        unsafe { winapi::GetProcAddress(kernel32, b"RtlUnwindEx\0".as_ptr()) } as usize;
    if unwind_ex == 0 {
        let _ = writeln!(console, "[winunwex/cocuk] RtlUnwindEx bulunamadi");
        return BIT_DONE;
    }

    let mut bits = BIT_DONE;
    let params = core::ptr::addr_of_mut!(PARAMS) as *mut usize;

    // --- Birinci kosum: HEDEFLI geri sarma ---------------------------
    unsafe {
        // Hedef yuvasi bilerek **gecersiz** baslatiliyor: `outer`
        // govdesinde kendi RSP'siyle uzerine yazacak. Sifir birakmak
        // yaniltici olurdu -- sifir "hedefsiz geri sarma" demektir ve
        // outer hic yazmasa bile sinav gecebilirdi.
        params.add(0).write(usize::MAX);
        params.add(1).write(tcmk_ux_outer_resume as *const () as usize);
        params.add(2).write(unwind_ex);
        params.add(3).write(RETURN_MARK);
    }
    let returned = unsafe { tcmk_ux_outer(params) };

    let landed_value = returned as usize;
    let _ = writeln!(
        console,
        "[winunwex/cocuk] hedefli kosum dondu: 0x{:x}  finally={}  sira=0x{:x}",
        landed_value,
        FINALLY_RAN.load(Ordering::SeqCst),
        ORDER.load(Ordering::SeqCst)
    );

    if landed_value == RETURN_MARK {
        bits |= BIT_LANDED;
    }
    if FINALLY_RAN.load(Ordering::SeqCst) == 2 {
        bits |= BIT_FINALLY;
    }
    if SEEN_FLAGS.load(Ordering::SeqCst) & seh::EXCEPTION_UNWINDING as usize != 0 {
        bits |= BIT_FLAGS;
    }
    if ORDER.load(Ordering::SeqCst) != 0 {
        bits |= BIT_WALKED;
    }
    if TARGET_RAN.load(Ordering::SeqCst) == 0 {
        bits |= BIT_TARGET_QUIET;
    }
    if MID_RAN.load(Ordering::SeqCst) == 0 {
        bits |= BIT_MID_SKIPPED;
    }
    // Ic cerceve once (1), dis sonra (2): kaydirma sirasi 0x12.
    if ORDER.load(Ordering::SeqCst) == 0x12 {
        bits |= BIT_ORDER;
    }

    // --- Ikinci kosum: ATLAMASIZ geri sarma ---------------------------
    //
    // `TargetIp` sifir: yalnizca temizlik yapilir ve cagri **normal
    // doner**. `outer` o zaman kendi normal yolundan `NO_UNWIND`
    // donuyor -- yani "atlamadi" ile "hic calismadi" ayirt ediliyor.
    //
    // Windows'un sozlesmesi bu ve i386 yuzu de boyle davraniyor;
    // `__finally`nin tek basina, bir `__except` olmadan kullanimi tam
    // olarak buraya karsilik geliyor.
    FINALLY_RAN.store(0, Ordering::SeqCst);
    ORDER.store(0, Ordering::SeqCst);
    unsafe {
        params.add(1).write(0);
        params.add(0).write(usize::MAX);
    }
    let returned2 = unsafe { tcmk_ux_outer(params) };
    let ran2 = FINALLY_RAN.load(Ordering::SeqCst);
    let _ = writeln!(
        console,
        "[winunwex/cocuk] hedefsiz kosum dondu: 0x{:x}  finally={}",
        returned2, ran2
    );
    if returned2 == NO_UNWIND && ran2 == 2 {
        bits |= BIT_EXIT_UNWIND;
    }

    bits
}

/// Ayni ikiliyi verilen argumanla baslatir; tutamaci doner (0 = olmadi).
fn spawn_child(arg: &str) -> usize {
    let mut command = [0u8; 32];
    let head = b"winunwex.exe ";
    command[..head.len()].copy_from_slice(head);
    command[head.len()..head.len() + arg.len()].copy_from_slice(arg.as_bytes());

    let mut info = winapi::ProcessInformation::new();
    let created = unsafe {
        winapi::CreateProcessA(
            b"C:\\bin\\winunwex.exe\0".as_ptr(),
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
///
/// Sinirsiz beklemek bu sinavda ozellikle kotu olurdu: geri sarma
/// kirildiginda cocuk cozulmus bir yigina atlar ve sonu belirsizdir.
fn child_bits(arg: &str) -> u32 {
    let child = spawn_child(arg);
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

fn main() {
    use core::fmt::Write;
    if tcmk::args::count() > 1 && tcmk::args::get(1) == Some(ARG_RUN) {
        unsafe { winapi::ExitProcess(unwind_child()) };
    }

    let _ = writeln!(winapi::Console, "[winunwex] sinav basliyor");
    let mut checks = [EMPTY; 7];

    let bits = child_bits(ARG_RUN);
    // Isaret bitine ek olarak ust yarinin **bos** olmasi da araniyor.
    //
    // Sebebi bir bozma sinavi: yigin isaretcisi hedefe cekilmeyince
    // cocuk cozulmus bir yiginda coktu ve cikis kodu `0xC0000005`
    // oldu -- bir `NTSTATUS`. O sayida 15. bit tesaduf eseri sifirdi,
    // ama baska bir cokme kodunda olmayabilirdi ve sinav bir cokmeyi
    // "cocuk cevap verdi" diye okurdu. Sinavin kendi cevaplari
    // 0x10000'in altinda; `NTSTATUS` kodlari cok uzerinde.
    let answered = bits & BIT_DONE != 0 && bits < 0x1_0000;
    let _ = writeln!(winapi::Console, "[winunwex] cocugun cevabi: 0x{:04x}", bits);

    // `needs_walk`: yurumenin gerceklesmis olmasini **sart** kosan
    // sinavlar (D ve E). Ikisi de "su isleyici cagrilmadi" diye
    // soruyor ve yurume hic baslamazsa bu cevap bos yere dogru cikar.
    let mut set = |index: usize,
                   bit: u32,
                   needs_walk: bool,
                   good: &'static str,
                   bad: &'static str| {
        let walked = bits & BIT_WALKED != 0;
        let measurable = answered && (!needs_walk || walked);
        let passed = measurable && bits & bit != 0;
        checks[index] = Check {
            name: NAMES[index],
            detail: if !answered {
                "cocuk cevap vermedi: geri sarma sureci goturdu"
            } else if !measurable {
                "yurume hic baslamadi: olculemedi"
            } else if passed {
                good
            } else {
                bad
            },
            passed,
        };
        say(&checks[index]);
    };

    set(
        0,
        BIT_LANDED,
        false,
        "TargetIp'te surdu ve RAX = ReturnValue",
        "hedefe ATLAMADI",
    );
    set(
        1,
        BIT_FINALLY,
        false,
        "yoldaki iki UHANDLER de cagrildi",
        "__finally KOSMADI",
    );
    set(
        2,
        BIT_FLAGS,
        false,
        "kayitta EXCEPTION_UNWINDING kurulu",
        "bayrak kurulu DEGIL: isleyici dagitim sanir",
    );
    set(
        3,
        BIT_TARGET_QUIET,
        true,
        "hedef cercevenin isleyicisi cagrilmadi",
        "hedef cerceve de COZULDU",
    );
    set(
        4,
        BIT_MID_SKIPPED,
        true,
        "yalnizca EHANDLER'i olan cerceve atlandi",
        "EHANDLER'i geri sarmada CAGRILDI",
    );
    set(
        5,
        BIT_ORDER,
        false,
        "once ic cerceve, sonra dis",
        "sira yanlis",
    );
    set(
        6,
        BIT_EXIT_UNWIND,
        false,
        "TargetIp=0 ile cagri normal dondu, temizlik yine kostu",
        "atlamasiz geri sarma yanlis davrandi",
    );

    let passed = checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(winapi::Console, "[winunwex] sonuc: {}/7 gecti", passed);
    show(&checks, bits);
}

fn show(checks: &[Check; 7], bits: u32) {
    let mut win = match Window::create("winunwex -- cerceveleri cozmek", 250, 120, 500, 250) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, checks, bits);
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 7], bits: u32) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "Bulmak yetmez: yigin COZULMELI", ACCENT);

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

    win.text(6, h - 30, "cocugun cevabi:", DIM);
    win.number(140, h - 30, bits as usize, FG);

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
