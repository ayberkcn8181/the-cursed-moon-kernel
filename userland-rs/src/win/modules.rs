//! `winmods.exe` -- PEB ve modul tablosu: "ben nereye yuklendim?"
//!
//! Bir Windows programi calisirken kendisi hakkindaki bilgiyi bir
//! **yapidan** okur: PEB. Adresi TEB'den gelir, iceriginde imaj tabani
//! ve yuklu modullerin baglantili listesi durur.
//!
//! POSIX ikizi (`probe`nin N/O/P sinavlari) ayni sorulara cevap verir
//! ama bilgi baska yerdedir:
//!
//! ```text
//!   POSIX   auxv    baslangic YIGININDA, (tur, deger) ciftleri
//!   Win32   PEB     segment tabanindan ulasilan bir YAPIDA
//! ```
//!
//! Fark omurlerinde: `auxv` yalnizca baslangicta vardir ve yigin
//! ilerledikce ustune yazilir. PEB surec boyunca durur ve istenildigi
//! zaman okunur -- bu program da tam olarak bunu yapiyor, `main`in
//! ortasinda.
//!
//! ## Alti sinav
//!
//! ```text
//!   A  PEB->ImageBaseAddress  -> GetModuleHandleA(NULL) ile AYNI
//!   B  modul listesi          -> halka kapali, ilk girdi surecin imaji
//!   C  GetModuleHandleA       -> "KERNEL32.dll" bulunur, uydurma bulunmaz
//!   D  GetProcAddress         -> ithal EDILMEYEN bir fonksiyon bulunur
//!   E  ayni adres             -> ikinci cagri ayni adresi verir
//!   F  ordinal ile arama      -> MAKEINTRESOURCE yolu calisir
//! ```
//!
//! D bu tablonun neden yazildigini tek satirda anlatiyor: `GetTickCount`
//! bu ikilinin ithal tablosunda **yok**, ama `GetProcAddress` onu
//! buluyor ve donen adres cagrilabiliyor. Ithal tablosu, calistirilabilir
//! seylerin tamami degil.
//!
//! B'nin "halka kapali" olmasi onemli: Windows'ta modul listesi bir
//! `LIST_ENTRY` halkasidir ve son girdinin `Flink`i basa doner. Diziye
//! cevirmek kolay olurdu ama listeyi gezen gercek bir kod halkayi yurur;
//! kirmak onu sonsuz donguye ya da cop veriye goturur.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::winapi::{self, Window};
use tcmk::{args, teb};

tcmk::entry!(main);

const BG: u32 = 0x0012_1A22;
const PANEL: u32 = 0x001E_2E3A;
const FG: u32 = 0x00E4_DEF0;
const DIM: u32 = 0x008C_84A0;
const ACCENT: u32 = 0x0080_D0FF;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// PEB icindeki alan ofsetleri (Windows ABI'siyla ayni).
#[cfg(target_arch = "x86")]
mod peb {
    pub const IMAGE_BASE: usize = 0x08;
    pub const LDR: usize = 0x0C;
    /// `PEB_LDR_DATA` icindeki `InLoadOrderModuleList`.
    pub const LDR_IN_LOAD_ORDER: usize = 0x0C;
    /// `LDR_DATA_TABLE_ENTRY` icindeki `DllBase`; baglantidan sonraki
    /// ofset (girdi baglantiyla basliyor).
    pub const ENTRY_DLL_BASE: usize = 0x18;
}

#[cfg(target_arch = "x86_64")]
mod peb {
    pub const IMAGE_BASE: usize = 0x10;
    pub const LDR: usize = 0x18;
    pub const LDR_IN_LOAD_ORDER: usize = 0x10;
    pub const ENTRY_DLL_BASE: usize = 0x30;
}

#[derive(Clone, Copy)]
struct Check {
    name: &'static str,
    detail: &'static str,
    passed: bool,
}

const EMPTY: Check = Check {
    name: "",
    detail: "",
    passed: false,
};

/// `PEB`in adresi: TEB'in `ProcessEnvironmentBlock` alanindan.
fn peb_address() -> usize {
    #[cfg(target_arch = "x86")]
    const PEB_OFFSET: usize = 0x30;
    #[cfg(target_arch = "x86_64")]
    const PEB_OFFSET: usize = 0x60;
    teb::read(PEB_OFFSET)
}

