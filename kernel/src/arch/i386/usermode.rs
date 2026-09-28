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

use crate::arch::i386::regs::UserContext;

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
    /* FS/GS is-parcacigi tanimlayicilarini gosterir (0x33 / 0x3B).
       Tabanlari sifirken 0x23 ile ayni davranirlar, yani TLS
       kullanmayan surec farki gormez (bkz. gdt::i386). */
    mov bx, 0x33
    mov fs, bx
    mov bx, 0x3B
    mov gs, bx

    push 0x23               /* SS     */
    push ecx                /* ESP    */
    push 0x202              /* EFLAGS: IF=1 */
    push 0x1B               /* CS  (0x18 | RPL 3) */
    push eax                /* EIP    */
    iretd

/* `fork` sonrasi cocugu ebeveynin durdugu tam noktadan baslatir.
 *
 * `arch_enter_user_mode`'dan farki, YALNIZCA EIP/ESP degil butun genel
 * registerlari da yuklemesidir: derleyici `int 0x80` sonrasinda
 * EBX/ESI/EDI/EBP'nin korundugunu varsayar, dolayisiyla cocuk bunlari
 * ebeveyninkiyle ayni gormezse syscall'dan sonraki ilk erisimde coker.
 *
 * Alan sirasi `regs::UserContext` ile birebir aynidir. */
.global arch_enter_user_mode_regs
.type arch_enter_user_mode_regs, @function
arch_enter_user_mode_regs:
    mov eax, [esp + 4]      /* &UserContext */
    mov edx, [esp + 8]      /* &resume_slot */

    push ebp
    push ebx
    push esi
    push edi
    pushfd
    mov [edx], esp          /* cekirdek baglamini sakla */

    /* Ring 3 veri segmentleri */
    mov bx, 0x23
    mov ds, bx
    mov es, bx
    /* FS/GS is-parcacigi tanimlayicilarini gosterir (0x33 / 0x3B).
       Tabanlari sifirken 0x23 ile ayni davranirlar, yani TLS
       kullanmayan surec farki gormez (bkz. gdt::i386). */
    mov bx, 0x33
    mov fs, bx
    mov bx, 0x3B
    mov gs, bx

    /* iret cercevesi: SS, ESP, EFLAGS, CS, EIP */
    push 0x23
    push [eax + 32]         /* esp    */
    push [eax + 36]         /* eflags */
    push 0x1B
    push [eax + 28]         /* eip    */

    /* Genel registerlar; EAX en sonda cunku taban isaretcisi odur. */
    mov edi, [eax + 0]
    mov esi, [eax + 4]
    mov ebp, [eax + 8]
    mov ebx, [eax + 12]
    mov edx, [eax + 16]
    mov ecx, [eax + 20]
    mov eax, [eax + 24]
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
    /* FS/GS is-parcacigi tanimlayicilarini gosterir (0x33 / 0x3B).
       Tabanlari sifirken 0x23 ile ayni davranirlar, yani TLS
       kullanmayan surec farki gormez (bkz. gdt::i386). */
    mov bx, 0x33
    mov fs, bx
    mov bx, 0x3B
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

    /// Tam bir Ring 3 baglamini geri yukleyerek Ring 3'e gecer (`fork`).
    fn arch_enter_user_mode_regs(context: *const UserContext, resume_slot: *mut usize);

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

