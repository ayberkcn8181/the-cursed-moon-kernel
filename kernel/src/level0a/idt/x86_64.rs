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

use crate::arch::cpu::regs::{ExceptionFrame, InterruptStackFrame, SyscallFrame};
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

// --- Istisna girisleri ------------------------------------------------
//
// Gerekce i386'daki ikiziyle ayni (bkz. `idt::i386`): `x86-interrupt`
// ABI'si genel registerlari vermez, Windows SEH ise tam bir CONTEXT
// ister. Bu yuzden girisler elle yazilir.
//
// x86_64'te `pusha` yoktur; 15 register tek tek itilir ve sira
// `ExceptionFrame`/`SyscallFrame` alan siralamasiyla birebir eslesir.
global_asm!(
    r#"
.section .text

.macro EX_NOCODE vec
.global ex_stub_\vec
.type ex_stub_\vec, @function
ex_stub_\vec:
    push 0                  /* sahte hata kodu -- duzeni tekduze yapar */
    push \vec
    jmp exception_common
.endm

.macro EX_CODE vec
.global ex_stub_\vec
.type ex_stub_\vec, @function
ex_stub_\vec:
    push \vec               /* hata kodunu CPU zaten itti */
    jmp exception_common
.endm

.irp v, 0,1,2,3,4,5,6,7,9,15,16,18,19,20,22,23,24,25,26,27,28,31
    EX_NOCODE \v
.endr
.irp v, 8,10,11,12,13,14,17,21,29,30
    EX_CODE \v
.endr

exception_common:
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
    mov rdi, rsp            /* &ExceptionFrame (System V ilk arguman) */
    call exception_dispatch_rust
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
    add rsp, 16             /* vector + error_code */
    iretq

/* Vektor -> stub tablosu; IDT kurulumu bunu tarar. */
.section .rodata
.balign 8
.global exception_stubs
exception_stubs:
.irp v, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    .quad ex_stub_\v
.endr
"#
);

extern "C" {
    static exception_stubs: [u64; 32];
}

/// Istisna cercevesini Level-0a'nin ortak isleyicisine verir.
///
/// Fonksiyon **donebilir**: hata kurtarildiysa (talep uzerine sayfalama,
/// copy-on-write) ya da cerceve bir SEH isleyicisine cevrildiyse stub
/// `iretq` calistirir ve Ring 3 devam eder.
#[no_mangle]
extern "C" fn exception_dispatch_rust(frame: *mut ExceptionFrame) {
    unsafe {
        let fault_addr = if (*frame).vector == 14 { read_cr2() } else { 0 };
        crate::level0a::exceptions::dispatch(&mut *frame, fault_addr);
    }
}

unsafe fn install_exception_handlers() {
    for vector in 0..32usize {
        IDT[vector].set(
            exception_stubs[vector],
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING0_INT,
        );
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
        crate::level0b2::dispatcher::handle_syscall(&mut *frame, true);
    }
}

#[no_mangle]
extern "C" fn nt_syscall_dispatch_rust(frame: *mut SyscallFrame) {
    unsafe {
        crate::level0b2::dispatcher::handle_nt_syscall(&mut *frame, true);
    }
}
