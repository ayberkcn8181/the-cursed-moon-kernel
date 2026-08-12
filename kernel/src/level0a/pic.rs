//! 8259A PIC remap (doc S.4).
//!
//! ## Vektor catismasi: neden slave 0x70'te
//!
//! Alisilmis yerlesim master=0x20, slave=0x28'dir; Linux da boyle yapar.
//! Ama o yerlesimde **IRQ14 vektor 46'ya** duser -- ve 46, TCMK'de
//! `int 0x2E`nin, yani **Windows NT sistem cagrisinin** vektorudur
//! (bkz. `nt_subsystem`). Ikisi ayni IDT girdisini paylasamaz: ya disk
//! kesmesi NT syscall isleyicisine duser ya da tersi.
//!
//! Ilk bakista "NT vektorunu tasi" akla gelir, ama 0x2E bir tercih degil
//! **ABI**dir: Windows ikilileri onu bekler. Tasinabilir olan taraf PIC'tir,
//! bu yuzden slave denetleyici 0x70'e alinmistir. Sonuc:
//!
//! ```text
//!   IRQ0  (PIT)      -> 32
//!   IRQ1  (klavye)   -> 33
//!   IRQ12 (fare)     -> 116
//!   IRQ14 (ATA)      -> 118
//!   int 0x80         -> 128   (Linux syscall)
//!   int 0x2E         -> 46    (NT syscall, artik kimseyle catismiyor)
//! ```
//!
//! Bu, IRQ14'un bugune kadar baglanmamis olmasinin gercek sebebiydi.

use crate::arch::cpu::outb;

const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

/// Master denetleyicinin ilk vektoru (IRQ0..7 -> 32..39).
pub const MASTER_BASE: u8 = 0x20;
/// Slave denetleyicinin ilk vektoru (IRQ8..15 -> 112..119).
pub const SLAVE_BASE: u8 = 0x70;

/// Bir IRQ hattinin IDT vektoru. Tek dogruluk kaynagi: IDT kurulumu da,
/// EOI de bu fonksiyonu kullanir.
pub const fn vector(irq: u8) -> usize {
    if irq < 8 {
        MASTER_BASE as usize + irq as usize
    } else {
        SLAVE_BASE as usize + (irq - 8) as usize
    }
}

pub const IRQ_TIMER: u8 = 0;
pub const IRQ_KEYBOARD: u8 = 1;
pub const IRQ_MOUSE: u8 = 12;
pub const IRQ_ATA_PRIMARY: u8 = 14;

pub fn remap() {
    unsafe {
        // ICW1: baslat, ICW4 gelecek
        outb(PIC1_CMD, 0x11);
        outb(PIC2_CMD, 0x11);
        // ICW2: vektor ofsetleri (yukaridaki gerekce)
        outb(PIC1_DATA, MASTER_BASE);
        outb(PIC2_DATA, SLAVE_BASE);
        // ICW3: master-slave kablolamasi (IRQ2 uzerinden slave)
        outb(PIC1_DATA, 0x04);
        outb(PIC2_DATA, 0x02);
        // ICW4: 8086 modu
        outb(PIC1_DATA, 0x01);
        outb(PIC2_DATA, 0x01);
        // Maske: IRQ0 (PIT), IRQ1 (klavye) ve IRQ2 (slave kaskadi) acik.
        // IRQ12/IRQ14 slave denetleyicidedir; kaskad hatti kapaliysa
        // onlarin kesmesi master'a hic ulasmaz -- bu klasik bir tuzaktir.
        outb(PIC1_DATA, 0xF8);
        // Slave: IRQ12 (bit 4) ve IRQ14 (bit 6) acik.
        outb(PIC2_DATA, 0xAF);
    }
}

pub fn send_eoi(irq: u8) {
    unsafe {
        if irq >= 8 {
            outb(PIC2_CMD, 0x20);
        }
        outb(PIC1_CMD, 0x20);
    }
}
