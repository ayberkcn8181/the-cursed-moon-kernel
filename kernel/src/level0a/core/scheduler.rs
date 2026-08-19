//! Round-robin gorev zamanlayici (doc S.7 Faz 2: "idle + worker").
//!
//! Faz 2'de gecisler **isbirlikcidir** (cooperative): gorevler `yield_now()`
//! cagirir, PIT kesmesi yalnizca "zaman dilimi doldu" bayragini kaldirir.
//! Kesme icinden dogrudan baglam degistirmek (preemption) IRQ frame'inin de
//! tasinmasini gerektirir ve Faz 4'teki mesaj kuyrugu/APIC calismasiyla
//! birlikte ele alinacaktir.
//!
//! Heartbeat, doc S.11 uyarinca **scheduler dongusunde** artirilir: boylece
//! "Level-0a yasiyor mu" sorusunun cevabi gercekten gorev dongusunun
//! ilerlemesine baglanir, sadece timer'in atmasina degil.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::arch::cpu::context::{arch_context_switch, bootstrap_stack};
use crate::level0a::core::kmalloc;

/// Gorev tablosu boyutu.
///
/// Sekizdi; kabuk komutlari ayri bir goreve tasininca (bkz.
/// `shell::command_task`) kalici tuketim bese cikti -- idle, masaustu,
/// kabuk ve acilista baslatilan iki uygulama. Kullaniciya uc yuva
/// kaliyordu ve `fork` yapan bir uygulama ikisini birden yiyordu.
///
/// Maliyet gorev basina 16 KiB cekirdek yigini + 16 KiB Ring 3 yigini;
/// dort yuva daha 128 KiB demek, heap 12 MiB.
pub const MAX_TASKS: usize = 12;
/// Idle gorevi her zaman 0 numaradadir ve ayni zamanda **masaustu
/// dongusudur** (bkz. `main.rs`); oncelik muhasebesinin disindadir.
pub const IDLE_TASK: usize = 0;
pub const TASK_STACK_SIZE: usize = 16 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskState {
    Unused,
    Ready,
    Running,
    /// Belirli bir PIT tick'ine kadar zamanlanmaz (`sleep`).
    Blocked,
    /// Bir cocuk gorevin bitmesini bekliyor (`waitpid`). Zamanla degil,
    /// **baska bir gorevin durumuyla** uyanir.
    ///
    Waiting,
    /// Bir **sinyal** bekliyor (`pause` / `sigsuspend`).
    ///
    /// `IoWait`ten ayri tutulmasi sart: `wake_io_waiters` butun aygit
    /// bekleyenlerini birden uyandirir, yani bir disk kesmesi sinyal
    /// bekleyen sureci de yanlislikla kaldirirdi. Sinyal beklemesi tek
    /// hedeflidir -- yalnizca sinyalin gonderildigi gorev uyanir.
    SigWait,
    /// Bir aygit kesmesi bekliyor (`TaskState::IoWait`).
    ///
    /// `Blocked`'tan farki uyanma kaynagidir: zaman degil, **donanim**.
    /// Ayri bir durum olmasi sart -- yoksa "5 tik sonra uyan" ile
    /// "disk hazir olunca uyan" ayni yuvayi paylasir ve biri otekini
    /// yanlislikla uyandirir.
    IoWait,
    Terminated,
}

#[derive(Clone, Copy)]
pub struct Task {
    pub state: TaskState,
    pub stack_pointer: usize,
    pub name: &'static str,
    /// Bu gorev Ring 3'e girdiginde TSS.esp0/rsp0'a yazilacak yigin.
    /// Her gorevin AYRI olmasi sarttir: aksi halde iki Ring 3 sureci
    /// ayni cekirdek yiginini ezer.
    pub kernel_stack_top: usize,
    /// Ring 3'ten `sys_exit` ile donus icin saklanan cekirdek baglami.
    pub user_resume: usize,
    /// Gorev su an Ring 3'te mi calisiyor?
    pub in_user_mode: bool,
    /// Gorevin adres uzayi (CR3). 0 = cekirdek uzayi.
    ///
    /// Baglam degisiminde yuklenir; boylece her Ring 3 sureci kendi
    /// kullanici bellegini gorur (bkz. `core::mmu`).
    pub address_space: usize,
    /// `Blocked` iken: bu PIT tick'inde uyandirilir.
    pub wake_tick: u32,
    /// `Waiting` iken: beklenen gorevin indeksi + 1 (0 = beklemiyor).
    /// Sifirdan farkli olmasi gerekir cunku 0 gecerli bir gorev indeksi.
    pub wait_for: usize,
    /// Gorev sonlandiginda birakilan cikis kodu; `waitpid` bunu okur.
    pub exit_code: u32,
    /// POSIX `nice` degeri: -20 (en yuksek oncelik) .. 19 (en dusuk).
    ///
    /// Oncelik, gorevin **zaman diliminin uzunlugunu** belirler; secim
    /// sirasini degil. Bu ayrim bilincli: sira degistirseydi dusuk
    /// oncelikli bir gorev, yuksek oncelikli biri kostugu surece hic
    /// zamanlanmazdi (aclik). Dilim uzunluguyla oynamak ise herkesin
    /// ilerlemesini garanti ederken CPU payini oranlar.
    pub nice: i8,
    /// Bu gorevin dilim icinde kalan tik hakki. Sifirlandiginda
    /// zamanlayici kesmesi baglam degisimi ister.
    pub credits: u32,
    /// Gorevi olusturan gorev. `waitpid(-1)` "cocuklarimdan herhangi
    /// biri" sorusunu bununla cevaplar.
    pub parent: usize,
    /// Sonlandiginda **zombi** olarak beklesin mi?
    ///
    /// Yalnizca `fork` cocuklari icin dogrudur: cikis kodunu toplayacak
    /// bir ebeveyn ancak orada vardir. Kabuktan baslatilan uygulamalari
    /// kimse beklemez, o yuzden onlarin yuvasi cikista hemen geri alinir
    /// -- aksi halde gorev tablosu birkac uygulama sonra dolardi.
    pub waitable: bool,
    /// Cekirdek yiginin TEPESI. Yuva geri kazanildiginda yigin yeniden
    /// ayrilmaz, ayni bellek `bootstrap_stack` ile bastan kurulur --
    /// `kmalloc` bir bump ayiricidir ve `free` sunmaz, yani yeniden
    /// ayirmak sizinti demek olurdu.
    pub stack_top: usize,
    /// Gorevin CPU'da gecirdigi toplam tik. Oncelik gercekten ise
    /// yariyor mu sorusunun tek dogru cevabi budur -- uygulamanin kendi
    /// sayaci uyku/G-C ile bozulabilir, bu sayac bozulmaz.
    pub cpu_ticks: u32,
}

