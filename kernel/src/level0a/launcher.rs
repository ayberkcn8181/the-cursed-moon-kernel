//! Uygulama baslatici: bir VFS yolunu alip Ring 3'te calisacak bir
//! scheduler gorevi olarak baslatir.
//!
//! Her uygulama kendi gorevinde kosar; gorev basina ayri cekirdek yigini
//! ve Ring 3 baglami oldugu icin (bkz. `scheduler::Task`) birden fazla GUI
//! uygulamasi ayni anda calisabilir.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::scheduler;

/// Baslatilmayi bekleyen uygulamanin yolu. Gorev girisi `extern "C" fn()`
/// oldugu icin arguman gecirilemiyor; yol bu slot uzerinden aktarilir.
const MAX_PENDING: usize = 8;
static mut PENDING: [Option<&'static str>; MAX_PENDING] = [None; MAX_PENDING];
static PENDING_HEAD: AtomicUsize = AtomicUsize::new(0);
static PENDING_TAIL: AtomicUsize = AtomicUsize::new(0);

/// Kabuktan gelen yol, sabit bir listeyle eslestirilir. Dinamik dize
/// sahipligi (heap'te string) yerine bu yontem secildi: cekirdekte
/// ayirmali dize yonetimi Faz 9+ konusudur.
static KNOWN_APPS: &[(&str, &str, &str)] = &[
    // (kisa ad, tam yol, gorev adi)
    ("paint", "/bin/paint", "paint"),
    ("plasma", "/bin/plasma", "plasma"),
    ("crash", "/bin/crash", "crash"),
    ("hog", "/bin/hog", "hog"),
];

fn resolve(path: &str) -> Option<(&'static str, &'static str)> {
    for (short, full, task) in KNOWN_APPS {
        if *short == path || *full == path {
            return Some((full, task));
        }
    }
    None
}

/// Kabugun `apps` komutu icin kullanilabilir uygulama listesi.
pub fn available() -> &'static [(&'static str, &'static str, &'static str)] {
    KNOWN_APPS
}

/// Bir uygulamayi yeni bir gorevde Ring 3'te baslatir.
pub fn spawn_user_app(path: &str) -> Result<(), &'static str> {
    let (resolved, task_name) = resolve(path).ok_or("bilinmeyen uygulama ('apps' ile listeleyin)")?;

    crate::arch::cpu::without_interrupts(|| unsafe {
        let head = PENDING_HEAD.load(Ordering::Relaxed);
        let next = (head + 1) % MAX_PENDING;
        if next == PENDING_TAIL.load(Ordering::Relaxed) {
            return Err("baslatma kuyrugu dolu");
        }
        let pending = core::ptr::addr_of_mut!(PENDING) as *mut Option<&'static str>;
        pending.add(head).write(Some(resolved));
        PENDING_HEAD.store(next, Ordering::Relaxed);
        Ok(())
    })?;

    let id = scheduler::spawn(task_name, app_task).ok_or("gorev olusturulamadi")?;
    crate::level0b2::ipc::post(
        crate::level0b2::ipc::Kind::AppStart,
        id,
        0,
        0,
        task_name,
    );
    Ok(())
}

fn take_pending() -> Option<&'static str> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let tail = PENDING_TAIL.load(Ordering::Relaxed);
        if tail == PENDING_HEAD.load(Ordering::Relaxed) {
            return None;
        }
        let pending = core::ptr::addr_of_mut!(PENDING) as *mut Option<&'static str>;
        let path = pending.add(tail).replace(None);
        PENDING_TAIL.store((tail + 1) % MAX_PENDING, Ordering::Relaxed);
        path
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
