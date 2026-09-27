//! `winunwind.exe` -- dagitimin ikinci yarisi: `__finally` nasil kosar.
//!
//! `winseh` istisna dagitiminin **birinci** yarisini olcuyordu: "bu
//! istisnayi kim sahipleniyor?" Bu program ondan hemen sonra gelen
//! soruyu olcuyor: **aradaki cerceveler ne olacak?**
//!
//! ```text
//!   __try { __try { patlar } __finally { A } } __except { B }
//!
//!   1. dagitim    ic isleyici  -> "sahiplenmiyorum"
//!                 dis isleyici -> "sahipleniyorum" -> RtlUnwind
//!   2. GERI SARMA ic isleyici  -> EXCEPTION_UNWINDING ile CAGRILIR -> A kosar
//!   3. hedef      fs:[0] dis kayda cekilir, yurutme B'ye gecer
//! ```
//!
//! Ikinci satir olmadan `A` **hic kosmaz**. Derleyicinin `__finally`
//! icin urettigi kod tam olarak orada durur, ve o yapi kaynak
//! temizliginden C++ yikicilarina kadar her yerde -- yani bu satir
//! olmadan cogu gercek Windows ikilisi sessizce yanlis calisir.
//!
//! ## Bir isleyici, iki is
//!
//! Windows'un burada yaptigi sey ilk bakista tuhaf: `__try`nin isleyicisi
//! **tek** bir fonksiyondur ve iki ayri soruya cevap verir. Hangisinin
//! soruldugunu yalnizca bir bayrak ayirir:
//!
//! ```text
//!   bayrak yok  ->  "sahipleniyor musun?"           (__except filtresi)
//!   bayrak var  ->  "cerceven yikiliyor, temizle"   (__finally)
//! ```
//!
//! POSIX'te karsiligi yok. Bir sinyal isleyicisi "aradaki cerceveleri
//! coz" diye cagrilmaz; `longjmp` yigini geri sarar ama yol ustundeki
//! hicbir koda haber vermez -- temizlik yapilmasi gerekiyorsa onu
//! programci elle yazar. Windows'un kalibi, temizligi **dile** gomuyor.
//!
//! ## Kayitlar burada elle kuruluyor
//!
//! Rust'ta `__try`/`__finally` yok, o yuzden derleyicinin urettigi sey
//! bu programda elle yaziliyor: `Registration` kayitlari `fs:[0]`a
//! takiliyor ve isleyiciler bayraga bakip dallaniyor. Gercek Windows'ta
//! bu isi `_except_handler3` ve derleyicinin urettigi kapsam tablosu
//! yapar -- o katman **derleyici calisma zamanina** aittir, cekirdege
//! degil. TCMK'nin sagladigi sey altindaki mekanizma: geri sarmanin
//! kendisi.
//!
//! ## Yedi sinav
//!
//! ```text
//!   A  __finally kostu -> ic isleyici EXCEPTION_UNWINDING ile cagrildi
//!   B  sira            -> ictekinden distakine dogru cozuldu
//!   C  zincir kisaldi  -> fs:[0] hedef kayda dustu
//!   D  cagiran devam   -> hedefsiz cagri NORMAL dondu, EAX tasindi
//!   E  hedefe atlama   -> hedef verilince oraya gidildi, EAX = deger
//!   F  GERCEK ISTISNA  -> hata -> sahiplen -> geri sar -> duzelt -> devam
//!   G  olmayan hedef   -> zincirde olmayan kayit reddedildi
//! ```
//!
//! F bu sinavin sebebi. A-E mekanizmayi tek tek olcuyor; F hepsini tek
//! bir gercek olayda birlestiriyor ve `__try`/`__finally`/`__except`
//! ucusunun ucunu de ayni anda calistiriyor.
//!
//! G bir **ret** sinavi: olmayan bir kayda "cekmek" `fs:[0]`i rastgele
//! bir adrese yazmak olurdu ve bir sonraki istisna cop veriye
//! dallanirdi.
//!
//! x86_64'te hepsi **atlanir** ve bu TCMK'nin eksigi degil, Windows'un
//! kendi tercihi: 64-bit'te zincir yoktur, cozum tablo tabanlidir
//! (`.pdata`). Ayni ayrim `winseh`in F ve H sinavlarinda da var.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

