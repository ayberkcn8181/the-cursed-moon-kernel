//! Is parcaciklari: `clone` ve `CreateThread`.
//!
//! Buraya kadar TCMK'de **bir gorev = bir surec = bir akis** idi ve bu,
//! README'de acikca yazili bir sadelestirmeydi:
//! `GetCurrentThreadId()` ile `GetCurrentProcessId()` ayni sayiyi
//! donduruyordu. Artik donmuyor.
//!
//! ## `fork` kopyalar, is parcacigi paylasir
//!
//! Iki yol da yeni bir gorev yaratir; ayrisan **neyin ortak oldugu**:
//!
//! | | `fork` | is parcacigi |
//! |---|---|---|
//! | adres uzayi | kopyalanir (copy-on-write) | **paylasilir** |
//! | tanimlayicilar | kopyalanir | **paylasilir** |
//! | calisma dizini | kopyalanir | **paylasilir** |
//! | ortam | kopyalanir | **paylasilir** |
//! | yigin | ayni adresler, ayri cerceveler | **ayri bolge** |
//! | TLS / TEB | devralinir | **ayri** |
//!
//! Paylasim, gorevin `group` alaniyla saglaniyor (bkz.
//! `scheduler::group_of`): tanimlayici tablosu, dizin ve ortam grup
//! numarasiyla indeksleniyor. Gercek `clone` bayraklarinin
//! (`CLONE_VM`, `CLONE_FILES`, `CLONE_FS`) anlami tam olarak budur.
//!
//! Son satir onemli: is parcacigi tabanlari **paylasilmaz**. Her akisin
//! kendi TEB'i olmak zorunda, cunku son hata kodu ve SEH zinciri orada
//! duruyor -- paylasilsalardi iki akis birbirinin istisnasini gorurdu.
//!
//! ## Iki ABI, ayni mekanizma
//!
//! ```text
//!   POSIX   clone(flags, cocuk_yigini, ...)  -> tid
//!             cocuk, verilen yiginin tepesinden devam eder
//!
//!   Win32   CreateThread(.., baslangic, parametre, ..) -> HANDLE
//!             cocuk, verilen fonksiyondan baslar
//! ```
//!
//! Fark, cagiranin ne kadarini kendi yaptigi. POSIX'te yigini
//! **cagiran** ayirir (genelde `mmap` ile) ve cekirdek yalnizca yigin
//! isaretcisini kurar; Windows'ta yigini **cekirdek** ayirir. TCMK
//! ikisini de destekliyor: yigin verilmezse ayirir, verilirse kullanir.
//!
//! ## Donus
//!
//! Bir is parcacigi fonksiyonu **dondugunde** akis bitmeli. Ring 3'te
//! "donulecek yer" olmadigi icin cekirdek yiginin tepesine kucuk bir
//! tramplen yazar ve donus adresi olarak onu verir; tramplen cikis
//! cagrisini yapar. Ayni desen istisna dagitiminda da kullaniliyor
//! (bkz. `seh.rs`).

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::arch::cpu::regs::UserContext;
use crate::arch::cpu::usermode;
use crate::level0a::core::{mmu, scheduler};

/// Yeni is parcacigi icin ayrilan yigin (cagiran vermediyse).
const THREAD_STACK_SIZE: usize = 8 * 1024;

/// Yiginin tepesinden donus trampleni icin ayrilan yer.
const TRAMPOLINE_RESERVE: usize = 32;

/// Henuz **kosmamis** akisin tam Ring 3 baglami -- gorev basina bir yuva.
///
/// Daha once burada dort ayri tablo (`START`, `PARAM`, `STACK`,
/// `WINDOWS`) duruyordu ve baglam yeni gorevin kendi icinde, Ring 3'e
/// gecmeden hemen once hesaplaniyordu. Sira bu yuzden degisti:
/// `CREATE_SUSPENDED` ile dogan bir akisin baglami, o akis bir komut
/// bile yurutmeden **okunabilir ve yazilabilir** olmali
/// (`GetThreadContext`/`SetThreadContext`). Gec hesaplanan bir sey
/// okunamaz; o yuzden artik `create` kuruyor, giris noktasi yalnizca
/// yukluyor.
///
/// Erken kurmanin bedava olmadigi tek yer TLS: `teb::install` segment
/// tabanini **hemen** etkinlestiriyor, yani cagiranin kendi tabanini geri
/// koymak gerekiyor (bkz. `create`).
static mut ENTRY: [UserContext; scheduler::MAX_TASKS] =
    [UserContext::ZERO; scheduler::MAX_TASKS];

