//! x86_64 gorev baglami degistirme -- i386 tarafiyla ayni sozlesme,
//! yalnizca System V AMD64 ABI'sinin callee-saved kumesi farklidir
//! (RBX, RBP, R12-R15).

use core::arch::global_asm;

global_asm!(
    r#"
.section .text
.global arch_context_switch
.type arch_context_switch, @function
arch_context_switch:
    /* System V AMD64: rdi = old_sp_slot, rsi = new_sp */
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    pushfq

    mov [rdi], rsp
    mov rsp, rsi

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
    pub fn arch_context_switch(old_sp_slot: *mut usize, new_sp: usize);
}

/// Yeni bir gorevin yigininin baslangic goruntusunu olusturur.
///
/// Bellekte yuksekten dusuge: [entry] [RBP] [RBX] [R12] [R13] [R14] [R15]
/// [RFLAGS]. Donen deger RFLAGS slotunu (gorevin baslangic RSP'sini) gosterir.
///
/// # Safety
/// `stack_top` gecerli, en az 8 kelime yer birakan, 16 bayt hizali bir
/// cekirdek yigini tepesi olmalidir.
pub unsafe fn bootstrap_stack(stack_top: *mut usize, entry: extern "C" fn() -> !) -> usize {
    let mut sp = stack_top;

    // IF=1 (bit 9) + her zaman 1 olan bit 1.
    const INITIAL_RFLAGS: usize = 0x0000_0202;

    sp = sp.offset(-1);
    sp.write(entry as usize); // `ret` hedefi
    for _ in 0..6 {
        sp = sp.offset(-1);
        sp.write(0); // RBP, RBX, R12, R13, R14, R15
    }
    sp = sp.offset(-1);
    sp.write(INITIAL_RFLAGS);

    sp as usize
}
