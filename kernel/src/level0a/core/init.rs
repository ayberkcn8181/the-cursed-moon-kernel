//! Level-0a acilis sirasi ve cekirdek ici servis yonetimi
//! (doc S.2.2.C "Sistem Araclari: Systemd benzeri cekirdek ici servis
//! yonetimi").
//!
//! Faz 2'de servisler basit bir kayit tablosudur: her servisin adi, durumu
//! ve baslatma fonksiyonu vardir. Servisler sirayla baslatilir ve durumlari
//! Level-0b2'nin State Monitor'una raporlanabilir. Servislerin ayri gorev
//! olarak kosmasi ve bagimlilik cozumleme Faz 9+ konusudur.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::{fd, kmalloc, mmu, scheduler, tcmkfs, vfs};
use crate::level0a::drivers::{block, gfx, partition, rtc};

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

// vmm, framebuffer, kmalloc, scheduler, vfs, fd-table, rtc, block, tcmkfs
pub const MAX_SERVICES: usize = 12;

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

/// Level-0a'yi ayaga kaldirir: sayfalama -> heap -> scheduler -> vfs/fd.
///
/// `ramfs_files`, RAMFS'e baglanacak (yol, icerik) ciftleridir; cekirdek
/// imajina gomulu dosyalar (ornegin `/bin/hello`) buradan gelir.
///
/// # Safety
/// Kesmeler kapaliyken, yalnizca bir kez cagrilmalidir.
pub unsafe fn bring_up(ramfs_files: &[(&'static str, &'static [u8])]) {
    crate::println!("[LEVEL-0a] Executor katmani baslatiliyor...");

    mmu::init();

    // DIKKAT -- SIRA KRITIK: framebuffer cekirdegin identity map'inin
    // DISINDADIR (tipik olarak ~0xFD000000). Sayfalama acildiktan sonraki
    // ILK `println!` bile framebuffer konsoluna cizmeye calisir; esleme
    // yapilmadan once hicbir sey yazdirilmamalidir, aksi halde page fault
    // -> triple fault.
    let fb_mapped = match gfx::physical_range() {
        Some((phys, len)) => {
            #[cfg(target_arch = "x86")]
            {
                mmu::map_mmio(phys, len)
            }
            // x86_64'te ust 1 GiB (0xC0000000+) boot stub'inda zaten eslendi.
            #[cfg(target_arch = "x86_64")]
            {
                let _ = (phys, len);
                true
            }
        }
        None => false,
    };

    // Bu noktadan itibaren yazdirmak guvenli.
    register("vmm", mmu::is_enabled());
    if gfx::available() {
        register("framebuffer", fb_mapped);
    }

    // Heap'in gercekten yazilabilir oldugunu tahsis ederek dogrula.
    let probe = kmalloc::kmalloc(64);
    register("kmalloc", probe.is_some());

    scheduler::init();
    register("scheduler", scheduler::task_count() == 1);

    let mut mounted = 0;
    for (path, data) in ramfs_files {
        if vfs::mount_static(path, data).is_some() {
            mounted += 1;
        }
    }
    register("vfs", mounted == ramfs_files.len() && vfs::node_count() == mounted);

    fd::init();
    // Acilista stdin/stdout/stderr disinda acik tanimlayici olmamali.
    register("fd-table", fd::open_count() == 0);

    // Gercek zaman saati: dosya zaman damgalari ve kullaniciya gosterilen
    // saat icin. Yoksa sistem calisir, yalnizca tarih bilinmez.
    if rtc::init() {
        register("rtc", true);
        let t = rtc::now().unwrap_or_default();
        crate::println!(
            "[LEVEL-0a] saat: {:04}-{:02}-{:02} {:02}:{:02}:{:02} (UTC)",
            t.year, t.month, t.day, t.hour, t.minute, t.second
        );
    } else {
        crate::println!("[LEVEL-0a] saat: CMOS RTC okunamadi, tarih bilinmiyor.");
    }

    // Disk: aygit + bolum tablosu. Disk yoksa servis kaydedilmez --
    // TCMK ISO'dan (disksiz) da acilabilmelidir.
    if block::init() {
        register("block", true);
        crate::println!(
            "[LEVEL-0a] disk: {} '{}' {} sektor ({} MiB)",
            block::name(),
            crate::level0a::drivers::ata::model(),
            block::sector_count(),
            block::capacity_mib()
        );
        let parts = partition::scan();
        for i in 0..parts {
            if let Some(p) = partition::get(i) {
                crate::println!(
                    "[LEVEL-0a] disk: bolum {} tur=0x{:02x} ({}) lba={} sektor={}{}",
                    p.index + 1,
                    p.kind,
                    partition::type_name(p.kind),
                    p.start_lba,
                    p.sectors,
                    if p.bootable { " [acilis]" } else { "" }
                );
            }
        }
    } else {
        crate::println!("[LEVEL-0a] disk: ATA aygiti bulunamadi (salt RAMFS modu).");
    }

    // Kalici bolumu bagla. Bicimlendirilmemis olmasi bir hata degildir:
    // TCMK ilk acilista bos bir diskle karsilasabilir, kullanici kabuktan
    // `format onayla` der. Bu yuzden servis yalnizca gercekten
    // baglandiginda kaydedilir.
    if partition::tcmkfs().is_some() {
        match tcmkfs::mount() {
            Ok(()) => {
                let files = vfs::mount_disk_files();
                register("tcmkfs", true);
                crate::println!(
                    "[LEVEL-0a] tcmkfs: baglandi -- {} dosya, {} KiB / {} KiB kullanimda",
                    files,
                    tcmkfs::used_kib(),
                    tcmkfs::total_kib()
                );
            }
            Err(e) => crate::println!(
                "[LEVEL-0a] tcmkfs: baglanamadi -- {}",
                tcmkfs::error_name(e)
            ),
        }
    }

    vfs::list();

    // Ortam tablosu: dosya sistemi bagliktan **sonra** kurulur, cunku
    // `HOME` gercekten var olan bir dizini gostermeli.
    super::env::init();
    crate::println!(
        "[LEVEL-0a] ortam: {} degisken (HOME={})",
        super::env::count(super::env::SESSION),
        super::env::get(super::env::SESSION, "HOME").unwrap_or("?")
    );

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

    // Level-0b2'ye acilisi bildir (doc S.10): durum paylasimli bolgeye
    // yazilir, olay mesaj kuyruguna birakilir.
    let shared = &crate::level0b2::ipc::SHARED;
    shared.level0a_status.store(
        if all_services_active() { 1 } else { 2 },
        Ordering::Relaxed,
    );
    crate::level0b2::ipc::post(
        crate::level0b2::ipc::Kind::BootComplete,
        0,
        service_count(),
        0,
        "Level-0a hazir",
    );
}

/// Kayitli servisin adi ve durumu (kabuk `svc` komutu).
pub fn service(index: usize) -> Option<Service> {
    if index >= SERVICE_COUNT.load(Ordering::Relaxed) {
        return None;
    }
    unsafe {
        let services = core::ptr::addr_of!(SERVICES) as *const Service;
        Some(*services.add(index))
    }
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