// Zincire ozel: yalnizca i386'da kullaniliyor (bkz. `mod chain`).
#[cfg(target_arch = "x86")]
use core::ffi::c_void;
#[cfg(target_arch = "x86")]
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use tcmk::winapi::{self, Window};

tcmk::entry!(main);

const BG: u32 = 0x0018_1422;
const PANEL: u32 = 0x0028_2038;
const FG: u32 = 0x00E8_E4F0;
const DIM: u32 = 0x0094_8CA4;
const ACCENT: u32 = 0x00D0_A0F0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

#[derive(Clone, Copy)]
struct Check {
    name: &'static str,
    detail: &'static str,
    passed: bool,
    skipped: bool,
}

const EMPTY: Check = Check {
    name: "",
    detail: "",
    passed: false,
    skipped: false,
};

const NAMES: [&str; 7] = [
    "A __finally kostu",
    "B sira",
    "C zincir kisaldi",
    "D cagiran devam",
    "E hedefe atlama",
    "F GERCEK ISTISNA",
    "G olmayan hedef",
];

/// `RtlUnwind`in hedefe atladigini gosteren deger. Taninabilir olsun
/// diye secildi: sifir ya da bir, baska yollarla da ortaya cikabilirdi.
const LANDING_MARK: usize = 0x00C0_FFEE;
/// Hedefsiz cagrinin donus degeri -- `LANDING_MARK`tan **ayri** olmasi
/// bilincli: ikisi ayni sayi olsaydi hangi yoldan gelindigi
/// karismis olurdu.
const UNWIND_ONLY_MARK: usize = 0x00BE_EF01;

fn main() {
    let mut checks = [EMPTY; 7];
    #[cfg(target_arch = "x86")]
    run(&mut checks);
    #[cfg(target_arch = "x86_64")]
    for i in 0..checks.len() {
        checks[i] = Check {
            name: NAMES[i],
            detail: "x64'te SEH zinciri yok (tablo tabanli)",
            passed: false,
            skipped: true,
        };
    }
    report(&checks);
    show(&checks);
}

// =====================================================================
// i386: zincirin oldugu tek mimari
// =====================================================================

#[cfg(target_arch = "x86")]
mod chain {
    use super::*;
    use tcmk::seh::{self, ExceptionRecord};

    /// Cozulme sirasini kaydeden sayac -- her isleyici kendi sirasini alir.
    pub static SEQUENCE: AtomicU32 = AtomicU32::new(1);
    /// Ic ve dis isleyicilerin geri sarmada aldigi sira (0 = hic cagrilmadi).
    pub static INNER_ORDER: AtomicU32 = AtomicU32::new(0);
    pub static OUTER_ORDER: AtomicU32 = AtomicU32::new(0);
    /// Geri sarma kaydinda gorulen bayraklar.
    pub static INNER_FLAGS: AtomicU32 = AtomicU32::new(0);
    /// Hedefe atlama saplamasina gelindi mi.
    pub static LANDED: AtomicUsize = AtomicUsize::new(0);

    /// Hatali yazmanin dusmesi gereken yer (bkz. `winseh`teki `SCRATCH`).
    pub static mut SCRATCH: usize = 0;
    /// Gercek istisna sinavinda yazilan deger.
    pub const MARK: usize = 0x1234_ABCD;

