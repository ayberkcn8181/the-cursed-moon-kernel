//! 256 vektorluk IDT (doc S.4). Faz 1'de sadece dort vektor doldurulur:
//! 0 (divide-by-zero), 32 (IRQ0/PIT), 33 (IRQ1/klavye), 128 (int 0x80).
//!
//! Handler'larin ilk isi -- kod olarak -- Level-0b2'ye (dispatcher /
//! load_balancer) haber vermek; CPU donanimsal olarak hepsini ayni tabloya
//! yazmamiza ragmen mantiksal akis dokumandaki "her seyin once Level-0b2'den
//! gectigi" kuraliyla eslesir.

use core::arch::{asm, global_asm};
use core::mem::size_of;

use crate::arch::cpu::regs::{InterruptStackFrame, SyscallFrame};
use crate::level0a::gdt::KERNEL_CODE_SELECTOR;

const GATE_PRESENT_RING0_INT32: u8 = 0x8E; // P=1 DPL=00 type=32-bit interrupt gate
const GATE_PRESENT_RING3_INT32: u8 = 0xEE; // P=1 DPL=11 (Ring3'ten int 0x80 cagrilabilir)

#[derive(Clone, Copy)]
#[repr(C, packed)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    zero: u8,
    type_attr: u8,
    offset_high: u16,
}

impl IdtEntry {
    const fn missing() -> Self {
        IdtEntry {
            offset_low: 0,
            selector: 0,
            zero: 0,
            type_attr: 0,
            offset_high: 0,
        }
    }

    fn set(&mut self, handler_addr: u32, selector: u16, type_attr: u8) {
        self.offset_low = (handler_addr & 0xFFFF) as u16;
        self.offset_high = ((handler_addr >> 16) & 0xFFFF) as u16;
        self.selector = selector;
        self.zero = 0;
        self.type_attr = type_attr;
    }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u32,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

pub fn init() {
    unsafe {
        IDT[0].set(
            divide_by_zero_handler as *const () as u32,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING0_INT32,
        );
        IDT[32].set(
            pit_handler as *const () as u32,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING0_INT32,
        );
        IDT[33].set(
            keyboard_handler as *const () as u32,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING0_INT32,
        );
        IDT[128].set(
            syscall_entry as *const () as u32,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING3_INT32,
        );
        // Vektor 46 = int 0x2E: Windows NT uyumlu sistem cagrisi (doc S.4).
        IDT[46].set(
            nt_syscall_entry as *const () as u32,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING3_INT32,
        );

        let ptr = IdtPointer {
            limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u32,
        };
        asm!("lidt [{0}]", in(reg) &ptr, options(nostack));
    }
}

extern "x86-interrupt" fn divide_by_zero_handler(_frame: InterruptStackFrame) -> ! {
    crate::level0b2::fallback::emergency(&["CPU istisnasi: divide-by-zero (vektor 0)."]);
    loop {
        crate::arch::cpu::halt();
    }
}

extern "x86-interrupt" fn pit_handler(_frame: InterruptStackFrame) {
    crate::level0a::pit::on_tick();
    crate::level0a::pic::send_eoi(0);
}

extern "x86-interrupt" fn keyboard_handler(_frame: InterruptStackFrame) {
    crate::level0a::keyboard::on_irq();
    crate::level0a::pic::send_eoi(1);
}

// int 0x80 girisi.
//
// `extern "x86-interrupt"` burada YETMEZ: Linux ABI'si syscall numarasini ve
// argumanlari REGISTERLARDA tasir (EAX/EBX/ECX/EDX/ESI/EDI), o ABI ise
// registerlara erisim vermez. Bu yuzden giris elle yazilir: `pusha` tum
// genel registerlari `SyscallFrame` duzeninde yigina koyar, frame'in adresi
// Rust tarafina verilir, `popa` ise -- handler EAX slotunu guncelledigi icin --
// donus degerini kullaniciya tasir.
global_asm!(
    r#"
.section .text

.global syscall_entry
.type syscall_entry, @function
syscall_entry:
    pusha
    push esp                /* &SyscallFrame */
    call syscall_dispatch_rust
    add esp, 4
    popa
    iretd

.global nt_syscall_entry
.type nt_syscall_entry, @function
nt_syscall_entry:
    pusha
    push esp                /* &SyscallFrame */
    call nt_syscall_dispatch_rust
    add esp, 4
    popa
    iretd
"#
);

extern "C" {
    fn syscall_entry();
    fn nt_syscall_entry();
}

#[no_mangle]
extern "C" fn syscall_dispatch_rust(frame: *mut SyscallFrame) {
    // Doc S.3: cagri once Level-0b2'ye duser, oradan Level-0b1'e dagitilir.
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
