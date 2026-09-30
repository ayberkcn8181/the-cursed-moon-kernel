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

// Not: bir zamanlar burada ayri bir `handler_of` vardi ve yalnizca
// isleyiciyi buluyordu. `virtual_unwind` ayni kaydi zaten bastan sona
// okudugu icin ikisi ayni ABI'nin iki kopyasi haline geldi -- ve bu
// dosyanin yorumlari tam olarak o duruma karsi uyariyordu. Isleyici
// artik geri sarmanin yan urunu.

// --- Sanal geri sarma: prologu **yorumlamak** ------------------------
//
// Tablo tabanli SEH'in ikinci yarisi. Birinci yari "hata adresinin
// isleyicisi kim" sorusunu cevapliyordu; bu yari onun ardindan gelen
// soruyu: **cagiran cerceveye nasil gecilir?**
//
// i386'da soru yok. Zincir yiginda duruyor ve her kayit bir oncekini
// gosteriyor -- yurumek bir isaretci izlemek. x64'te zincir yok, yani
// cagiranin RSP'sini bulmanin tek yolu, callee'nin **prologunu geri
// almak**: hangi registerlar itildi, yigina ne kadar yer acildi, cerceve
// registeri kuruldu mu.
//
// Derleyici bunu bir kod dizisi olarak yaziyor ve cekirdek onu
// **yurutuyor** -- ters yonde. Maliyetin uygulamadan cekirdege
// gecmesinin en somut hali burasi: i386'da iki kelime okunuyordu,
// burada kucuk bir yorumlayici kosuyor.

/// Geri sarma islemleri (Windows x64 ABI).
pub const UWOP_PUSH_NONVOL: u8 = 0;
pub const UWOP_ALLOC_LARGE: u8 = 1;
pub const UWOP_ALLOC_SMALL: u8 = 2;
pub const UWOP_SET_FPREG: u8 = 3;
pub const UWOP_SAVE_NONVOL: u8 = 4;
pub const UWOP_SAVE_NONVOL_FAR: u8 = 5;
pub const UWOP_SAVE_XMM128: u8 = 8;
pub const UWOP_SAVE_XMM128_FAR: u8 = 9;
pub const UWOP_PUSH_MACHFRAME: u8 = 10;

/// Geri sarma sirasindaki makine durumu.
///
/// `regs` **register numarasiyla** indeksleniyor (0=RAX .. 15=R15) ve bu
/// bir kolaylik degil, ABI'nin kendi secimi: geri sarma kodlarindaki
/// `op_info` alani dogrudan o numarayi tasiyor. Ayri bir esleme tablosu
/// yazmak, ABI'nin zaten verdigi cevabi ikinci kez uydurmak olurdu.
#[derive(Clone, Copy)]
pub struct UnwindState {
    pub rip: u64,
    pub rsp: u64,
    pub regs: [u64; 16],
}

/// `CONTEXT` icinde bir genel registerin ofseti.
///
/// Registerlar 0x78'den itibaren **register numarasi sirasinda** duruyor
/// (RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI, R8..R15). Bu da tesadufi
/// degil: ayni numaralandirma komut kodlamasinda, geri sarma kodlarinda
/// ve `CONTEXT`te birden kullaniliyor.
const CONTEXT_GPR_BASE: usize = 0x78;
const CONTEXT_RIP: usize = 0xF8;

fn gpr_offset(reg: usize) -> usize {
    CONTEXT_GPR_BASE + reg * 8
}

/// `CONTEXT` kaydindan geri sarma durumunu okur.
///
/// # Safety
/// `at` gecerli bir x64 `CONTEXT` kaydi olmalidir.
pub unsafe fn load_state(at: usize) -> UnwindState {
    let mut regs = [0u64; 16];
    for (i, slot) in regs.iter_mut().enumerate() {
        *slot = ((at + gpr_offset(i)) as *const u64).read_unaligned();
    }
    UnwindState {
        rip: ((at + CONTEXT_RIP) as *const u64).read_unaligned(),
        rsp: regs[4],
        regs,
    }
}

