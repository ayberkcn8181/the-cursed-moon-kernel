//! Windows istisna yapilari -- SEH ve VEH icin kullanici tarafi.
//!
//! Cekirdek bir istisnayi dagitirken yigina iki kayit yazar
//! (`EXCEPTION_RECORD` ve `CONTEXT`) ve isleyiciye onlarin adresini
//! verir. Bu modul o kayitlari **Windows'un yerlesimiyle** okuyup yazar:
//! ofsetler uydurma degil, gercek Win32 basliklarindan gelir. Bu yuzden
//! buradaki kod, gercek bir Windows derleyicisinin urettigi kodla ayni
//! baytlara dokunur.
//!
//! ## Isleyici neye karar verir
//!
//! Bir isleyicinin uc secenegi vardir:
//!
//!   * **Sirakine gec** -- "bu benim istisnam degil".
//!   * **Devam et** -- CONTEXT'i duzeltip yurutmeyi surdur.
//!   * (Windows'ta ayrica geri sarma vardir; TCMK henuz yapmiyor.)
//!
//! "Devam et" derken CONTEXT'i degistirmek sarttir: aksi halde ayni
//! komut yeniden calisir ve ayni hatayi verir. Tipik duzeltmeler:
//! hatali bir isaretci tutan registeri gecerli bir adrese cevirmek, ya
//! da `Eip`/`Rip`i kurtarma koduna tasimak.
//!
//! ## Iki mekanizma, iki sayi
//!
//! Vektorlu isleyiciler `-1`/`0` doner, zincir isleyicileri `0`/`1`.
//! Ayni anlam, farkli sayilar -- Windows'un kendi tuhafligi. Karistirmayi
//! zorlastirmak icin ikisi ayri sabit kumesi olarak duruyor.

use core::ffi::c_void;

// --- Vektorlu isleyici (VEH) donus degerleri --------------------------
/// Yurutme, isleyicinin duzenledigi CONTEXT'ten devam etsin.
pub const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;
/// Bu isleyici sahiplenmiyor; sira sonrakine gecsin.
pub const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

// --- Zincir isleyicisi (SEH) donus degerleri --------------------------
/// Zincir karsiligi: yurutme devam etsin.
pub const EXCEPTION_CONTINUE_EXECUTION_SEH: i32 = 0;
/// Zincir karsiligi: sirakine gec.
pub const EXCEPTION_CONTINUE_SEARCH_SEH: i32 = 1;

// --- Sik gorulen istisna kodlari --------------------------------------
pub const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const STATUS_ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
pub const STATUS_INTEGER_DIVIDE_BY_ZERO: u32 = 0xC000_0094;
pub const STATUS_INTEGER_OVERFLOW: u32 = 0xC000_0095;
pub const STATUS_PRIVILEGED_INSTRUCTION: u32 = 0xC000_0096;

/// `EXCEPTION_NONCONTINUABLE`: isleyici "devam et" diyemez.
pub const EXCEPTION_NONCONTINUABLE: u32 = 0x1;

/// VEH isleyicisinin aldigi tek arguman.
#[repr(C)]
pub struct ExceptionPointers {
    pub exception_record: *mut ExceptionRecord,
    pub context_record: *mut c_void,
}

/// `EXCEPTION_RECORD` -- ne oldugunu anlatan kayit.
///
/// Alan sirasi Windows ile birebir. `information` dizisi istisnaya gore
/// doldurulur; erisim ihlalinde `[0]` erisim turu (0 okuma, 1 yazma),
/// `[1]` hedef adrestir.
#[repr(C)]
pub struct ExceptionRecord {
    pub code: u32,
    pub flags: u32,
    pub nested: *mut ExceptionRecord,
    pub address: *mut c_void,
    pub number_parameters: u32,
    pub information: [usize; 15],
}

/// Zincir kaydi (`EXCEPTION_REGISTRATION_RECORD`).
///
/// **Yiginda** durur ve `fs:[0]` en son kurulani gosterir. Derleyicinin
/// `__try` icin urettigi sey tam olarak budur; burada elle kuruyoruz
/// cunku Rust'ta `__try` yok.
#[cfg(target_arch = "x86")]
#[repr(C)]
pub struct Registration {
    /// Zincirde bir onceki kayit; sonu `-1`dir, `0` degil.
    pub next: usize,
    pub handler: usize,
}

/// Zincir isleyicisinin imzasi.
#[cfg(target_arch = "x86")]
pub type ChainHandler = unsafe extern "C" fn(
    record: *mut ExceptionRecord,
    establisher: *mut c_void,
    context: *mut c_void,
    dispatcher: *mut c_void,
) -> i32;

/// Bir zincir kaydini `fs:[0]`a takar ve dustugunde geri cikarir.
///
/// RAII kullanmanin sebebi Windows'un kuralinin katiligi: zincir
/// **yiginda** durur, yani kaydi kuran fonksiyon donerken onu
/// cikarmazsa `fs:[0]` artik var olmayan bir yigin cercevesini gosterir
/// ve bir sonraki istisna cop veriye dallanir.
#[cfg(target_arch = "x86")]
pub struct ChainGuard {
    record: Registration,
    installed: bool,
}

