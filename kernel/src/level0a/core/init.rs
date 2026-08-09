//! Level-0a acilis sirasi ve cekirdek ici servis yonetimi
//! (doc S.2.2.C "Sistem Araclari: Systemd benzeri cekirdek ici servis
//! yonetimi").
//!
//! Faz 2'de servisler basit bir kayit tablosudur: her servisin adi, durumu
//! ve baslatma fonksiyonu vardir. Servisler sirayla baslatilir ve durumlari
//! Level-0b2'nin State Monitor'una raporlanabilir. Servislerin ayri gorev
//! olarak kosmasi ve bagimlilik cozumleme Faz 9+ konusudur.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::{kmalloc, mmu, scheduler};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServiceState {
    Inactive,
    Active,
    Failed,
}

#[derive(Clone, Copy)]
pub struct Service {
    pub name: &'static str,
    pub state: ServiceState,
}

pub const MAX_SERVICES: usize = 8;

static mut SERVICES: [Service; MAX_SERVICES] = [Service {
    name: "",
    state: ServiceState::Inactive,
}; MAX_SERVICES];
static SERVICE_COUNT: AtomicUsize = AtomicUsize::new(0);

fn register(name: &'static str, ok: bool) {
    let index = SERVICE_COUNT.load(Ordering::Relaxed);
    if index >= MAX_SERVICES {
        return;
    }
    unsafe {
        let services = core::ptr::addr_of_mut!(SERVICES) as *mut Service;
        (*services.add(index)).name = name;
        (*services.add(index)).state = if ok {
            ServiceState::Active
        } else {
            ServiceState::Failed
        };
    }
    SERVICE_COUNT.store(index + 1, Ordering::Relaxed);

    crate::println!(
        "[LEVEL-0a] servis {:<12} {}",
        name,
        if ok { "[ACTIVE]" } else { "[FAILED]" }
    );
}

/// Level-0a'yi ayaga kaldirir: sayfalama -> heap -> scheduler.
///
/// # Safety
/// Kesmeler kapaliyken, yalnizca bir kez cagrilmalidir.
pub unsafe fn bring_up() {
    crate::println!("[LEVEL-0a] Executor katmani baslatiliyor...");

    mmu::init();
    register("vmm", mmu::is_enabled());

    // Heap'in gercekten yazilabilir oldugunu tahsis ederek dogrula.
    let probe = kmalloc::kmalloc(64);
    register("kmalloc", probe.is_some());

    scheduler::init();
    register("scheduler", scheduler::task_count() == 1);

    crate::println!(
        "[LEVEL-0a] paging={} identity={} MiB | heap {} B kullanildi, {} KiB bos",
        if mmu::is_enabled() { "acik" } else { "KAPALI" },
        mmu::identity_mapped_bytes() / (1024 * 1024),
        kmalloc::used_bytes(),
        kmalloc::free_bytes() / 1024
    );
    crate::println!(
        "[LEVEL-0a] {} servis kayitli, hepsi aktif: {}",
        service_count(),
        if all_services_active() { "evet" } else { "HAYIR" }
    );
}

pub fn service_count() -> usize {
    SERVICE_COUNT.load(Ordering::Relaxed)
}

/// Tum kayitli servisler `Active` mi? Level-0b2 State Monitor bunu
/// saglik raporunda kullanir.
pub fn all_services_active() -> bool {
    let count = SERVICE_COUNT.load(Ordering::Relaxed);
    unsafe {
        let services = core::ptr::addr_of!(SERVICES) as *const Service;
        (0..count).all(|i| (*services.add(i)).state == ServiceState::Active)
    }
}
