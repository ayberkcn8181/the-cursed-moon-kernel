//! Uygulama baslatici: bir VFS yolunu alip Ring 3'te calisacak bir
//! scheduler gorevi olarak baslatir.
//!
//! Her uygulama kendi gorevinde kosar; gorev basina ayri cekirdek yigini
//! ve Ring 3 baglami oldugu icin (bkz. `scheduler::Task`) birden fazla GUI
//! uygulamasi ayni anda calisabilir.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::scheduler;

/// Baslatilmayi bekleyen uygulamanin yolu. Gorev girisi `extern "C" fn()`
/// oldugu icin arguman gecirilemiyor; yol bu kuyruk uzerinden aktarilir.
///
/// Yol **kopyalanarak** saklanir (`&'static str` degil): diskteki bir
/// dosyanin yolu kabuktan geldiginde omru gecici bir tampondur, oysa
/// gorev o yolu ancak baslatildiktan sonra okur.
const MAX_PENDING: usize = 8;
const MAX_PATH: usize = 64;
static mut PENDING: [[u8; MAX_PATH]; MAX_PENDING] = [[0; MAX_PATH]; MAX_PENDING];
static mut PENDING_LEN: [usize; MAX_PENDING] = [0; MAX_PENDING];

/// Yolla birlikte tasinan **arguman dizesi**.
///
/// Yolun yaninda ayri bir dizi: yol her zaman tek bir simge, argumanlar
/// ise bosluklu bir metin. Ikisini tek tamponda birlestirmek, yolun
/// icinde bosluk olabilecegi gunu bastan bozardi.
const MAX_ARGS: usize = 96;
static mut PENDING_ARGS: [[u8; MAX_ARGS]; MAX_PENDING] = [[0; MAX_ARGS]; MAX_PENDING];
static mut PENDING_ARGS_LEN: [usize; MAX_PENDING] = [0; MAX_PENDING];
static PENDING_HEAD: AtomicUsize = AtomicUsize::new(0);
static PENDING_TAIL: AtomicUsize = AtomicUsize::new(0);

/// `execve` istegi: gorev basina "bir sonraki program" yuvasi.
///
/// Ring 3'ten gelen exec cagrisi imaji **yerinde** degistiremez -- surec
/// o anda kendi kodunun icinde kosuyor. Bunun yerine istek buraya
/// yazilir, surec Ring 3'ten cikar ve `app_task` dongusu yeni imaji
/// yukler. Boylece exec, "cik ve yerine sunu yukle" haline gelir; adres
/// uzayi da dogal olarak sifirdan kurulur (execve semantigi zaten bu).
static mut EXEC_PATH: [[u8; MAX_PATH]; scheduler::MAX_TASKS] =
    [[0; MAX_PATH]; scheduler::MAX_TASKS];
static mut EXEC_LEN: [usize; scheduler::MAX_TASKS] = [0; scheduler::MAX_TASKS];
static mut EXEC_ARGS: [[u8; MAX_ARGS]; scheduler::MAX_TASKS] =
    [[0; MAX_ARGS]; scheduler::MAX_TASKS];
static mut EXEC_ARGS_LEN: [usize; scheduler::MAX_TASKS] = [0; scheduler::MAX_TASKS];

/// Gorev icin `execve` istegi kaydeder.
///
/// `args` **blok** bicimindedir (`argv[0]` dahil, NUL ayrili) -- bkz.
/// `level0b1::argv`.
pub fn request_exec(task: usize, path: &str, args: &str) -> bool {
    if task >= scheduler::MAX_TASKS || path.is_empty() || path.len() >= MAX_PATH {
        return false;
    }
    let args = if args.len() >= MAX_ARGS { "" } else { args };
    crate::arch::cpu::without_interrupts(|| unsafe {
        let slot = (core::ptr::addr_of_mut!(EXEC_PATH) as *mut u8).add(task * MAX_PATH);
        core::ptr::copy_nonoverlapping(path.as_ptr(), slot, path.len());
        (core::ptr::addr_of_mut!(EXEC_LEN) as *mut usize)
            .add(task)
            .write(path.len());

        let arg_slot = (core::ptr::addr_of_mut!(EXEC_ARGS) as *mut u8).add(task * MAX_ARGS);
        core::ptr::copy_nonoverlapping(args.as_ptr(), arg_slot, args.len());
        (core::ptr::addr_of_mut!(EXEC_ARGS_LEN) as *mut usize)
            .add(task)
            .write(args.len());
    });
    true
}