impl Task {
    const fn empty() -> Self {
        Task {
            state: TaskState::Unused,
            stack_pointer: 0,
            name: "",
            kernel_stack_top: 0,
            user_resume: 0,
            in_user_mode: false,
            address_space: 0,
            wake_tick: 0,
            wait_for: 0,
            nice: 0,
            credits: 0,
            cpu_ticks: 0,
            parent: 0,
            waitable: false,
            stack_top: 0,
            exit_code: 0,
        }
    }
}

static mut TASKS: [Task; MAX_TASKS] = [Task::empty(); MAX_TASKS];
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static TASK_COUNT: AtomicUsize = AtomicUsize::new(0);
static NEED_RESCHED: AtomicBool = AtomicBool::new(false);
static SWITCHES: AtomicUsize = AtomicUsize::new(0);
/// Zorla baglam degisimi (preemption) sayaci.
static IO_WAITS: AtomicUsize = AtomicUsize::new(0);
static PREEMPTIONS: AtomicUsize = AtomicUsize::new(0);
/// Sifirdan buyukse zamanlayici kesmesi baglam degistirmez.
static PREEMPT_LOCK: AtomicUsize = AtomicUsize::new(0);

/// Halihazirda calisan cekirdek akisini 0 numarali gorev ("idle") olarak
/// kaydeder. Bu gorevin yigini zaten `_start` tarafindan kurulmustur, bu
/// yuzden ayrica tahsis edilmez.
pub fn init() {
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(0)).state = TaskState::Running;
        (*tasks.add(0)).name = "idle";
    }
    CURRENT.store(0, Ordering::Relaxed);
    TASK_COUNT.store(1, Ordering::Relaxed);
}

/// Yeni bir cekirdek gorevi olusturur; yigini kmalloc'tan alinir.
/// Basarisizlik nedenleri: gorev tablosu dolu veya heap tukendi.
pub fn spawn(name: &'static str, entry: extern "C" fn() -> !) -> Option<usize> {
    spawn_inner(name, entry, false)
}

/// **Uyutulamayan** gorev: masaustu dongusu.
///
/// 0 numara (idle) her zaman uyutulamazdi -- uyuyacak baska gorev
/// olmayabilir. Masaustu ise ayri bir sebeple uyutulamaz: ekrani cizen,
/// girdiyi dagitan ve **kabugu kosturan** gorev odur. Uykuya girerse
/// ekran donar ve kullanici hicbir sey yapamaz.
///
/// Bu, masaustu ayri bir goreve tasindiginda (o zamana kadar kabuk 0
/// numarada kosuyordu) sessizce kirilmisti: `wait_for_io` yalnizca
/// "0 numara" istisnasina bakiyordu, oysa kabuk artik baska bir
/// numaradaydi. Sonuc, kabuktan verilen ilk disk **yazmasinda** butun
/// masaustunun donmasiydi -- okuma yollari inode onbelleginden
/// dondugu icin belirti yalnizca yazmada gorunuyordu.
static NO_BLOCK: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Bu gorevi uyutulamaz olarak isaretler.
pub fn mark_no_block(index: usize) {
    NO_BLOCK.store(index, Ordering::Relaxed);
}

/// Gorev uyutulabilir mi? Idle ve masaustu haric hepsi uyutulabilir.
pub fn can_block(index: usize) -> bool {
    index != IDLE_TASK && index != NO_BLOCK.load(Ordering::Relaxed)
}

/// Calisan gorev uyutulabilir mi?
pub fn current_can_block() -> bool {
    can_block(CURRENT.load(Ordering::Relaxed))
}

/// `fork` icin: cocuk **beklenebilir** olarak isaretlenir, yani cikis
/// kodu toplanana kadar zombi olarak kalir.
pub fn spawn_child(name: &'static str, entry: extern "C" fn() -> !) -> Option<usize> {
    spawn_inner(name, entry, true)
}

