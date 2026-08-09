//! Klavye suruculugu (IRQ1): port 0x60'tan scancode set 1 okunur, event-driven
//! G/C ile ekrana yansitilir (doc S.4). Faz 1'de sadece temel US duzeni
//! (kucuk harf) desteklenir; shift/caps Faz 2+'ya birakilir.

use crate::arch::cpu::inb;

const DATA_PORT: u16 = 0x60;

#[rustfmt::skip]
const SCANCODE_ASCII: [u8; 128] = [
    0, 0, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'=', 0x08, b'\t',
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p', b'[', b']', b'\n', 0,
    b'a', b's', b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';', b'\'', b'`', 0, b'\\',
    b'z', b'x', b'c', b'v', b'b', b'n', b'm', b',', b'.', b'/', 0, b'*', 0, b' ',
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// IRQ1 handler'i tarafindan cagrilir.
pub fn on_irq() {
    let scancode = unsafe { inb(DATA_PORT) };

    // Bit 7 = 1 -> tus birakma (release); Faz 1'de yok sayilir.
    if scancode & 0x80 != 0 {
        return;
    }

    let ch = SCANCODE_ASCII[scancode as usize];
    if ch != 0 {
        crate::print!("{}", ch as char);
    }
}