fn main() {
    let mut console = winapi::Console;
    let mut checks = [EMPTY; 6];

    let peb_at = peb_address();
    let handle = winapi::image_base() as usize;

    // --- A: PEB'deki taban ile cagrinin dondugu ayni mi ---
    //
    // Iki kaynak: biri bellekten okunuyor, digeri cekirdege soruluyor.
    // Ayrisirlarsa, PEB'i dogrudan okuyan derlenmis bir kod yanlis
    // adresle calisir.
    let peb_base = if peb_at != 0 {
        unsafe { ((peb_at + peb::IMAGE_BASE) as *const usize).read_unaligned() }
    } else {
        0
    };
    let a = peb_at != 0 && peb_base != 0 && peb_base == handle;
    checks[0] = Check {
        name: "A PEB imaj tabani",
        detail: if peb_at == 0 {
            "PEB isaretcisi bos"
        } else if peb_base == 0 {
            "ImageBaseAddress doldurulmamis"
        } else if a {
            "PEB ve GetModuleHandleA ayni adresi veriyor"
        } else {
            "iki kaynak AYRISIYOR"
        },
        passed: a,
    };

    // --- B: modul listesi halkasi ---
    //
    // Bastan yuruyup basa donuluyor. Ilk girdinin `DllBase`i surecin
    // imaj tabani olmali (Windows'un sirasi: once kendi imaji).
    let (ring_ok, module_count, first_base) = walk_modules(peb_at);
    let b = ring_ok && module_count >= 2 && first_base == handle;
    checks[1] = Check {
        name: "B modul listesi",
        detail: if !ring_ok {
            "halka kapali degil"
        } else if module_count < 2 {
            "listede DLL girdisi yok"
        } else if first_base != handle {
            "ilk girdi surecin imaji degil"
        } else {
            "halka kapali, ilk girdi surecin imaji"
        },
        passed: b,
    };

    // --- C: ada gore tanitici ---
    let kernel32 = unsafe { winapi::GetModuleHandleA(b"KERNEL32.dll\0".as_ptr()) } as usize;
    let missing = unsafe { winapi::GetModuleHandleA(b"YOKBOYLE.dll\0".as_ptr()) } as usize;
    let last = unsafe { winapi::GetLastError() };
    let c = kernel32 != 0 && missing == 0 && last == winapi::ERROR_MOD_NOT_FOUND;
    checks[2] = Check {
        name: "C GetModuleHandleA",
        detail: if kernel32 == 0 {
            "KERNEL32.dll bulunamadi"
        } else if missing != 0 {
            "OLMAYAN DLL icin tanitici dondu"
        } else if last != winapi::ERROR_MOD_NOT_FOUND {
            "hata kodu 126 degil"
        } else {
            "kernel32 bulundu, uydurma bulunmadi"
        },
        passed: c,
    };

    // --- D: ithal edilmeyen bir fonksiyon ---
    //
    // `Sleep` bu ikilinin ithal tablosunda yok (kaynak onu hic
    // cagirmiyor). `GetProcAddress` yine de bulmali ve donen adres
    // **cagrilabilir** olmali -- cekirdek istendigi anda bir thunk
    // uretiyor.
    let sleep_at = unsafe { winapi::GetProcAddress(kernel32 as _, b"Sleep\0".as_ptr()) };
    let called = if !sleep_at.is_null() {
        let sleep: extern "system" fn(winapi::Dword) =
            unsafe { core::mem::transmute(sleep_at) };
        let before = unsafe { winapi::GetTickCount() };
        sleep(30);
        let after = unsafe { winapi::GetTickCount() };
        after >= before
    } else {
        false
    };
    let d = !sleep_at.is_null() && called;
    checks[3] = Check {
        name: "D ithal edilmeyen cagri",
        detail: if sleep_at.is_null() {
            "GetProcAddress bulamadi"
        } else if !called {
            "adres bulundu ama cagrilamadi"
        } else {
            "uretilen thunk gercekten calisti"
        },
        passed: d,
    };

    // --- E: ayni fonksiyon, ayni adres ---
    //
    // Windows'un sozlesmesi: `GetProcAddress` her cagride ayni adresi
    // verir. Her seferinde yeni thunk uretmek hem alani tuketir hem de
    // adresleri karsilastiran kodu bozardi.
    let again = unsafe { winapi::GetProcAddress(kernel32 as _, b"Sleep\0".as_ptr()) };
    let e = !sleep_at.is_null() && again == sleep_at;
    checks[4] = Check {
        name: "E ayni adres",
        detail: if sleep_at.is_null() {
            "ilk cagri basarisiz"
        } else if e {
            "ikinci cagri ayni adresi verdi"
        } else {
            "her cagri YENI adres uretiyor"
        },
        passed: e,
    };

    // --- F: ordinal ile arama ---
    //
    // `GetTickCount` gomulu tabloda 3 numarali ihracat ve `.def`
    // dosyasinda bilerek NONAME. Gercek DLL'lerde de ordinal-only
    // ihracatlar vardir; tek erisim yolu budur.
    let by_ordinal = unsafe { winapi::proc_address_by_ordinal(kernel32 as _, 3) };
    let by_name = unsafe { winapi::GetProcAddress(kernel32 as _, b"GetTickCount\0".as_ptr()) };
    let f = !by_ordinal.is_null() && by_ordinal == by_name;
    checks[5] = Check {
        name: "F ordinal ile arama",
        detail: if by_ordinal.is_null() {
            "ordinal yolu bulamadi"
        } else if by_name.is_null() {
            "ad yolu bulamadi"
        } else if f {
            "ordinal 3 ile ad ayni adresi verdi"
        } else {
            "iki yol FARKLI adres veriyor"
        },
        passed: f,
    };

    for check in &checks {
        let _ = core::fmt::Write::write_str(&mut console, "[winmods] ");
        let _ = core::fmt::Write::write_str(&mut console, check.name);
        let _ = core::fmt::Write::write_str(&mut console, ": ");
        let _ = core::fmt::Write::write_str(
            &mut console,
            if check.passed { "gecti" } else { "KALDI" },
        );
        let _ = core::fmt::Write::write_str(&mut console, " (");
        let _ = core::fmt::Write::write_str(&mut console, check.detail);
        let _ = core::fmt::Write::write_str(&mut console, ")\n");
    }

    let mut win = match Window::create("winmods -- PEB ve modul tablosu", 300, 160, 460, 180) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.get_message() == b'q' {
            break;
        }
        draw(&mut win, &checks, module_count, handle);
        win.frame(60);
    }
}

