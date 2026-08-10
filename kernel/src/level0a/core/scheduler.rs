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
        }
    }
}

static mut TASKS: [Task; MAX_TASKS] = [Task::empty(); MAX_TASKS];
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static TASK_COUNT: AtomicUsize = AtomicUsize::new(0);
static NEED_RESCHED: AtomicBool = AtomicBool::new(false);
static SWITCHES: AtomicUsize = AtomicUsize::new(0);

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

        TASK_COUNT.store(index + 1, Ordering::Relaxed);
        Some(index)
    })
}

/// PIT kesmesinden cagrilir: zaman dilimi doldu bayragini kaldirir.
pub fn on_timer_tick() {
    NEED_RESCHED.store(true, Ordering::Relaxed);
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

        let old_sp_slot = core::ptr::addr_of_mut!((*tasks.add(current_index)).stack_pointer);
        let new_sp = (*tasks.add(next_index)).stack_pointer;
        arch_context_switch(old_sp_slot, new_sp);
    }
}

/// Calisan gorevi sonlandirir ve bir daha asla ona donmez.
pub fn terminate_current() -> ! {
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
    crate::arch::cpu::without_interrupts(|| unsafe {
        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        match (*tasks.add(index)).state {
            TaskState::Unused => Err("gorev yok"),
            TaskState::Terminated => Err("gorev zaten sonlandirilmis"),
            _ => {
                (*tasks.add(index)).state = TaskState::Terminated;
                Ok(())
            }
        }
    })
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

pub fn current_in_user_mode() -> bool {
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS) as *const Task;
        (*tasks.add(CURRENT.load(Ordering::Relaxed))).in_user_mode
    }
}
