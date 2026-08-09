//! Etkilesimli kabuk -- GUI icinde bir terminal penceresi.
//!
//! Kabugu cekirdek tarafinda tutmak bilincli bir tercihtir: Ring 3'te
//! calisan tam bir kabuk, satir duzenleme ve `fork/exec` gerektirir
//! (doc Faz 8/11-12). Buradaki kabuk cekirdek servislerine dogrudan
//! erisip sistemi gozlemlemeyi ve **Ring 3 uygulamalari baslatmayi**
//! saglar; boylece "uygulama calistirilabilir" iddiasi somutlasir.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::{fd, kmalloc, scheduler, vfs};
use crate::level0a::drivers::gfx;
use crate::level0a::{pit, wm};

const MAX_ROWS: usize = 24;
const MAX_COLS: usize = 78;
const PROMPT: &str = "tcmk> ";

static mut SCREEN: [[u8; MAX_COLS]; MAX_ROWS] = [[b' '; MAX_COLS]; MAX_ROWS];
static ROW: AtomicUsize = AtomicUsize::new(0);
static COL: AtomicUsize = AtomicUsize::new(0);

static mut INPUT: [u8; MAX_COLS] = [0; MAX_COLS];
static INPUT_LEN: AtomicUsize = AtomicUsize::new(0);

static WINDOW: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Kabuk penceresini olusturur.
pub fn start(x: usize, y: usize) {
    let w = MAX_COLS * gfx::font_width() + 8;
    let h = MAX_ROWS * gfx::font_height() + 6;

    match wm::create("TCMK Shell", x, y, w, h, true) {
        Some(id) => {
            WINDOW.store(id, Ordering::Relaxed);
            wm::mark_shell(id);
            wm::focus(id);
            write_line("The Cursed Moon Kernel -- etkilesimli kabuk");
            write_line("'help' yazip Enter'a basin.");
            write_line("");
            prompt();
        }
        None => crate::println!("[LEVEL-0a] shell: pencere olusturulamadi."),
    }
}

fn newline() {
    COL.store(0, Ordering::Relaxed);
    let row = ROW.load(Ordering::Relaxed);
    if row + 1 >= MAX_ROWS {
        // Yukari kaydir.
        unsafe {
            let screen = core::ptr::addr_of_mut!(SCREEN) as *mut [u8; MAX_COLS];
            for r in 1..MAX_ROWS {
                let src = screen.add(r).read();
                screen.add(r - 1).write(src);
            }
            screen.add(MAX_ROWS - 1).write([b' '; MAX_COLS]);
        }
    } else {
        ROW.store(row + 1, Ordering::Relaxed);
    }
}

fn put(ch: u8) {
    if ch == b'\n' {
        newline();
        return;
    }
    let col = COL.load(Ordering::Relaxed);
    if col >= MAX_COLS {
        newline();
    }
    let (row, col) = (ROW.load(Ordering::Relaxed), COL.load(Ordering::Relaxed));
    unsafe {
        let screen = core::ptr::addr_of_mut!(SCREEN) as *mut [u8; MAX_COLS];
        (*screen.add(row))[col] = ch;
    }
    COL.store(col + 1, Ordering::Relaxed);
}

pub fn write_str(s: &str) {
    for b in s.bytes() {
        put(b);
    }
}

pub fn write_line(s: &str) {
    write_str(s);
    newline();
}

fn prompt() {
    write_str(PROMPT);
}

/// WM'den gelen tus olayi.
pub fn on_key(ascii: u8) {
    match ascii {
        b'\n' => {
            newline();
            let len = INPUT_LEN.load(Ordering::Relaxed);
            let line = unsafe {
                let input = core::ptr::addr_of!(INPUT) as *const u8;
                core::slice::from_raw_parts(input, len)
            };
            let command = core::str::from_utf8(line).unwrap_or("");
            execute(command);
            INPUT_LEN.store(0, Ordering::Relaxed);
            prompt();
        }
        0x08 => {
            // Backspace
            let len = INPUT_LEN.load(Ordering::Relaxed);
            if len > 0 {
                INPUT_LEN.store(len - 1, Ordering::Relaxed);
                let col = COL.load(Ordering::Relaxed);
                if col > PROMPT.len() {
                    COL.store(col - 1, Ordering::Relaxed);
                    put(b' ');
                    COL.store(col - 1, Ordering::Relaxed);
                }
            }
        }
        0x20..=0x7E => {
            let len = INPUT_LEN.load(Ordering::Relaxed);
            if len < MAX_COLS - PROMPT.len() - 1 {
                unsafe {
                    let input = core::ptr::addr_of_mut!(INPUT) as *mut u8;
                    input.add(len).write(ascii);
                }
                INPUT_LEN.store(len + 1, Ordering::Relaxed);
                put(ascii);
            }
        }
        _ => {}
    }
}

