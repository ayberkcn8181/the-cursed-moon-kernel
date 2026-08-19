//! Kullanici sureci calistirma akisi (Faz 3 + Faz 7).
//!
//! Bir ikiliyi Level-0b1'in yukleyicileriyle bellege alir, kullanici
//! bolgesini Ring 3'e acar, TSS.esp0'i ayarlar ve `iret` ile Ring 3'e gecer.
//! Ring 3 -> Ring 0 gecisi (int 0x80 / int 0x2E) TSS sayesinde otomatiktir.
//!
//! **Cift uyumluluk burada baslar:** ayni akis hem ELF (Linux) hem PE
//! (Windows) ikililerini calistirir; format magic baytlarindan secilir ve
//! ikisi de ayni Ring 3 ortamina, ayni kullanici bellek tabanina yuklenir.

use crate::arch::cpu::usermode;
use crate::level0a::core::{env, kmalloc, mmu, scheduler, vfs};
use crate::level0a::gdt;
use crate::level0a::kernel_api;
#[cfg(target_arch = "x86")]
use crate::level0b1::binary_loader::{elf32, pe32};
#[cfg(target_arch = "x86_64")]
use crate::level0b1::binary_loader::{elf64, pe64};

/// Ring 3 yigini icin ayrilan alan (kullanici bolgesinin tepesinde).
const USER_STACK_SIZE: usize = 8 * 1024;
/// Ring 3'ten kesme geldiginde CPU'nun gececegi cekirdek yigini.
const KERNEL_STACK_SIZE: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryFormat {
    #[cfg(target_arch = "x86")]
    Elf32,
    #[cfg(target_arch = "x86")]
    Pe32,
    #[cfg(target_arch = "x86_64")]
    Elf64,
    #[cfg(target_arch = "x86_64")]
    Pe32Plus,
}