fn spawn_inner(
    name: &'static str,
    entry: extern "C" fn() -> !,
    waitable: bool,
) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;

        // Once bosalmis bir yuva aranir. Onceki surum yalnizca tablonun
        // sonuna EKLIYORDU: sonlanan gorevlerin yuvasi hicbir zaman geri
        // alinmiyordu, yani sistem toplam MAX_TASKS kadar gorev
        // baslatabiliyor ve sonra hicbir uygulama acilamiyordu.
        let mut index = usize::MAX;
        let count = TASK_COUNT.load(Ordering::Relaxed);
        for i in 0..count {
            if (*tasks.add(i)).state == TaskState::Unused {
                index = i;
                break;
            }
        }
        if index == usize::MAX {
            if count >= MAX_TASKS {
                return None;
            }
            index = count;
            TASK_COUNT.store(count + 1, Ordering::Relaxed);
        }

        // Yigin: yuva daha once kullanildiysa ayni bellek yeniden kurulur.
        // `kmalloc` bir bump ayiricidir ve `free` sunmaz -- her geri
        // kazanimda yeniden ayirmak, gorev basina 32 KiB'lik sessiz bir
        // sizinti olurdu.
        let (stack_top, kstack_top) = if (*tasks.add(index)).stack_top != 0 {
            (
                (*tasks.add(index)).stack_top,
                (*tasks.add(index)).kernel_stack_top,
            )
        } else {
            let stack = kmalloc::kmalloc_aligned(TASK_STACK_SIZE, 16)?;
            let kstack = kmalloc::kmalloc_aligned(TASK_STACK_SIZE, 16)?;
            (
                stack.add(TASK_STACK_SIZE) as usize,
                kstack.add(TASK_STACK_SIZE) as usize,
            )
        };

        let sp = bootstrap_stack(stack_top as *mut usize, entry);

        // Donem hakki BASLANGICTA verilmeli. Sifir birakilirsa gorev,
        // secimde "hakki yok" diye atlanir ve ilk donem yenilenene kadar
        // hic kosmaz -- yeni surecin dogar dogmaz ac kalmasi demektir.
        (*tasks.add(index)).nice = 0;
        (*tasks.add(index)).credits = slice_ticks(0);
        (*tasks.add(index)).cpu_ticks = 0;
        (*tasks.add(index)).state = TaskState::Ready;
        (*tasks.add(index)).stack_pointer = sp;
        (*tasks.add(index)).stack_top = stack_top;
        (*tasks.add(index)).name = name;
        (*tasks.add(index)).kernel_stack_top = kstack_top;
        (*tasks.add(index)).user_resume = 0;
        (*tasks.add(index)).in_user_mode = false;
        (*tasks.add(index)).address_space = 0;
        (*tasks.add(index)).wake_tick = 0;
        (*tasks.add(index)).wait_for = 0;
        (*tasks.add(index)).exit_code = 0;
        (*tasks.add(index)).parent = CURRENT.load(Ordering::Relaxed);
        (*tasks.add(index)).waitable = waitable;

        // Calisma dizini yuvaya baglidir, imaja degil. Sifirlamanin
        // burada olmasi `execve`nin cwd'yi **korumasini** kendiliginden
        // saglar -- POSIX'in istedigi de budur: exec yuvayi yeniden
        // kullanir, yeni bir yuva ayirmaz. `fork` ise ayri yuva aldigi
        // icin ebeveynin dizinini ayrica kopyalar (bkz. `cwd::clone_into`).
        crate::level0a::core::cwd::reset(index);
        // Ortam da yuvaya bagli ve ayni gerekceyle: `execve` korusun,
        // `fork` ayrica kopyalasin. Yeni yuva **oturum tablosundan**
        // doguyor -- kabukta `set` ile kurulan ortam boylece miras
        // kaliyor.
        crate::level0a::core::env::reset(index);

        Some(index)
    })
}

/// Bir gorevin ebeveyni (POSIX `getppid`).
///
/// Yuva geri kazanildigi icin "ebeveyn hala yasiyor mu" sorusu ayri;
/// burada yalnizca kayitli deger doner. Init'in ebeveyni kendisidir --
/// gercek POSIX'te de `getppid()` bir noktada 1'e (ya da 0'a) dayanir.
pub fn parent_of(task: usize) -> usize {
    if task >= MAX_TASKS {
        return 0;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (*tasks.add(task)).parent
    })
}

/// Sonlanmis bir gorevin yuvasini geri verir.
///
/// Yigin bellegi **birakilmaz**, yuvada saklanir: bir sonraki `spawn` onu
/// yeniden kullanir (bkz. `spawn_inner`).
fn release_slot(index: usize) {
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(index)).state = TaskState::Unused;
        (*tasks.add(index)).waitable = false;
        (*tasks.add(index)).wait_for = 0;
        (*tasks.add(index)).credits = 0;
        (*tasks.add(index)).name = "";
    }
}

/// Ebeveyni artik var olmayan zombileri temizler.
///
/// Zombi, cikis kodu toplansin diye bekleyen sonlanmis bir gorevdir. Onu
/// toplayacak ebeveyn de olduyse kimse toplamayacak demektir; yuva
/// sonsuza kadar tutulmamalidir.
fn reap_orphans() {
    let count = TASK_COUNT.load(Ordering::Relaxed);
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        for i in 0..count {
            if (*tasks.add(i)).state != TaskState::Terminated || !(*tasks.add(i)).waitable {
                continue;
            }
            let parent = (*tasks.add(i)).parent;
            let parent_alive = parent < count
                && (*tasks.add(parent)).state != TaskState::Unused
                && (*tasks.add(parent)).state != TaskState::Terminated;
            if !parent_alive {
                release_slot(i);
            }
        }
    }
}

/// PIT kesmesinden cagrilir: zaman dilimi doldu bayragini kaldirir.
/// Bir gorevin `nice` degerine karsilik gelen **donem hakki** (PIT tik'i).
///
/// PIT 100 Hz oldugu icin 1 tik = 10 ms. Varsayilan (`nice = 0`) iki
/// tiktir; en dusuk oncelik bile **en az bir tik** alir, yani hicbir
/// gorev tamamen ac kalmaz. En yuksek oncelik en dusugun sekiz kati
/// CPU'ya kadar cikabilir.
///
/// ## Neden "dilim uzunlugu" degil de "donem hakki"
///
/// Ilk surumde bu deger yalnizca zaman diliminin uzunluguydu: yuksek
/// oncelikli gorev daha uzun kesintisiz kosuyordu. Olcum bunun **hicbir
/// ise yaramadigini** gosterdi -- iki `race` kopyasi -20 ve 19 ile
/// kosturuldugunda sayaclari tipatip ayni arttir (+385 / +386).
///
/// Sebep: GUI uygulamalari her kare sonunda `win_flush` ile CPU'yu
/// GONULLU birakir, yani dilimlerini hicbir zaman doldurmaz. Dilim
/// uzunlugu ancak zorla kesilen bir gorev icin anlamlidir; gonullu
/// birakan iki gorev, dilimleri ne olursa olsun sirayla birer tur alir.
///
/// Bu yuzden deger artik bir **donem butcesi**dir: gorev tiklerini
/// harcadikca hakki azalir ve hakki bitince, donem yenilenene kadar
/// SECILMEZ. Boylece oncelik "ne kadar uzun kostugu"nu degil "kac kez
/// secildigi"ni belirler -- gonullu birakan yuklerde de calisan tek
/// yontem budur.
///
/// Tablo, formul yerine bilincli olarak acik yazildi: zamanlayici
/// davranisini okurken "hangi nice ne kadar hak alir" sorusunun cevabi
/// hesap yapmadan gorunsun.
pub fn slice_ticks(nice: i8) -> u32 {
    match nice {
        i8::MIN..=-10 => 8,
        -9..=-1 => 4,
        0..=4 => 2,
        _ => 1,
    }
}

