//! x86_64 tablo tabanli SEH -- islev tablolari ve `UNWIND_INFO`.
//!
//! ## Ayni karar, iki ayri tasima
//!
//! i386'da bir `__try` **calisma zamaninda** para oder: derleyici her
//! girisde yigina iki kelimelik bir kayit iter ve `fs:[0]`i ona cevirir.
//! Zincir boylece yigin uzerinde yasar ve istisna aninda gezilir.
//!
//! x86_64'te Microsoft bu maliyeti tamamen kaldirdi: `__try` **hicbir
//! komut uretmez**. Bunun yerine derleyici, ikili dosyaya her fonksiyon
//! icin bir kayit yazar ve o kayit fonksiyonun isleyicisini gosterir.
//!
//! ```text
//!   i386   zincir  ->  yiginda, CALISMA ZAMANINDA kurulur
//!                      bedeli: her __try girisinde iki yazma
//!                      cekirdegin isi: iki kelimeyi okuyup izlemek
//!
//!   x64    tablo   ->  ikilide, DERLEME ZAMANINDA kurulur
//!                      bedeli: sifir komut
//!                      cekirdegin isi: bir tablo cozumleyicisi
//! ```
//!
//! Maliyet kaybolmadi, **yer degistirdi**: uygulamadan cekirdege. Bu
//! dosya o bedelin kendisi.
//!
//! ## Ayrilan sey yalnizca "nasil bulunur"
//!
//! Isleyicinin **imzasi** ve **karar kumesi** iki mimaride aynidir:
//! dort arguman alir, `ExceptionContinueExecution` (0) ya da
//! `ExceptionContinueSearch` (1) doner. Yani ayrilan sey yalnizca
//! isleyiciye nasil ulasildigi; ulasildiktan sonrasi ortak
//! (bkz. `seh::advance`).
//!
//! ## Tablo nereden geliyor
//!
//! Iki kaynak vardir: PE'nin kendi `.pdata` bolumu (derleyici yazar) ve
//! `RtlAddFunctionTable` (calisma zamaninda kaydedilir -- JIT'ler icin).
//!
//! TCMK yalnizca ikincisini okuyor ve sebebi somut: userland Rust ile
//! yaziliyor, Rust'in COFF cikisi `.pdata` degil `.eh_frame` uretiyor
//! (DWARF). Yani TCMK'nin kendi PE'lerinde okunacak bir `.pdata` **yok**.
//! Varmis gibi bir cozumleyici yazmak, hicbir zaman kosmayan ve bu
//! yuzden hicbir zaman dogrulanmayan kod birakmak olurdu.

use crate::level0a::core::{mmu, scheduler};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Bir gorevin kaydedebilecegi en fazla islev tablosu.
pub const MAX_TABLES: usize = 4;

/// `UNWIND_INFO` bayraklari.
///
/// Ilk ikisi ayni alanda durur ve **ayri sorulara** cevap verir:
/// `EHANDLER` "bu fonksiyonun bir `__except` filtresi var", `UHANDLER`
/// "bir `__finally` blogu var". Bir fonksiyonda ikisi de olabilir ve o
/// zaman ayni isleyici iki kez cagrilir -- bayrak, hangi soru icin
/// cagrildigini soyler. TCMK su an yalnizca birinci yariyi (dagitim)
/// yaptigi icin `EHANDLER` araniyor.
pub const UNW_FLAG_EHANDLER: u8 = 0x1;
pub const UNW_FLAG_UHANDLER: u8 = 0x2;
/// Kayit kendi bilgisini tasimiyor, **baska bir kaydi** gosteriyor.
///
/// Derleyici bir fonksiyonu parcalara bolunce (soguk/sicak ayrimi)
/// parcalar tek bir ana kaydi paylasir. Izlemezsek bolunmus bir
/// fonksiyonun isleyicisi hic bulunamazdi.
pub const UNW_FLAG_CHAININFO: u8 = 0x4;

/// Zincirli kayitlarda en fazla kac adim izlenir.
///
/// Sinir sart: kayitlar kullanici alanindadir, yani bir surec kendini
/// gosteren bir zincir kurup cekirdegi sonsuz donguye sokabilirdi.
const MAX_CHAIN: usize = 4;

/// `RUNTIME_FUNCTION` -- ikili dosyadaki uc `DWORD`.
///
/// Ucu de **RVA**dir (goruntu tabanina gore goreli adres), mutlak degil.
/// Sebep dogrudan yeniden yerlesim: bir DLL baska bir adrese yuklenince
/// mutlak adresler bozulurdu, RVA'lar bozulmaz.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RuntimeFunction {
    pub begin: u32,
    pub end: u32,
    pub unwind: u32,
}

