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
static PENDING_HEAD: AtomicUsize = AtomicUsize::new(0);
static PENDING_TAIL: AtomicUsize = AtomicUsize::new(0);

/// Kisa adlar: `run paint` yazabilmek icin.
///
/// Listede olmayan bir yol da calistirilabilir -- VFS'te varsa yeter.
/// Boylece diske kopyalanan ("kurulan") uygulamalar cekirdegi yeniden
/// derlemeden calisir.
static KNOWN_APPS: &[(&str, &str, &str)] = &[
    // (kisa ad, tam yol, gorev adi)
    ("paint", "/bin/paint", "paint"),
    ("plasma", "/bin/plasma", "plasma"),
    ("crash", "/bin/crash", "crash"),
    ("hog", "/bin/hog", "hog"),
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
pub fn spawn_user_app(path: &str) -> Result<(), &'static str> {
    let (resolved, task_name) =
        resolve(path).ok_or("bilinmeyen uygulama ('apps'/'ls' ile listeleyin)")?;
    if resolved.len() >= MAX_PATH {
        return Err("yol cok uzun");
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
        PENDING_HEAD.store(next, Ordering::Relaxed);
        Ok(())
    })?;

    let id = scheduler::spawn(task_name, app_task).ok_or("gorev olusturulamadi")?;
    crate::level0b2::ipc::post(crate::level0b2::ipc::Kind::AppStart, id, 0, 0, task_name);
    Ok(())
}

/// Kuyruktaki yolu alir. Donen dilim static tabloyu gosterir; gorev onu
/// hemen kullanir, bu yuzden slotun ileride tekrar yazilmasi sorun degil.
fn take_pending() -> Option<&'static str> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let tail = PENDING_TAIL.load(Ordering::Relaxed);
        if tail == PENDING_HEAD.load(Ordering::Relaxed) {
            return None;
        }
        let slot = (core::ptr::addr_of!(PENDING) as *const u8).add(tail * MAX_PATH);
        let len = (core::ptr::addr_of!(PENDING_LEN) as *const usize)
            .add(tail)
            .read();
        PENDING_TAIL.store((tail + 1) % MAX_PENDING, Ordering::Relaxed);
        core::str::from_utf8(core::slice::from_raw_parts(slot, len)).ok()
    })
}

/// Uygulama gorevinin giris noktasi: kuyruktaki yolu alir ve Ring 3'e girer.
extern "C" fn app_task() -> ! {
    let path = take_pending();

    match path {
        Some(p) => {
            crate::println!("[launcher] '{}' Ring 3'te baslatiliyor.", p);
            let result = unsafe { crate::level0b1::process::run_from_vfs_dynamic(p) };
            match result {
                Ok(()) => crate::println!("[launcher] '{}' sonlandi.", p),
                Err(e) => crate::println!("[launcher] '{}' basarisiz: {:?}", p, e),
            }
        }
        None => crate::println!("[launcher] baslatilacak uygulama bulunamadi."),
    }

    scheduler::terminate_current()
}
