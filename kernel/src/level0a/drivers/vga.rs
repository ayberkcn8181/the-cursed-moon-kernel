//! VGA metin modu (80x25) suruculugu (doc S.2.2.C, S.4). Kesme handler'lari
//! (klavye) da yaziyor olabileceginden erisim `without_interrupts` ile
//! korunur -- tek cekirdek oldugu icin bu, harici bir spinlock crate'ine
//! ihtiyac birakmadan yeterli mutual exclusion saglar.

use core::cell::UnsafeCell;
use core::fmt;
use core::fmt::Write;

const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;
const VGA_BUFFER_ADDR: usize = 0xB8000;
const DEFAULT_ATTR: u8 = 0x0F; // siyah zemin, beyaz yazi

struct Writer {
    col: usize,
    row: usize,
}

impl Writer {
    const fn new() -> Self {
        Writer { col: 0, row: 0 }
    }

    fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            byte => {
                if self.col >= VGA_WIDTH {
                    self.new_line();
                }
                self.put_at(self.row, self.col, byte);
                self.col += 1;
            }
        }
    }

    fn put_at(&mut self, row: usize, col: usize, byte: u8) {
        let offset = (row * VGA_WIDTH + col) * 2;
        unsafe {
            let ptr = (VGA_BUFFER_ADDR + offset) as *mut u8;
            core::ptr::write_volatile(ptr, byte);
            core::ptr::write_volatile(ptr.add(1), DEFAULT_ATTR);
        }
    }

    fn new_line(&mut self) {
        self.col = 0;
        if self.row + 1 >= VGA_HEIGHT {
            self.scroll();
        } else {
            self.row += 1;
        }
    }

    fn scroll(&mut self) {
        unsafe {
            for row in 1..VGA_HEIGHT {
                for col in 0..VGA_WIDTH {
                    let src = (VGA_BUFFER_ADDR + (row * VGA_WIDTH + col) * 2) as *const u16;
                    let dst = (VGA_BUFFER_ADDR + ((row - 1) * VGA_WIDTH + col) * 2) as *mut u16;
                    let val = core::ptr::read_volatile(src);
                    core::ptr::write_volatile(dst, val);
                }
            }
            let blank: u16 = ((DEFAULT_ATTR as u16) << 8) | (b' ' as u16);
            for col in 0..VGA_WIDTH {
                let dst = (VGA_BUFFER_ADDR + ((VGA_HEIGHT - 1) * VGA_WIDTH + col) * 2) as *mut u16;
                core::ptr::write_volatile(dst, blank);
            }
        }
    }

    fn clear(&mut self) {
        unsafe {
            let blank: u16 = ((DEFAULT_ATTR as u16) << 8) | (b' ' as u16);
            for i in 0..(VGA_WIDTH * VGA_HEIGHT) {
                let dst = (VGA_BUFFER_ADDR + i * 2) as *mut u16;
                core::ptr::write_volatile(dst, blank);
            }
        }
        self.row = 0;
        self.col = 0;
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            match byte {
                0x20..=0x7e | b'\n' => self.write_byte(byte),
                _ => self.write_byte(0xfe),
            }
        }
        Ok(())
    }
}

struct WriterCell(UnsafeCell<Writer>);
unsafe impl Sync for WriterCell {}

static WRITER: WriterCell = WriterCell(UnsafeCell::new(Writer::new()));

pub fn init() {
    crate::arch::cpu::without_interrupts(|| unsafe {
        (*WRITER.0.get()).clear();
    });
}

/// VGA'ya ve (otomatik test/gozlem icin) COM1 seri porta ayni metni yazar.
struct DualWriter;

impl fmt::Write for DualWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        unsafe { (*WRITER.0.get()).write_str(s)? };
        super::serial::write_str(s);
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    crate::arch::cpu::without_interrupts(|| {
        let _ = DualWriter.write_fmt(args);
    });
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::level0a::drivers::vga::_print(core::format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", core::format_args!($($arg)*)));
}
