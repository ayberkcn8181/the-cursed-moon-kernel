//! Yigin otomatik buyume -- koruma sayfasi artik bir **duvar degil**.
//!
//! Onceki bati yigin tasmasini **gorunur** kildi: yigin ile program
//! break arasina Ring 3'e kapali bir sayfa konuldu ve tasma oraya
//! dokununca sayfa hatasi olustu. Ama o sayfa bir duvardi -- tasan
//! program oluyordu, yalnizca tanisiyla birlikte.
//!
//! Gercek sistemlerde yigin **buyur**. Duvara dokunmak bir son degil,
//! bir **istek**: "daha fazla yigin lazim". Cekirdek sayfayi yigina
//! katiyor ve bir asagisini yeni duvar yapiyor.
//!
//! ```text
//!   once:   [ brk ... ][ DUVAR ][ yigin ]
//!   sonra:  [ brk ... ][ DUVAR ][ yigin + 1 sayfa ]
//!                       ^ bir sayfa asagi kaydi
//! ```
//!
//! ## Sinir nerede
//!
//! Iki yandan sinirli ve ikisi de gercek:
//!
//! ```text
//!   yukaridan  STACK_MAX      -- bir surec sinirsiz yigin alamaz
//!   asagidan   program break  -- heap'in ustune buyuyemez
//! ```
//!
//! Ikincisi TCMK'nin yerlesiminden geliyor ve gercek sistemlerde de
//! vardir: Linux'ta yigin asagi, heap yukari buyur ve aralarinda bir
//! **bosluk** olmak zorundadir. Bosluk bitince buyume durur -- orada da
//! `SIGSEGV`.
//!
//! ## Iki ABI, iki sozlesme
//!
//! Buyumenin **mekanizmasi** iki yuzde de ayni, ama **sozlesmesi**
//! degil:
//!
//! ```text
//!   POSIX    cekirdek sessizce buyutur.
//!            Program hicbir sey gormez; sinirda SIGSEGV gelir.
//!
//!   Windows  ilk dokunusta STATUS_GUARD_PAGE_VIOLATION atilir.
//!            Program onu GORUR -- "yigin sonuna yaklasiyorum" diye
//!            okuyabilir, ve koruma sayfasini kendi yeniden kurabilir.
//! ```
//!
//! TCMK su an POSIX'in sessiz bicimini iki yuze de uyguluyor; Windows'un
//! gorunur bicimi (tek atimlik istisna) yok (bkz. README).

use crate::level0a::core::{mmu, scheduler};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Sayfa olcusu -- yigin sayfa sayfa buyur.
const PAGE: usize = 4096;

/// Bir yiginin buyuyebilecegi en buyuk olcu.
///
/// Sinir sart ve sebebi tek cumlede: sinirsiz buyume, sonsuz
/// ozyinelemeyi bir hata olmaktan cikarip **butun sistemi tuketen** bir
/// olaya cevirirdi. Gercek Linux'ta bu sinirin adi `RLIMIT_STACK`;
/// TCMK'de sabit, cunku kaynak sinirlari henuz yok.
pub const STACK_MAX: usize = 128 * 1024;

/// Gorevin su anki koruma sayfasi; 0 = kayit yok.
static GUARD: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];
/// Yiginin **tepesi** -- buyume bundan asagi olculuyor.
static TOP: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Olcum: kac sayfa eklendi, kac istek reddedildi.
static GROWN: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);
/// Gorulen en buyuk yigin (bayt).
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// `(eklenen sayfa, reddedilen istek, en buyuk yigin)` -- kabuk raporu.
pub fn stats() -> (usize, usize, usize) {
    (
        GROWN.load(Ordering::Relaxed),
        REFUSED.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed),
    )
}

/// Yeni bir imajin yigin yerlesimini kaydeder.
pub fn install(task: usize, guard: usize, top: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    GUARD[task].store(guard, Ordering::Relaxed);
    TOP[task].store(top, Ordering::Relaxed);
    PEAK.fetch_max(top.saturating_sub(guard + PAGE), Ordering::Relaxed);
}

/// `fork`: cocuk ayni yerlesimi devralir.
///
/// Adres uzayi kopyalandigi icin adresler oldugu gibi gecerli -- cocuk
/// ayni yerde, ayni kadar buyumus bir yigin goruyor. Devretmemek,
/// cocugun yigininin **hic** buyuyememesi olurdu.
pub fn clone_into(parent: usize, child: usize) {
    if parent >= scheduler::MAX_TASKS || child >= scheduler::MAX_TASKS {
        return;
    }
    GUARD[child].store(GUARD[parent].load(Ordering::Relaxed), Ordering::Relaxed);
    TOP[child].store(TOP[parent].load(Ordering::Relaxed), Ordering::Relaxed);
}

/// Yuvayi unutur (gorev cikisi, yuva yeniden kullanimi).
pub fn forget(task: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    GUARD[task].store(0, Ordering::Relaxed);
    TOP[task].store(0, Ordering::Relaxed);
}

