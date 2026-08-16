//! Uygulama baslatici: bir VFS yolunu alip Ring 3'te calisacak bir
//! scheduler gorevi olarak baslatir.
//!
//! Her uygulama kendi gorevinde kosar; gorev basina ayri cekirdek yigini
//! ve Ring 3 baglami oldugu icin (bkz. `scheduler::Task`) birden fazla GUI
//! uygulamasi ayni anda calisabilir.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::{scheduler, vfs};

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
fn take_exec(task: usize) -> Option<(&'static str, &'static str)> {
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
    // Windows ikilisi: yol .exe ile biter, cekirdek bicimi magic'ten
    // anlar (bkz. vfs::format) ve PE yukleyicisine yonlendirir. Yol iki
    // mimaride de aynidir; VFS'te duran ikilinin PE32 mi PE32+ mi oldugu
    // derleme aninda secilir.
    ("winclock", "/bin/winclock.exe", "winclock"),
    ("winpad", "/bin/winpad.exe", "winpad"),
    ("winfiles", "/bin/winfiles.exe", "winfiles"),
    ("winenv", "/bin/winenv.exe", "winenv"),
];

/// Kabuktan gelen adi tam yola ve gorev adina cevirir.
fn resolve(path: &str) -> Option<(&str, &'static str)> {
    for (short, full, task) in KNOWN_APPS {
        if *short == path || *full == path {
            return Some((full, task));
        }
    }
    // Bilinen listede yok: VFS'te varsa dogrudan calistirilir.
    if vfs::lookup(path).is_some() {
        return Some((path, "app"));
    }
    None
}

/// Kabugun `apps` komutu icin kullanilabilir uygulama listesi.
pub fn available() -> &'static [(&'static str, &'static str, &'static str)] {
    KNOWN_APPS
}

/// Bir uygulamayi yeni bir gorevde Ring 3'te baslatir.
pub fn spawn_user_app(path: &str, args: &str) -> Result<(), &'static str> {
    let (resolved, task_name) =
        resolve(path).ok_or("bilinmeyen uygulama ('apps'/'ls' ile listeleyin)")?;
    if resolved.len() >= MAX_PATH {
        return Err("yol cok uzun");
    }
    if args.len() >= MAX_ARGS {
        return Err("arguman cok uzun");
    }

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

    let id = scheduler::spawn(task_name, app_task).ok_or("gorev olusturulamadi")?;
    crate::level0b2::ipc::post(crate::level0b2::ipc::Kind::AppStart, id, 0, 0, task_name);
    Ok(())
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

/// Uygulama gorevinin giris noktasi: kuyruktaki yolu alir ve Ring 3'e girer.
extern "C" fn app_task() -> ! {
    let mut next = take_pending();
    if next.is_none() {
        crate::println!("[launcher] baslatilacak uygulama bulunamadi.");
    }

    // Dongu `execve` icin: surec yerine baska bir program isterse ayni
    // gorevde, yeni bir adres uzayiyla devam edilir.
    while let Some((path, args)) = next {
        if args.is_empty() {
            crate::println!("[launcher] '{}' Ring 3'te baslatiliyor.", path);
        } else {
            crate::println!("[launcher] '{}' baslatiliyor (arguman: {}).", path, args);
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