/// Bekleyen `execve` istegini alir ve yuvayi bosaltir.
///
/// `pub`: `fork` cocugu da bu yolu kullanir. Cocuk `app_task`ta
/// kosmadigi icin (giris noktasi `fork::child_task`) kendi exec
/// zincirini kendi yurutmek zorunda -- bkz. asagidaki not.
pub fn take_exec(task: usize) -> Option<(&'static str, &'static str)> {
    if task >= scheduler::MAX_TASKS {
        return None;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let len_slot = (core::ptr::addr_of_mut!(EXEC_LEN) as *mut usize).add(task);
        let len = len_slot.read();
        if len == 0 {
            return None;
        }
        len_slot.write(0);
        let slot = (core::ptr::addr_of!(EXEC_PATH) as *const u8).add(task * MAX_PATH);
        let path = core::str::from_utf8(core::slice::from_raw_parts(slot, len)).ok()?;

        let args_len = (core::ptr::addr_of!(EXEC_ARGS_LEN) as *const usize)
            .add(task)
            .read();
        let args_slot = (core::ptr::addr_of!(EXEC_ARGS) as *const u8).add(task * MAX_ARGS);
        let args = core::str::from_utf8(core::slice::from_raw_parts(args_slot, args_len))
            .unwrap_or("");
        Some((path, args))
    })
}

/// Kisa adlar: `run paint` yazabilmek icin.
///
/// Listede olmayan bir yol da calistirilabilir -- VFS'te varsa yeter.
/// Boylece diske kopyalanan ("kurulan") uygulamalar cekirdegi yeniden
/// derlemeden calisir.
static KNOWN_APPS: &[(&str, &str, &str)] = &[
    // (kisa ad, tam yol, gorev adi)
    //
    // Liste her iki mimaride de aynidir; VFS'te hangi ikilinin durdugu
    // (ELF32 mi ELF64 mu) mimariye gore degisir, `resolve` yalnizca yola
    // bakar. Yani `run plasma` iki mimarida de calisir.
    ("paint", "/bin/paint", "paint"),
    ("plasma", "/bin/plasma", "plasma"),
    ("crash", "/bin/crash", "crash"),
    ("hog", "/bin/hog", "hog"),
    ("spin", "/bin/spin", "spin"),
    ("notes", "/bin/notes", "notes"),
    ("menu", "/bin/menu", "menu"),
    ("twins", "/bin/twins", "twins"),
    ("relay", "/bin/relay", "relay"),
    ("echo2", "/bin/echo2", "echo2"),
    ("sigdemo", "/bin/sigdemo", "sigdemo"),
    ("race", "/bin/race", "race"),
    ("reaper", "/bin/reaper", "reaper"),
    ("redirect", "/bin/redirect", "redirect"),
    ("mux", "/bin/mux", "mux"),
    ("masked", "/bin/masked", "masked"),
    ("arena", "/bin/arena", "arena"),
    ("seeker", "/bin/seeker", "seeker"),
    ("browse", "/bin/browse", "browse"),
    ("waiter", "/bin/waiter", "waiter"),
    ("heir", "/bin/heir", "heir"),
    ("nested", "/bin/nested", "nested"),
    ("bequest", "/bin/bequest", "bequest"),
    ("probe", "/bin/probe", "probe"),
    ("quoted", "/bin/quoted", "quoted"),
    ("mapped", "/bin/mapped", "mapped"),
    ("threads", "/bin/threads", "threads"),
    // Windows ikilisi: yol .exe ile biter, cekirdek bicimi magic'ten
    // anlar (bkz. vfs::format) ve PE yukleyicisine yonlendirir. Yol iki
    // mimaride de aynidir; VFS'te duran ikilinin PE32 mi PE32+ mi oldugu
    // derleme aninda secilir.
    ("winclock", "/bin/winclock.exe", "winclock"),
    ("winpad", "/bin/winpad.exe", "winpad"),
    ("winfiles", "/bin/winfiles.exe", "winfiles"),
    ("winenv", "/bin/winenv.exe", "winenv"),
    ("winprobe", "/bin/winprobe.exe", "winprobe"),
    ("winseh", "/bin/winseh.exe", "winseh"),
    ("winargv", "/bin/winargv.exe", "winargv"),
    ("winmods", "/bin/winmods.exe", "winmods"),
    ("winmap", "/bin/winmap.exe", "winmap"),
    ("winthread", "/bin/winthread.exe", "winthread"),
];

