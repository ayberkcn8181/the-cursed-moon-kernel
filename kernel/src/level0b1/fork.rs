//! `fork` -- surecin kendini ikiye ayirmasi (doc S.7 Faz 8+).
//!
//! ## Neden bu kadar ozel
//!
//! Diger butun syscall'lar "bir sey yap, don" kalibindadir. `fork` tek
//! bir cagriyla **iki kez doner**: ebeveynde cocugun kimligiyle, cocukta
//! sifirla. Bunun icin cagiran anin Ring 3 baglami eksiksiz yakalanip
//! ikinci bir gorevde canlandirilmalidir.
//!
//! ## Nasil
//!
//! 1. Ebeveynin adres uzayi **icerigiyle** kopyalanir
//!    (`mmu::clone_user_space`).
//! 2. Syscall cercevesinden ebeveynin tam Ring 3 baglami okunur
//!    (`SyscallFrame::user_context`) ve donus degeri sifirlanir -- cocugun
//!    `fork`'tan gorecegi donus degeri budur.
//! 3. Bu baglam cocuk gorevin yuvasina yazilir; gorev `child_task` ile
//!    baslar ve dogrudan Ring 3'e, ebeveynin durdugu komuta doner.
//! 4. Ebeveyn cagriden cocugun gorev kimligiyle doner.
//!
//! Cocuk kendi cekirdek yiginini alir; ebeveynin yiginini paylasmasi
//! (ikisi ayni anda kernel'e girdiginde) aninda cokme demek olurdu.
//!
//! ## Bilerek yapilmayanlar
//!
//! * **Copy-on-write yok.** Sayfalar hemen kopyalanir. COW, sayfalari
//!   salt okunur isaretleyip page fault'ta ayirmayi gerektirir; dogruluk
//!   acisindan fark yoktur, yalnizca maliyet farkidir.
//! * (Dosya tanimlayicilari ve program break **kopyalanir** -- bkz.
//!   `core::fd::clone_into` ve `kernel_api::clone_program_break`.)
//! * **Pencere miras alinmaz.** Pencereler gorev kimligine bagli
//!   (`wm::close_owned_by`); cocuk kendi penceresini acmalidir.

use crate::arch::cpu::regs::{SyscallFrame, UserContext};
use crate::arch::cpu::usermode;
use crate::level0a::core::{mmu, scheduler};

/// Cocuk gorevin Ring 3 baglami. Gorev girisi `extern "C" fn()` oldugu
/// icin arguman gecirilemez; baglam bu tablo uzerinden aktarilir
/// (`launcher`'in `EXEC_PATH` ile ayni kalip).
static mut CHILD_CONTEXT: [UserContext; scheduler::MAX_TASKS] =
    [UserContext::ZERO; scheduler::MAX_TASKS];
static mut CHILD_PENDING: [bool; scheduler::MAX_TASKS] = [false; scheduler::MAX_TASKS];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkError {
    /// Cagiran Ring 3'te degil -- `fork` yalnizca bir surecten anlamlidir.
    NotUserProcess,
    /// Gorev tablosu ya da cerceve havuzu doldu.
    OutOfResources,
}

/// Cagiran sureci ikiye ayirir; ebeveyne cocugun gorev kimligini doner.
///
/// # Safety
/// Yalnizca Ring 3'ten gelen bir syscall isleyicisinden, gecerli bir
/// `frame` ile cagrilmalidir.
pub unsafe fn fork(frame: &SyscallFrame) -> Result<usize, ForkError> {
    let parent = scheduler::current_id();
    let parent_space = scheduler::address_space_of(parent);
    if parent_space == 0 || !usermode::in_user_mode() {
        return Err(ForkError::NotUserProcess);
    }

    // 1. Adres uzayini icerigiyle kopyala.
    let child_space = mmu::clone_user_space(parent_space).ok_or(ForkError::OutOfResources)?;

    // 2. Cocugun gorevini olustur. Basarisiz olursa yeni uzay sizmamali.
    let child = match scheduler::spawn("fork", child_task) {
        Some(id) => id,
        None => {
            mmu::destroy_user_space(child_space);
            return Err(ForkError::OutOfResources);
        }
    };

    // 3. Ebeveynin baglamini al, donus degerini sifirla: cocuk `fork`'tan
    //    0 ile doner -- POSIX'in "hangisiyim?" sorusuna verdigi cevap.
    let mut context = frame.user_context();
    context.set_return(0);

    let slot = core::ptr::addr_of_mut!(CHILD_CONTEXT) as *mut UserContext;
    slot.add(child).write(context);
    (core::ptr::addr_of_mut!(CHILD_PENDING) as *mut bool)
        .add(child)
        .write(true);

    // Tanimlayicilar POSIX'in dedigi gibi **kopyalanir**: cocuk ayni
    // nesnelere bakan kendi tablosunu alir.
    crate::level0a::core::fd::clone_into(child);

    // Program break de kopyalanir: adres uzayi kopyalandigi icin degerler
    // oldugu gibi gecerlidir.
    crate::level0a::kernel_api::clone_program_break(child);

    scheduler::set_address_space(child, child_space);

    crate::println!(
        "[LEVEL-0b1] fork: gorev #{} -> #{} (eip=0x{:08x}, {} sayfa kopyalandi)",
        parent,
        child,
        context.instruction_pointer(),
        mmu::user_pages(child_space)
    );

    Ok(child)
}

/// Cocuk gorevin giris noktasi: dogrudan Ring 3'e, ebeveynin durdugu
/// komuta doner.
extern "C" fn child_task() -> ! {
    let id = scheduler::current_id();

    let context = unsafe {
        let pending = (core::ptr::addr_of_mut!(CHILD_PENDING) as *mut bool).add(id);
        if !pending.read() {
            crate::println!("[LEVEL-0b1] fork: gorev #{} icin baglam yok.", id);
            scheduler::terminate_current();
        }
        pending.write(false);
        (core::ptr::addr_of!(CHILD_CONTEXT) as *const UserContext)
            .add(id)
            .read()
    };

    // Cekirdek yiginini ayrica kurmaya gerek yok: `scheduler::spawn` her
    // goreve Ring 3 gecisleri icin AYRI bir yigin ayirir ve baglam
    // degisimi TSS.esp0'i o goreve gecerken zaten yukler. Ebeveynin
    // yiginini paylasmak, ikisi ayni anda syscall yaptiginda birbirinin
    // cercevesini ezmek demek olurdu.
    unsafe { usermode::resume_user_context(&context) };

    // Cocuk `sys_exit` cagirdi: penceresi varsa kapansin, adres uzayi
    // birakilsin -- gorev sonlanirken scheduler bunu zaten yapiyor.
    crate::level0a::wm::close_owned_by(id);
    scheduler::terminate_current()
}