/// Modul halkasini yurur.
///
/// Doner: (halka kapali mi, girdi sayisi, ilk girdinin `DllBase`i).
///
/// Halkanin **basi listenin icindedir**: `PEB_LDR_DATA`daki `LIST_ENTRY`
/// hem ilk hem son girdiyi gosterir, ve son girdinin `Flink`i basa
/// doner. Yurume o basa geri gelince biter.
fn walk_modules(peb_at: usize) -> (bool, usize, usize) {
    if peb_at == 0 {
        return (false, 0, 0);
    }
    let ldr = unsafe { ((peb_at + peb::LDR) as *const usize).read_unaligned() };
    if ldr == 0 {
        return (false, 0, 0);
    }
    let head = ldr + peb::LDR_IN_LOAD_ORDER;
    let mut node = unsafe { (head as *const usize).read_unaligned() };
    let mut count = 0usize;
    let mut first_base = 0usize;

    // Girdi, baglantiyla basliyor: `DllBase`e ulasmak icin girdinin
    // basindan sayilan ofset kullanilir ve baglanti ofseti sifir.
    while node != head && node != 0 && count < 16 {
        let base = unsafe { ((node + peb::ENTRY_DLL_BASE) as *const usize).read_unaligned() };
        if count == 0 {
            first_base = base;
        }
        count += 1;
        node = unsafe { (node as *const usize).read_unaligned() };
    }
    (node == head && count > 0, count, first_base)
}

fn draw(win: &mut Window, checks: &[Check; 6], modules: usize, base: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "ben nereye yuklendim?", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            320,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.fill(6, h - 42, w - 12, 20, PANEL);
    win.text(12, h - 39, "modul:", DIM);
    win.number(70, h - 39, modules, FG);
    win.text(110, h - 39, "taban:", DIM);
    win.number(170, h - 39, base, FG);
    win.text(300, h - 39, args::get(0).unwrap_or("?"), DIM);
    win.text(
        6,
        h - 14,
        if passed == checks.len() {
            "hepsi gecti   q cik"
        } else {
            "BIR SINAV KALDI   q cik"
        },
        if passed == checks.len() { OK } else { WARN },
    );
}
