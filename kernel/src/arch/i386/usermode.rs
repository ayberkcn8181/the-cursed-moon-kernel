//! Ring 0 <-> Ring 3 gecisleri (doc S.7 Faz 3: "Ring 3 user mode (iret)").
//!
//! Ring 3'e gecis `iret` ile yapilir: CPU'ya sanki bir kesmeden donuyormus
//! gibi SS/ESP/EFLAGS/CS/EIP yigina konur ve `iretd` calistirilir. CS/SS'in
//! RPL'i 3 oldugu icin CPU ayricalik seviyesini dusurur.
//!
//! Geri donus (sys_exit) icin **klasik bir gorev degistirme kullanilamaz**:
//! kullanici programi kendi Ring 3 yiginindadir ve syscall aninda CPU
//! TSS.esp0'daki ayri bir cekirdek yiginina gecmistir. Bu yuzden Ring 3'e
//! girmeden hemen once cekirdek baglami (callee-saved registerlar + EFLAGS)
//! saklanir; `sys_exit` geldiginde bu baglam geri yuklenerek
//! `enter_user_mode`'un cagrildigi yere donulur -- yani bir setjmp/longjmp
//! cifti. Kayit duzeni bilincli olarak `arch_context_switch` ile aynidir.

use core::arch::global_asm;

global_asm!(
    r#"
.section .text

.global arch_enter_user_mode
.type arch_enter_user_mode, @function
arch_enter_user_mode:
    mov eax, [esp + 4]      /* entry (Ring 3 EIP)      */
    mov ecx, [esp + 8]      /* user_stack_top (Ring 3 ESP) */
    mov edx, [esp + 12]     /* &resume_slot            */

    push ebp
    push ebx
    push esi
    push edi
    pushfd
    mov [edx], esp          /* cekirdek baglamini sakla */

    /* Ring 3 veri segmentleri (secici 0x20 | RPL 3 = 0x23) */
    mov bx, 0x23
    mov ds, bx
    mov es, bx
    mov fs, bx
    mov gs, bx

    push 0x23               /* SS     */
    push ecx                /* ESP    */
    push 0x202              /* EFLAGS: IF=1 */
    push 0x1B               /* CS  (0x18 | RPL 3) */
    push eax                /* EIP    */
    iretd

.global arch_return_from_user
.type arch_return_from_user, @function
arch_return_from_user:
    mov eax, [esp + 4]      /* &resume_slot */
    mov esp, [eax]          /* cekirdek yiginina geri don */

    /* Ring 0 veri segmentlerini geri yukle */
    mov bx, 0x10
    mov ds, bx
    mov es, bx
    mov fs, bx
    mov gs, bx

    popfd
    pop edi
    pop esi
    pop ebx
    pop ebp
    ret
"#
);

extern "C" {
    /// Ring 3'e gecer. Program `sys_exit` cagirana kadar geri DONMEZ;
    /// donus `arch_return_from_user` uzerinden gerceklesir.
    fn arch_enter_user_mode(entry: usize, user_stack_top: usize, resume_slot: *mut usize);

    /// `enter_user_mode`'un cagrildigi noktaya geri doner.
    fn arch_return_from_user(resume_slot: *mut usize) -> !;
}

/// Ring 3 baglaminin geri donus noktasi. Tek bir kullanici programi
/// destekledigimiz icin (Faz 3) tek slot yeterli; Faz 8'de process basina
/// tasinacak.
/// Ring 3 baglami artik **gorev basina** tutulur (bkz.
/// `scheduler::current_resume_slot`). Tek global slot kullanmak, ikinci bir
/// GUI uygulamasi baslatildiginda ilkinin donus adresini eziyordu.

/// Ring 3'e gecip kullanici programini calistirir; program `sys_exit`
/// cagirdiginda buraya doner.
///
/// # Safety
/// `entry` ve `user_stack_top` kullaniciya acik (PTE User biti set) ve
/// gecerli sayfalarda olmalidir; TSS gecerli bir cekirdek yiginini
/// gostermelidir.
pub unsafe fn run_user_program(entry: usize, user_stack_top: usize) {
    use crate::level0a::core::scheduler;

    scheduler::set_current_in_user_mode(true);
    arch_enter_user_mode(entry, user_stack_top, scheduler::current_resume_slot());
    scheduler::set_current_in_user_mode(false);
}

/// Calisan gorev Ring 3'te bir program yurutuyor mu?
pub fn in_user_mode() -> bool {
    crate::level0a::core::scheduler::current_in_user_mode()
}

/// # Safety
/// Yalnizca `in_user_mode()` dogruyken cagrilmalidir.
pub unsafe fn leave_user_mode() -> ! {
    arch_return_from_user(crate::level0a::core::scheduler::current_resume_slot())
}