/// `RUNTIME_FUNCTION`un bayt olcusu -- ABI'nin parcasi.
pub const RUNTIME_FUNCTION_SIZE: usize = 12;

/// Kayitli bir islev tablosu.
#[derive(Clone, Copy)]
struct Table {
    /// Tablonun kullanici alanindaki adresi; 0 = bos yuva.
    at: usize,
    /// Kac `RUNTIME_FUNCTION` var.
    count: usize,
    /// RVA'larin cozulecegi goruntu tabani.
    base: usize,
}

impl Table {
    const EMPTY: Self = Table {
        at: 0,
        count: 0,
        base: 0,
    };
}

static mut TABLES: [[Table; MAX_TABLES]; scheduler::MAX_TASKS] =
    [[Table::EMPTY; MAX_TABLES]; scheduler::MAX_TASKS];

/// Olcum sayaclari (kabuk raporu).
static REGISTERED: AtomicUsize = AtomicUsize::new(0);
static LOOKUPS: AtomicUsize = AtomicUsize::new(0);
static FOUND: AtomicUsize = AtomicUsize::new(0);

/// `(kayitli tablo, arama, bulunan)` -- kabuk raporu.
pub fn stats() -> (usize, usize, usize) {
    (
        REGISTERED.load(Ordering::Relaxed),
        LOOKUPS.load(Ordering::Relaxed),
        FOUND.load(Ordering::Relaxed),
    )
}

/// Bir bolgenin tamami Ring 3'ten okunabilir mi?
///
/// Talep uzerine eslenen sayfalar da kabul ediliyor: cekirdegin oraya
/// dokunmasi kurtarilabilir bir hata uretir. Kati denetim burada yanlis
/// cevap verirdi (bkz. `seh::writable`de ayni duzeltme).
fn readable(from: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let mut page = from & !0xFFF;
    let last = (from + len - 1) & !0xFFF;
    loop {
        if !mmu::is_user_or_demand(page) {
            return false;
        }
        if page == last {
            return true;
        }
        page += 0x1000;
    }
}

/// `RtlAddFunctionTable(FunctionTable, EntryCount, BaseAddress)`.
///
/// Doner: kabul edildi mi.
pub fn add_table(task: usize, at: usize, count: usize, base: usize) -> bool {
    if task >= scheduler::MAX_TASKS || at == 0 || count == 0 {
        return false;
    }
    // Tablonun tamami okunabilir olmali. Denetim **kayit aninda**
    // yapiliyor ki istisna anindaki yol kisa kalsin -- ama orada da
    // yeniden bakiliyor, cunku surec arada bellegi serbest birakmis
    // olabilir.
    let Some(bytes) = count.checked_mul(RUNTIME_FUNCTION_SIZE) else {
        return false;
    };
    if !readable(at, bytes) {
        return false;
    }

    crate::arch::cpu::without_interrupts(|| {
        // SAFETY: yuva gorev ve indekse ozel.
        unsafe {
            let row = (core::ptr::addr_of_mut!(TABLES) as *mut Table).add(task * MAX_TABLES);
            for i in 0..MAX_TABLES {
                if row.add(i).read().at == 0 {
                    row.add(i).write(Table { at, count, base });
                    REGISTERED.fetch_add(1, Ordering::Relaxed);
                    return true;
                }
            }
        }
        false
    })
}

/// `RtlDeleteFunctionTable(FunctionTable)`.
pub fn delete_table(task: usize, at: usize) -> bool {
    if task >= scheduler::MAX_TASKS || at == 0 {
        return false;
    }
    crate::arch::cpu::without_interrupts(|| {
        // SAFETY: yuva gorev ve indekse ozel.
        unsafe {
            let row = (core::ptr::addr_of_mut!(TABLES) as *mut Table).add(task * MAX_TABLES);
            for i in 0..MAX_TABLES {
                if row.add(i).read().at == at {
                    row.add(i).write(Table::EMPTY);
                    return true;
                }
            }
        }
        false
    })
}

/// Gorevin butun tablolarini unutur (`execve`, yuva yeniden kullanimi).
pub fn reset(task: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| {
        // SAFETY: butun satir kendi yuvasi.
        unsafe {
            let row = (core::ptr::addr_of_mut!(TABLES) as *mut Table).add(task * MAX_TABLES);
            for i in 0..MAX_TABLES {
                row.add(i).write(Table::EMPTY);
            }
        }
    });
}