/// `ENTRY[id]` yuvasinin durumu.
///
/// Uc degerin **ucu de** ayri bir cevap: "boyle bir akis yok", "var ve
/// hic kosmadi", "kostu". Ucuncusunu ikinciden ayirmak sart, cunku
/// kosmus bir akisin baglami artik bu yuvada durmuyor -- cekirdek onu
/// okuyup Ring 3'e vermis. O durumda dogru cevap eski baglami
/// gostermek degil, **reddetmek**.
const CONTEXT_NONE: usize = 0;
const CONTEXT_PENDING: usize = 1;
const CONTEXT_STARTED: usize = 2;

static CONTEXT_STATE: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(CONTEXT_NONE) }; scheduler::MAX_TASKS];

/// Kac is parcacigi yaratildi (kabuk raporu).
static CREATED: AtomicUsize = AtomicUsize::new(0);

pub fn created() -> usize {
    CREATED.load(Ordering::Relaxed)
}

/// Bir gorev yuvasi yeniden kullanilirken baglam kaydi silinir.
///
/// `nt_syscalls::forget_suspend` ile ayni gerekce ve ayni cagri yeri
/// (`scheduler::spawn_inner`): yuva numarasi yeniden kullanildiginda
/// onceki kiracinin baglami yeni gorevin baglami gibi gorunurdu.
pub fn forget_context(task: usize) {
    if task < scheduler::MAX_TASKS {
        CONTEXT_STATE[task].store(CONTEXT_NONE, Ordering::Relaxed);
    }
}

/// Henuz kosmamis bir akisin baglami -- yoksa `None`.
///
/// `None` iki ayri anlami birlestiriyor ("boyle bir akis yok" ve "akis
/// kostu") ama cagiran icin ikisinin sonucu ayni: gosterilecek dogru bir
/// baglam yok.
pub fn entry_context(task: usize) -> Option<UserContext> {
    if task >= scheduler::MAX_TASKS
        || CONTEXT_STATE[task].load(Ordering::SeqCst) != CONTEXT_PENDING
    {
        return None;
    }
    // SAFETY: yuva gorev numarasina ozel ve gorev henuz kosmuyor.
    Some(unsafe { (core::ptr::addr_of!(ENTRY) as *const UserContext).add(task).read() })
}

/// Henuz kosmamis bir akisin baglamini **degistirir**.
///
/// Doner: yazildi mi. Yazilmadiysa sebep tek: akis ya yok ya da artik
/// kosuyor.
pub fn set_entry_context(task: usize, context: &UserContext) -> bool {
    if task >= scheduler::MAX_TASKS
        || CONTEXT_STATE[task].load(Ordering::SeqCst) != CONTEXT_PENDING
    {
        return false;
    }
    // SAFETY: entry_context ile ayni kosul.
    unsafe {
        (core::ptr::addr_of_mut!(ENTRY) as *mut UserContext)
            .add(task)
            .write(*context)
    };
    true
}

/// Baglami saklar ve yuvayi "kosmadi" isaretler.
fn store_entry_context(task: usize, context: &UserContext) {
    // SAFETY: yuva gorev numarasina ozel; gorev daha kosmaya baslamadi.
    unsafe {
        (core::ptr::addr_of_mut!(ENTRY) as *mut UserContext)
            .add(task)
            .write(*context)
    };
    CONTEXT_STATE[task].store(CONTEXT_PENDING, Ordering::SeqCst);
}

/// Baglami **tuketir**: okur ve yuvayi "kostu" isaretler.
///
/// Isaretlemenin okumayla ayni yerde olmasi onemli: bu satirdan sonra
/// `SetThreadContext` artik dogru cevap veremez, o yuzden reddetmesi
/// gerekir.
fn take_entry_context(task: usize) -> Option<UserContext> {
    let context = entry_context(task)?;
    CONTEXT_STATE[task].store(CONTEXT_STARTED, Ordering::SeqCst);
    Some(context)
}

#[derive(Debug)]
pub enum ThreadError {
    /// Cagiran Ring 3'te bir surec degil.
    NotUserProcess,
    /// Gorev tablosu ya da bellek doldu.
    OutOfResources,
}