/// Zamanlayici kesmesi: calisan gorevin donem hakkini eksiltir.
///
/// Onceden her tik kosulsuz baglam degisimi isterdi -- yani hak diye bir
/// sey yoktu ve oncelik de olamazdi.
pub fn on_timer_tick() {
    let current = CURRENT.load(Ordering::Relaxed);
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(current)).cpu_ticks = (*tasks.add(current)).cpu_ticks.wrapping_add(1);
        let credits = &mut (*tasks.add(current)).credits;
        if *credits > 1 {
            *credits -= 1;
            return;
        }
        *credits = 0;
    }
    NEED_RESCHED.store(true, Ordering::Relaxed);
}

/// Yeni donem: butun gorevlerin hakki `nice` degerine gore tazelenir.
///
/// Donem, "hicbir hazir gorevin hakki kalmadi" aninda baslar -- ayri bir
/// zamanlayici ya da sayac gerekmez, kosul secim aninda zaten bilinir.
fn refill_credits(count: usize) {
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        for i in 0..count {
            (*tasks.add(i)).credits = slice_ticks((*tasks.add(i)).nice);
        }
    }
}

/// Gorevin donem icinde kalan hakki (tanilama icin).
pub fn credits_of(index: usize) -> u32 {
    if index >= MAX_TASKS {
        return 0;
    }
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (*tasks.add(index)).credits
    }
}

/// Gorevin CPU'da gecirdigi toplam tik (10 ms).
pub fn cpu_ticks_of(index: usize) -> u32 {
    if index >= MAX_TASKS {
        return 0;
    }
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (*tasks.add(index)).cpu_ticks
    }
}

/// Gorevin oncelik degerini okur.
pub fn nice_of(index: usize) -> i8 {
    if index >= MAX_TASKS {
        return 0;
    }
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (*tasks.add(index)).nice
    }
}

/// Gorevin oncelik degerini ayarlar; POSIX araligina kirpar.
///
/// Yeni dilim **hemen** gecerli olmaz: calisan gorevin kalan hakki
/// tuketilir, sonraki secimde yeni deger uygulanir. Boylece bir gorev
/// kendi oncelikini yukselterek dilim ortasinda ek sure kazanamaz.
pub fn set_nice(index: usize, value: i8) -> Result<(), &'static str> {
    if index >= MAX_TASKS {
        return Err("gecersiz gorev numarasi");
    }
    let clamped = value.clamp(-20, 19);
    crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        if (*tasks.add(index)).state == TaskState::Unused {
            return Err("gorev yok");
        }
        (*tasks.add(index)).nice = clamped;
        Ok(())
    })
}