#[cfg(target_arch = "x86")]
impl ChainGuard {
    /// Kaydi olusturur; `install` cagrilana kadar zincire girmez.
    pub fn new(handler: ChainHandler) -> Self {
        ChainGuard {
            record: Registration {
                next: 0,
                handler: handler as usize,
            },
            installed: false,
        }
    }

    /// Kaydi zincirin **basina** takar.
    ///
    /// # Safety
    /// `self` yigin uzerinde durmali ve `drop` edilene kadar
    /// tasinmamalidir: `fs:[0]`a yazilan adres bu nesnenin adresidir.
    pub unsafe fn install(&mut self) {
        self.record.next = crate::teb::exception_list();
        crate::teb::set_exception_list(&self.record as *const Registration as usize);
        self.installed = true;
    }
}

#[cfg(target_arch = "x86")]
impl Drop for ChainGuard {
    fn drop(&mut self) {
        if self.installed {
            unsafe { crate::teb::set_exception_list(self.record.next) };
        }
    }
}

// --- CONTEXT erisimi ---------------------------------------------------
//
// CONTEXT 716 (x86) / 1232 (x64) bayttir ve cogu alani bizi ilgilendirmez.
// Tam yapiyi tanimlamak yerine **ofsetlerle** okuyup yaziyoruz; okunan
// sayilar Windows ABI'sinin parcasidir, degistirilemez.

#[cfg(target_arch = "x86")]
mod offsets {
    pub const EDI: usize = 0x9C;
    pub const ESI: usize = 0xA0;
    pub const EBX: usize = 0xA4;
    pub const EDX: usize = 0xA8;
    pub const ECX: usize = 0xAC;
    pub const EAX: usize = 0xB0;
    pub const EBP: usize = 0xB4;
    pub const IP: usize = 0xB8;
    pub const SP: usize = 0xC4;
}

#[cfg(target_arch = "x86_64")]
mod offsets {
    pub const RAX: usize = 0x78;
    pub const RCX: usize = 0x80;
    pub const RDX: usize = 0x88;
    pub const RBX: usize = 0x90;
    pub const SP: usize = 0x98;
    pub const RBP: usize = 0xA0;
    pub const RSI: usize = 0xA8;
    pub const RDI: usize = 0xB0;
    pub const IP: usize = 0xF8;
}

/// CONTEXT icindeki bir register.
///
/// Adlar mimariye gore degisir ama anlamlari degismez; test kodu
/// `Reg::Arg` gibi soyut bir ad yerine gercek register adini kullansin
/// diye ayri ayri veriliyor.
#[derive(Clone, Copy)]
pub enum Reg {
    /// Komut isaretcisi (`Eip` / `Rip`).
    Ip,
    /// Yigin isaretcisi (`Esp` / `Rsp`).
    Sp,
    /// `Eax` / `Rax`.
    A,
    /// `Ecx` / `Rcx`.
    C,
    /// `Edx` / `Rdx`.
    D,
    /// `Ebx` / `Rbx`.
    B,
}

fn offset_of(reg: Reg) -> usize {
    #[cfg(target_arch = "x86")]
    match reg {
        Reg::Ip => offsets::IP,
        Reg::Sp => offsets::SP,
        Reg::A => offsets::EAX,
        Reg::C => offsets::ECX,
        Reg::D => offsets::EDX,
        Reg::B => offsets::EBX,
    }
    #[cfg(target_arch = "x86_64")]
    match reg {
        Reg::Ip => offsets::IP,
        Reg::Sp => offsets::SP,
        Reg::A => offsets::RAX,
        Reg::C => offsets::RCX,
        Reg::D => offsets::RDX,
        Reg::B => offsets::RBX,
    }
}

/// CONTEXT'ten bir register okur.
///
/// # Safety
/// `context`, cekirdegin verdigi gecerli bir CONTEXT isaretcisi olmalidir.
pub unsafe fn get_reg(context: *mut c_void, reg: Reg) -> usize {
    let at = (context as usize) + offset_of(reg);
    #[cfg(target_arch = "x86")]
    {
        (at as *const u32).read_unaligned() as usize
    }
    #[cfg(target_arch = "x86_64")]
    {
        (at as *const u64).read_unaligned() as usize
    }
}

/// CONTEXT'e bir register yazar -- "devam et" demenin tek anlamli yolu.
///
/// # Safety
/// `get_reg` ile ayni kosul.
pub unsafe fn set_reg(context: *mut c_void, reg: Reg, value: usize) {
    let at = (context as usize) + offset_of(reg);
    #[cfg(target_arch = "x86")]
    {
        (at as *mut u32).write_unaligned(value as u32);
    }
    #[cfg(target_arch = "x86_64")]
    {
        (at as *mut u64).write_unaligned(value as u64);
    }
}

// Kullanilmayan ofsetler mimariye gore degisiyor; ikisini de tutmak
// istedigimiz icin uyariyi susturuyoruz.
#[cfg(target_arch = "x86")]
#[allow(dead_code)]
const _UNUSED: [usize; 3] = [offsets::EDI, offsets::ESI, offsets::EBP];
#[cfg(target_arch = "x86_64")]
#[allow(dead_code)]
const _UNUSED: [usize; 3] = [offsets::RDI, offsets::RSI, offsets::RBP];
