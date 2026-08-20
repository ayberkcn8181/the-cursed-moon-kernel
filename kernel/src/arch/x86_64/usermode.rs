//! x86_64 Ring 0 <-> Ring 3 gecisleri.
//!
//! i386 tarafiyla ayni felsefe (bkz. `arch/i386/usermode.rs`): Ring 3'e
//! `iretq` ile girilir, geri donus icin cekirdek baglami saklanip
//! `sys_exit`'te geri yuklenir (setjmp/longjmp cifti).
//!
//! Fark: 64-bit modda `iretq` cercevesi 5 x 8 bayttir ve segment
//! seciciler GDT64'ten gelir (kullanici kod 0x1B, kullanici veri 0x23).

use core::arch::global_asm;

use crate::arch::x86_64::regs::UserContext;

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
    /* FS/GS BILEREK YUKLENMIYOR.
       Long mode'da bir segment registerina secici yuklemek, o registerin
       TABAN MSR'sini (IA32_FS_BASE / IA32_GS_BASE) **sifirlar** -- duz
       64-bit veri tanimlayicisinin tabani sifir oldugu icin. Burada
       yuklemek, cekirdegin az once yazdigi is-parcacigi tabanini
       silerdi (bkz. `level0a::core::tls`).

       Bir kez oyleydi ve sonucu net bir sayfa hatasiydi: PE'nin ilk
       `gs:[0x30]` okumasi 0x30 adresine gitti. i386'da tam tersi gecerli
       -- orada taban tanimlayicida durur ve register YUKLENMEK zorunda. */

    /* iretq cercevesi: SS, RSP, RFLAGS, CS, RIP */
    push 0x1B               /* SS  (kullanici veri) */
    push rsi                /* RSP                  */
    push 0x202              /* RFLAGS: IF=1         */
    push 0x23               /* CS  (kullanici kod)  */
    push rdi                /* RIP                  */
    iretq

/* `fork` sonrasi cocugu ebeveynin durdugu tam noktadan baslatir.
 *
 * `arch_enter_user_mode`'dan farki butun genel registerlari da
 * yuklemesidir: derleyici syscall sonrasi cagri-korumali registerlarin
 * durdugunu varsayar, cocuk bunlari ebeveyninkiyle ayni gormezse ilk
 * erisimde coker.
 *
 * `sysretq` degil `iretq` kullanilir: sysretq RCX/R11'i kendi
 * sozlesmesi icin ister, oysa burada ikisi de geri yuklenecek gercek
 * kullanici degerleridir.
 *
 * Alan sirasi `regs::UserContext` ile birebir aynidir. */
.global arch_enter_user_mode_regs
.type arch_enter_user_mode_regs, @function
arch_enter_user_mode_regs:
    /* rdi = &UserContext, rsi = &resume_slot */
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    pushfq
    mov [rsi], rsp          /* cekirdek baglamini sakla */

    mov ax, 0x1B
    mov ds, ax
    mov es, ax
    /* FS/GS yuklenmiyor: secici yuklemek taban MSR'sini sifirlar
       (bkz. `arch_enter_user_mode`). Cocuk, ebeveynden devraldigi
       is-parcacigi tabanini boylece koruyor. */

    /* iretq cercevesi: SS, RSP, RFLAGS, CS, RIP */
    push 0x1B
    push qword ptr [rdi + 128]   /* rsp    */
    push qword ptr [rdi + 136]   /* rflags */
    push 0x23
    push qword ptr [rdi + 120]   /* rip    */

    /* Genel registerlar; RDI en sonda cunku taban isaretcisi odur. */
    mov rax, [rdi + 0]
    mov rbx, [rdi + 8]
    mov rcx, [rdi + 16]
    mov rdx, [rdi + 24]
    mov rsi, [rdi + 32]
    mov rbp, [rdi + 48]
    mov r8,  [rdi + 56]
    mov r9,  [rdi + 64]
    mov r10, [rdi + 72]
    mov r11, [rdi + 80]
    mov r12, [rdi + 88]
    mov r13, [rdi + 96]
    mov r14, [rdi + 104]
    mov r15, [rdi + 112]
    mov rdi, [rdi + 40]
    iretq

.global arch_return_from_user
.type arch_return_from_user, @function
arch_return_from_user:
    /* rdi = &resume_slot */
    mov rsp, [rdi]

    /* Ring 0 veri segmentlerini geri yukle.
       FS/GS yine disarida: cekirdek onlari kullanmiyor, ve yuklemek
       calisan gorevin is-parcacigi tabanini silerdi. */
    mov ax, 0x10
    mov ds, ax
    mov es, ax

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
    /// Tam bir Ring 3 baglamini geri yukleyerek Ring 3'e gecer (`fork`).
    fn arch_enter_user_mode_regs(context: *const UserContext, resume_slot: *mut usize);
    fn arch_return_from_user(resume_slot: *mut usize) -> !;
}

/// `fork` edilmis bir cocugu ebeveynin baglamiyla Ring 3'te surdurur.
///
/// # Safety
/// `context` Ring 3'ten alinmis gecerli bir baglam olmalidir ve cagiran
/// gorevin adres uzayi o baglamin gectigi uzay olmalidir.
pub unsafe fn resume_user_context(context: &UserContext) {
    use crate::level0a::core::scheduler;

    scheduler::set_current_in_user_mode(true);
    arch_enter_user_mode_regs(context as *const UserContext, scheduler::current_resume_slot());
    scheduler::set_current_in_user_mode(false);
}

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

/// Ring 3 yigininin ustune bir **sinyal cercevesi** kurar ve baglami
/// isleyiciye cevirir.
///
/// System V AMD64 duzeni: ilk arguman RDI'dedir, yigina yalnizca donus
/// adresi konur.
///
/// ```text
///   RDI     = signo
///   [rsp]   = restorer   (isleyici `ret` ile buraya doner)
///
/// ```
///
/// Iki incelik:
///
/// * **Kirmizi bolge** (red zone): `rsp`'nin 128 bayt altini yaprak
///   fonksiyonlar cekirdekten haber vermeden kullanabilir. Cerceve oraya
///   kurulursa calisan kodun yerel degiskenleri ezilir; bu yuzden once
///   128 bayt atlanir. (Linux de aynisini yapar.)
/// * **Hizalama**: `call` sonrasi `rsp % 16 == 8` beklenir; donus
///   adresini ittikten sonra tam bu durum olusur.
///
/// # Safety
/// Cagiran gorevin adres uzayi etkin olmalidir; yazilan adres dogrulanir.
pub unsafe fn build_signal_frame(
    context: &mut UserContext,
    signo: u32,
    handler: usize,
    restorer: usize,
) -> Option<()> {
    use crate::level0a::core::mmu;

    let sp = ((context.stack_pointer() - 128) & !0xF) - 8;
    if !mmu::is_user_accessible(sp) || !mmu::is_user_accessible(sp + 7) {
        return None;
    }
    (sp as *mut u64).write_unaligned(restorer as u64);
    context.rdi = signo as u64;
    context.redirect(handler, sp);
    Some(())
}
