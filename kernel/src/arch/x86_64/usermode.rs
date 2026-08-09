//! x86_64 Ring 0 <-> Ring 3 gecisleri.
//!
//! i386 tarafiyla ayni felsefe (bkz. `arch/i386/usermode.rs`): Ring 3'e
//! `iretq` ile girilir, geri donus icin cekirdek baglami saklanip
//! `sys_exit`'te geri yuklenir (setjmp/longjmp cifti).
//!
//! Fark: 64-bit modda `iretq` cercevesi 5 x 8 bayttir ve segment
//! seciciler GDT64'ten gelir (kullanici kod 0x1B, kullanici veri 0x23).

use core::arch::global_asm;

global_asm!(
    r#"
.section .text

.global arch_enter_user_mode
.type arch_enter_user_mode, @function
arch_enter_user_mode:
    /* System V AMD64: rdi = entry, rsi = user_stack_top, rdx = &resume_slot */
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    pushfq
    mov [rdx], rsp          /* cekirdek baglamini sakla */

    /* Ring 3 veri segmentleri.
       DIKKAT: x86_64 GDT'sinde kullanici VERI'si kullanici KOD'undan once
       gelir (sysret sozlesmesi), yani i386'nin tam tersi:
         kullanici veri = 0x18 | RPL 3 = 0x1B
         kullanici kod  = 0x20 | RPL 3 = 0x23 */
    mov ax, 0x1B
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    /* iretq cercevesi: SS, RSP, RFLAGS, CS, RIP */
    push 0x1B               /* SS  (kullanici veri) */
    push rsi                /* RSP                  */
    push 0x202              /* RFLAGS: IF=1         */
    push 0x23               /* CS  (kullanici kod)  */
    push rdi                /* RIP                  */
    iretq

.global arch_return_from_user
.type arch_return_from_user, @function
arch_return_from_user:
    /* rdi = &resume_slot */
    mov rsp, [rdi]

    /* Ring 0 veri segmentlerini geri yukle */
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    popfq
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret
"#
);

extern "C" {
    fn arch_enter_user_mode(entry: usize, user_stack_top: usize, resume_slot: *mut usize);
    fn arch_return_from_user(resume_slot: *mut usize) -> !;
}

static mut RESUME_SLOT: usize = 0;
static mut IN_USER_MODE: bool = false;

/// Ring 3'e gecip kullanici programini calistirir; program `sys_exit`
/// cagirdiginda buraya doner.
///
/// # Safety
/// `entry` ve `user_stack_top` kullaniciya acik (PTE User biti set) ve
/// gecerli sayfalarda olmalidir; TSS.rsp0 gecerli bir cekirdek yiginini
/// gostermelidir.
pub unsafe fn run_user_program(entry: usize, user_stack_top: usize) {
    IN_USER_MODE = true;
    arch_enter_user_mode(entry, user_stack_top, core::ptr::addr_of_mut!(RESUME_SLOT));
    IN_USER_MODE = false;
}

pub fn in_user_mode() -> bool {
    unsafe { core::ptr::addr_of!(IN_USER_MODE).read_volatile() }
}

/// # Safety
/// Yalnizca `in_user_mode()` dogruyken cagrilmalidir.
pub unsafe fn leave_user_mode() -> ! {
    arch_return_from_user(core::ptr::addr_of_mut!(RESUME_SLOT))
}
