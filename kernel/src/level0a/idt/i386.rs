//! 256 vektorluk IDT (doc S.4). Faz 1'de sadece dort vektor doldurulur:
//! 0 (divide-by-zero), 32 (IRQ0/PIT), 33 (IRQ1/klavye), 128 (int 0x80).
//!
//! Handler'larin ilk isi -- kod olarak -- Level-0b2'ye (dispatcher /
//! load_balancer) haber vermek; CPU donanimsal olarak hepsini ayni tabloya
//! yazmamiza ragmen mantiksal akis dokumandaki "her seyin once Level-0b2'den
//! gectigi" kuraliyla eslesir.

use core::arch::{asm, global_asm};
use core::mem::size_of;

use crate::arch::cpu::regs::{ExceptionFrame, InterruptStackFrame, SyscallFrame};
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
        // Tum 32 CPU istisnasi baglanir. Onceden yalnizca vektor 0 vardi
        // ve baglanmamis her istisna triple fault'a gidiyordu.
        install_exception_handlers();
        IDT[crate::level0a::pic::vector(crate::level0a::pic::IRQ_TIMER)].set(
            pit_handler as *const () as u32,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING0_INT32,
        );
        IDT[crate::level0a::pic::vector(crate::level0a::pic::IRQ_KEYBOARD)].set(
            keyboard_handler as *const () as u32,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING0_INT32,
        );
        // IRQ12 = PS/2 fare, IRQ14 = birincil ATA. Vektorler `pic::vector`
        // ile hesaplanir; slave denetleyici 0x70'e alindigi icin bunlar 44
        // degil 116/118'dir (gerekce: `pic.rs`).
        IDT[crate::level0a::pic::vector(crate::level0a::pic::IRQ_MOUSE)].set(
            mouse_handler as *const () as u32,
            KERNEL_CODE_SELECTOR,
            GATE_PRESENT_RING0_INT32,
        );
        IDT[crate::level0a::pic::vector(crate::level0a::pic::IRQ_ATA_PRIMARY)].set(
            ata_handler as *const () as u32,
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

/// CR2 -- page fault'ta hataya yol acan dogrusal adres.
fn read_cr2() -> usize {
    let value: u32;
    unsafe { asm!("mov {0}, cr2", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value as usize
}

// --- Istisna girisleri ------------------------------------------------
//
// Onceden bunlar `extern "x86-interrupt" fn`'lerdi. O ABI kolaydi ama
// **genel registerlari vermiyordu**: handler yalnizca EIP/CS/EFLAGS/ESP/SS
// goruyordu. Hatayi raporlamak icin yeterliydi; Windows SEH icin degil.
//
// SEH bir **CONTEXT** kaydi ister -- yani butun registerlar -- ve isleyici
// o kaydi degistirip "devam et" derse degisiklikler CPU'ya geri
// yazilabilmelidir. Bu yuzden girisler artik elle yazilir ve `syscall_entry`
// ile ayni deseni kullanir: `pusha` + cerceve adresini Rust'a ver.
//
// Iki stub turu var, cunku CPU bazi vektorlerde yigina bir hata kodu
// itip bazilarinda itmez. Hata kodu itmeyenler icin stub sifir iter,
// boylece Rust tarafi **tek** bir cerceve tipi gorur.
global_asm!(
    r#"
.section .text

.macro EX_NOCODE vec
.global ex_stub_\vec
.type ex_stub_\vec, @function
ex_stub_\vec:
    push 0                  /* sahte hata kodu -- duzeni tekdüze yapar */
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
    pusha                   /* ExceptionFrame'in ilk 32 bayti */
    push esp                /* &ExceptionFrame */
    call exception_dispatch_rust
    add esp, 4
    popa
    add esp, 8              /* vector + error_code */
    iretd

/* Vektor -> stub tablosu. IDT kurulumu bunu tarar; 32 tane `extern`
   bildirimi yazmaktansa tek bir dizi okumak daha az hataya acik. */
.section .rodata
.balign 4
.global exception_stubs
exception_stubs:
.irp v, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    .long ex_stub_\v
.endr
"#
);

extern "C" {
    static exception_stubs: [u32; 32];
}

/// Istisna cercevesini Level-0a'nin ortak isleyicisine verir.
///
/// Fonksiyon **donebilir**: hata kurtarildiysa (talep uzerine sayfalama,
/// copy-on-write) ya da cerceve bir SEH isleyicisine cevrildiyse stub
/// `iretd` calistirir ve Ring 3 devam eder.
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
            GATE_PRESENT_RING0_INT32,
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
        crate::level0b2::dispatcher::handle_syscall(&mut *frame, true);
    }
}

#[no_mangle]
extern "C" fn nt_syscall_dispatch_rust(frame: *mut SyscallFrame) {
    unsafe {
        crate::level0b2::dispatcher::handle_nt_syscall(&mut *frame, true);
    }
}