/// Kabuktan gelen adi tam yola ve gorev adina cevirir.
///
/// Uc basamak, gittikce genelleserek:
///
///   1. **Gomulu liste**: kisa ad -> tam yol. Gorev adi da buradan gelir,
///      yani `ps` tablosunda "app" degil "winfiles" gorunur.
///   2. **`PATH`/`PATHEXT` aramasi**: listede olmayan bir ad, ortamdaki
///      dizinlerde aranir (bkz. `kernel_api::resolve_program`). Kabugun
///      gomulu listeye bagimliligi boylece bir **kolaylik** oldu,
///      zorunluluk degil -- diske atilan yeni bir ikili de calisir.
///   3. Bulunamazsa hata.
///
/// Ikinci basamak `PATHEXT`i de kapsadigi icin `run winfiles` artik iki
/// ayri yoldan calisiyor: listeden (`/bin/winfiles.exe`) ve aramadan
/// (`winfiles` + `.exe`). Liste silinse bile calismaya devam ederdi.
fn resolve<'a>(path: &str, buf: &'a mut [u8]) -> Option<(&'a str, &'static str)> {
    for (short, full, task) in KNOWN_APPS {
        if *short == path || *full == path {
            return Some((full, task));
        }
    }
    crate::level0a::kernel_api::resolve_program(path, buf).map(|found| (found, "app"))
}

/// Kabugun `apps` komutu icin kullanilabilir uygulama listesi.
pub fn available() -> &'static [(&'static str, &'static str, &'static str)] {
    KNOWN_APPS
}

/// Bir uygulamayi yeni bir gorevde Ring 3'te baslatir.
///
/// `args` kabugun yazdigi **ham komut satiri kuyrugudur** (`argv[0]`
/// haric). Windows alintilama kurallariyla bolunur, yani
/// `run notes "iki kelime"` tek bir arguman gecirir.
pub fn spawn_user_app(path: &str, args: &str) -> Result<(), &'static str> {
    spawn_user_app_id(path, args).map(|_| ())
}

/// Komut satiri kuyrugundan tasiyici blogu kurar: `argv[0]` = programin
/// kendisi, kalani alintilama kurallariyla bolunmus.
fn block_from_line(program: &str, line: &str, out: &mut [u8]) -> usize {
    let mut written = program.len().min(out.len());
    out[..written].copy_from_slice(&program.as_bytes()[..written]);
    if written < out.len() {
        out[written] = crate::level0b1::argv::SEP;
        written += 1;
    }
    written + crate::level0b1::argv::split(line, &mut out[written..])
}

/// Ayni is, ama **gorev kimligini** dondurur.
///
/// Kabuk icin kimlik gereksizdi (`run` yalnizca "basladi" der), ama
/// Win32'nin `CreateProcess`i onu dondurmek **zorunda**: cagiran hemen
/// `PROCESS_INFORMATION` yapisini dolduracak ve sonra o kimlikle
/// bekleyecek. POSIX'te ayni bilgi `fork`un donus degeriyle gelir.
pub fn spawn_user_app_id(path: &str, args: &str) -> Result<usize, &'static str> {
    spawn_inner_app(path, args, false, false)
}