/// Yeni bir is parcacigi yaratir.
///
/// `stack` sifirsa cagiranin ayirdigi yigin kullanilir (POSIX `clone`
/// kalibi); sifirsa cekirdek `mmap` penceresinden ayirir (Win32
/// `CreateThread` kalibi).
///
/// Doner: yeni gorevin kimligi (`tid`).
///
/// # Safety
/// Yalnizca Ring 3'ten gelen bir syscall isleyicisinden cagrilmalidir.
pub unsafe fn create(
    entry: usize,
    param: usize,
    stack: usize,
    windows: bool,
) -> Result<usize, ThreadError> {
    let parent = scheduler::current_id();
    let space = scheduler::address_space_of(parent);
    if space == 0 || !usermode::in_user_mode() || entry == 0 {
        return Err(ThreadError::NotUserProcess);
    }

    // Yigin: verilmemisse paylasilan adres uzayindan ayrilir. Bolge
    // `mmap` penceresinden geliyor, yani surec onu `munmap` ile geri de
    // verebilir.
    let top = if stack != 0 {
        stack
    } else {
        let base = mmu::mmap_user(space, THREAD_STACK_SIZE).ok_or(ThreadError::OutOfResources)?;
        base + THREAD_STACK_SIZE
    };

    let id = scheduler::spawn_thread("thread", thread_task).ok_or(ThreadError::OutOfResources)?;

    // Is parcacigi tabanlari **devralinmaz**: her akisin kendi TEB'i
    // olmak zorunda (son hata kodu ve SEH zinciri orada duruyor).
    crate::level0a::core::tls::reset(id);

    // --- Ring 3 kurulumu, **burada** ---
    //
    // Butun bunlar bir zamanlar yeni gorevin kendi icinde (`thread_task`)
    // yapiliyordu ve o zaman da dogruydu: adres uzayi paylasilmis, yani
    // yazilar iki taraftan da ayni yere gidiyor. Buraya tasinmasinin
    // sebebi zamanlama: `CREATE_SUSPENDED` ile dogan bir akisin baglami
    // daha kosmadan sorulabilmeli.
    let mut top = top - TRAMPOLINE_RESERVE;
    let trampoline = top;
    unsafe { emit_exit_trampoline(trampoline, windows) };

    if windows {
        // Yigin tabani: cagiran kendi yiginini verdiyse gercek sinirini
        // bilmiyoruz, o yuzden ayni olcu varsayiliyor. TEB'deki
        // `StackLimit` yalnizca bilgi amacli -- cekirdek onu kullanmiyor.
        let bottom = top.saturating_sub(THREAD_STACK_SIZE);
        top = unsafe { crate::level0b1::nt_subsystem::teb::install(id, top, bottom) };
        // `teb::install` yeni akisin segment tabanini **hemen**
        // etkinlestiriyor (bkz. `tls::set_fs`). Cagiran hala biziz ve
        // Ring 3'e o donecek: taban geri konmazsa cagiran, cocugunun
        // TEB'ini kendi TEB'i sanarak devam eder -- son hata kodu ve SEH
        // zinciri yanlis yerden okunur.
        crate::level0a::core::tls::activate(parent);
    }

    let context = unsafe { build_entry_context(entry, top, trampoline, param) };
    store_entry_context(id, &context);

    CREATED.fetch_add(1, Ordering::Relaxed);
    crate::println!(
        "[LEVEL-0b1] is parcacigi: gorev #{} -> #{} (giris=0x{:08x}, yigin=0x{:08x})",
        parent,
        id,
        entry,
        top
    );
    Ok(id)
}

/// Yeni gorevin cekirdek tarafindaki giris noktasi.
///
/// Artik yalnizca **yukluyor**: tramplen, TEB ve baglamin kendisi
/// `create` icinde, akis dogarken kurulmus durumda. Ayrimin sebebi
/// `GetThreadContext`: gec kurulan bir baglam, kosmayi bekleyen bir akis
/// icin okunamaz olurdu.
///
/// Adres uzayi paylasilmis ve baglam degisimi CR3'u zaten yukledi;
/// burada ayrica gecmeye gerek yok (bkz. `fork::child_task`).
extern "C" fn thread_task() -> ! {
    let id = scheduler::current_id();

    let Some(context) = take_entry_context(id) else {
        crate::println!("[LEVEL-0b1] is parcacigi: gorev #{} icin baglam yok.", id);
        scheduler::terminate_current();
    };
    unsafe { usermode::resume_user_context(&context) };

    // Buraya donulduyse is parcacigi cikti.
    //
    // Once verilen soz: `clone` cagrisi bir `clear_child_tid` adresi
    // birakmissa oraya 0 yazilip bekleyenler uyandirilir. Sirasi onemli
    // -- adres uzayi yikildiktan sonra yazacak yer kalmazdi.
    unsafe { crate::level0b1::futex::clear_child_tid(id) };

    let space = scheduler::address_space_of(id);
    if space != 0 && scheduler::address_space_users(space) == 1 {
        // Grubun son gorevi: uzayi birakma isi normal cikis yoluna ait,
        // ama son kalan is parcacigiysa burada yapilir.
        scheduler::set_current_address_space(0);
        unsafe {
            mmu::switch_to(mmu::kernel_cr3());
            mmu::destroy_user_space(space);
        }
    } else {
        scheduler::set_current_address_space(space);
    }
    scheduler::terminate_current()
}

