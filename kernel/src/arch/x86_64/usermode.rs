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

// --- `SA_SIGINFO` cercevesi (x86_64) ----------------------------------
//
// i386'daki ikiziyle ayni gerekce: sayilar Linux ABI'sinin parcasidir
// (bkz. orada). Duzen yine de **ayni degil** -- `siginfo_t`nin birlesimi
// 8'e hizalandigi icin arada bir dolgu var, ve `ucontext_t` registerlari
// bir dizide (`gregs`) tutuyor.

const SIGINFO_SIZE: usize = 128;

/// `siginfo_t` alan ofsetleri (x86_64).
mod si {
    pub const SIGNO: usize = 0x00;
    pub const ERRNO: usize = 0x04;
    pub const CODE: usize = 0x08;
    /// 0x0C'de dolgu var: birlesim 8'e hizali basliyor.
    pub const ADDR: usize = 0x10;
    pub const PID: usize = 0x10;
    pub const UID: usize = 0x14;
}

/// `ucontext_t` icin ayrilan yer (glibc olcusu; bkz. i386 ikizi).
const UCONTEXT_SIZE: usize = 968;

/// `ucontext_t` alan ofsetleri (x86_64).
///
/// `uc_mcontext` 0x28'te basliyor ve ilk alani `gregs`, 23 kelimelik bir
/// dizi. Asagidaki ofsetler `ucontext_t`nin basina goredir; her biri
/// `0x28 + REG_* * 8`.
mod uc {
    pub const FLAGS: usize = 0x00;
    pub const LINK: usize = 0x08;
    pub const STACK: usize = 0x10;
    pub const MCONTEXT: usize = 0x28;

    pub const R8: usize = MCONTEXT;
    pub const R9: usize = MCONTEXT + 8;
    pub const R10: usize = MCONTEXT + 16;
    pub const R11: usize = MCONTEXT + 24;
    pub const R12: usize = MCONTEXT + 32;
    pub const R13: usize = MCONTEXT + 40;
    pub const R14: usize = MCONTEXT + 48;
    pub const R15: usize = MCONTEXT + 56;
    pub const RDI: usize = MCONTEXT + 64;
    pub const RSI: usize = MCONTEXT + 72;
    pub const RBP: usize = MCONTEXT + 80;
    pub const RBX: usize = MCONTEXT + 88;
    pub const RDX: usize = MCONTEXT + 96;
    pub const RAX: usize = MCONTEXT + 104;
    pub const RCX: usize = MCONTEXT + 112;
    pub const RSP: usize = MCONTEXT + 120;
    pub const RIP: usize = MCONTEXT + 128;
    pub const EFL: usize = MCONTEXT + 136;
    pub const CSGSFS: usize = MCONTEXT + 144;
    pub const ERR: usize = MCONTEXT + 152;
    pub const TRAPNO: usize = MCONTEXT + 160;
    pub const OLDMASK: usize = MCONTEXT + 168;
    /// `cr2` -- sayfa hatasinin adresi.
    pub const CR2: usize = MCONTEXT + 176;
    pub const SIGMASK: usize = 0x128;
}

/// i386 ikiziyle ayni: bayraklarin kullaniciya birakilan bitleri.
const USER_FLAGS: u64 = 0x0000_0CD5;

