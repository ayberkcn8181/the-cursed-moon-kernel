//! Is Parcacigi Ortam Blogu (TEB) -- Windows'un surece bakan yuzu.
//!
//! Bir Windows ikilisi cok sey icin cekirdege **hic sormaz**: kendi
//! kimligini, yigin sinirlarini, hatta son hata kodunu bir bellek
//! yapisindan okur. O yapinin adresi bir segment tabaninda durur:
//!
//! ```text
//!   i386     fs:[0x18] -> TEB'in kendi adresi (NtTib.Self)
//!   x86_64   gs:[0x30] -> ayni alan, 64-bit yerlesimde
//! ```
//!
//! Yani `GetLastError` gercek Windows'ta bir sistem cagrisi **degildir**:
//! `return NtCurrentTeb()->LastErrorValue;` satirindan ibarettir. Ayni
//! sekilde SEH zinciri (`fs:[0]`) ve yigin koruyucusu da buradan okur.
//!
//! TEB olmadan bu kod yollarinin hepsi sifir adresten okur ve surec
//! coker. TCMK'de FS/GS tabanlari gorev basina tasinabildigi icin
//! (bkz. `level0a::core::tls`) artik gercek bir blok kurulabiliyor.
//!
//! ## POSIX ile ayrim
//!
//! POSIX tarafinda bunun karsiligi **yok**. Orada TLS blogunun icerigini
//! tamamen **program** belirler (glibc kendi `struct pthread`ini koyar);
//! cekirdek yalnizca tabani tutar. Windows'ta ise yerlesim
//! **cekirdegin sozlesmesidir**: alanlarin ofsetleri derlenmis kodun
//! icine gomuludur.
//!
//! ## TCMK'nin sadelestirmesi
//!
//! Gercek TEB kilobaytlarca; burada yalnizca **doldurulan** alanlar var
//! ve blok yigin bolgesinin tepesinden ayriliyor (ayri bir tahsis yok).
//! Doldurulmayan alanlar sifir kalir -- ki Windows'ta da coguna sifir
//! gecerli bir baslangictir.

use crate::level0a::core::scheduler;

/// Yigin bolgesinin tepesinden TEB icin ayrilan yer.
///
/// Tam sayfa degil: TCMK'nin doldurdugu alanlar `0x68`e kadar, PEB de
/// ayni blogun icinde. Kullanici yigini 8 KiB oldugu icin bir sayfa
/// ayirmak yiginin yarisini yemek olurdu.
pub const RESERVE: usize = 512;

/// TEB icindeki PEB'in yeri.
const PEB_OFFSET: usize = 0x180;

/// Istisna dagitim tramplenin yeri (ayrilmis alanin bos ortasi).
///
/// Windows'ta bunun karsiligi `ntdll!KiUserExceptionDispatcher`dir --
/// yani DLL icinde duran gercek bir fonksiyon. TCMK'de ntdll yok, o
/// yuzden cekirdek bu birkac bayti dogrudan surecin adres uzayina yazar.
/// Linux'un eski sinyal tramplenleri de aynen boyleydi.
const TRAMPOLINE_OFFSET: usize = 0x100;

// --- Alan ofsetleri (Windows ile ayni) --------------------------------
//
// Bu sayilar derlenmis Windows kodunun icine gomuludur: `mov eax,
// fs:[0x18]` gibi. Degistirmek ikili uyumu bozar.
#[cfg(target_arch = "x86")]
mod layout {
    pub const EXCEPTION_LIST: usize = 0x00;
    pub const STACK_BASE: usize = 0x04;
    pub const STACK_LIMIT: usize = 0x08;
    pub const SELF: usize = 0x18;
    pub const UNIQUE_PROCESS: usize = 0x20;
    pub const UNIQUE_THREAD: usize = 0x24;
    pub const PEB: usize = 0x30;
    pub const LAST_ERROR: usize = 0x34;
}