#[derive(Debug)]
pub enum SpawnError {
    /// Surece adres uzayi kurulamadi ve paylasimli uzay zaten baska bir
    /// Ring 3 sureci tarafindan kullaniliyor.
    AddressSpaceBusy,
    // Alanlar yalnizca turetilmis Debug uzerinden okunur; dead-code analizi
    // bunu gormedigi icin acikca izin veriliyor.
    #[cfg(target_arch = "x86")]
    Elf(#[allow(dead_code)] elf32::ElfError),
    #[cfg(target_arch = "x86")]
    Pe(#[allow(dead_code)] pe32::PeError),
    #[cfg(target_arch = "x86_64")]
    Elf64(#[allow(dead_code)] elf64::Elf64Error),
    #[cfg(target_arch = "x86_64")]
    Pe64(#[allow(dead_code)] pe64::PeError),
    OutOfMemory,
    NoRoomForStack,
    NotFound,
}

/// Yuklenmis imajin ortak tanimi -- hangi formattan geldigi onemsizdir.
struct Prepared {
    entry: usize,
    end: usize,
    format: BinaryFormat,
}

/// VFS'teki bir yoldan ikili calistirir; format magic baytlarindan secilir
/// (doc S.7 Faz 7: "VFS magic onceligi, PE basarisizsa ELF fallback").
///
/// # Safety
/// `run_image` ile ayni onkosullar.
pub unsafe fn run_from_vfs_dynamic(path: &str, args: &str) -> Result<(), SpawnError> {
    let node = vfs::lookup(path).ok_or(SpawnError::NotFound)?;
    let image = vfs::load(node).ok_or(SpawnError::NotFound)?;
    run_image(path, image, args)
}

pub unsafe fn run_from_vfs(path: &'static str) -> Result<(), SpawnError> {
    let node = vfs::lookup(path).ok_or(SpawnError::NotFound)?;
    let image = vfs::load(node).ok_or(SpawnError::NotFound)?;
    crate::println!("[LEVEL-0b1] VFS'ten yukleniyor: {}", path);
    run_image(path, image, "")
}

/// Bir ikiliyi formatini tespit ederek Ring 3'te calistirir.
///
/// # Safety
/// Sayfalama acik ve TSS kurulmus olmalidir.
pub unsafe fn run_image(name: &str, image: &[u8], args: &str) -> Result<(), SpawnError> {
    crate::println!(
        "[LEVEL-0b1] Binary Loader: '{}' ({} bayt) yukleniyor.",
        name,
        image.len()
    );

    // --- Adres uzayi ---
    // Once bos bir kullanici adres uzayi kurulur ve YUKLEMEDEN ONCE gecilir:
    // yukleyici imaji 0x00C00000'e kopyalarken artik bu surecin kendi
    // cerceveleri eslenmis olur. Onceki modelde tum surecler ayni bolgeyi
    // paylastigi icin her uygulama farkli bir "slot"a linklenmek zorundaydi.
    let space = mmu::create_user_space();

    // Adres uzayi kurulamadiysa (cerceve havuzu tukendi) paylasimli yola
    // duseriz -- ama baska bir Ring 3 sureci kosuyorsa bu, onun kodunun
    // uzerine yazmak demektir: butun imajlar ayni sanal adrese yuklenir.
    // Sessizce bozmaktansa acikca reddetmek dogru olan; belirtisi baska
    // turlu "calisan uygulamanin kendi kodunda page fault almasi" olurdu.
    if space.is_none() && scheduler::user_task_count() > 0 {
        return Err(SpawnError::AddressSpaceBusy);
    }
    if let Some(cr3) = space {
        mmu::switch_to(cr3);
        scheduler::set_current_address_space(cr3);
        // Pesin ESLEME degil, **ayirma**: cerceveler ilk dokunusta
        // veriliyor (bkz. `mmu::reserve_user_range`). Yukleyicinin imaji
        // kopyalarken yaptigi yazmalar da bu yoldan geciyor -- yani
        // gercekten kullanilan sayfalar aninda doluyor, gerisi bos
        // kaliyor.
        if !mmu::reserve_user_range(cr3, mmu::USER_MEM_START, mmu::USER_MAP_SIZE) {
            release_space(space);
            return Err(SpawnError::OutOfMemory);
        }
    }

    let result = detect_and_load(image).and_then(|prepared| enter_ring3(prepared, space, name, args));

    // Surec bitti (ya da yuklenemedi): cerceveleri havuza geri ver.
    release_space(space);
    result
}

/// Surecin adres uzayini birakir ve gorevi cekirdek uzayina dondurur.
unsafe fn release_space(space: Option<usize>) {
    if let Some(cr3) = space {
        scheduler::set_current_address_space(0);
        mmu::switch_to(mmu::kernel_cr3());
        mmu::destroy_user_space(cr3);
    }
}

/// Format secimi ve yukleme -- mimariye gore hangi yukleyicilerin mevcut
/// oldugu degisir (i386: ELF32 + PE32, x86_64: ELF64 + PE32+).
#[cfg(target_arch = "x86")]
unsafe fn detect_and_load(image: &[u8]) -> Result<Prepared, SpawnError> {
    if pe32::is_pe(image) {
        crate::println!("[LEVEL-0b1] format: PE32 (Windows NT alt sistemi)");
        match pe32::load(image) {
            Ok(img) => {
                return Ok(Prepared {
                    entry: img.entry as usize,
                    end: img.end as usize,
                    format: BinaryFormat::Pe32,
                })
            }
            // Doc S.7: PE basarisiz olursa ELF'e geri dusulur.
            Err(pe_err) => {
                crate::println!("[LEVEL-0b1] PE yuklenemedi ({:?}), ELF deneniyor.", pe_err);
                let img = elf32::load(image).map_err(|_| SpawnError::Pe(pe_err))?;
                return Ok(Prepared {
                    entry: img.entry as usize,
                    end: img.end as usize,
                    format: BinaryFormat::Elf32,
                });
            }
        }
    }

    crate::println!("[LEVEL-0b1] format: ELF32 (Linux POSIX alt sistemi)");
    let img = elf32::load(image).map_err(SpawnError::Elf)?;
    Ok(Prepared {
        entry: img.entry as usize,
        end: img.end as usize,
        format: BinaryFormat::Elf32,
    })
}

#[cfg(target_arch = "x86_64")]
unsafe fn detect_and_load(image: &[u8]) -> Result<Prepared, SpawnError> {
    // i386 tarafiyla ayni oncelik: magic PE ise once PE denenir, o
    // basarisiz olursa ELF'e dusulur.
    if pe64::is_pe(image) {
        crate::println!("[LEVEL-0b1] format: PE32+ (Windows NT alt sistemi, x86_64)");
        match pe64::load(image) {
            Ok(img) => {
                return Ok(Prepared {
                    entry: img.entry as usize,
                    end: img.end as usize,
                    format: BinaryFormat::Pe32Plus,
                })
            }
            Err(pe_err) => {
                crate::println!(
                    "[LEVEL-0b1] PE32+ yuklenemedi ({:?}), ELF deneniyor.",
                    pe_err
                );
                let img = elf64::load(image).map_err(|_| SpawnError::Pe64(pe_err))?;
                return Ok(Prepared {
                    entry: img.entry,
                    end: img.end,
                    format: BinaryFormat::Elf64,
                });
            }
        }
    }

    if !elf64::is_elf64(image) {
        crate::println!("[LEVEL-0b1] ne PE ne ELF64 imzasi var -- yine de ELF64 deneniyor.");
    }
    crate::println!("[LEVEL-0b1] format: ELF64 (Linux POSIX alt sistemi)");
    let img = elf64::load(image).map_err(SpawnError::Elf64)?;
    Ok(Prepared {
        entry: img.entry,
        end: img.end,
        format: BinaryFormat::Elf64,
    })
}

/// En fazla arguman sayisi (`argv[0]` dahil).
const MAX_ARGV: usize = 8;

/// Win32 surecinin komut satirinin adresi -- **surec basina**.
///
/// `GetCommandLineA` bunu doner. POSIX tarafinda karsiligi yok: orada
/// argumanlar yiginda, `argc`/`argv` olarak durur.
/// Surecin **imaj yolu** -- cekirdek tarafinda saklanan kopya.
///
/// `argv[0]` kullanicinin yigininda duruyor ama ona guvenilemez: surec
/// kendi yigininda yazdigi seyi degistirebilir. `GetModuleFileNameA`nin
/// dondurdugu deger, gercek Windows'ta da cekirdegin bildigi yoldur.
const PROGRAM_PATH_MAX: usize = 64;
static mut PROGRAM_PATH: [[u8; PROGRAM_PATH_MAX]; scheduler::MAX_TASKS] =
    [[0; PROGRAM_PATH_MAX]; scheduler::MAX_TASKS];
static PROGRAM_LEN: [core::sync::atomic::AtomicUsize; scheduler::MAX_TASKS] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; scheduler::MAX_TASKS];

fn remember_program(task: usize, program: &str) {
    let slot = task % scheduler::MAX_TASKS;
    let taken = program.len().min(PROGRAM_PATH_MAX);
    crate::arch::cpu::without_interrupts(|| unsafe {
        let base = (core::ptr::addr_of_mut!(PROGRAM_PATH) as *mut u8).add(slot * PROGRAM_PATH_MAX);
        core::ptr::copy_nonoverlapping(program.as_ptr(), base, taken);
    });
    PROGRAM_LEN[slot].store(taken, core::sync::atomic::Ordering::Relaxed);
}

/// Calisan surecin imaj yolu (Win32 `GetModuleFileNameA`).
pub fn program_path() -> &'static str {
    let slot = scheduler::current_id() % scheduler::MAX_TASKS;
    let len = PROGRAM_LEN[slot].load(core::sync::atomic::Ordering::Relaxed);
    unsafe {
        let base = (core::ptr::addr_of!(PROGRAM_PATH) as *const u8).add(slot * PROGRAM_PATH_MAX);
        core::str::from_utf8(core::slice::from_raw_parts(base, len)).unwrap_or("")
    }
}

static COMMAND_LINE: [core::sync::atomic::AtomicUsize; scheduler::MAX_TASKS] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; scheduler::MAX_TASKS];

/// Calisan surecin komut satiri (Win32 `GetCommandLineA` icin).
pub fn command_line_ptr() -> usize {
    COMMAND_LINE[scheduler::current_id() % scheduler::MAX_TASKS]
        .load(core::sync::atomic::Ordering::Relaxed)
}

/// Baslangic yigininin argumanlarini yerlestirir; yeni ESP/RSP doner.
///
/// ## Iki ABI, iki bicim
///
/// Bu, POSIX ile Win32'nin **en gorunur** ayrildigi yerlerden biri ve
/// ikisi de burada oldugu gibi korunuyor:
///
/// ```text
///   POSIX (ELF)   yiginda:  [argc][argv0][argv1]..[NULL][envp NULL]
///                 -- bir DIZI; her arguman ayri, NUL ile biter
///
///   Win32 (PE)    tek bir dize: "browse /notlar"
///                 -- GetCommandLineA onu doner, bolmek CRT'nin isi
/// ```
///
/// Cekirdek argumanlari **bir kez** aliyor; farkli olan yalnizca
/// sunum. Ayni sozlesmeyi tek bicime indirmek, iki taraftan birinin
/// beklentisini bozardi -- Windows programi `argv` aramaz, Linux
/// programi tek dize beklemez.
unsafe fn build_start_stack(
    format: BinaryFormat,
    stack_top: usize,
    program: &str,
    args: &str,
) -> usize {
    let task = scheduler::current_id();
    COMMAND_LINE[task % scheduler::MAX_TASKS].store(0, core::sync::atomic::Ordering::Relaxed);
    // Yeni imaj: is-parcacigi tabanlari sifirlanir. `cwd`/ortamdan
    // farkli olarak burada **korumak yanlis** olurdu -- eski imajin TLS
    // blogu birakildi, tabani tutmak serbest kalmis bellege isaret eden
    // bir segment birakmak demek.
    crate::level0a::core::tls::reset(task);
    // Imajin yolu her iki ABI'de de gerekli ama **farkli sekilde**:
    // POSIX'te `argv[0]` olarak yigina konuyor, Win32'de
    // `GetModuleFileNameA` ile sorulunca dondurulmesi gerekiyor. Bir
    // Windows programi kendi dizinini bu cagriyla bulur, yani yalnizca
    // yigina koymak yetmiyor.
    remember_program(task, program);

    match format {
        #[cfg(target_arch = "x86")]
        BinaryFormat::Pe32 => build_win32_command_line(task, stack_top, program, args),
        #[cfg(target_arch = "x86_64")]
        BinaryFormat::Pe32Plus => build_win32_command_line(task, stack_top, program, args),
        _ => build_posix_stack(stack_top, program, args),
    }
}

/// Win32: tek bir dize, yiginin tepesine yazilir.
unsafe fn build_win32_command_line(
    task: usize,
    stack_top: usize,
    program: &str,
    args: &str,
) -> usize {
    // "program args" + NUL
    let total = program.len() + if args.is_empty() { 0 } else { args.len() + 1 } + 1;
    let start = (stack_top - total) & !3;
    let mut at = start;
    for byte in program.bytes() {
        (at as *mut u8).write(byte);
        at += 1;
    }
    if !args.is_empty() {
        (at as *mut u8).write(b' ');
        at += 1;
        for byte in args.bytes() {
            (at as *mut u8).write(byte);
            at += 1;
        }
    }
    (at as *mut u8).write(0);

    COMMAND_LINE[task % scheduler::MAX_TASKS].store(start, core::sync::atomic::Ordering::Relaxed);
    // ESP hizalanmis olarak dizenin altinda baslar.
    (start - 16) & !15
}

/// POSIX: `argc`/`argv`/`envp`, SysV baslangic yigini duzeninde.
unsafe fn build_posix_stack(stack_top: usize, program: &str, args: &str) -> usize {
    let word = core::mem::size_of::<usize>();

    // Once dizeler yiginin tepesine kopyalanir; isaretcileri saklanir.
    // `argv[0]` gelenege gore programin kendi adidir.
    let mut pointers = [0usize; MAX_ARGV];
    let mut count = 0usize;
    let mut sp = stack_top;

    let place = |text: &str, sp: &mut usize| -> usize {
        *sp -= text.len() + 1;
        let at = *sp;
        for (i, byte) in text.bytes().enumerate() {
            ((at + i) as *mut u8).write(byte);
        }
        ((at + text.len()) as *mut u8).write(0);
        at
    };

    pointers[0] = place(program, &mut sp);
    count += 1;
    for token in args.split_whitespace() {
        if count >= MAX_ARGV {
            break;
        }
        pointers[count] = place(token, &mut sp);
        count += 1;
    }

    // Ortam: **calisan gorevin kendi tablosu** (bkz. `core::env`). Yuva
    // ayrilirken oturumdan kopyalanmis, `fork`ta ebeveynden devralinmis,
    // `setenv` ile degistirilmis olabilir -- yigina yazilan o son
    // halidir. Her girdi `AD=deger` biciminde, tipki gercek `environ`
    // gibi.
    let table = crate::level0a::core::scheduler::current_id();
    let mut environment = [0usize; env::MAX_VARS];
    let mut env_count = 0usize;
    for i in 0..env::count(table) {
        if let Some(text) = env::entry_at(table, i) {
            environment[env_count] = place(text, &mut sp);
            env_count += 1;
        }
    }

    // Isaretci dizisi kelime hizali olmali.
    sp &= !(word - 1);

    // [argc][argv0..argvN][NULL][envp0..envpM][NULL]
    let words = 1 + count + 1 + env_count + 1;
    sp -= words * word;
    // x86_64 SysV: giriste yigin 16'ya hizali olmali.
    sp &= !15;

    (sp as *mut usize).write(count);
    let mut slot = 1usize;
    for pointer in &pointers[..count] {
        ((sp + slot * word) as *mut usize).write(*pointer);
        slot += 1;
    }
    // `argv` sonlandiricisi: `envp` hemen ardindan basliyor.
    ((sp + slot * word) as *mut usize).write(0);
    slot += 1;
    for pointer in &environment[..env_count] {
        ((sp + slot * word) as *mut usize).write(*pointer);
        slot += 1;
    }
    ((sp + slot * word) as *mut usize).write(0);
    sp
}

/// Formattan bagimsiz ortak bolum: yigin yerlesimi, sayfa izinleri,
/// TSS ve Ring 3 gecisi.
unsafe fn enter_ring3(
    prepared: Prepared,
    space: Option<usize>,
    program: &str,
    args: &str,
) -> Result<(), SpawnError> {
    // Kullanici yigini: imajin bittigi yerden sonra, sayfa hizali.
    let stack_bottom = (prepared.end + 0xFFF) & !0xFFF;
    let stack_top = stack_bottom + USER_STACK_SIZE;
    // Sinir, gercekten eslenmis pencere: kendi adres uzayinda 512 KiB,
    // paylasimli modelde (x86_64) tum bolge.
    if stack_top > mmu::USER_MEM_START + mmu::USER_MAP_SIZE {
        return Err(SpawnError::NoRoomForStack);
    }

    match space {
        // Kendi adres uzayi: sayfalar zaten tahsis edildi ve Ring 3'e acik.
        Some(_) => {}
        // Paylasimli model: bolgeyi Ring 3'e ac (eski davranis).
        None => mmu::protect_user_range(mmu::USER_MEM_START, stack_top - mmu::USER_MEM_START),
    }

    // Program break: imajin bittigi yerden yigin tabanina kadar buyuyebilir.
    kernel_api::set_program_break(prepared.end, stack_bottom);

    // Yeni imaj eski sinyal isleyicilerini devralmaz: kayitli adresler
    // artik var olmayan bir programa aittir, calistirilirsa surec kendi
    // kodunun ortasina dallanir. `execve` yolunda bu sart.
    crate::level0b1::signal::reset(scheduler::current_id());

    // Win32'nin `GetLastError` degeri de surece aittir. Gorev yuvalari
    // geri kazanildigi icin temizlenmesi sart: yeni bir surec, ayni
    // yuvada calismis oncekinin hatasini gormemeli.
    crate::level0b1::nt_subsystem::nt_syscalls::clear_last_error(scheduler::current_id());

    // Ring 3 -> Ring 0 gecisleri icin ayri bir cekirdek yigini.
    let kstack = kmalloc::kmalloc_aligned(KERNEL_STACK_SIZE, 16).ok_or(SpawnError::OutOfMemory)?;
    let kstack_top = kstack.add(KERNEL_STACK_SIZE) as usize;
    gdt::set_kernel_stack(kstack_top);
    // x86_64'te `syscall` komutu TSS kullanmaz; yigini ayrica bildirmeliyiz.
    #[cfg(target_arch = "x86_64")]
    crate::level0a::syscall_msr::set_kernel_stack(kstack_top);

    crate::println!(
        "[LEVEL-0b1] {:?} entry=0x{:08x} user_stack=0x{:08x} kernel_stack=0x{:08x}",
        prepared.format,
        prepared.entry,
        stack_top,
        kstack_top
    );
    crate::println!("[LEVEL-0b1] Ring 3'e geciliyor (iret)...");

    // Argumanlar yiginin **tepesine** yerlestirilir; ESP onlarin altinda
    // baslar. Iki ABI'nin bicimi burada ayrisir (bkz. `build_start_stack`).
    let sp = build_start_stack(prepared.format, stack_top, program, args);
    usermode::run_user_program(prepared.entry, sp);

    crate::println!("[LEVEL-0b1] Ring 3 programi sonlandi, Ring 0'a donuldu.");
    Ok(())
}