/// `fork` edilmis bir cocugu ebeveynin baglamiyla Ring 3'te surdurur;
/// cocuk `sys_exit` cagirdiginda buraya doner.
///
/// # Safety
/// `context`, Ring 3'ten alinmis gecerli bir baglam olmalidir ve cagiran
/// gorevin adres uzayi o baglamin gectigi uzay olmalidir.
pub unsafe fn resume_user_context(context: &UserContext) {
    use crate::level0a::core::scheduler;

    scheduler::set_current_in_user_mode(true);
    arch_enter_user_mode_regs(context as *const UserContext, scheduler::current_resume_slot());
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
/// i386 cdecl duzeni, isleyiciye girildigi andaki yigin:
///
/// ```text
///   [esp]   = restorer   (donus adresi -- isleyici `ret` ile buraya doner)
///   [esp+4] = signo      (arg1)
/// ```
///
/// `restorer`, kullanici tarafinin verdigi kucuk bir tramplendir; tek
/// isi `sigreturn` cagirmaktir. Gercek i386 Linux'ta da bu boyledir
/// (`sigaction.sa_restorer`) -- cekirdek kullanici yiginina kod yazmak
/// zorunda kalmasin diye.
///
/// Hizalama: cagri geleneginde fonksiyon girisinde `esp+4`'un 16'ya
/// bolunmesi beklenir (`call` donus adresini ittikten sonraki durum), o
/// yuzden `esp % 16 == 12` secilir.
///
/// # Safety
/// Cagiran gorevin adres uzayi etkin olmalidir; yazilan adresler
/// dogrulanir, dogrulama basarisizsa `None` doner.
pub unsafe fn build_signal_frame(
    context: &mut UserContext,
    stack: usize,
    signo: u32,
    handler: usize,
    restorer: usize,
) -> Option<()> {
    use crate::level0a::core::mmu;

    let sp = ((stack - 8) & !0xF) - 4;
    if !mmu::is_user_or_demand(sp) || !mmu::is_user_or_demand(sp + 7) {
        return None;
    }
    (sp as *mut u32).write_unaligned(restorer as u32);
    ((sp + 4) as *mut u32).write_unaligned(signo);
    context.redirect(handler, sp);
    Some(())
}

// --- `SA_SIGINFO` cercevesi (i386) ------------------------------------
//
// Asagidaki sayilar Linux ABI'sinin parcasidir: derlenmis bir program
// `info->si_addr` ya da `uc->uc_mcontext.gregs[REG_EIP]` yazdiginda tam
// bu ofsetlere gider. Uydurulmus bir duzen, kaydi okuyan her kodu
// bozardi.

/// `siginfo_t` -- her mimaride 128 bayt.
const SIGINFO_SIZE: usize = 128;

/// `siginfo_t` alan ofsetleri (i386).
mod si {
    pub const SIGNO: usize = 0x00;
    pub const ERRNO: usize = 0x04;
    pub const CODE: usize = 0x08;
    /// Birlesimin (union) basi. `SIGSEGV`/`SIGFPE`/`SIGILL`'de
    /// `si_addr`, `kill` ile gelenlerde `si_pid`.
    pub const ADDR: usize = 0x0C;
    pub const PID: usize = 0x0C;
    pub const UID: usize = 0x10;
    /// `si_value` -- `sigqueue`in tasidigi kelime. Yalnizca `si_pid`
    /// yolunda gecerli: ayni birlesimin ucuncu alani.
    pub const VALUE: usize = 0x14;
}

/// `ucontext_t` icin ayrilan yer.
///
/// Cekirdegin doldurdugu alanlar 236 bayta kadar uzaniyor; glibc'nin
/// yapisi kayan nokta bolgesiyle birlikte 348. Buyuk olani ayirmak,
/// kaydi glibc duzeniyle okuyan bir kodun yigin disina tasmamasi icin.
/// Doldurulmayan bolge **sifirlaniyor**.
const UCONTEXT_SIZE: usize = 348;

/// `ucontext_t` alan ofsetleri (i386).
///
/// `uc_mcontext` 0x14'te bir `struct sigcontext`tir; asagidaki register
/// ofsetleri ona degil, `ucontext_t`nin **basina** goredir.
mod uc {
    pub const FLAGS: usize = 0x00;
    pub const LINK: usize = 0x04;
    pub const STACK: usize = 0x08;
    /// `uc_mcontext` burada basliyor.
    pub const MCONTEXT: usize = 0x14;

    pub const GS: usize = MCONTEXT;
    pub const FS: usize = MCONTEXT + 4;
    pub const ES: usize = MCONTEXT + 8;
    pub const DS: usize = MCONTEXT + 12;
    pub const EDI: usize = MCONTEXT + 16;
    pub const ESI: usize = MCONTEXT + 20;
    pub const EBP: usize = MCONTEXT + 24;
    pub const ESP: usize = MCONTEXT + 28;
    pub const EBX: usize = MCONTEXT + 32;
    pub const EDX: usize = MCONTEXT + 36;
    pub const ECX: usize = MCONTEXT + 40;
    pub const EAX: usize = MCONTEXT + 44;
    pub const TRAPNO: usize = MCONTEXT + 48;
    pub const ERR: usize = MCONTEXT + 52;
    pub const EIP: usize = MCONTEXT + 56;
    pub const CS: usize = MCONTEXT + 60;
    pub const EFLAGS: usize = MCONTEXT + 64;
    pub const ESP_AT_SIGNAL: usize = MCONTEXT + 68;
    pub const SS: usize = MCONTEXT + 72;
    /// `cr2` -- sayfa hatasinin adresi. `si_addr` ile ayni bilgi, ama
    /// gercek Linux ikisini de dolduruyor.
    pub const CR2: usize = MCONTEXT + 84;
    pub const SIGMASK: usize = 0x6C;
}

/// Bayraklarin **kullaniciya birakilan** bitleri (CF, PF, AF, ZF, SF,
/// OF ve yon bayragi).
///
/// Geri kalanlar cekirdegin degeriyle kaliyor: bir program bu kapidan
/// kendi IOPL'unu ya da kesme bayragini degistirememeli. Win32 yuzunde
/// de ayni kural var (bkz. `seh::read_context`).
const USER_FLAGS: u32 = 0x0000_0CD5;

/// Ring 3 yigininin ustune bir **`SA_SIGINFO` cercevesi** kurar.
///
/// Duzen, tek argumanli yuzun genisletilmis hali:
///
/// ```text
///   [esp]    = restorer   (donus adresi)
///   [esp+4]  = signo      (arg1)
///   [esp+8]  = siginfo_t* (arg2)
///   [esp+12] = ucontext_t*(arg3)
///
///   ... yukarida, yigin tepesine yakin:
///   [siginfo_t]   128 bayt
///   [ucontext_t]  348 bayt
/// ```
///
/// Kayitlarin cerceveden **yukarida** olmasi sart: isleyici kendi yerel
/// degiskenlerini `esp`nin altina koyacak ve kayitlari ezmemeli.
///
/// Doner: Ring 3'teki `ucontext_t`nin adresi -- `sigreturn` baglami
/// oradan geri okuyor (bkz. `read_ucontext`).
///
/// # Safety
/// `build_signal_frame` ile ayni kosul.
pub unsafe fn build_siginfo_frame(
    context: &mut UserContext,
    stack: usize,
    signo: u32,
    handler: usize,
    restorer: usize,
    info: &crate::level0b1::signal::SigInfo,
) -> Option<usize> {
    use crate::level0a::core::mmu;

    // Talep uzerine eslenecek sayfalar da kabul ediliyor: cekirdegin
    // oraya yazmasi hata uretir ama o hata kurtarilabilir. Kati denetim
    // burada yanlis cevap verirdi -- henuz dokunulmamis bir yigin
    // sayfasi yuzunden teslim edilemeyen bir sinyal (bkz. `seh.rs`teki
    // ayni duzeltme).
    const RECORDS: usize = SIGINFO_SIZE + UCONTEXT_SIZE;
    let sp = stack;
    if sp < RECORDS + 64 {
        return None;
    }
    let base = (sp - RECORDS) & !0xF;
    // Kayitlar iki ucundan dogrulaniyor; arada sayfa sinirlari olabilir,
    // o yuzden her sayfa ayri ayri.
    let mut probe = base;
    while probe < base + RECORDS {
        if !mmu::is_user_or_demand(probe) {
            return None;
        }
        probe += 4096;
    }
    if !mmu::is_user_or_demand(base + RECORDS - 1) {
        return None;
    }

    let siginfo_at = base;
    let ucontext_at = base + SIGINFO_SIZE;

    // cdecl: dort kelime, ve girisde `esp + 4` 16'ya bolunmeli.
    let call = ((base - 16) & !0xF) - 4;
    if !mmu::is_user_or_demand(call) || !mmu::is_user_or_demand(call + 15) {
        return None;
    }

    core::ptr::write_bytes(siginfo_at as *mut u8, 0, RECORDS);
    let put = |at: usize, value: u32| (at as *mut u32).write_unaligned(value);

    put(siginfo_at + si::SIGNO, signo);
    put(siginfo_at + si::ERRNO, 0);
    put(siginfo_at + si::CODE, info.code as u32);
    // Birlesim: hata sinyallerinde adres, `kill` ile gelenlerde kimlik.
    // Ikisi ayni ofsette durdugu icin **secmek** zorunlu.
    if info.code == crate::level0b1::signal::SI_QUEUE {
        // `sigqueue` yolu: gonderen + **deger**. Ucu de ayni birlesimin
        // ardisik alanlari, o yuzden hepsi birlikte yaziliyor.
        put(siginfo_at + si::PID, info.pid as u32);
        put(siginfo_at + si::UID, 0);
        put(siginfo_at + si::VALUE, info.value as u32);
    } else if info.addr != 0 {
        put(siginfo_at + si::ADDR, info.addr as u32);
    } else {
        put(siginfo_at + si::PID, info.pid as u32);
        put(siginfo_at + si::UID, 0);
    }

    write_ucontext(ucontext_at, context, info);

    put(call, restorer as u32);
    put(call + 4, signo);
    put(call + 8, siginfo_at as u32);
    put(call + 12, ucontext_at as u32);

    context.redirect(handler, call);
    Some(ucontext_at)
}

/// Kesilen baglami `ucontext_t`ye doker.
unsafe fn write_ucontext(
    at: usize,
    context: &UserContext,
    info: &crate::level0b1::signal::SigInfo,
) {
    let put = |offset: usize, value: u32| ((at + offset) as *mut u32).write_unaligned(value);
    put(uc::FLAGS, 0);
    put(uc::LINK, 0);
    // `uc_stack`: isleyicinin ustunde kostugu yigin. `sigaltstack`
    // kuruluysa **o** yazilir, cunku POSIX'in sordugu sey budur:
    // "bu isleyici hangi yiginda?" Kurulu degilse alanlar sifir kalir.
    {
        let (alt_sp, alt_size, alt_flags) = crate::level0b1::signal::current_alt_stack();
        put(uc::STACK, alt_sp as u32);
        ((at + uc::STACK + 4) as *mut u32).write_unaligned(alt_flags);
        put(uc::STACK + 8, alt_size as u32);
    }
    put(uc::EDI, context.edi);
    put(uc::ESI, context.esi);
    put(uc::EBP, context.ebp);
    put(uc::ESP, context.esp);
    put(uc::EBX, context.ebx);
    put(uc::EDX, context.edx);
    put(uc::ECX, context.ecx);
    put(uc::EAX, context.eax);
    put(uc::EIP, context.eip);
    put(uc::EFLAGS, context.eflags);
    put(uc::ESP_AT_SIGNAL, context.esp);
    put(uc::CR2, info.addr as u32);
    // Segment secicileri: Ring 3 degerleri (bkz. `gdt::i386`).
    put(uc::CS, 0x1B);
    put(uc::SS, 0x23);
    put(uc::DS, 0x23);
    put(uc::ES, 0x23);
    put(uc::FS, 0x33);
    put(uc::GS, 0x3B);
    put(uc::TRAPNO, 0);
    put(uc::ERR, 0);
    let _ = uc::SIGMASK;
}

/// Tersi: isleyicinin (belki degistirdigi) `ucontext_t`sini okur.
///
/// Segment secicileri ve `cr2` **alinmaz**: ikisi de cekirdegin isi.
/// Bayraklarin yalnizca durum bitleri aliniyor (bkz. `USER_FLAGS`).
///
/// # Safety
/// `at` Ring 3'e ait, okunabilir bir `ucontext_t` olmalidir.
pub unsafe fn read_ucontext(at: usize, context: &mut UserContext) {
    let get = |offset: usize| ((at + offset) as *const u32).read_unaligned();
    context.edi = get(uc::EDI);
    context.esi = get(uc::ESI);
    context.ebp = get(uc::EBP);
    context.esp = get(uc::ESP);
    context.ebx = get(uc::EBX);
    context.edx = get(uc::EDX);
    context.ecx = get(uc::ECX);
    context.eax = get(uc::EAX);
    context.eip = get(uc::EIP);
    context.eflags = (get(uc::EFLAGS) & USER_FLAGS) | 0x202;
}
