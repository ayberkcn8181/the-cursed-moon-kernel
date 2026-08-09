//! x86_64 `syscall`/`sysret` altyapisi (doc S.15: x86_64 -> `syscall`).
//!
//! i386'da sistem cagrisi bir kesmedir (int 0x80); x86_64'te ise `syscall`
//! komutu MSR'lerle yapilandirilir:
//!
//!   EFER.SCE = 1   `syscall` komutunu etkinlestirir
//!   STAR           cekirdek/kullanici segment seciciler
//!   LSTAR          `syscall` sonrasi atlanacak RIP
//!   SFMASK         `syscall` sirasinda temizlenecek RFLAGS bitleri
//!
//! `syscall`, i386'daki kesmenin aksine **yigin degistirmez**: RSP hala
//! kullanicinindir ve TSS devreye girmez. Bu yuzden giris stub'i once
//! cekirdek yiginina gecmek zorundadir.

use crate::arch::cpu::{rdmsr, wrmsr, MSR_EFER, MSR_LSTAR, MSR_SFMASK, MSR_STAR};
use core::arch::global_asm;
use core::sync::atomic::{AtomicUsize, Ordering};

const EFER_SCE: u64 = 1 << 0;
/// RFLAGS.IF | RFLAGS.DF | RFLAGS.TF -- syscall girisinde temizlenir.
const SFMASK_BITS: u64 = 0x0000_0700;

/// `syscall` girisinde gecilecek cekirdek yigini (TSS.rsp0'in esdegeri).
static KERNEL_RSP: AtomicUsize = AtomicUsize::new(0);

#[no_mangle]
static mut SYSCALL_KERNEL_RSP: usize = 0;
#[no_mangle]
static mut SYSCALL_USER_RSP: usize = 0;

/// `syscall` komutunu etkinlestirir.
///
/// STAR duzeni: [63:48] = sysret taban secicisi, [47:32] = syscall taban
/// secicisi. `sysret` CS = taban+16, SS = taban+8 kullanir; bu yuzden
/// GDT'de kullanici veri segmenti kullanici kodundan ONCE gelmelidir
/// (bkz. `level0a/gdt/x86_64.rs`).
///
/// # Safety
/// GDT kurulduktan sonra cagrilmalidir.
pub unsafe fn init(kernel_stack_top: usize) {
    set_kernel_stack(kernel_stack_top);

    wrmsr(MSR_EFER, rdmsr(MSR_EFER) | EFER_SCE);

    // syscall tabani 0x08 (cekirdek kod), sysret tabani 0x10.
    // sysret: CS = 0x10 + 16 = 0x20 (kullanici kod), SS = 0x10 + 8 = 0x18.
    let star: u64 = (0x10u64 << 48) | (0x08u64 << 32);
    wrmsr(MSR_STAR, star);
    wrmsr(MSR_LSTAR, syscall_instruction_entry as *const () as u64);
    wrmsr(MSR_SFMASK, SFMASK_BITS);
}

pub fn set_kernel_stack(kernel_stack_top: usize) {
    KERNEL_RSP.store(kernel_stack_top, Ordering::Relaxed);
    unsafe {
        core::ptr::addr_of_mut!(SYSCALL_KERNEL_RSP).write(kernel_stack_top);
    }
}

// `syscall` yigin degistirmedigi icin giris stub'i once cekirdek yiginina
// gecer, sonra `SyscallFrame` duzenini kurar. `syscall` RCX'e donus RIP'ini,
// R11'e RFLAGS'i koydugundan `sysretq` bu ikisini geri ister -- cerceve
// ikisini de koruyup geri yukler.
global_asm!(
    r#"
.section .text
.global syscall_instruction_entry
.type syscall_instruction_entry, @function
syscall_instruction_entry:
    /* Kullanici RSP'sini sakla, cekirdek yiginina gec */
    mov [rip + SYSCALL_USER_RSP], rsp
    mov rsp, [rip + SYSCALL_KERNEL_RSP]

    /* SyscallFrame duzeni (idt/x86_64.rs ile ayni sira) */
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

    mov rdi, rsp
    call syscall_instruction_dispatch

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

    /* Kullanici yiginina don ve Ring 3'e geri dus */
    mov rsp, [rip + SYSCALL_USER_RSP]
    sysretq
"#
);

extern "C" {
    fn syscall_instruction_entry();
}

/// `syscall` ile gelen cagrilar da ayni Level-0b2 kapisindan gecer.
///
/// Numara 0x1000 ve uzeriyse NT servisidir (doc S.7: "syscall rax>=0x1000
/// NT dispatch"), aksi halde POSIX.
#[no_mangle]
extern "C" fn syscall_instruction_dispatch(frame: *mut crate::arch::cpu::regs::SyscallFrame) {
    unsafe {
        let f = &mut *frame;
        if crate::level0b1::nt_subsystem::nt_syscalls::is_nt_service(f.number()) {
            crate::level0b2::dispatcher::handle_nt_syscall(f);
        } else {
            crate::level0b2::dispatcher::handle_syscall(f);
        }
    }
}
