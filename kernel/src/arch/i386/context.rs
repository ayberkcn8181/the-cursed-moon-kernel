//! Gorev baglami degistirme (context switch) -- mimariye ozel tek parca
//! (doc S.15 ilke 2: scheduler'in kendisi mimari bagimsiz kalir, sadece bu
//! dosya i386'ya ozeldir).
//!
//! Cagri sozlesmesi:
//!   `arch_context_switch(old_sp_slot: *mut u32, new_sp: u32)`
//!
//! Mevcut gorevin callee-saved registerlari + EFLAGS yigina itilir, olusan
//! ESP `old_sp_slot`'a yazilir; ardindan `new_sp`'ye gecilip ayni sira ile
//! geri yuklenir. `ret`, yeni gorevin yigininin tepesindeki adrese doner --
//! bu ya kesintiye ugradigi yerdir ya da (yeni gorevler icin)
//! `scheduler::bootstrap_stack` tarafindan elle yerlestirilmis giris
//! noktasidir.

use core::arch::global_asm;

global_asm!(
    r#"
.section .text
.global arch_context_switch
.type arch_context_switch, @function
arch_context_switch:
    mov eax, [esp + 4]      /* old_sp_slot */
    mov edx, [esp + 8]      /* new_sp      */

    push ebp
    push ebx
    push esi
    push edi
    pushfd

    mov [eax], esp          /* eski gorevin ESP'sini sakla */
    mov esp, edx            /* yeni gorevin yiginina gec   */

    popfd
    pop edi
    pop esi
    pop ebx
    pop ebp
    ret
"#
);

extern "C" {
    /// Bkz. yukaridaki `global_asm!` blogu.
    pub fn arch_context_switch(old_sp_slot: *mut u32, new_sp: u32);
}

/// Yeni bir gorevin yigininin, `arch_context_switch`'in geri yukleme
/// sirasina birebir uyan baslangic goruntusunu olusturur.
///
/// Bellekte yuksekten dusuge: [entry] [EBP] [EBX] [ESI] [EDI] [EFLAGS]
/// Donen deger EFLAGS slotunu (yani gorevin baslangic ESP'sini) gosterir;
/// `popfd / pop edi / pop esi / pop ebx / pop ebp / ret` zinciri tam olarak
/// `entry`'ye dallanir.
///
/// # Safety
/// `stack_top` gecerli, en az 6 kelime yer birakan, 4 bayt hizali bir
/// cekirdek yigini tepesi olmalidir.
pub unsafe fn bootstrap_stack(stack_top: *mut u32, entry: extern "C" fn() -> !) -> u32 {
    let mut sp = stack_top;

    // EFLAGS baslangici: IF=1 (bit 9) + her zaman 1 olan bit 1.
    const INITIAL_EFLAGS: u32 = 0x0000_0202;

    sp = sp.offset(-1);
    sp.write(entry as usize as u32); // `ret` hedefi
    sp = sp.offset(-1);
    sp.write(0); // EBP
    sp = sp.offset(-1);
    sp.write(0); // EBX
    sp = sp.offset(-1);
    sp.write(0); // ESI
    sp = sp.offset(-1);
    sp.write(0); // EDI
    sp = sp.offset(-1);
    sp.write(INITIAL_EFLAGS);

    sp as u32
}