/// **Beklenebilir** cocuk olarak baslatir (Win32 `CreateProcess`).
///
/// Fark cikis kodunun omrunde: siradan bir `run` sonrasinda launcher
/// yuvayi hemen geri veriyor, cunku kimse kodu sormayacak. Windows'ta
/// ise tutamac acikken cikis kodu **yasamak zorunda** --
/// `GetExitCodeProcess` surec bittikten sonra cagrilir.
///
/// POSIX tarafinda ayni isaret `fork` icin kullaniliyor; iki dunyanin
/// "cocuk kodunu ebeveyn toplar" kurali burada ayni mekanizmaya iniyor.
///
/// Argumanlar **hazir blok** olarak gelir: `execve` ve `CreateProcessA`
/// bu yolu kullanir: ikisi de kendi
/// bicimini (dizi / komut satiri) zaten bloga cevirmis durumda ve blok
/// `argv[0]`i **iceriyor**. Kabuk yolundan farki tam olarak bu -- orada
/// `argv[0]` cozulmus yoldan uretilir.
pub fn spawn_child_app_block(path: &str, block: &str) -> Result<usize, &'static str> {
    spawn_inner_app(path, block, true, true)
}

fn spawn_inner_app(
    path: &str,
    args: &str,
    args_are_block: bool,
    waitable: bool,
) -> Result<usize, &'static str> {
    let mut found = [0u8; MAX_PATH];
    let (resolved, task_name) =
        resolve(path, &mut found).ok_or("bilinmeyen uygulama ('apps'/'ls' ile listeleyin)")?;
    if resolved.len() >= MAX_PATH {
        return Err("yol cok uzun");
    }
    if args.len() >= MAX_ARGS {
        return Err("arguman cok uzun");
    }

    // Tasiyici her zaman blok (bkz. `level0b1::argv`). Kabuk yolundan
    // gelen ham satir burada bolunur; hazir blok oldugu gibi gecer.
    let mut block = [0u8; MAX_ARGS];
    let block_len = if args_are_block {
        let len = args.len().min(MAX_ARGS);
        block[..len].copy_from_slice(&args.as_bytes()[..len]);
        len
    } else {
        block_from_line(resolved, args, &mut block)
    };
    let args = core::str::from_utf8(&block[..block_len]).unwrap_or("");

    crate::arch::cpu::without_interrupts(|| unsafe {
        let head = PENDING_HEAD.load(Ordering::Relaxed);
        let next = (head + 1) % MAX_PENDING;
        if next == PENDING_TAIL.load(Ordering::Relaxed) {
            return Err("baslatma kuyrugu dolu");
        }
        let slot = (core::ptr::addr_of_mut!(PENDING) as *mut u8).add(head * MAX_PATH);
        core::ptr::copy_nonoverlapping(resolved.as_ptr(), slot, resolved.len());
        (core::ptr::addr_of_mut!(PENDING_LEN) as *mut usize)
            .add(head)
            .write(resolved.len());

        let arg_slot = (core::ptr::addr_of_mut!(PENDING_ARGS) as *mut u8).add(head * MAX_ARGS);
        core::ptr::copy_nonoverlapping(args.as_ptr(), arg_slot, args.len());
        (core::ptr::addr_of_mut!(PENDING_ARGS_LEN) as *mut usize)
            .add(head)
            .write(args.len());

        PENDING_HEAD.store(next, Ordering::Relaxed);
        Ok(())
    })?;

    let id = if waitable {
        scheduler::spawn_child(task_name, app_task)
    } else {
        scheduler::spawn(task_name, app_task)
    }
    .ok_or("gorev olusturulamadi")?;
    // Yuva geri kazanilmis olabilir: onceki sahibinden kalan bir exec
    // istegi bu surece ait degildir.
    clear_exec(id);
    crate::level0b2::ipc::post(crate::level0b2::ipc::Kind::AppStart, id, 0, 0, task_name);
    Ok(id)
}

