//! CPU istisnalari ve **hata izolasyonu**.
//!
//! Faz 1-14 boyunca yalnizca vektor 0 baglilydi; sonuc olarak Ring 3'teki
//! bir uygulamanin urettigi herhangi bir page fault veya GPF, CPU'nun
//! gidecek yeri olmadigi icin **triple fault** ve yeniden baslatma
//! demekti. Yani hatali tek bir uygulama tum sistemi dusuruyordu.
//!
//! Bu modul bunu duzeltir ve dokumandaki hata tolerans vaadini gercek
//! kilar (doc S.11):
//!
//!   * Hata **Ring 3'ten** geldiyse: tani yazilir, **yalnizca o surec**
//!     sonlandirilir, sistem calismaya devam eder.
//!   * Hata **Ring 0'dan** geldiyse: bu bir cekirdek hatasidir; Level-0b2
//!     Fallback Interface devreye girer.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Istisna adlari (Intel SDM vektor sirasi).
pub const NAMES: [&str; 32] = [
    "divide-by-zero",
    "debug",
    "NMI",
    "breakpoint",
    "overflow",
    "bound-range",
    "invalid-opcode",
    "device-not-available",
    "double-fault",
    "coprocessor-overrun",
    "invalid-TSS",
    "segment-not-present",
    "stack-fault",
    "general-protection",
    "page-fault",
    "reserved-15",
    "x87-fp",
    "alignment-check",
    "machine-check",
    "simd-fp",
    "virtualization",
    "control-protection",
    "reserved-22",
    "reserved-23",
    "reserved-24",
    "reserved-25",
    "reserved-26",
    "reserved-27",
    "hypervisor-injection",
    "vmm-communication",
    "security",
    "reserved-31",
];

/// Toplam yakalanan istisna sayisi ve sonlandirilan surec sayisi --
/// kabuktaki `faults` komutu bunlari gosterir.
static TOTAL: AtomicUsize = AtomicUsize::new(0);
static KILLED_PROCESSES: AtomicUsize = AtomicUsize::new(0);
static LAST_VECTOR: AtomicUsize = AtomicUsize::new(usize::MAX);

pub fn total() -> usize {
    TOTAL.load(Ordering::Relaxed)
}

pub fn killed_processes() -> usize {
    KILLED_PROCESSES.load(Ordering::Relaxed)
}

pub fn last_vector() -> Option<usize> {
    let v = LAST_VECTOR.load(Ordering::Relaxed);
    if v == usize::MAX {
        None
    } else {
        Some(v)
    }
}

/// Page fault hata kodunun anlamini cozer (Intel SDM 4.7).
fn describe_page_fault(error_code: usize) -> (&'static str, &'static str, &'static str) {
    let present = if error_code & 1 != 0 {
        "koruma-ihlali"
    } else {
        "sayfa-yok"
    };
    let access = if error_code & 2 != 0 { "yazma" } else { "okuma" };
    let ring = if error_code & 4 != 0 { "Ring 3" } else { "Ring 0" };
    (present, access, ring)
}

/// Tum istisnalarin ortak govdesi.
///
/// `from_user`: hata Ring 3'ten mi geldi (CS'in RPL'i 3 mu)?
/// `fault_addr`: page fault icin CR2, digerleri icin 0.
pub fn handle(vector: usize, error_code: usize, instruction_ptr: usize, from_user: bool, fault_addr: usize) -> ! {
    TOTAL.fetch_add(1, Ordering::Relaxed);
    LAST_VECTOR.store(vector, Ordering::Relaxed);

    let name = NAMES.get(vector).copied().unwrap_or("bilinmeyen");

    crate::println!();
    crate::println!(
        "[LEVEL-0b2] ISTISNA #{} ({}) -- {} kaynakli",
        vector,
        name,
        if from_user { "Ring 3" } else { "Ring 0" }
    );
    crate::println!(
        "            IP=0x{:08x}  hata_kodu=0x{:x}",
        instruction_ptr,
        error_code
    );

    if vector == 14 {
        let (present, access, ring) = describe_page_fault(error_code);
        crate::println!(
            "            adres=0x{:08x}  {} / {} / {}",
            fault_addr,
            present,
            access,
            ring
        );
    }

    if from_user {
        // --- HATA IZOLASYONU ---
        // Surec olduruluyor, sistem ayakta kaliyor. Bu, TCMK'nin
        // "yuksek hata toleransi" iddiasinin en somut noktasi.
        KILLED_PROCESSES.fetch_add(1, Ordering::Relaxed);
        crate::println!(
            "[LEVEL-0b2] Surec sonlandirildi; sistem calismaya devam ediyor."
        );
        crate::println!();

        // Gorevin saklanmis cekirdek baglamina geri don. Bu, kesme
        // cercevesini terk eder -- hatali kullanici koduna asla donulmez.
        unsafe { crate::arch::cpu::usermode::leave_user_mode() }
    }

    // --- CEKIRDEK HATASI ---
    // Buradan guvenli bir sekilde devam edilemez.
    crate::level0b2::fallback::emergency(&[
        "Cekirdek baglaminda istisna -- kurtarilamaz.",
        "Fallback Interface: sistem guvenli duruma aliniyor.",
    ]);

    loop {
        crate::arch::cpu::disable_interrupts();
        crate::arch::cpu::halt();
    }
}