/// Bolunemez bir is boyunca zorla baglam degisimini kapatir.
///
/// Bir donem tek kullanicisi ATA surucusuydu: komut dizisi (surucu
/// secimi -> LBA -> komut -> veri) bir butundur ve ortasinda baska bir
/// gorev ikinci bir komut baslatirsa denetleyicinin durumu bozulur.
///
/// O kullanim **kaldirildi**. "Kimse calismasin" demek, korunmasi gereken
/// sey yalnizca *bir aygit* iken fazla genis bir cozumdu; artik surucu
/// kendi aygit kilidini tutuyor (bkz. `drivers::ata::DeviceLock`) ve
/// disk calisirken diger gorevler serbestce kosuyor.
///
/// Primitif yine de duruyor: kesmelerin acik kalmasi gereken ama baglam
/// degisiminin olmemesi gereken isler icin dogru arac budur.
#[allow(dead_code)]
pub fn preempt_disable() {
    PREEMPT_LOCK.fetch_add(1, Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn preempt_enable() {
    PREEMPT_LOCK.fetch_sub(1, Ordering::Relaxed);
}

/// Zamanlayici kesmesinden cagrilir: zaman dilimi dolduysa gorevi
/// **kendi istegi olmadan** birakir.
///
/// Isbirlikci modelde `yield` cagirmayan bir dongu tum sistemi
/// kilitliyordu; Yuk Dengeleyici yalnizca syscall yagmurunu
/// hafifletebiliyordu, saf bir hesap dongusune caresizdi.
///
/// Yapisal olarak syscall yolundan farki yok: orada da baglam degisimi
/// kesme yigininda (Ring 3 icin TSS.esp0) yapiliyor ve donuste ayni
/// noktaya donuluyor. Kesme kapisi IF=0 ile girildigi icin ic ice
/// preemption olmaz; `without_interrupts` bolgeleri de dogal olarak
/// korunur.
pub fn preempt_from_timer() {
    // Scheduler daha ayaga kalkmadiysa gorev tablosu bostur.
    if TASK_COUNT.load(Ordering::Relaxed) < 2 {
        return;
    }
    if PREEMPT_LOCK.load(Ordering::Relaxed) != 0 {
        return;
    }
    if !NEED_RESCHED.load(Ordering::Relaxed) {
        return;
    }
    PREEMPTIONS.fetch_add(1, Ordering::Relaxed);
    yield_now();
}

pub fn preemptions() -> usize {
    PREEMPTIONS.load(Ordering::Relaxed)
}

/// Zamanlayici baglam degisimi istiyor mu?
///
/// Su an yalnizca `preempt_from_timer` icinden okunuyor; disariya acik
/// kalmasi bilincli, cunku uzun suren cekirdek islerinin arasinda
/// "sirami birakmali miyim" diye sormasi dogru olan yontemdir.
#[allow(dead_code)]
pub fn needs_resched() -> bool {
    NEED_RESCHED.load(Ordering::Relaxed)
}

/// CPU'yu bir sonraki hazir goreve birakir. Baska calistirilabilir gorev
/// yoksa hicbir sey yapmadan doner.
pub fn yield_now() {
    NEED_RESCHED.store(false, Ordering::Relaxed);

    // Doc S.11: nabiz "scheduler dongusu ilerliyor mu" sorusunu olcer,
    // "baglam degisiyor mu" sorusunu DEGIL. Bu yuzden beat() asagidaki erken
    // donusten ONCE gelir: tek gorev (idle) kaldiginda sistem saglikli
    // sekilde bosta calisiyordur, olu degil.
    crate::level0a::pit::beat();

    // Secim VE takas tek parcada, kesmeler kapali yapilir.
    //
    // Onceden yalnizca secim korunuyordu; takasin kendisi kesmelere
    // acikti. O aralikta gelen bir zamanlayici kesmesi `preempt_from_timer`
    // uzerinden `yield_now`'a IC ICE girebiliyordu: CURRENT yeni goreve
    // yazilmis ama yigin henuz degismemis oluyor, bu yuzden ic cagri
    // ESKI yigini YENI gorevin yuvasina kaydediyordu. Sonucu, bir
    // sonraki takasta `popfl`'in bozuk bir EFLAGS okumasi -- ve TF biti
    // oradan gelirse aninda #DB istisnasi.
    //
    // Kesmeleri kapatmak burada tutarlidir: her gorev ayni sarmalayicinin
    // icinde takas edilir, yani IF=0 ile cikip IF=0 ile geri girer ve
    // sarmalayici cikista onu geri acar. Yeni dogan gorev ise
    // `bootstrap_stack`'ten IF=1 ile baslar.
    crate::arch::cpu::without_interrupts(|| unsafe {
        let current_index = CURRENT.load(Ordering::Relaxed);
        let next_index = pick_next(current_index);
        if next_index == current_index {
            return;
        }

        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;

        if (*tasks.add(current_index)).state == TaskState::Running {
            (*tasks.add(current_index)).state = TaskState::Ready;
        }
        // Donem hakki TUR basina da harcanir, yalnizca tik basina degil.
        //
        // Yalnizca tik sayilsaydi, tik sinirini nadiren gecen ince taneli
        // isler (her karede `win_flush` cagiran GUI uygulamalari gibi)
        // kredilerini neredeyse hic tuketmez, hepsi surekli uygun kalir ve
        // sira 1:1 doner -- yani oncelik hicbir sey ifade etmezdi. Olcum
        // tam olarak bunu gosterdi: -20 ve 19 ile kosan iki surecin
        // sayaclari tipatip ayni artiyordu.
        //
        // Tur basina da harcayinca hak, "kac tik kostu"nun yani sira "kac
        // kez secildi"yi de sinirlar; agirlikli sirali dagitim (weighted
        // round-robin) boyle olusur.
        if current_index != IDLE_TASK {
            let credits = &mut (*tasks.add(current_index)).credits;
            *credits = credits.saturating_sub(1);
        }
        // Idle'a gecerken donem yenilenir: baska gorev kalmadigi icin
        // idle secildiyse, sonraki uyanmada herkesin hakki hazir olsun.
        if next_index == IDLE_TASK {
            let count = TASK_COUNT.load(Ordering::Relaxed);
            for i in 0..count {
                (*tasks.add(i)).credits = slice_ticks((*tasks.add(i)).nice);
            }
        }
        (*tasks.add(next_index)).state = TaskState::Running;
        CURRENT.store(next_index, Ordering::Relaxed);
        SWITCHES.fetch_add(1, Ordering::Relaxed);

        // Gelen gorevin cekirdek yiginini donanima bildir. Bu satir olmadan
        // iki Ring 3 sureci ayni TSS yiginini paylasir ve birbirinin
        // syscall cercevesini ezer.
        let incoming_kstack = (*tasks.add(next_index)).kernel_stack_top;
        if incoming_kstack != 0 {
            crate::level0a::gdt::set_kernel_stack(incoming_kstack);
            #[cfg(target_arch = "x86_64")]
            crate::level0a::syscall_msr::set_kernel_stack(incoming_kstack);
        }

        // Adres uzayini degistir. Cekirdek ve yiginlar tum uzaylarda ayni
        // yerde eslendigi icin bu satirdan sonraki kod ve `arch_context_switch`
        // guvenle calisir; degisen tek sey kullanici bolgesidir.
        let incoming_space = (*tasks.add(next_index)).address_space;
        crate::level0a::core::mmu::switch_to(if incoming_space != 0 {
            incoming_space
        } else {
            crate::level0a::core::mmu::kernel_cr3()
        });

        let old_sp_slot = core::ptr::addr_of_mut!((*tasks.add(current_index)).stack_pointer);
        let new_sp = (*tasks.add(next_index)).stack_pointer;
        arch_context_switch(old_sp_slot, new_sp);
    });
}

/// Calisan gorevi sonlandirir ve bir daha asla ona donmez.
pub fn terminate_current() -> ! {
    // Tanimlayicilari birak. Boru uclari icin sart: kapanmayan bir yazma
    // ucu, okuyan tarafta "dosya sonu"nun hic gorunmemesi demektir.
    crate::level0a::core::fd::close_all(CURRENT.load(Ordering::Relaxed));

    crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        let current = CURRENT.load(Ordering::Relaxed);
        (*tasks.add(current)).state = TaskState::Terminated;
        // Kimse beklemeyecekse yuva hemen geri verilir. Beklenebilir
        // olanlar (fork cocuklari) `wait_for_task` toplayana kadar zombi
        // kalir; ebeveyni de oldulerse `reap_orphans` temizler.
        if !(*tasks.add(current)).waitable {
            release_slot(current);
        }
        reap_orphans();
    });

    loop {
        yield_now();
        // Baska hazir gorev kalmadiysa (ornegin tum worker'lar bitti) CPU'yu
        // bosuna dondurmemek icin bir sonraki kesmeye kadar bekle.
        crate::arch::cpu::halt();
    }
}