/// Kuyruktaki yolu alir. Donen dilim static tabloyu gosterir; gorev onu
/// hemen kullanir, bu yuzden slotun ileride tekrar yazilmasi sorun degil.
fn take_pending() -> Option<(&'static str, &'static str)> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let tail = PENDING_TAIL.load(Ordering::Relaxed);
        if tail == PENDING_HEAD.load(Ordering::Relaxed) {
            return None;
        }
        let slot = (core::ptr::addr_of!(PENDING) as *const u8).add(tail * MAX_PATH);
        let len = (core::ptr::addr_of!(PENDING_LEN) as *const usize)
            .add(tail)
            .read();
        let args_slot = (core::ptr::addr_of!(PENDING_ARGS) as *const u8).add(tail * MAX_ARGS);
        let args_len = (core::ptr::addr_of!(PENDING_ARGS_LEN) as *const usize)
            .add(tail)
            .read();
        PENDING_TAIL.store((tail + 1) % MAX_PENDING, Ordering::Relaxed);
        let path = core::str::from_utf8(core::slice::from_raw_parts(slot, len)).ok()?;
        let args = core::str::from_utf8(core::slice::from_raw_parts(args_slot, args_len))
            .unwrap_or("");
        Some((path, args))
    })
}

/// Bir gorev yuvasi yeniden kullanilmadan once bekleyen `execve`
/// istegini siler.
///
/// Sahipsiz bir istek sessiz ama yikici: yuva geri kazanilip baska bir
/// surece verildiginde o surec, hicbir zaman istemedigi bir imaji
/// yuklerdi. (Olcumde tam olarak bu oldu -- bkz. README.)
pub fn clear_exec(task: usize) {
    if task >= scheduler::MAX_TASKS {
        return;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        (core::ptr::addr_of_mut!(EXEC_LEN) as *mut usize)
            .add(task)
            .write(0);
        (core::ptr::addr_of_mut!(EXEC_ARGS_LEN) as *mut usize)
            .add(task)
            .write(0);
    });
}

/// Uygulama gorevinin giris noktasi: kuyruktaki yolu alir ve Ring 3'e girer.
extern "C" fn app_task() -> ! {
    let mut next = take_pending();
    if next.is_none() {
        crate::println!("[launcher] baslatilacak uygulama bulunamadi.");
    }

    // Dongu `execve` icin: surec yerine baska bir program isterse ayni
    // gorevde, yeni bir adres uzayiyla devam edilir.
    while let Some((path, args)) = next {
        // Blok NUL icerdigi icin dogrudan basilamaz; gunluge okunabilir
        // bicimiyle, yani komut satirina cevrilmis haliyle yazilir.
        let mut shown = [0u8; MAX_ARGS];
        let shown_len = crate::level0b1::argv::join(args, &mut shown);
        let shown = core::str::from_utf8(&shown[..shown_len]).unwrap_or("");
        if crate::level0b1::argv::count(args) <= 1 {
            crate::println!("[launcher] '{}' Ring 3'te baslatiliyor.", path);
        } else {
            crate::println!("[launcher] '{}' baslatiliyor ({}).", path, shown);
        }
        let result = unsafe { crate::level0b1::process::run_from_vfs_dynamic(path, args) };
        match result {
            Ok(()) => crate::println!("[launcher] '{}' sonlandi.", path),
            Err(e) => crate::println!("[launcher] '{}' basarisiz: {:?}", path, e),
        }

        next = take_exec(scheduler::current_id());
        if let Some((p, _)) = next {
            // Yeni imaj eskinin penceresini devralmaz.
            crate::level0a::wm::close_owned_by(scheduler::current_id());
            crate::println!("[launcher] execve -> '{}'", p);
        }
    }

    // Uygulama bitti: penceresi de kapanmali. Adres uzayini `process`
    // zaten birakti.
    crate::level0a::wm::close_owned_by(scheduler::current_id());
    scheduler::terminate_current()
}
