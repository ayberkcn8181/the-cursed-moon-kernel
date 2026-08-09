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
}

impl Task {
    const fn empty() -> Self {
        Task {
            state: TaskState::Unused,
            stack_pointer: 0,
            name: "",
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

        let tasks = core::ptr::addr_of_mut!(TASKS) as *mut Task;
        (*tasks.add(index)).state = TaskState::Ready;
        (*tasks.add(index)).stack_pointer = sp;
        (*tasks.add(index)).name = name;

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
