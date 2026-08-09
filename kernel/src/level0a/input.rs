//! Girdi alt sistemi: klavye + PS/2 fare olaylari tek bir kuyrukta.
//!
//! Kesme baglaminda (IRQ1/IRQ12) yalnizca olay kuyruga atilir; yorumlama
//! isini pencere yoneticisi ve uygulamalar yapar. Boylece kesme
//! handler'lari kisa kalir.

use core::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use crate::arch::cpu::{inb, outb};
use crate::level0a::drivers::gfx;

const PS2_DATA: u16 = 0x60;
const PS2_STATUS: u16 = 0x64;
const PS2_COMMAND: u16 = 0x64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    /// Basilan tusun ASCII karsiligi (0 = yazdirilamayan).
    Key(u8),
    /// Fare hareketi/tik durumu: (x, y, sol_tus_basili).
    Mouse { x: usize, y: usize, left: bool },
}

const QUEUE_SIZE: usize = 64;
static mut QUEUE: [Option<Event>; QUEUE_SIZE] = [None; QUEUE_SIZE];
static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);

fn push(event: Event) {
    let head = HEAD.load(Ordering::Relaxed);
    let next = (head + 1) % QUEUE_SIZE;
    if next == TAIL.load(Ordering::Relaxed) {
        return; // kuyruk dolu -- en eski olayi korumak icin yeniyi at
    }
    unsafe {
        let q = core::ptr::addr_of_mut!(QUEUE) as *mut Option<Event>;
        q.add(head).write(Some(event));
    }
    HEAD.store(next, Ordering::Relaxed);
}

pub fn poll() -> Option<Event> {
    crate::arch::cpu::without_interrupts(|| {
        let tail = TAIL.load(Ordering::Relaxed);
        if tail == HEAD.load(Ordering::Relaxed) {
            return None;
        }
        let event = unsafe {
            let q = core::ptr::addr_of_mut!(QUEUE) as *mut Option<Event>;
            q.add(tail).replace(None)
        };
        TAIL.store((tail + 1) % QUEUE_SIZE, Ordering::Relaxed);
        event
    })
}

// --- Fare durumu ---

static MOUSE_X: AtomicIsize = AtomicIsize::new(400);
static MOUSE_Y: AtomicIsize = AtomicIsize::new(300);
static MOUSE_LEFT: AtomicUsize = AtomicUsize::new(0);

pub fn mouse_x() -> usize {
    MOUSE_X.load(Ordering::Relaxed).max(0) as usize
}

pub fn mouse_y() -> usize {
    MOUSE_Y.load(Ordering::Relaxed).max(0) as usize
}

pub fn mouse_left() -> bool {
    MOUSE_LEFT.load(Ordering::Relaxed) != 0
}

// --- Klavye ---

pub fn on_key(ascii: u8) {
    push(Event::Key(ascii));
}

// --- PS/2 fare ---

fn wait_write() {
    for _ in 0..100_000 {
        if unsafe { inb(PS2_STATUS) } & 0x02 == 0 {
            return;
        }
    }
}

fn wait_read() {
    for _ in 0..100_000 {
        if unsafe { inb(PS2_STATUS) } & 0x01 != 0 {
            return;
        }
    }
}

unsafe fn mouse_command(cmd: u8) {
    wait_write();
    outb(PS2_COMMAND, 0xD4); // sonraki bayt fareye gitsin
    wait_write();
    outb(PS2_DATA, cmd);
    wait_read();
    let _ack = inb(PS2_DATA);
}

/// PS/2 fareyi acar ve IRQ12'yi etkinlestirir.
///
/// # Safety
/// PIC remap edilmis olmalidir.
pub unsafe fn init_mouse() {
    // Yardimci cihaz (fare) portunu ac
    wait_write();
    outb(PS2_COMMAND, 0xA8);

    // Denetleyici yapilandirmasini oku, IRQ12'yi ac
    wait_write();
    outb(PS2_COMMAND, 0x20);
    wait_read();
    let mut status = inb(PS2_DATA);
    status |= 0x02; // yardimci cihaz kesmesi
    status &= !0x20; // yardimci saat acik
    wait_write();
    outb(PS2_COMMAND, 0x60);
    wait_write();
    outb(PS2_DATA, status);

    mouse_command(0xF6); // varsayilanlar
    mouse_command(0xF4); // veri akisini baslat
}

/// IRQ12 handler'i tarafindan cagrilir. Fare 3 baytlik paketler gonderir.
pub fn on_mouse_irq() {
    static PACKET: [AtomicUsize; 3] = [
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ];
    static PHASE: AtomicUsize = AtomicUsize::new(0);

    let byte = unsafe { inb(PS2_DATA) };
    let phase = PHASE.load(Ordering::Relaxed);

    // Ilk bayt her zaman bit 3 set olmalidir; degilse senkron kaybi var.
    if phase == 0 && byte & 0x08 == 0 {
        return;
    }

    PACKET[phase].store(byte as usize, Ordering::Relaxed);

    if phase < 2 {
        PHASE.store(phase + 1, Ordering::Relaxed);
        return;
    }
    PHASE.store(0, Ordering::Relaxed);

    let flags = PACKET[0].load(Ordering::Relaxed) as u8;
    let dx_raw = PACKET[1].load(Ordering::Relaxed) as u8;
    let dy_raw = PACKET[2].load(Ordering::Relaxed) as u8;

    // Tasma bayraklari varsa paketi at.
    if flags & 0xC0 != 0 {
        return;
    }

    // 9-bit isaretli degerler: isaret biti flags icinde.
    let dx = if flags & 0x10 != 0 {
        dx_raw as isize - 256
    } else {
        dx_raw as isize
    };
    let dy = if flags & 0x20 != 0 {
        dy_raw as isize - 256
    } else {
        dy_raw as isize
    };

    let max_x = gfx::width().saturating_sub(1) as isize;
    let max_y = gfx::height().saturating_sub(1) as isize;

    let x = (MOUSE_X.load(Ordering::Relaxed) + dx).clamp(0, max_x);
    // PS/2'de +Y yukaridir, ekranda asagi -- isaret ters cevrilir.
    let y = (MOUSE_Y.load(Ordering::Relaxed) - dy).clamp(0, max_y);

    MOUSE_X.store(x, Ordering::Relaxed);
    MOUSE_Y.store(y, Ordering::Relaxed);

    let left = flags & 0x01 != 0;
    MOUSE_LEFT.store(left as usize, Ordering::Relaxed);

    push(Event::Mouse {
        x: x as usize,
        y: y as usize,
        left,
    });
}