/// Is parcacigi fonksiyonunun girecegi **tam baglam**.
///
/// Yalnizca yigin isaretcisi yetmez: parametre cagri gelenegine gore
/// yiginda ya da registerda gecer. `fork`un cocugu resume ettigi yolun
/// aynisi kullaniliyor -- orada da butun registerlar yukleniyor.
unsafe fn build_entry_context(
    entry: usize,
    top: usize,
    trampoline: usize,
    param: usize,
) -> UserContext {
    let word = core::mem::size_of::<usize>();
    let mut context = UserContext::ZERO;

    #[cfg(target_arch = "x86")]
    {
        // cdecl: [esp] = donus adresi, [esp+4] = parametre.
        // Girisde `esp + 4` 16'ya bolunmeli.
        let sp = ((top - 2 * word) & !0xF) - 4;
        (sp as *mut usize).write_unaligned(trampoline);
        ((sp + word) as *mut usize).write_unaligned(param);
        context.redirect(entry, sp);
        context.eflags = 0x202;
    }
    #[cfg(target_arch = "x86_64")]
    {
        // Donus adresi yiginda, parametre registerda. Golge alan Win64'un
        // sarti; System V'de zararsiz bir bosluk, o yuzden ikisi ayni
        // cerceveyi paylasabiliyor.
        const SHADOW: usize = 32;
        let sp = ((top - word - SHADOW) & !0xF) - word;
        (sp as *mut usize).write_unaligned(trampoline);
        core::ptr::write_bytes((sp + word) as *mut u8, 0, SHADOW);
        context.redirect(entry, sp);
        context.rflags = 0x202;
        // RCX Win64'un, RDI System V'nin ilk arguman registeri. Ikisini
        // de kurmak, tek cercevenin iki dunyada da calismasini sagliyor.
        context.rcx = param as u64;
        context.rdi = param as u64;
    }
    context
}

/// Is parcacigi fonksiyonu donunce calisacak kod.
///
/// Donus degeri (EAX/RAX) cikis kodu olur. Buradan geri donus yoktur.
#[cfg(target_arch = "x86")]
unsafe fn emit_exit_trampoline(at: usize, windows: bool) {
    let code = at as *mut u8;
    if windows {
        // 50              push eax           ; cikis kodu -> arg1
        // 8D 54 24 00     lea edx, [esp]
        // B8 xx xx xx xx  mov eax, NtExitThread
        // CD 2E           int 0x2E
        code.write(0x50);
        code.add(1).write(0x8D);
        code.add(2).write(0x54);
        code.add(3).write(0x24);
        code.add(4).write(0x00);
        code.add(5).write(0xB8);
        (code.add(6) as *mut u32).write_unaligned(
            crate::level0b1::nt_subsystem::nt_syscalls::NT_EXIT_THREAD,
        );
        code.add(10).write(0xCD);
        code.add(11).write(0x2E);
        code.add(12).write(0xCC);
    } else {
        // 89 C3           mov ebx, eax       ; cikis kodu
        // B8 01 00 00 00  mov eax, 1         ; SYS_EXIT
        // CD 80           int 0x80
        code.write(0x89);
        code.add(1).write(0xC3);
        code.add(2).write(0xB8);
        (code.add(3) as *mut u32).write_unaligned(1);
        code.add(7).write(0xCD);
        code.add(8).write(0x80);
        code.add(9).write(0xCC);
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn emit_exit_trampoline(at: usize, windows: bool) {
    let code = at as *mut u8;
    if windows {
        // 48 89 44 24 08  mov [rsp+8], rax
        // 48 8D 54 24 08  lea rdx, [rsp+8]
        // B8 xx xx xx xx  mov eax, NtExitThread
        // CD 2E           int 0x2E
        const CODE: [u8; 18] = [
            0x48, 0x89, 0x44, 0x24, 0x08, 0x48, 0x8D, 0x54, 0x24, 0x08, 0xB8, 0x00, 0x00,
            0x00, 0x00, 0xCD, 0x2E, 0xCC,
        ];
        core::ptr::copy_nonoverlapping(CODE.as_ptr(), code, CODE.len());
        (code.add(11) as *mut u32).write_unaligned(
            crate::level0b1::nt_subsystem::nt_syscalls::NT_EXIT_THREAD,
        );
    } else {
        // 48 89 C7        mov rdi, rax       ; cikis kodu
        // B8 3C 00 00 00  mov eax, 60        ; SYS_EXIT
        // 0F 05           syscall
        const CODE: [u8; 11] = [
            0x48, 0x89, 0xC7, 0xB8, 0x3C, 0x00, 0x00, 0x00, 0x0F, 0x05, 0xCC,
        ];
        core::ptr::copy_nonoverlapping(CODE.as_ptr(), code, CODE.len());
    }
}