/// `RtlLookupFunctionEntry(ControlPc, &ImageBase, HistoryTable)`.
///
/// Bir komut adresini iceren `RUNTIME_FUNCTION`u bulur.
///
/// Doner: `(kaydin kullanici alanindaki adresi, kaydin kendisi, taban)`.
/// Kaydin **adresi** de doniyor, cunku `DISPATCHER_CONTEXT`in
/// `FunctionEntry` alani isaretci ister -- kopya degil.
pub fn lookup(task: usize, pc: usize) -> Option<(usize, RuntimeFunction, usize)> {
    if task >= scheduler::MAX_TASKS {
        return None;
    }
    LOOKUPS.fetch_add(1, Ordering::Relaxed);

    crate::arch::cpu::without_interrupts(|| {
        // SAFETY: yalnizca kendi satiri okunuyor; her tablo erisimi
        // ayrica `readable` ile dogrulaniyor.
        unsafe {
            let row = (core::ptr::addr_of!(TABLES) as *const Table).add(task * MAX_TABLES);
            for i in 0..MAX_TABLES {
                let table = row.add(i).read();
                if table.at == 0 {
                    continue;
                }
                let bytes = table.count * RUNTIME_FUNCTION_SIZE;
                if !readable(table.at, bytes) {
                    continue;
                }
                // Dogrusal arama. Gercek Windows ikili arama yapar
                // cunku tablo siralidir; burada tablolar sinav
                // olcusunde ve siralilik **kullanicinin sozu**, yani
                // ona guvenip ikili arama yapmak yanlis cevap
                // uretebilirdi.
                for j in 0..table.count {
                    let entry_at = table.at + j * RUNTIME_FUNCTION_SIZE;
                    let begin = (entry_at as *const u32).read_unaligned();
                    let end = ((entry_at + 4) as *const u32).read_unaligned();
                    let unwind = ((entry_at + 8) as *const u32).read_unaligned();
                    if end <= begin {
                        continue;
                    }
                    let lo = table.base.wrapping_add(begin as usize);
                    let hi = table.base.wrapping_add(end as usize);
                    if pc >= lo && pc < hi {
                        FOUND.fetch_add(1, Ordering::Relaxed);
                        return Some((
                            entry_at,
                            RuntimeFunction { begin, end, unwind },
                            table.base,
                        ));
                    }
                }
            }
        }
        None
    })
}

/// Bir `UNWIND_INFO` kaydindan **dil isleyicisini** cikarir.
///
/// Kaydin duzeni (Windows x64 ABI):
///
/// ```text
///   +0  Version:3 | Flags:5
///   +1  SizeOfProlog
///   +2  CountOfCodes
///   +3  FrameRegister:4 | FrameOffset:4
///   +4  UnwindCode[CountOfCodes]      (2 bayt her biri, CIFT'e yuvarlanir)
///   ..  ExceptionHandler (RVA)        (yalnizca EHANDLER/UHANDLER varsa)
///   ..  ExceptionData[]               (dil'e ozel -- "scope table")
/// ```
///
/// Isleyicinin ofseti **degisken**: geri sarma kodlarinin sayisina
/// bagli. Ciftle yuvarlama ABI'nin parcasi (kayit `DWORD` hizali kalsin
/// diye), ve unutmak isleyici yerine kod okumak demek olurdu.
///
/// Doner: `(isleyicinin mutlak adresi, dil verisinin adresi)`.
pub fn handler_of(base: usize, unwind_rva: u32) -> Option<(usize, usize)> {
    let mut rva = unwind_rva;
    for _ in 0..MAX_CHAIN {
        let info = base.wrapping_add(rva as usize);
        // Basligin dort bayti + en az bir kelime.
        if !readable(info, 8) {
            return None;
        }
        // SAFETY: bolge yukarida dogrulandi.
        let (version_flags, count) = unsafe {
            (
                (info as *const u8).read(),
                ((info + 2) as *const u8).read() as usize,
            )
        };
        let version = version_flags & 0x7;
        let flags = version_flags >> 3;
        // Surum 1 disindaki kayitlarin duzeni farkli olabilir; okumaya
        // calismak, kod adresi yerine cop dondurmek olurdu.
        if version != 1 {
            return None;
        }

        // Geri sarma kodlari **cifte yuvarlanir**.
        let codes = (count + 1) & !1;
        let after_codes = info + 4 + codes * 2;
        if !readable(after_codes, RUNTIME_FUNCTION_SIZE) {
            return None;
        }

        if flags & UNW_FLAG_CHAININFO != 0 {
            // Zincirli kayit: isleyici alaninda bir `RUNTIME_FUNCTION`
            // duruyor. Onun `unwind` alanini izliyoruz.
            // SAFETY: bolge yukarida dogrulandi.
            let next = unsafe { ((after_codes + 8) as *const u32).read_unaligned() };
            if next == rva {
                // Kendini gosteren zincir: donguye girmek yerine birak.
                return None;
            }
            rva = next;
            continue;
        }

        if flags & (UNW_FLAG_EHANDLER | UNW_FLAG_UHANDLER) == 0 {
            // Bu fonksiyonun isleyicisi yok. Kayit yine de gecerli --
            // yalnizca geri sarma bilgisi tasiyor.
            return None;
        }

        // SAFETY: bolge yukarida dogrulandi.
        let handler_rva = unsafe { (after_codes as *const u32).read_unaligned() };
        if handler_rva == 0 {
            return None;
        }
        let handler = base.wrapping_add(handler_rva as usize);
        if !mmu::is_user_accessible(handler) {
            return None;
        }
        // `HandlerData`: isleyici RVA'sinin hemen ardindaki alan.
        // Dil'e ozeldir (MSVC'de "scope table"); cekirdek icerigini
        // yorumlamaz, yalnizca adresini gecirir.
        return Some((handler, after_codes + 4));
    }
    None
}