#[cfg(target_arch = "x86_64")]
mod layout {
    pub const EXCEPTION_LIST: usize = 0x00;
    pub const STACK_BASE: usize = 0x08;
    pub const STACK_LIMIT: usize = 0x10;
    pub const SELF: usize = 0x30;
    pub const UNIQUE_PROCESS: usize = 0x40;
    pub const UNIQUE_THREAD: usize = 0x48;
    pub const PEB: usize = 0x60;
    pub const LAST_ERROR: usize = 0x68;
}

/// Calisan gorevin TEB adresi -- yoksa sifir.
static TEB_ADDRESS: [core::sync::atomic::AtomicUsize; scheduler::MAX_TASKS] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Bir goreve TEB kurar ve segment tabanini ona yoneltir.
///
/// `top` yigin bolgesinin en ust adresi, `bottom` en alt. TEB o bolgenin
/// tepesinden ayrilir ve **kullanilabilir yigin tepesi** olarak TEB'in
/// kendi adresi doner -- cagiran argumanlari onun altina yerlestirmeli.
///
/// # Safety
/// `top`/`bottom` gecerli, Ring 3'e acik ve eslenmis olmalidir.
pub unsafe fn install(task: usize, top: usize, bottom: usize) -> usize {
    let teb = top - RESERVE;
    let peb = teb + PEB_OFFSET;
    let word = core::mem::size_of::<usize>();

    core::ptr::write_bytes(teb as *mut u8, 0, RESERVE);

    let put = |offset: usize, value: usize| {
        ((teb + offset) as *mut usize).write_unaligned(value);
    };

    // Windows'ta SEH zincirinin sonu -1'dir, 0 degil: sifir "gecerli bir
    // kayit" gibi gorunur ve zinciri yuruyen kod oraya dallanir.
    put(layout::EXCEPTION_LIST, usize::MAX);
    put(layout::STACK_BASE, teb);
    put(layout::STACK_LIMIT, bottom);
    // `Self`: TEB kendi adresini tasir. Bir Windows programi TEB'e
    // erisirken once bunu okur, cunku segment tabanini dogrudan
    // ogrenmenin baska yolu yoktur.
    put(layout::SELF, teb);
    put(layout::UNIQUE_PROCESS, task);
    put(layout::UNIQUE_THREAD, task);
    put(layout::PEB, peb);
    ((teb + layout::LAST_ERROR) as *mut u32).write_unaligned(0);

    // PEB: su an yalnizca varligi onemli (`BeingDebugged` = 0). Isaretci
    // gecerli bir adres gostermeli, yoksa onu okuyan kod sifir adrese
    // gider.
    let _ = word;

    emit_trampoline(teb + TRAMPOLINE_OFFSET);

    TEB_ADDRESS[task % scheduler::MAX_TASKS]
        .store(teb, core::sync::atomic::Ordering::Relaxed);

    // Register secimi mimariye gore **ters**: Windows i386'da FS,
    // x86_64'te GS kullanir. (Linux tam tersini yapar; bkz. `core::tls`.)
    #[cfg(target_arch = "x86")]
    crate::level0a::core::tls::set_fs(task, teb);
    #[cfg(target_arch = "x86_64")]
    crate::level0a::core::tls::set_gs(task, teb);

    teb
}

/// Gorevin TEB'i yok sayilir (ELF surecleri, ya da yeni imaj).
pub fn clear(task: usize) {
    if task < scheduler::MAX_TASKS {
        TEB_ADDRESS[task].store(0, core::sync::atomic::Ordering::Relaxed);
    }
}

/// Gorevin TEB adresi -- yoksa sifir. Sifir olmasi "bu bir PE degil"
/// demektir; istisna dagiticisi bunu boyle okur.
pub fn address(task: usize) -> usize {
    if task >= scheduler::MAX_TASKS {
        return 0;
    }
    TEB_ADDRESS[task].load(core::sync::atomic::Ordering::Relaxed)
}

