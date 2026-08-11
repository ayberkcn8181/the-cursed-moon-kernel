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

pub const MAX_TASKS: usize = 8;
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
    crate::arch::cpu::without_interrupts(|| unsafe {
        let index = TASK_COUNT.load(Ordering::Relaxed);
        if index >= MAX_TASKS {
            return None;
        }

        let stack = kmalloc::kmalloc_aligned(TASK_STACK_SIZE, 16)?;
        let stack_top = stack.add(TASK_STACK_SIZE) as *mut usize;
        let sp = bootstrap_stack(stack_top, entry);

        // Ring 3 gecisleri icin AYRI bir cekirdek yigini.
        let kstack = kmalloc::kmalloc_aligned(TASK_STACK_SIZE, 16)?;
        let kstack_top = kstack.add(TASK_STACK_SIZE) as usize;

        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(index)).state = TaskState::Ready;
        (*tasks.add(index)).stack_pointer = sp;
        (*tasks.add(index)).name = name;
        (*tasks.add(index)).kernel_stack_top = kstack_top;
        (*tasks.add(index)).user_resume = 0;
        (*tasks.add(index)).in_user_mode = false;
        (*tasks.add(index)).address_space = 0;
        (*tasks.add(index)).wake_tick = 0;
        (*tasks.add(index)).wait_for = 0;
        (*tasks.add(index)).exit_code = 0;

        TASK_COUNT.store(index + 1, Ordering::Relaxed);
        Some(index)
    })
}

/// PIT kesmesinden cagrilir: zaman dilimi doldu bayragini kaldirir.
pub fn on_timer_tick() {
    NEED_RESCHED.store(true, Ordering::Relaxed);
}

/// Bolunemez bir is boyunca zorla baglam degisimini kapatir.
///
/// Gerekli oldugu tek yer su an disk: ATA komutlari (surucu secimi ->
/// LBA -> komut -> veri) bir butundur; ortasinda baska bir gorev ikinci
/// bir komut baslatirsa denetleyicinin durumu bozulur. Kesmeleri
/// kapatmak da ise yarardi ama 2 MiB'lik bir okuma boyunca kesmesiz
/// kalmak saati ve girdiyi dondururdu; burada yalnizca **baglam degisimi**
/// ertelenir, kesmeler islenmeye devam eder.
pub fn preempt_disable() {
    PREEMPT_LOCK.fetch_add(1, Ordering::Relaxed);
}

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

    let (current_index, next_index) = crate::arch::cpu::without_interrupts(|| {
        let current = CURRENT.load(Ordering::Relaxed);
        (current, pick_next(current))
    });

    if next_index == current_index {
        return;
    }

    unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;

        if (*tasks.add(current_index)).state == TaskState::Running {
            (*tasks.add(current_index)).state = TaskState::Ready;
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
    }
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
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        for offset in 1..=count {
            let candidate = (current + offset) % count;
            if (*tasks.add(candidate)).state == TaskState::Ready {
                return candidate;
            }
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

    // Zaten bitmisse beklemeye gerek yok.
    if state_of(child) == TaskState::Terminated {
        return Some(exit_code_of(child));
    }

    crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(current)).wait_for = child + 1;
        (*tasks.add(current)).state = TaskState::Waiting;
    });

    // Idle her zaman hazir oldugu icin `yield_now` mutlaka baska bir
    // goreve gecer; dongu ancak cocuk bitince kirilir.
    loop {
        yield_now();
        if state_of(child) == TaskState::Terminated {
            break;
        }
    }

    Some(exit_code_of(child))
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