// --- DISPATCHER_CONTEXT ----------------------------------------------
//
// x64 isleyicisinin dorduncu argumani. i386'da bu alan cekirdek icindi
// ve kullanilmiyordu (TCMK sifir geciyordu); x64'te **kayittir** ve
// isleyici ondan goruntu tabanini, kendi `RUNTIME_FUNCTION`unu ve dil
// verisini okur. Yani dorduncu argumanin bos gecilmesi burada kabul
// edilemez: MSVC'nin urettigi isleyici ilk isi olarak oraya bakar.

/// `DISPATCHER_CONTEXT`in bayt olcusu.
pub const DISPATCHER_SIZE: usize = 0x50;

/// Alan ofsetleri -- Windows x64 ABI'sinin parcasi.
pub mod disp {
    pub const CONTROL_PC: usize = 0x00;
    pub const IMAGE_BASE: usize = 0x08;
    pub const FUNCTION_ENTRY: usize = 0x10;
    pub const ESTABLISHER_FRAME: usize = 0x18;
    pub const TARGET_IP: usize = 0x20;
    pub const CONTEXT_RECORD: usize = 0x28;
    pub const LANGUAGE_HANDLER: usize = 0x30;
    pub const HANDLER_DATA: usize = 0x38;
    pub const HISTORY_TABLE: usize = 0x40;
    pub const SCOPE_INDEX: usize = 0x48;
}

/// `DISPATCHER_CONTEXT` kaydini kullanici yiginina yazar.
///
/// # Safety
/// `at` en az `DISPATCHER_SIZE` bayt yazilabilir olmalidir.
#[allow(clippy::too_many_arguments)]
pub unsafe fn write_dispatcher(
    at: usize,
    control_pc: usize,
    image_base: usize,
    function_entry: usize,
    establisher: usize,
    context_at: usize,
    handler: usize,
    handler_data: usize,
) {
    core::ptr::write_bytes(at as *mut u8, 0, DISPATCHER_SIZE);
    let put = |off: usize, value: usize| ((at + off) as *mut usize).write_unaligned(value);
    put(disp::CONTROL_PC, control_pc);
    put(disp::IMAGE_BASE, image_base);
    put(disp::FUNCTION_ENTRY, function_entry);
    put(disp::ESTABLISHER_FRAME, establisher);
    // `TargetIp` yalnizca geri sarmada anlamli; dagitimda sifir.
    put(disp::TARGET_IP, 0);
    put(disp::CONTEXT_RECORD, context_at);
    put(disp::LANGUAGE_HANDLER, handler);
    put(disp::HANDLER_DATA, handler_data);
    // `HistoryTable` ve `ScopeIndex` sifir: ilki bir onbellek, ikincisi
    // geri sarmanin kaldigi yeri tasiyor. Ikisi de TCMK'de yok.
    put(disp::HISTORY_TABLE, 0);
}