fn pick_next(current: usize) -> usize {
    let count = TASK_COUNT.load(Ordering::Relaxed);
    wake_expired(count);

    // Iki gecis. Once donem hakki kalmis hazir gorevler aranir; hicbiri
    // yoksa donem kapanir (herkesin hakki tazelenir) ve tekrar bakilir.
    //
    // Donem kapaninca kullanilmamis hak SILINIR, devretmez. Devretseydi
    // yuksek oncelikli bir gorev hak biriktirip sirayi hic birakmazdi --
    // olculdu: dusuk oncelikli surec 1305 tike karsi 8 tik aldi, yani
    // 8:1 olmasi gereken oran 163:1'e cikti.
    for pass in 0..2 {
        unsafe {
            let tasks = core::ptr::addr_of!(TASKS) as *const Task;
            for offset in 1..=count {
                let candidate = (current + offset) % count;
                if (*tasks.add(candidate)).state != TaskState::Ready {
                    continue;
                }
                // Idle SON CARE: normal taramada hic secilmez. Bir donem
                // masaustu dongusuyle birlesikti ve her turda secilmesi
                // gerekiyordu; masaustu ayri bir goreve tasindiktan sonra
                // 0 numaranin tek isi bos beklemek kaldi.
                if candidate == IDLE_TASK {
                    continue;
                }
                if (*tasks.add(candidate)).credits == 0 {
                    continue;
                }
                return candidate;
            }
        }
        if pass == 0 {
            refill_credits(count);
        }
    }

    // Hazir baska gorev yok. Bu, mevcut gorevin kosamayacagi anlamina
    // GELMEZ: tarama yalnizca `Ready` olanlara bakar, oysa cagri aninda
    // mevcut gorevin durumu `Running`'dir -- yani kendisi hicbir zaman
    // aday olamaz. Hakki duruyorsa devam etmesi gerekir.
    //
    // Bu atlanirsa sistem, kosmaya hazir bir gorev VARKEN idle'a duser ve
    // idle `hlt` ettigi icin CPU gercekten bosa gider (olculdu: tiklerin
    // %60'i idle'a yaziliyordu).
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        if current != IDLE_TASK
            && (*tasks.add(current)).state == TaskState::Running
            && (*tasks.add(current)).credits > 0
        {
            return current;
        }
        if (*tasks.add(IDLE_TASK)).state == TaskState::Ready
            || (*tasks.add(IDLE_TASK)).state == TaskState::Running
        {
            return IDLE_TASK;
        }
    }
    current
}

/// Suresi dolan uyuyan gorevleri hazir hale getirir.
///
/// Uyandirmayi **secim aninda** yapmak, ayri bir zamanlayici kuyruguna
/// gerek birakmaz: gorev sayisi kucuk oldugu icin dogrusal tarama
/// kuyruk yonetiminden ucuzdur ve zaman kaymasi olusturmaz.
fn wake_expired(count: usize) {
    let now = crate::level0a::pit::ticks();
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        for i in 0..count {
            let task = &mut *tasks.add(i);
            match task.state {
                TaskState::Blocked => {
                    // Tick sayaci sarsa bile dogru calissin diye fark
                    // uzerinden karsilastirilir.
                    if now.wrapping_sub(task.wake_tick) < 0x8000_0000 {
                        task.state = TaskState::Ready;
                    }
                }
                TaskState::Waiting => {
                    // Zamanla degil, beklenen gorevin bitmesiyle uyanir.
                    let target = task.wait_for - 1;
                    if (*tasks.add(target)).state == TaskState::Terminated {
                        task.state = TaskState::Ready;
                        task.wait_for = 0;
                    }
                }
                _ => {}
            }
        }
    }
}

/// Calisan gorevi `ticks` PIT tik'i boyunca uykuya alir.
///
/// Uyuyan gorev `pick_next` tarafindan **atlanir**; suresi dolunca
/// `wake_expired` onu yeniden hazir yapar. Idle gorevi uyutulmaz --
/// masaustu dongusudur ve her zaman kosabilir olmalidir, ayrica uyanacak
/// baska gorev kalmadiginda sistemi ilerletecek tek akis odur.
pub fn sleep_ticks(ticks: u32) {
    let current = CURRENT.load(Ordering::Relaxed);
    if current == 0 || ticks == 0 {
        yield_now();
        return;
    }

    let wake = crate::level0a::pit::ticks().wrapping_add(ticks);
    crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(current)).wake_tick = wake;
        (*tasks.add(current)).state = TaskState::Blocked;
    });

    // Idle her zaman hazir oldugu icin `yield_now` mutlaka baska bir
    // goreve gecer; bu dongu ancak uyandirildiktan sonra kirilir.
    loop {
        yield_now();
        let now = crate::level0a::pit::ticks();
        if now.wrapping_sub(wake) < 0x8000_0000 {
            break;
        }
    }

    crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(current)).state = TaskState::Running;
    });
}

/// Calisan gorevi bir aygit kesmesi gelene kadar askiya alir.
///
/// `sleep_ticks` ile ayni kalip, tek farki uyanma kosulu: burada
/// `wake_io_waiters` (yani bir kesme isleyicisi) gorevi `Ready` yapana
/// kadar beklenir.
///
/// `ready` kapanisi, uyumadan **once** kosulu bir kez daha denetler:
/// kesme, gorev `IoWait`'e gecmeden hemen once gelmis olabilir ve o
/// uyandirma kaybolursa surec sonsuza kadar beklerdi (klasik "kacirilmis
/// uyandirma" yarisi).
pub fn wait_for_io(ready: impl Fn() -> bool, timeout_ticks: u32) -> bool {
    // Olcum: kac kez gercekten uyunuldu. Sayacin sifirdan buyuk olmasi,
    // disk beklemesinin CPU'yu bosa harcamadiginin dogrudan kanitidir.
    let current = CURRENT.load(Ordering::Relaxed);
    if !can_block(current) {
        // Uyutulamayan baglam: yoklamaya dusulur (bkz. `can_block`).
        return ready();
    }

    let deadline = crate::level0a::pit::ticks().wrapping_add(timeout_ticks);
    loop {
        let armed = crate::arch::cpu::without_interrupts(|| unsafe {
            if ready() {
                return false;
            }
            let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
            (*tasks.add(current)).state = TaskState::IoWait;
            IO_WAITS.fetch_add(1, Ordering::Relaxed);
            true
        });
        if !armed {
            return true;
        }

        yield_now();

        if ready() {
            break;
        }
        // Kesme hic gelmezse (kayip IRQ, arizali aygit) sistem burada
        // takilip kalmaz: sure dolunca cagirana yoklama yolu birakilir.
        if crate::level0a::pit::ticks().wrapping_sub(deadline) < 0x8000_0000 {
            crate::arch::cpu::without_interrupts(|| unsafe {
                let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
                (*tasks.add(current)).state = TaskState::Running;
            });
            return false;
        }
    }

    crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(current)).state = TaskState::Running;
    });
    true
}

/// Kac kez bir gorev aygit kesmesi bekleyerek uyudu.
/// Kac kez sinyal beklemeye girildi (olcum icin).
static SIG_WAITS: AtomicUsize = AtomicUsize::new(0);

