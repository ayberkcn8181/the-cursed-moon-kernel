//! x86_64 IDT: 256 girdi, her biri **16 bayt** (i386'daki 8 bayta karsi).
//!
//! Vektor tahsisi i386 ile aynidir (doc S.4):
//!   0        CPU istisnasi (divide-by-zero)
//!   32/33    IRQ0 (PIT) / IRQ1 (klavye)  -- PIC remap sonrasi
//!   46       int 0x2E  Windows NT uyumlu sistem cagrisi
//!   128      int 0x80  Linux uyumlu sistem cagrisi
//!
//! x86_64'te Linux asil olarak `syscall` komutunu kullanir (bkz.
//! `level0a::syscall_msr`); int 0x80 yolu geriye donuk uyumluluk ve
//! i386 ile ayni test akisini kurabilmek icin korunur.

use core::arch::{asm, global_asm};
use core::mem::size_of;

use crate::arch::cpu::regs::{InterruptStackFrame, SyscallFrame};
use crate::level0a::gdt::KERNEL_CODE_SELECTOR;

const GATE_PRESENT_RING0_INT: u8 = 0x8E; // P=1 DPL=00 type=E (64-bit interrupt gate)
const GATE_PRESENT_RING3_INT: u8 = 0xEE; // P=1 DPL=11 (Ring 3'ten cagrilabilir)

#[derive(Clone, Copy)]
#[repr(C, packed)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        IdtEntry {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }

    fn set(&mut self, handler: u64, selector: u16, type_attr: u8) {
        self.offset_low = (handler & 0xFFFF) as u16;
        self.offset_mid = ((handler >> 16) & 0xFFFF) as u16;
        self.offset_high = ((handler >> 32) & 0xFFFF_FFFF) as u32;
        self.selector = selector;
        self.ist = 0;
        self.type_attr = type_attr;
        self.reserved = 0;
    }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

pub fn init() {
    unsafe {
        install_exception_handlers();
        IDT[crate::level0a::pic::vector(crate::level0a::pic::IRQ_TIMER)].set(
            pit_handler as *const () as u64,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING0_INT,
        );
        IDT[crate::level0a::pic::vector(crate::level0a::pic::IRQ_KEYBOARD)].set(
            keyboard_handler as *const () as u64,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING0_INT,
        );
        // IRQ12 = PS/2 fare, IRQ14 = birincil ATA. Vektorler `pic::vector`
        // ile hesaplanir; slave denetleyici 0x70'e alindigi icin bunlar 44
        // degil 116/118'dir (gerekce: `pic.rs`).
        IDT[crate::level0a::pic::vector(crate::level0a::pic::IRQ_MOUSE)].set(
            mouse_handler as *const () as u64,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING0_INT,
        );
        IDT[crate::level0a::pic::vector(crate::level0a::pic::IRQ_ATA_PRIMARY)].set(
            ata_handler as *const () as u64,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING0_INT,
        );
        IDT[128].set(
            syscall_entry as *const () as u64,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING3_INT,
        );
        IDT[46].set(
            nt_syscall_entry as *const () as u64,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING3_INT,
        );

        let ptr = IdtPointer {
            limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u64,
        };
        asm!("lidt [{0}]", in(reg) &ptr, options(nostack));
    }
}