/// Istisna dagitim tramplenin adresi -- TEB yoksa sifir.
pub fn trampoline(task: usize) -> usize {
    let teb = address(task);
    if teb == 0 {
        0
    } else {
        teb + TRAMPOLINE_OFFSET
    }
}

/// Tramplen kodu: isleyicinin karari (EAX/RAX) ile `NtContinueDispatch`
/// cagirir.
///
/// SEH ve VEH isleyicileri kararlarini siradan bir donus degeri olarak
/// verir. Bir isleyici `ret` ettiginde buraya duser, tramplen de o
/// degeri cekirdege tasir. Buradan **geri donus yoktur**: cekirdek ya
/// yurutmeyi kaldigi yerden surdurur ya da siradaki isleyiciye gecer,
/// ikisinde de baglami kendisi kurar.
///
/// # Safety
/// `at` en az 16 bayt yazilabilir ve Ring 3'te **yurutulebilir**
/// olmalidir. TEB yigin bolgesinde durdugu icin bu saglanir.
#[cfg(target_arch = "x86")]
unsafe fn emit_trampoline(at: usize) {
    // 50              push eax           ; karar -> arg1
    // 8D 54 24 00     lea edx, [esp]     ; &arg1  (NT cagri sozlesmesi)
    // B8 xx xx xx xx  mov eax, servis
    // CD 2E           int 0x2E
    // CC              int3               ; buraya asla donulmez
    let code = at as *mut u8;
    code.write(0x50);
    code.add(1).write(0x8D);
    code.add(2).write(0x54);
    code.add(3).write(0x24);
    code.add(4).write(0x00);
    code.add(5).write(0xB8);
    (code.add(6) as *mut u32)
        .write_unaligned(super::nt_syscalls::NT_CONTINUE_DISPATCH);
    code.add(10).write(0xCD);
    code.add(11).write(0x2E);
    code.add(12).write(0xCC);
}

/// x86_64 tramplen. Fark, NT cagri sozlesmesinin argumanlari **golge
/// alandan** okumasi (bkz. `dll::emit_thunk`): karar oraya dokulur.
///
/// # Safety
/// `emit_trampoline` (i386) ile ayni kosul.
#[cfg(target_arch = "x86_64")]
unsafe fn emit_trampoline(at: usize) {
    // 48 89 44 24 08  mov [rsp+8], rax   ; karar -> golge alan
    // 48 8D 54 24 08  lea rdx, [rsp+8]   ; &arg1
    // B8 xx xx xx xx  mov eax, servis
    // CD 2E           int 0x2E
    // CC              int3
    const CODE: [u8; 18] = [
        0x48, 0x89, 0x44, 0x24, 0x08,
        0x48, 0x8D, 0x54, 0x24, 0x08,
        0xB8, 0x00, 0x00, 0x00, 0x00,
        0xCD, 0x2E,
        0xCC,
    ];
    let code = at as *mut u8;
    core::ptr::copy_nonoverlapping(CODE.as_ptr(), code, CODE.len());
    (code.add(11) as *mut u32)
        .write_unaligned(super::nt_syscalls::NT_CONTINUE_DISPATCH);
}

/// Son hata kodunu TEB'e de yazar.
///
/// Gercek Windows'ta `GetLastError` **yalnizca** buradan okur; cekirdege
/// hic gitmez. TCMK'de cagri da var (0x3018) ama iki yerin ayrilmasi,
/// TEB'i okuyan derlenmis bir kodun yanlis deger gormesi demek olurdu.
pub fn store_last_error(task: usize, code: u32) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    let teb = TEB_ADDRESS[task].load(core::sync::atomic::Ordering::Relaxed);
    if teb == 0 {
        return;
    }
    // Yalnizca **calisan** gorevin adres uzayi eslenmis durumda; baska
    // bir gorevin TEB'ine yazmak yanlis sayfaya dokunmak olurdu.
    if scheduler::current_id() != task {
        return;
    }
    unsafe { ((teb + layout::LAST_ERROR) as *mut u32).write_unaligned(code) };
}