/// Gorevi, teslim edilebilir bir sinyal gelene kadar uyutur.
///
/// `deliverable` cagrisi **kesmeler kapaliyken** sinanir: sinyal, gorev
/// `SigWait`e gecmeden hemen once gelmis olabilir ve o zaman uyandiracak
/// kimse kalmazdi. `wait_for_io`daki ayni yaris, ayni cozum.
///
/// Zaman asimi **yok**: POSIX `pause` gercekten de sonsuza kadar bekler.
/// Sikisan bir surec `kill` ile kaldirilabilir -- sinyal gonderimi zaten
/// uyandirma yolunun kendisidir.
pub fn wait_for_signal(deliverable: impl Fn() -> bool) -> bool {
    let current = CURRENT.load(Ordering::Relaxed);
    if !can_block(current) {
        // Uyutulamayan baglam (masaustu/kabuk gorevi): yoklamaya dusulur.
        return deliverable();
    }

    loop {
        let armed = crate::arch::cpu::without_interrupts(|| unsafe {
            if deliverable() {
                return false;
            }
            let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
            (*tasks.add(current)).state = TaskState::SigWait;
            SIG_WAITS.fetch_add(1, Ordering::Relaxed);
            true
        });
        if !armed {
            return true;
        }

        yield_now();

        // Uyandirildik: sinyal geldiyse cikilir, gelmediyse (sahte
        // uyanma) yeniden uyunur.
        if deliverable() {
            break;
        }
    }

    crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(current)).state = TaskState::Running;
    });
    true
}

/// Sinyal bekleyen **tek** bir gorevi uyandirir.
///
/// Kesme baglamindan da cagrilabilir (`alarm` PIT tikinden gelir), bu
/// yuzden kilit almaz ve gorev degistirmez.
pub fn wake_signal_waiter(task: usize) {
    if task >= TASK_COUNT.load(Ordering::Relaxed) {
        return;
    }
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        if (*tasks.add(task)).state == TaskState::SigWait {
            (*tasks.add(task)).state = TaskState::Ready;
        }
    }
}

/// Kac kez sinyal beklemeye girildi.
pub fn sig_waits() -> usize {
    SIG_WAITS.load(Ordering::Relaxed)
}

pub fn io_waits() -> usize {
    IO_WAITS.load(Ordering::Relaxed)
}

/// Aygit kesmesi bekleyen butun gorevleri hazir yapar.
///
/// Kesme baglamindan cagrilir; bu yuzden kilit almaz ve gorev
/// degistirmez -- yalnizca durum alanina dokunur.
pub fn wake_io_waiters() -> usize {
    let count = TASK_COUNT.load(Ordering::Relaxed);
    let mut woken = 0;
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        for i in 0..count {
            if (*tasks.add(i)).state == TaskState::IoWait {
                (*tasks.add(i)).state = TaskState::Ready;
                woken += 1;
            }
        }
    }
    woken
}

/// Kac gorev su an uyuyor (kabuk raporu icin).
pub fn sleeping_count() -> usize {
    let count = TASK_COUNT.load(Ordering::Relaxed);
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (0..count)
            .filter(|i| (*tasks.add(*i)).state == TaskState::Blocked)
            .count()
    }
}

/// Calisan gorevin numarasi. Level-0b2'nin Yuk Dengeleyicisi cagrilari
/// goreve yazabilmek icin buna ihtiyac duyar.
pub fn current_id() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

/// Verilen gorevin adi (gorev yoksa bos dize).
pub fn name_of(index: usize) -> &'static str {
    if index >= MAX_TASKS {
        return "";
    }
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (*tasks.add(index)).name
    }
}

/// Verilen gorevin durumu.
pub fn state_of(index: usize) -> TaskState {
    if index >= MAX_TASKS {
        return TaskState::Unused;
    }
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (*tasks.add(index)).state
    }
}

/// Bir gorevi disaridan sonlandirir (kabuk `kill` komutu).
///
/// Gorev **calisirken** oldurulemez: kendi yiginindaki cagri zinciri
/// yarim kalirdi. `Terminated` isaretlenen gorev bir sonraki secimde
/// atlanir; Ring 3'te ise ilk sistem cagrisinda cikisa yonlendirilir.
pub fn terminate(index: usize) -> Result<(), &'static str> {
    if index == 0 {
        return Err("idle gorevi sonlandirilamaz");
    }
    if index >= MAX_TASKS {
        return Err("gecersiz gorev numarasi");
    }
    if index == current_id() {
        return Err("calisan gorev bu yoldan sonlandirilamaz");
    }
    let space = crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        match (*tasks.add(index)).state {
            TaskState::Unused => Err("gorev yok"),
            TaskState::Terminated => Err("gorev zaten sonlandirilmis"),
            _ => {
                (*tasks.add(index)).state = TaskState::Terminated;
                let space = (*tasks.add(index)).address_space;
                (*tasks.add(index)).address_space = 0;
                if !(*tasks.add(index)).waitable {
                    release_slot(index);
                }
                Ok(space)
            }
        }
    })?;

    // Gorevin izleri: penceresi ekranda kalirsa artik kimsenin cizmedigi
    // olu bir dikdortgen olur; adres uzayi birakilmazsa cerceveler sizar.
    // Normal cikista bunlari surecin kendi yolu yapar (bkz.
    // `level0b1::process`), disaridan oldurmede buraya duser.
    crate::level0a::wm::close_owned_by(index);
    crate::level0a::core::fd::close_all(index);
    if space != 0 {
        unsafe { crate::level0a::core::mmu::destroy_user_space(space) };
    }
    Ok(())
}

pub fn current_name() -> &'static str {
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (*tasks.add(CURRENT.load(Ordering::Relaxed))).name
    }
}

pub fn switch_count() -> usize {
    SWITCHES.load(Ordering::Relaxed)
}

pub fn task_count() -> usize {
    TASK_COUNT.load(Ordering::Relaxed)
}

// --- Ring 3 baglami (gorev basina) ---

/// Calisan gorevin Ring 3 `resume` slotuna isaretci.
pub fn current_resume_slot() -> *mut usize {
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        core::ptr::addr_of_mut!((*tasks.add(CURRENT.load(Ordering::Relaxed))).user_resume)
    }
}

pub fn set_current_in_user_mode(flag: bool) {
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(CURRENT.load(Ordering::Relaxed))).in_user_mode = flag;
    }
}

/// Calisan gorevin adres uzayini kaydeder (surec baslatilirken).
pub fn set_current_address_space(cr3: usize) {
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(CURRENT.load(Ordering::Relaxed))).address_space = cr3;
    }
}