    /// Ic kayit: yalnizca temizlik yapar, hicbir istisnayi sahiplenmez.
    ///
    /// Derleyicinin `__finally` icin urettigi seyin karsiligi. Iki dalin
    /// **ayni fonksiyonda** olmasi Windows'un kalibi; ayiran tek sey
    /// bayrak.
    pub unsafe extern "C" fn inner(
        record: *mut ExceptionRecord,
        _establisher: *mut c_void,
        _context: *mut c_void,
        _dispatcher: *mut c_void,
    ) -> i32 {
        if seh::unwinding(record) {
            INNER_FLAGS.store((*record).flags, Ordering::SeqCst);
            INNER_ORDER.store(SEQUENCE.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
            // Geri sarmada donus degeri yok sayilir; Windows da oyle.
            return seh::EXCEPTION_CONTINUE_SEARCH_SEH;
        }
        seh::EXCEPTION_CONTINUE_SEARCH_SEH
    }

    /// Dis kayit: geri sarmada temizlik yapar.
    pub unsafe extern "C" fn outer(
        record: *mut ExceptionRecord,
        _establisher: *mut c_void,
        _context: *mut c_void,
        _dispatcher: *mut c_void,
    ) -> i32 {
        if seh::unwinding(record) {
            OUTER_ORDER.store(SEQUENCE.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
            return seh::EXCEPTION_CONTINUE_SEARCH_SEH;
        }
        seh::EXCEPTION_CONTINUE_SEARCH_SEH
    }

    /// Gercek istisnayi **sahiplenen** dis kayit (`__except`in karsiligi).
    ///
    /// Sirasiyla: once aradaki cerceveleri cozer (`RtlUnwind`), sonra
    /// hatali registeri duzeltip "devam et" der. Ikisinin bu sirada
    /// olmasi sart -- geri sarma, yurutme surmeden once bitmeli.
    pub unsafe extern "C" fn owner(
        record: *mut ExceptionRecord,
        establisher: *mut c_void,
        context: *mut c_void,
        _dispatcher: *mut c_void,
    ) -> i32 {
        if seh::unwinding(record) {
            OUTER_ORDER.store(SEQUENCE.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
            return seh::EXCEPTION_CONTINUE_SEARCH_SEH;
        }

        // Hedefsiz geri sarma: zincir **bu** kayda kadar cozulur ve
        // cagri buraya normal doner.
        winapi::RtlUnwind(
            establisher,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            0,
        );

        // Artik temizlik bitti; hatayi duzeltip yurutmeyi surdurebiliriz.
        seh::set_reg(
            context,
            seh::Reg::C,
            core::ptr::addr_of_mut!(SCRATCH) as usize,
        );
        seh::EXCEPTION_CONTINUE_EXECUTION_SEH
    }

    /// Gecersiz bir adrese yazar; hedef adres `ecx`te durur.
    ///
    /// `winseh::write_through_null` ile **ayni komut** -- duzeltilecek
    /// sey tam olarak o register.
    #[inline(never)]
    pub unsafe fn write_through_null(value: usize) {
        core::arch::asm!("mov [ecx], edx", inout("ecx") 0usize => _, inout("edx") value => _);
    }

    // `RtlUnwind`in hedef adresi olarak verilen inis saplamasi.
    //
    // Buraya gelindiginde yigin, `RtlUnwind` thunk'inin `int 0x2E`
    // aninda birakip gittigi haldedir: `[esp]` cagirana donus adresi,
    // ustundeki dort kelime de `__stdcall` argumanlaridir. Yani
    // thunk'in kendi `ret 16`si yerine ayni isi burasi yapiyor --
    // arada yalnizca "buraya gelindi" isareti birakiliyor.
    //
    // `EAX` degistirilmiyor: `RtlUnwind`in verdigi `ReturnValue` orada
    // duruyor ve cagirana o donuyor.
    // Etiketin uc alt cizgiyle yazilmasi bir yazim hatasi degil:
    // i386 PE/COFF'ta C sembollerine baglayici bir `_` onek ekler, yani
    // Rust'in gordugu `__tcmk_unwind_land`in nesne dosyasindaki adi
    // `___tcmk_unwind_land` olur. `global_asm!` ise adi **oldugu gibi**
    // yazar, o yuzden oneki burada elle koymak gerekiyor.
    core::arch::global_asm!(
        ".globl ___tcmk_unwind_land",
        "___tcmk_unwind_land:",
        "mov dword ptr [{landed}], 1",
        "ret 16",
        landed = sym LANDED,
    );

    extern "C" {
        pub fn __tcmk_unwind_land();
    }
}

#[cfg(target_arch = "x86")]
fn run(checks: &mut [Check; 7]) {
    use chain::*;
    use tcmk::seh::ChainGuard;

    // --- A / B / C / D: hedefsiz geri sarma, istisna olmadan ---
    //
    // Istisnasiz olculmesi kasitli: geri sarma **ayri** bir mekanizma ve
    // tek basina da calismali. Bir istisnayla birlikte olculseydi,
    // gecen bir sinav dagitimin mi geri sarmanin mi calistigini
    // ayirt edemezdi.
    //
    // Uc kayit takiliyor; ortadaki hedef. Yurume ic kayittan baslayip
    // hedefte durmali, yani yalnizca **bir** isleyici cozulmeli.
    let mut target = ChainGuard::new(outer);
    unsafe { target.install() };
    let target_at = &target as *const ChainGuard as usize;

    let mut inner_guard = ChainGuard::new(inner);
    unsafe { inner_guard.install() };

    let before = tcmk::teb::exception_list();
    // Cagri elle kuruluyor cunku `EAX` okunacak: `RtlUnwind`in Rust
    // bildiriminde donus tipi yok (Windows'ta da `void`), oysa
    // `ReturnValue` tam olarak orada tasiniyor.
    let unwind_only_eax: usize;
    unsafe {
        core::arch::asm!(
            "push {value}",
            "push 0",
            "push 0",
            "push {frame}",
            "call {unwind}",
            value = in(reg) UNWIND_ONLY_MARK,
            frame = in(reg) target_at,
            unwind = sym winapi::RtlUnwind,
            out("eax") unwind_only_eax,
            out("ecx") _,
            out("edx") _,
        );
    }
    // Bu satira gelinmesi D'nin yarisi; oteki yarisi degerin tasinmasi.
    let returned = unwind_only_eax == UNWIND_ONLY_MARK;
    let after = tcmk::teb::exception_list();

    let inner_order = INNER_ORDER.load(Ordering::SeqCst);
    let outer_order = OUTER_ORDER.load(Ordering::SeqCst);
    let inner_flags = INNER_FLAGS.load(Ordering::SeqCst);

    checks[0] = Check {
        name: NAMES[0],
        detail: if inner_order == 0 {
            "ic isleyici geri sarmada CAGRILMADI"
        } else if inner_flags & tcmk::seh::EXCEPTION_UNWINDING == 0 {
            "cagrildi ama EXCEPTION_UNWINDING bayragi yok"
        } else {
            "ic isleyici EXCEPTION_UNWINDING ile cagrildi"
        },
        passed: inner_order != 0 && inner_flags & tcmk::seh::EXCEPTION_UNWINDING != 0,
        skipped: false,
    };

    // B: hedef kaydin **kendisi** cozulmemeli -- yurume orada durur.
    // Windows'un sozlesmesi bu: hedef, hayatta kalan ilk cercevedir.
    let order_ok = inner_order == 1 && outer_order == 0;
    checks[1] = Check {
        name: NAMES[1],
        detail: if outer_order != 0 {
            "HEDEF kayit da cozuldu -- yurume durmadi"
        } else if order_ok {
            "yalnizca hedefin altindakiler cozuldu"
        } else {
            "cozulme sirasi beklenenden farkli"
        },
        passed: order_ok,
        skipped: false,
    };

    let chain_ok = before != after && after == target_at;
    checks[2] = Check {
        name: NAMES[2],
        detail: if after == before {
            "fs:[0] HIC degismedi"
        } else if chain_ok {
            "fs:[0] hedef kayda dustu"
        } else {
            "fs:[0] baska bir yeri gosteriyor"
        },
        passed: chain_ok,
        skipped: false,
    };

    // Cagrinin **hic donmemesi** de bir basarisizliktir, ama o durumda
    // surec coker ve bu satira hic gelinmez. Olculebilen kisim degerin
    // tasinmasi: donus yolu dogru kurulmussa `EAX` verilen sayidir.
    checks[3] = Check {
        name: NAMES[3],
        detail: if returned {
            "hedefsiz cagri dondu ve EAX = verilen deger"
        } else {
            "cagri dondu ama EAX yanlis"
        },
        passed: returned,
        skipped: false,
    };

    // Ic kayit zincirden cozuldu; `Drop` onu bir daha cikarmamali.
    core::mem::forget(inner_guard);

    // --- E: hedefe atlama ---
    //
    // Ayni cagri, bu sefer hedef adresle. Yurutme saplamaya gecmeli ve
    // `ReturnValue` `EAX`te tasinmali. Ikisini birden olcmek sart: tek
    // basina "saplamaya gelindi", degerin tasindigini gostermez.
    let mut second = ChainGuard::new(inner);
    unsafe { second.install() };
    let second_at = &second as *const ChainGuard as usize;

    let landed_value: usize;
    unsafe {
        core::arch::asm!(
            "push {value}",
            "push 0",
            "push {ip}",
            "push {frame}",
            "call {unwind}",
            value = in(reg) LANDING_MARK,
            ip = in(reg) __tcmk_unwind_land as *const () as usize,
            frame = in(reg) second_at,
            unwind = sym winapi::RtlUnwind,
            out("eax") landed_value,
            out("ecx") _,
            out("edx") _,
        );
    }
    let landed = LANDED.load(Ordering::SeqCst);
    let jump_ok = landed == 1 && landed_value == LANDING_MARK;
    checks[4] = Check {
        name: NAMES[4],
        detail: if landed == 0 {
            "hedefe HIC gidilmedi"
        } else if jump_ok {
            "saplamaya gidildi ve EAX = verilen deger"
        } else {
            "saplamaya gidildi ama EAX yanlis"
        },
        passed: jump_ok,
        skipped: false,
    };
    core::mem::forget(second);

    // --- F: gercek istisna, ucu birden ---
    //
    // Sinavin sebebi. Tek bir olayda: hata olusur, ic kayit
    // sahiplenmez, dis kayit sahiplenir ve once **geri sarar** (ic
    // kaydin `__finally`si kosar), sonra hatali registeri duzeltip
    // "devam et" der. Komut tekrarlanir ve yazma dogru yere duser.
    SEQUENCE.store(1, Ordering::SeqCst);
    INNER_ORDER.store(0, Ordering::SeqCst);
    OUTER_ORDER.store(0, Ordering::SeqCst);
    unsafe { SCRATCH = 0 };

    let mut owner_guard = ChainGuard::new(owner);
    unsafe { owner_guard.install() };
    let owner_at = &owner_guard as *const ChainGuard as usize;
    {
        let mut cleanup = ChainGuard::new(inner);
        unsafe { cleanup.install() };
        unsafe { write_through_null(MARK) };
        // Kayit zaten cozuldu; `Drop` onu bir daha cikarmamali.
        core::mem::forget(cleanup);
    }
    let landed_write = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(SCRATCH)) };
    let chain_after = tcmk::teb::exception_list();
    let cleanup_ran = INNER_ORDER.load(Ordering::SeqCst) != 0;
    let real_ok = cleanup_ran && landed_write == MARK && chain_after == owner_at;
    checks[5] = Check {
        name: NAMES[5],
        detail: if !cleanup_ran {
            "istisnada __finally KOSMADI"
        } else if landed_write != MARK {
            "temizlik kostu ama komut tekrarlanmadi"
        } else if chain_after != owner_at {
            "temizlik ve duzeltme oldu ama zincir cozulmedi"
        } else {
            "coz, temizle, duzelt, devam et -- ucu de"
        },
        passed: real_ok,
        skipped: false,
    };
    core::mem::forget(owner_guard);

    // --- G: zincirde olmayan hedef ---
    //
    // Bir **ret** sinavi. Kabul edilseydi `fs:[0]` yiginda var olmayan
    // bir adrese yazilirdi ve bir sonraki istisna cop veriye dallanirdi.
    let mut live = ChainGuard::new(inner);
    unsafe { live.install() };
    let head_before = tcmk::teb::exception_list();
    let bogus = 0x0000_2000usize;
    unsafe {
        winapi::RtlUnwind(
            bogus as *mut c_void,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            0,
        )
    };
    let head_after = tcmk::teb::exception_list();
    let refused = head_after == head_before;
    checks[6] = Check {
        name: NAMES[6],
        detail: if refused {
            "olmayan hedef reddedildi, zincire dokunulmadi"
        } else {
            "olmayan hedef KABUL edildi"
        },
        passed: refused,
        skipped: false,
    };
    drop(live);
}

fn report(checks: &[Check; 7]) {
    use core::fmt::Write;
    let mut console = winapi::Console;
    for check in checks {
        let _ = writeln!(
            console,
            "[winunwind] {}: {} ({})",
            check.name,
            if check.skipped {
                "atlandi"
            } else if check.passed {
                "gecti"
            } else {
                "KALDI"
            },
            check.detail
        );
    }
}

fn show(checks: &[Check; 7]) {
    let mut win = match Window::create("winunwind -- geri sarma", 255, 165, 500, 200) {
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
    win.text(6, 3, "__finally ancak geri sarma varsa kosar", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        let (label, color) = if check.skipped {
            ("atlandi", DIM)
        } else if check.passed {
            ("gecti", OK)
        } else {
            ("KALDI", WARN)
        };
        win.text(395, y, label, color);
        y += 16;
    }

    let done = checks.iter().filter(|c| c.passed || c.skipped).count();
    win.text(
        6,
        h - 14,
        if done == checks.len() {
            "hepsi gecti   q cik"
        } else {
            "BIR SINAV KALDI   q cik"
        },
        if done == checks.len() { OK } else { WARN },
    );
}