/// Ring 3 yigininin ustune bir **`SA_SIGINFO` cercevesi** kurar.
///
/// System V duzeni: uc arguman da registerda gider, yigina yalnizca
/// donus adresi konur.
///
/// ```text
///   RDI   = signo
///   RSI   = siginfo_t*
///   RDX   = ucontext_t*
///   [rsp] = restorer
///
///   ... yukarida (kirmizi bolgenin de ustunde):
///   [siginfo_t]    128 bayt
///   [ucontext_t]   968 bayt
/// ```
///
/// Kirmizi bolge yine atlaniyor (bkz. `build_signal_frame`).
///
/// Doner: Ring 3'teki `ucontext_t`nin adresi.
///
/// # Safety
/// `build_signal_frame` ile ayni kosul.
pub unsafe fn build_siginfo_frame(
    context: &mut UserContext,
    signo: u32,
    handler: usize,
    restorer: usize,
    info: &crate::level0b1::signal::SigInfo,
) -> Option<usize> {
    use crate::level0a::core::mmu;

    const RECORDS: usize = SIGINFO_SIZE + UCONTEXT_SIZE;
    let sp = context.stack_pointer();
    if sp < RECORDS + 256 {
        return None;
    }
    let base = (sp - 128 - RECORDS) & !0xF;
    let mut probe = base;
    while probe < base + RECORDS {
        if !mmu::is_user_accessible(probe) {
            return None;
        }
        probe += 4096;
    }
    if !mmu::is_user_accessible(base + RECORDS - 1) {
        return None;
    }

    let siginfo_at = base;
    let ucontext_at = base + SIGINFO_SIZE;

    let call = (base & !0xF) - 8;
    if !mmu::is_user_accessible(call) || !mmu::is_user_accessible(call + 7) {
        return None;
    }

    core::ptr::write_bytes(siginfo_at as *mut u8, 0, RECORDS);
    let put32 = |at: usize, value: u32| (at as *mut u32).write_unaligned(value);

    put32(siginfo_at + si::SIGNO, signo);
    put32(siginfo_at + si::ERRNO, 0);
    put32(siginfo_at + si::CODE, info.code as u32);
    if info.addr != 0 {
        ((siginfo_at + si::ADDR) as *mut u64).write_unaligned(info.addr as u64);
    } else {
        put32(siginfo_at + si::PID, info.pid as u32);
        put32(siginfo_at + si::UID, 0);
    }

    write_ucontext(ucontext_at, context, info);

    (call as *mut u64).write_unaligned(restorer as u64);
    context.rdi = signo as u64;
    context.rsi = siginfo_at as u64;
    context.rdx = ucontext_at as u64;

    context.redirect(handler, call);
    Some(ucontext_at)
}

/// Kesilen baglami `ucontext_t`ye doker.
unsafe fn write_ucontext(
    at: usize,
    context: &UserContext,
    info: &crate::level0b1::signal::SigInfo,
) {
    let put = |offset: usize, value: u64| ((at + offset) as *mut u64).write_unaligned(value);
    put(uc::FLAGS, 0);
    put(uc::LINK, 0);
    put(uc::STACK, 0);
    put(uc::R8, context.r8);
    put(uc::R9, context.r9);
    put(uc::R10, context.r10);
    put(uc::R11, context.r11);
    put(uc::R12, context.r12);
    put(uc::R13, context.r13);
    put(uc::R14, context.r14);
    put(uc::R15, context.r15);
    put(uc::RDI, context.rdi);
    put(uc::RSI, context.rsi);
    put(uc::RBP, context.rbp);
    put(uc::RBX, context.rbx);
    put(uc::RDX, context.rdx);
    put(uc::RAX, context.rax);
    put(uc::RCX, context.rcx);
    put(uc::RSP, context.rsp);
    put(uc::RIP, context.rip);
    put(uc::EFL, context.rflags);
    // `csgsfs`: CS, GS, FS seciciileri tek kelimede paketlenir. Ring 3
    // CS'i 0x33'tur (bkz. `gdt::x86_64`).
    put(uc::CSGSFS, 0x33);
    put(uc::ERR, 0);
    put(uc::TRAPNO, 0);
    put(uc::OLDMASK, 0);
    put(uc::CR2, info.addr as u64);
    let _ = uc::SIGMASK;
}

/// Tersi: isleyicinin (belki degistirdigi) `ucontext_t`sini okur.
///
/// # Safety
/// `at` Ring 3'e ait, okunabilir bir `ucontext_t` olmalidir.
pub unsafe fn read_ucontext(at: usize, context: &mut UserContext) {
    let get = |offset: usize| ((at + offset) as *const u64).read_unaligned();
    context.r8 = get(uc::R8);
    context.r9 = get(uc::R9);
    context.r10 = get(uc::R10);
    context.r11 = get(uc::R11);
    context.r12 = get(uc::R12);
    context.r13 = get(uc::R13);
    context.r14 = get(uc::R14);
    context.r15 = get(uc::R15);
    context.rdi = get(uc::RDI);
    context.rsi = get(uc::RSI);
    context.rbp = get(uc::RBP);
    context.rbx = get(uc::RBX);
    context.rdx = get(uc::RDX);
    context.rax = get(uc::RAX);
    context.rcx = get(uc::RCX);
    context.rsp = get(uc::RSP);
    context.rip = get(uc::RIP);
    context.rflags = (get(uc::EFL) & USER_FLAGS) | 0x202;
}