/// Geri sarma durumunu `CONTEXT` kaydina geri yazar.
///
/// # Safety
/// `load_state` ile ayni kosul.
pub unsafe fn store_state(at: usize, state: &UnwindState) {
    for (i, value) in state.regs.iter().enumerate() {
        // RSP ayri tutuluyor: geri sarma onu `regs[4]`ten bagimsiz
        // guncelliyor ve ikisini karistirmak, cagiranin yigin
        // isaretcisini callee'nin degeriyle ezmek olurdu.
        let value = if i == 4 { state.rsp } else { *value };
        ((at + gpr_offset(i)) as *mut u64).write_unaligned(value);
    }
    ((at + CONTEXT_RIP) as *mut u64).write_unaligned(state.rip);
}

/// Bir geri sarma kodunun kac yuva tuttugu.
///
/// Yuva sayisini yanlis hesaplamak, bir sonraki kodu **kodun ortasindan**
/// okumak demek: yorumlayici sessizce cop yurutmeye baslar.
fn slots_of(op: u8, info: u8) -> usize {
    match op {
        UWOP_ALLOC_LARGE => {
            if info == 0 {
                2
            } else {
                3
            }
        }
        UWOP_SAVE_NONVOL | UWOP_SAVE_XMM128 => 2,
        UWOP_SAVE_NONVOL_FAR | UWOP_SAVE_XMM128_FAR => 3,
        _ => 1,
    }
}

/// Yigindan bir kelime okur; okunamazsa `None`.
unsafe fn peek(at: u64) -> Option<u64> {
    let at = at as usize;
    if !readable(at, 8) {
        return None;
    }
    Some((at as *const u64).read_unaligned())
}

