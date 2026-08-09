//! 8259A PIC remap. IRQ'lar vektor 32'den baslar (doc S.4). Faz 1'de sadece
//! IRQ0 (PIT) ve IRQ1 (klavye) maskesi acilir; digerleri Faz 4'te
//! APIC/IOAPIC gelene kadar maskeli kalir.

use crate::arch::cpu::outb;

const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

pub fn remap() {
    unsafe {
        // ICW1: baslat, ICW4 gelecek
        outb(PIC1_CMD, 0x11);
        outb(PIC2_CMD, 0x11);
        // ICW2: vektor ofsetleri (master=32, slave=40)
        outb(PIC1_DATA, 0x20);
        outb(PIC2_DATA, 0x28);
        // ICW3: master-slave kablolamasi (IRQ2 uzerinden slave)
        outb(PIC1_DATA, 0x04);
        outb(PIC2_DATA, 0x02);
        // ICW4: 8086 modu
        outb(PIC1_DATA, 0x01);
        outb(PIC2_DATA, 0x01);
        // Maske: sadece IRQ0 (PIT) ve IRQ1 (klavye) acik.
        outb(PIC1_DATA, 0xFC);
        outb(PIC2_DATA, 0xFF);
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
