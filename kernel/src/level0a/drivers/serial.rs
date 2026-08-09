//! COM1 (16550 UART, port 0x3F8) -- VGA ciktisinin yaninda kolay
//! otomatik dogrulama icin (doc S.8'deki `qemu ... -serial stdio`
//! komutuyla eslesir). Faz 1'de sadece polling tabanli yazma vardir.

use crate::arch::cpu::{inb, outb};

const COM1: u16 = 0x3F8;

pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00); // kesmeleri kapat
        outb(COM1 + 3, 0x80); // DLAB ac
        outb(COM1, 0x03); // 38400 baud, bolen dusuk bayt
        outb(COM1 + 1, 0x00); // bolen yuksek bayt
        outb(COM1 + 3, 0x03); // 8N1
        outb(COM1 + 2, 0xC7); // FIFO ac, temizle, 14-bayt esik
        outb(COM1 + 4, 0x0B); // IRQ'lar acik, RTS/DSR set
    }
}

fn transmit_ready() -> bool {
    unsafe { inb(COM1 + 5) & 0x20 != 0 }
}

/// Belirli sayida deneme icinde THRE bitini bekler; hic gelmezse bu bayti
/// sessizce atlar. VGA birincil/guvenilir cikis oldugundan seri port sadece
/// "best-effort" bir gozlem kanali -- donanim/emulator zamanlama
/// tuhafliklarinda cekirdegi sonsuza kadar kilitlemesi kabul edilemez.
const MAX_SPIN: u32 = 100_000;

pub fn write_byte(byte: u8) {
    let mut spins = 0;
    while !transmit_ready() {
        spins += 1;
        if spins >= MAX_SPIN {
            return;
        }
    }
    unsafe { outb(COM1, byte) };
}

pub fn write_str(s: &str) {
    for b in s.bytes() {
        write_byte(b);
    }
}