/// Bir cercevenin geri sarma bilgisini **yurutur**.
///
/// `state` girisde callee'nin durumu; cikista **cagiranin** durumu olur.
/// Doner: `(EstablisherFrame, isleyici)`.
///
/// `pc` hatanin adresi. Prolog **ortasinda** olabiliriz ve o zaman
/// kodlarin bir kismi henuz yurutulmemistir -- her kodun kendi prolog
/// ofseti var ve ondan ilerideki kodlar atlaniyor. Atlamayi unutmak,
/// henuz itilmemis bir registeri yigindan "geri almak" demek olurdu.
///
/// # Safety
/// Cagiran gorevin adres uzayi etkin olmalidir.
pub unsafe fn virtual_unwind(
    base: usize,
    function: &RuntimeFunction,
    pc: usize,
    state: &mut UnwindState,
) -> Option<(usize, Option<(usize, usize)>)> {
    let mut rva = function.unwind;
    // Prolog icindeki konum: fonksiyonun basindan kac bayt ileride.
    let mut pc_offset = (pc.wrapping_sub(base).wrapping_sub(function.begin as usize)) as u32;
    let mut handler = None;
    // `EstablisherFrame`: **fonksiyonun govdesindeki** yigin tabani,
    // geri sarilmis hali degil. Kodlar uygulanmadan once yakalaniyor;
    // cerceve registeri kuran bir fonksiyonda `SET_FPREG` onu
    // yeniden hesaplayacak (asagi bkz.).
    //
    // Sirayi karistirmak sessiz bir hata olurdu: geri sarma bittikten
    // sonraki RSP **cagiranin** yiginini gosterir, yani isleyici kendi
    // yerel degiskenlerini bir ust cercevede arardi.
    let mut establisher = state.rsp as usize;

    for _ in 0..MAX_CHAIN {
        let info = base.wrapping_add(rva as usize);
        if !readable(info, 4) {
            return None;
        }
        let version_flags = (info as *const u8).read();
        if version_flags & 0x7 != 1 {
            return None;
        }
        let flags = version_flags >> 3;
        let count = ((info + 2) as *const u8).read() as usize;
        let frame_byte = ((info + 3) as *const u8).read();
        let frame_reg = (frame_byte & 0xF) as usize;
        let frame_off = (frame_byte >> 4) as u64;

        let codes_at = info + 4;
        if !readable(codes_at, count * 2 + 2) {
            return None;
        }

        let mut i = 0usize;
        while i < count {
            let slot = ((codes_at + i * 2) as *const u16).read_unaligned();
            let prolog_offset = (slot & 0xFF) as u32;
            let op = ((slot >> 8) & 0xF) as u8;
            let op_info = ((slot >> 12) & 0xF) as u8;
            let width = slots_of(op, op_info);

            // Bu kod henuz **yurutulmedi**: prologun o noktasina
            // varilmadan hata olustu.
            if prolog_offset > pc_offset {
                i += width;
                continue;
            }

            match op {
                UWOP_PUSH_NONVOL => {
                    state.regs[op_info as usize] = peek(state.rsp)?;
                    state.rsp = state.rsp.wrapping_add(8);
                }
                UWOP_ALLOC_LARGE => {
                    let size = if op_info == 0 {
                        ((codes_at + (i + 1) * 2) as *const u16).read_unaligned() as u64 * 8
                    } else {
                        ((codes_at + (i + 1) * 2) as *const u32).read_unaligned() as u64
                    };
                    state.rsp = state.rsp.wrapping_add(size);
                }
                UWOP_ALLOC_SMALL => {
                    state.rsp = state.rsp.wrapping_add((op_info as u64 + 1) * 8);
                }
                UWOP_SET_FPREG => {
                    // Cerceve registeri kurulmus: yigin isaretcisi ondan
                    // **yeniden hesaplaniyor**. Kodlar ters yurutme
                    // sirasinda geldigi icin bu, itmelerden once
                    // geliyor -- yani once dogru RSP bulunuyor, sonra
                    // itilenler geri aliniyor.
                    state.rsp = state.regs[frame_reg].wrapping_sub(frame_off * 16);
                    // Cerceve registeri olan bir fonksiyonda yigin
                    // tabani budur; yukarida yakalanan ham RSP degil.
                    establisher = state.rsp as usize;
                }
                UWOP_SAVE_NONVOL => {
                    let off =
                        ((codes_at + (i + 1) * 2) as *const u16).read_unaligned() as u64 * 8;
                    state.regs[op_info as usize] = peek(state.rsp.wrapping_add(off))?;
                }
                UWOP_SAVE_NONVOL_FAR => {
                    let off = ((codes_at + (i + 1) * 2) as *const u32).read_unaligned() as u64;
                    state.regs[op_info as usize] = peek(state.rsp.wrapping_add(off))?;
                }
                // XMM kaydetmeleri yigin isaretcisini degistirmiyor ve
                // TCMK kayan nokta baglamini tasimiyor: yalnizca dogru
                // sayida yuva atlaniyor.
                UWOP_SAVE_XMM128 | UWOP_SAVE_XMM128_FAR => {}
                // Kesme cercevesi yalnizca cekirdek kipinde olur. Ring
                // 3'te gormek, kaydin bozuk oldugunu soyler.
                UWOP_PUSH_MACHFRAME => return None,
                _ => return None,
            }
            i += width;
        }

        // `EstablisherFrame`: cerceve registeri varsa ondan, yoksa
        // fonksiyonun govdesindeki RSP.
        if handler.is_none() && flags & (UNW_FLAG_EHANDLER | UNW_FLAG_UHANDLER) != 0 {
            let codes = (count + 1) & !1;
            let after = info + 4 + codes * 2;
            if readable(after, 4) {
                let handler_rva = (after as *const u32).read_unaligned();
                if handler_rva != 0 {
                    let at = base.wrapping_add(handler_rva as usize);
                    if mmu::is_user_accessible(at) {
                        handler = Some((at, after + 4));
                    }
                }
            }
        }

        if flags & UNW_FLAG_CHAININFO != 0 {
            let codes = (count + 1) & !1;
            let after = info + 4 + codes * 2;
            if !readable(after, RUNTIME_FUNCTION_SIZE) {
                return None;
            }
            let next = ((after + 8) as *const u32).read_unaligned();
            if next == rva {
                return None;
            }
            rva = next;
            // Zincirlenen **ana** kaydin prologu tumuyle yurutulmustur:
            // parca fonksiyon zaten onun govdesinde kosuyordu.
            pc_offset = u32::MAX;
            continue;
        }
        break;
    }

    // Cagiranin durumu: donus adresi yiginin tepesinde.
    let return_to = peek(state.rsp)?;
    state.rsp = state.rsp.wrapping_add(8);
    state.rip = return_to;
    state.regs[4] = state.rsp;
    Some((establisher, handler))
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
    /// Geri sarmanin kaldigi yer. TCMK dagitimda sifir geciyor;
    /// anlamli olmasi icin once geri sarmanin **ikinci** yarisi
    /// (`RtlUnwindEx`) gerekiyor.
    #[allow(dead_code)]
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