/// Calisan gorevin cikis kodunu kaydeder (`waitpid` bunu okur).
pub fn set_current_exit_code(code: u32) {
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(CURRENT.load(Ordering::Relaxed))).exit_code = code;
    }
}

/// Bir gorevin cikis kodu.
pub fn exit_code_of(index: usize) -> u32 {
    if index >= MAX_TASKS {
        return 0;
    }
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (*tasks.add(index)).exit_code
    }
}

/// `waitpid`: `child` gorevinin bitmesini bekler, cikis kodunu doner.
///
/// Beklerken gorev `Waiting` durumundadir ve `pick_next` tarafindan
/// **atlanir** -- yani bekleyen bir surec CPU harcamaz. Uyanma zamanla
/// degil, `wake_expired` icinde beklenen gorevin `Terminated` olmasiyla
/// gerceklesir.
///
/// `None` doner: gecersiz indeks, kendini beklemek, ya da idle gorevinin
/// cagirmasi (idle bloke edilemez -- masaustu dongusudur).
pub fn wait_for_task(child: usize) -> Option<u32> {
    let current = CURRENT.load(Ordering::Relaxed);
    let count = TASK_COUNT.load(Ordering::Relaxed);
    if child >= count || child == current || current == 0 {
        return None;
    }

    if state_of(child) == TaskState::Unused {
        return None;
    }

    // Zaten bitmisse beklemeye gerek yok; toplandigi icin yuva serbest.
    if state_of(child) == TaskState::Terminated {
        let code = exit_code_of(child);
        crate::arch::cpu::without_interrupts(|| release_slot(child));
        return Some(code);
    }

    crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(current)).wait_for = child + 1;
        (*tasks.add(current)).state = TaskState::Waiting;
    });

    // Bekleyen gorev `Waiting` durumunda oldugu icin zamanlanmaz; dongu
    // ancak cocuk bitince kirilir. Baska hicbir sey kosmuyorsa `yield_now`
    // idle'i secer, o da bir sonraki kesmeye kadar bekler.
    loop {
        yield_now();
        if state_of(child) == TaskState::Terminated {
            break;
        }
    }

    let code = exit_code_of(child);
    crate::arch::cpu::without_interrupts(|| release_slot(child));
    Some(code)
}

/// `waitpid(-1)`: cagiranin **herhangi bir** cocugunu bekler.
///
/// Once sonlanmis bir cocuk aranir (varsa hemen toplanir); yoksa canli
/// bir cocuk bulunup onun bitmesi beklenir. Hic cocuk yoksa `None`
/// doner -- POSIX'te bu `ECHILD`'dir.
pub fn wait_for_any() -> Option<(usize, u32)> {
    let current = CURRENT.load(Ordering::Relaxed);
    let count = TASK_COUNT.load(Ordering::Relaxed);

    // 1. Zaten bitmis bir cocuk var mi?
    for i in 0..count {
        if i == current {
            continue;
        }
        unsafe {
            let tasks = core::ptr::addr_of!(TASKS) as *const Task;
            if (*tasks.add(i)).parent == current
                && (*tasks.add(i)).state == TaskState::Terminated
            {
                let code = exit_code_of(i);
                crate::arch::cpu::without_interrupts(|| release_slot(i));
                return Some((i, code));
            }
        }
    }

    // 2. Canli bir cocuk bul ve onu bekle.
    for i in 0..count {
        if i == current {
            continue;
        }
        let is_child = unsafe {
            let tasks = core::ptr::addr_of!(TASKS) as *const Task;
            (*tasks.add(i)).parent == current
                && (*tasks.add(i)).state != TaskState::Unused
        };
        if is_child {
            return wait_for_task(i).map(|code| (i, code));
        }
    }

    None
}

/// Bitmis bir cocugu bekleMEDEN toplar (`waitpid` + `WNOHANG`).
pub fn reap_finished_child(parent: usize) -> Option<(usize, u32)> {
    let count = TASK_COUNT.load(Ordering::Relaxed);
    for i in 0..count {
        if i == parent {
            continue;
        }
        let finished = unsafe {
            let tasks = core::ptr::addr_of!(TASKS) as *const Task;
            (*tasks.add(i)).parent == parent && (*tasks.add(i)).state == TaskState::Terminated
        };
        if finished {
            let code = exit_code_of(i);
            crate::arch::cpu::without_interrupts(|| release_slot(i));
            return Some((i, code));
        }
    }
    None
}

/// Cagiranin toplanmamis cocugu var mi (`waitpid` icin ECHILD ayrimi)?
pub fn has_children(index: usize) -> bool {
    let count = TASK_COUNT.load(Ordering::Relaxed);
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (0..count).any(|i| {
            i != index
                && (*tasks.add(i)).parent == index
                && (*tasks.add(i)).state != TaskState::Unused
        })
    }
}

/// Baska bir gorevin adres uzayini kaydeder.
///
/// `fork` icin gerekir: cocugun uzayi ebeveyn tarafindan, cocuk daha hic
/// calismadan kurulur. Baglam degisimi CR3'u buradan yukler.
pub fn set_address_space(index: usize, cr3: usize) {
    if index >= MAX_TASKS {
        return;
    }
    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(index)).address_space = cr3;
    }
}

/// Gorevin adres uzayi (kabuk raporu icin).
pub fn address_space_of(index: usize) -> usize {
    if index >= MAX_TASKS {
        return 0;
    }
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (*tasks.add(index)).address_space
    }
}

/// Su an Ring 3'te bir imaj yuruten gorev sayisi.
///
/// Paylasimli adres uzayi modelinde (x86_64) bu sayinin **birden fazla
/// olmamasi** gerekir: butun imajlar ayni sanal adrese yuklendigi icin
/// ikinci bir surec birincinin kodunun uzerine yazardi.
pub fn user_task_count() -> usize {
    let count = TASK_COUNT.load(Ordering::Relaxed);
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (0..count)
            .filter(|&i| {
                (*tasks.add(i)).in_user_mode && (*tasks.add(i)).state != TaskState::Terminated
            })
            .count()
    }
}

pub fn current_in_user_mode() -> bool {
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (*tasks.add(CURRENT.load(Ordering::Relaxed))).in_user_mode
    }
}