/// Kucuk bir sayi bicimleyici (no_std, ayirma yok).
fn write_num(mut n: usize) {
    if n == 0 {
        put(b'0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut i = 0;
    while n > 0 {
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        put(digits[i]);
    }
}

fn execute(line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }

    // Komut ve argumani ayir.
    let (cmd, arg) = match line.find(' ') {
        Some(i) => (&line[..i], line[i + 1..].trim()),
        None => (line, ""),
    };

    match cmd {
        "help" => {
            write_line("komutlar:");
            write_line("  help          bu yardim");
            write_line("  ps            gorev listesi");
            write_line("  mem           bellek kullanimi");
            write_line("  ls            VFS icerigi");
            write_line("  cat <yol>     dosya icerigini goster");
            write_line("  run <yol>     Ring 3 uygulamasi baslat");
            write_line("  win           pencere listesi");
            write_line("  uptime        calisma suresi");
            write_line("  clear         ekrani temizle");
        }
        "ps" => {
            write_str("gorev sayisi: ");
            write_num(scheduler::task_count());
            newline();
            write_str("baglam degisimi: ");
            write_num(scheduler::switch_count());
            newline();
            write_str("calisan: ");
            write_line(scheduler::current_name());
        }
        "mem" => {
            write_str("heap kullanilan: ");
            write_num(kmalloc::used_bytes());
            write_line(" bayt");
            write_str("heap bos: ");
            write_num(kmalloc::free_bytes() / 1024);
            write_line(" KiB");
            write_str("acik dosya: ");
            write_num(fd::open_count());
            newline();
        }
        "ls" => {
            for i in 0..vfs::node_count() {
                if let Some(path) = vfs::path_of(i) {
                    write_str("  ");
                    write_str(path);
                    write_str("  ");
                    write_num(vfs::size(i).unwrap_or(0));
                    write_line(" bayt");
                }
            }
        }
        "cat" => match vfs::lookup(arg) {
            Some(node) => {
                let mut buf = [0u8; 256];
                let n = vfs::read(node, 0, &mut buf).unwrap_or(0);
                for &b in &buf[..n] {
                    put(if b == b'\n' { b'\n' } else { b });
                }
                if n > 0 && buf[n - 1] != b'\n' {
                    newline();
                }
            }
            None => write_line("dosya bulunamadi"),
        },
        "run" => {
            if arg.is_empty() {
                write_line("kullanim: run <yol>");
            } else {
                match crate::level0a::launcher::spawn_user_app(arg) {
                    Ok(()) => {
                        write_str("baslatildi: ");
                        write_line(arg);
                    }
                    Err(msg) => write_line(msg),
                }
            }
        }
        "win" => {
            for i in 0..wm::window_count() {
                if let Some(w) = wm::get(i) {
                    write_str("  #");
                    write_num(i);
                    write_str(" ");
                    let title = core::str::from_utf8(&w.title[..w.title_len]).unwrap_or("?");
                    write_str(title);
                    write_str("  ");
                    write_num(w.width);
                    write_str("x");
                    write_num(w.height);
                    newline();
                }
            }
        }
        "uptime" => {
            write_str("tick: ");
            write_num(pit::ticks() as usize);
            write_str("  (~");
            write_num(pit::ticks() as usize / 100);
            write_line(" saniye)");
        }
        "clear" => {
            unsafe {
                let screen = core::ptr::addr_of_mut!(SCREEN) as *mut [u8; MAX_COLS];
                for r in 0..MAX_ROWS {
                    screen.add(r).write([b' '; MAX_COLS]);
                }
            }
            ROW.store(0, Ordering::Relaxed);
            COL.store(0, Ordering::Relaxed);
        }
        _ => {
            write_str("bilinmeyen komut: ");
            write_line(cmd);
        }
    }
}

/// Kabuk penceresinin icerigini piksel tamponuna cizer (WM cagirir).
pub fn render(w: &wm::Window) {
    let pixels = w.buffer as *mut u32;
    unsafe {
        for i in 0..(w.width * w.height) {
            pixels.add(i).write(0x0008_0C10);
        }
    }

    let fh = gfx::font_height();
    let fw = gfx::font_width();

    unsafe {
        let screen = core::ptr::addr_of!(SCREEN) as *const [u8; MAX_COLS];
        for r in 0..MAX_ROWS {
            let row = &*screen.add(r);
            for c in 0..MAX_COLS {
                let ch = row[c];
                if ch != b' ' {
                    wm::draw_char_into_window(w, 4 + c * fw, 3 + r * fh, ch, 0x0080_F080);
                }
            }
        }
    }

    // Imlec
    let (r, c) = (ROW.load(Ordering::Relaxed), COL.load(Ordering::Relaxed));
    if c < MAX_COLS && r < MAX_ROWS && (pit::ticks() / 50) % 2 == 0 {
        wm::fill_into_window(w, 4 + c * fw, 3 + r * fh, fw, fh, 0x0040_8040);
    }
}