/// Gorevin su anki koruma sayfasi (kabuk raporu / sinav icin).
pub fn guard_of(task: usize) -> usize {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    GUARD[task].load(Ordering::Relaxed)
}

/// Yiginin su anki olcusu (bayt).
pub fn size_of(task: usize) -> usize {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    let guard = GUARD[task].load(Ordering::Relaxed);
    let top = TOP[task].load(Ordering::Relaxed);
    top.saturating_sub(guard + PAGE)
}

/// Koruma sayfasina dokunan bir hatayi **buyume istegi** olarak okur.
///
/// Doner: yigin buyudu mu. `false` ise hata gercekten olumcul -- cagiran
/// olagan yola devam eder (`SIGSEGV` ya da Windows istisnasi).
///
/// ## Neden yalnizca koruma sayfasinin kendisi
///
/// Buyume istegi diye kabul edilen tek adres, su anki koruma
/// sayfasidir. Daha asagisina dokunan bir erisim yigin tasmasi degil,
/// **kacik bir isaretci**: duvarin uzerinden atlamis demektir ve onu
/// buyume sayip araya sayfa dosemek, hatayi gizlemek olurdu.
///
/// Gercek Linux'ta kural biraz daha genis (yigin isaretcisine yakin
/// adresler de kabul edilir, cunku derleyici buyuk yerel diziler icin
/// `sub rsp, N` yapip ortadan dokunabilir). TCMK'de derleyicinin yigin
/// yoklamasi (`__chkstk`) yok ve sayfa sayfa buyume yeterli.
///
/// ## Bu denetim su an **sinanamiyor**
///
/// Kaldirildiginda hicbir sinav kalmiyor, ve sebebi kaydedilmeye deger:
/// `grow` yalnizca **korumali** bir sayfaya dusen hatalar icin cagriliyor
/// (`exceptions.rs`), bugunku TCMK'de bir kullanici adres uzayindaki tek
/// korumali sayfa da surecin yigin duvari. Yani Ring 3'ten buraya
/// uyusmayan bir adresle gelmek mumkun degil: kacik isaretciler eslenmemis
/// sayfalara duser ve bu kod yolunu hic gormez.
///
/// Denetim yine de duruyor, cunku kodladigi sart gercek ve yakinda
/// ulasilabilir olacak: iplik yiginlari kendi duvarlarini aldigi anda bir
/// ipligin digerinin duvarina dokunmasi mumkun olur -- ve o zaman bu
/// denetim, A ipliginin yigininin B'nin duvarina dogru sessizce
/// buyumesini engelleyen tek sey olacak.
///
/// # Safety
/// Cagiran gorevin adres uzayi etkin olmalidir.
pub unsafe fn grow(fault_addr: usize) -> bool {
    let task = scheduler::current_id();
    if task >= scheduler::MAX_TASKS {
        return false;
    }
    let guard = GUARD[task].load(Ordering::Relaxed);
    let top = TOP[task].load(Ordering::Relaxed);
    if guard == 0 || top == 0 {
        return false;
    }
    if fault_addr & !(PAGE - 1) != guard {
        return false;
    }

    // --- Yukaridan sinir: yigin ne kadar buyuyebilir ---
    //
    // Koruma sayfasi bir asagi inerse yigin `top - guard` olur.
    let next_size = top.saturating_sub(guard);
    if next_size > STACK_MAX {
        REFUSED.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    // --- Asagidan sinir: heap'in ustune buyuyemez ---
    //
    // Yeni duvar `guard - PAGE` olacak. O adres program break'in
    // altinda kalirsa yigin heap'e girmis olurdu -- ve iki bolgenin
    // birbirine karismasi, sessizce birbirinin verisini ezmek demek.
    let next_guard = guard.wrapping_sub(PAGE);
    let brk = crate::level0a::kernel_api::program_break();
    if next_guard < brk || next_guard < mmu::USER_MEM_START {
        REFUSED.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    // Once yeni duvar kuruluyor, sonra eskisi aciliyor.
    //
    // Sira onemli: ters olsaydi iki islem arasinda duvarsiz bir an
    // olurdu ve tam o anda gelen ikinci bir tasma hatasiz gecerdi.
    if !mmu::guard_user_page(next_guard) {
        REFUSED.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    if !mmu::unguard_user_page(guard) {
        // Yeni duvar kuruldu ama eskisi acilamadi: geri al, yoksa
        // ust uste iki duvar kalirdi.
        mmu::unguard_user_page(next_guard);
        REFUSED.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    GUARD[task].store(next_guard, Ordering::Relaxed);
    // Break'in tavani da duvarla birlikte iniyor. Birakilsaydi `brk`
    // artik yigina ait olan bir adrese kadar buyuyebilirdi.
    crate::level0a::kernel_api::lower_break_limit(next_guard);

    GROWN.fetch_add(1, Ordering::Relaxed);
    PEAK.fetch_max(next_size, Ordering::Relaxed);
    true
}