fn read_cr2() -> usize {
    let value: u64;
    unsafe { asm!("mov {0}, cr2", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value as usize
}

macro_rules! exception_no_code {
    ($name:ident, $vector:expr) => {
        extern "x86-interrupt" fn $name(frame: InterruptStackFrame) -> ! {
            let from_user = frame.code_segment & 3 == 3;
            crate::level0a::exceptions::handle(
                $vector,
                0,
                frame.instruction_pointer as usize,
                from_user,
                0,
            )
        }
    };
}

macro_rules! exception_with_code {
    ($name:ident, $vector:expr) => {
        extern "x86-interrupt" fn $name(frame: InterruptStackFrame, error_code: u64) -> ! {
            let from_user = frame.code_segment & 3 == 3;
            let fault_addr = if $vector == 14 { read_cr2() } else { 0 };
            crate::level0a::exceptions::handle(
                $vector,
                error_code as usize,
                frame.instruction_pointer as usize,
                from_user,
                fault_addr,
            )
        }
    };
}

exception_no_code!(ex0, 0);
exception_no_code!(ex1, 1);
exception_no_code!(ex2, 2);
exception_no_code!(ex3, 3);
exception_no_code!(ex4, 4);
exception_no_code!(ex5, 5);
exception_no_code!(ex6, 6);
exception_no_code!(ex7, 7);
exception_with_code!(ex8, 8);
exception_no_code!(ex9, 9);
exception_with_code!(ex10, 10);
exception_with_code!(ex11, 11);
exception_with_code!(ex12, 12);
exception_with_code!(ex13, 13);
exception_with_code!(ex14, 14);
exception_no_code!(ex16, 16);
exception_with_code!(ex17, 17);
exception_no_code!(ex18, 18);
exception_no_code!(ex19, 19);
exception_no_code!(ex20, 20);
exception_with_code!(ex21, 21);

unsafe fn install_exception_handlers() {
    let no_code: [(usize, *const ()); 15] = [
        (0, ex0 as *const ()),
        (1, ex1 as *const ()),
        (2, ex2 as *const ()),
        (3, ex3 as *const ()),
        (4, ex4 as *const ()),
        (5, ex5 as *const ()),
        (6, ex6 as *const ()),
        (7, ex7 as *const ()),
        (9, ex9 as *const ()),
        (16, ex16 as *const ()),
        (18, ex18 as *const ()),
        (19, ex19 as *const ()),
        (20, ex20 as *const ()),
        (15, ex0 as *const ()),
        (31, ex0 as *const ()),
    ];
    for (vector, handler) in no_code {
        IDT[vector].set(handler as u64, KERNEL_CODE_SELECTOR, GATE_PRESENT_RING0_INT);
    }

    let with_code: [(usize, *const ()); 8] = [
        (8, ex8 as *const ()),
        (21, ex21 as *const ()),
        (10, ex10 as *const ()),
        (11, ex11 as *const ()),
        (12, ex12 as *const ()),
        (13, ex13 as *const ()),
        (14, ex14 as *const ()),
        (17, ex17 as *const ()),
    ];
    for (vector, handler) in with_code {
        IDT[vector].set(handler as u64, KERNEL_CODE_SELECTOR, GATE_PRESENT_RING0_INT);
    }
}

extern "x86-interrupt" fn pit_handler(_frame: InterruptStackFrame) {
    crate::level0a::pit::on_tick();
    // EOI baglam degisiminden ONCE: gorevi burada birakirsak PIC hala
    // hizmet bekliyor olur ve bir daha IRQ0 gelmez.
    crate::level0a::pic::send_eoi(crate::level0a::pic::IRQ_TIMER);

    // --- PREEMPTION ---
    // Zaman dilimi dolduysa gorev kendi istegi olmadan birakilir. Baglam
    // degisimi bu kesme yigininda yapilir (Ring 3 icin TSS.esp0); donuste
    // ayni noktaya donulup `iret` calisir -- syscall yolundaki desenin
    // aynisi.
    crate::level0a::core::scheduler::preempt_from_timer();
}

extern "x86-interrupt" fn keyboard_handler(_frame: InterruptStackFrame) {
    crate::level0a::keyboard::on_irq();
    crate::level0a::pic::send_eoi(crate::level0a::pic::IRQ_KEYBOARD);
}

extern "x86-interrupt" fn mouse_handler(_frame: InterruptStackFrame) {
    crate::level0a::input::on_mouse_irq();
    crate::level0a::pic::send_eoi(crate::level0a::pic::IRQ_MOUSE);
}

/// IRQ14 -- birincil ATA kanali "veri hazir / komut bitti" der.
extern "x86-interrupt" fn ata_handler(_frame: InterruptStackFrame) {
    crate::level0a::drivers::ata::on_irq();
    crate::level0a::pic::send_eoi(crate::level0a::pic::IRQ_ATA_PRIMARY);
}

// x86_64'te `pusha` yoktur; registerlar `SyscallFrame` alan sirasina
// **tam olarak uyacak** sekilde elle itilir (son itilen = en dusuk adres).
global_asm!(
    r#"
.section .text

.macro PUSH_SYSCALL_FRAME
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
.endm

.macro POP_SYSCALL_FRAME
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
.endm

.global syscall_entry
.type syscall_entry, @function
syscall_entry:
    PUSH_SYSCALL_FRAME
    mov rdi, rsp                /* &SyscallFrame */
    call syscall_dispatch_rust
    POP_SYSCALL_FRAME
    iretq

.global nt_syscall_entry
.type nt_syscall_entry, @function
nt_syscall_entry:
    PUSH_SYSCALL_FRAME
    mov rdi, rsp
    call nt_syscall_dispatch_rust
    POP_SYSCALL_FRAME
    iretq
"#
);

extern "C" {
    fn syscall_entry();
    fn nt_syscall_entry();
}

#[no_mangle]
extern "C" fn syscall_dispatch_rust(frame: *mut SyscallFrame) {
    unsafe {
        crate::level0b2::dispatcher::handle_syscall(&mut *frame);
    }
}

#[no_mangle]
extern "C" fn nt_syscall_dispatch_rust(frame: *mut SyscallFrame) {
    unsafe {
        crate::level0b2::dispatcher::handle_nt_syscall(&mut *frame);
    }
}
