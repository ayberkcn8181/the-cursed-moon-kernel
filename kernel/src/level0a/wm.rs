//! Pencere Yoneticisi ve kompozitor (doc S.7 Faz 14: "basit WM").
//!
//! Her pencerenin kendi piksel tamponu vardir; uygulamalar oraya cizer ve
//! `flush` ile "hazir" isaretler. Kompozitor tum pencereleri z-sirasina
//! gore arka tampona cizer, uzerine fare imlecini koyar ve tek seferde
//! ekrana aktarir.
//!
//! Pencere tamponlari kmalloc'tan gelir ve **Ring 3'e acilir**, boylece
//! kullanici uygulamalari dogrudan kendi piksellerini yazabilir (cekirdek
//! araciligi olmadan cizim = hizli).

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::level0a::core::{kmalloc, mmu, scheduler};
use crate::level0a::drivers::{console, gfx};
use crate::level0a::input::{self, Event};

pub const MAX_WINDOWS: usize = 8;
pub const TITLE_HEIGHT: usize = 20;
pub const BORDER: usize = 1;
const MAX_TITLE: usize = 32;

#[derive(Clone, Copy)]
pub struct Window {
    pub used: bool,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    /// Icerik alaninin piksel tamponu (width * height).
    pub buffer: usize,
    pub title: [u8; MAX_TITLE],
    pub title_len: usize,
    /// Cekirdek tarafindan cizilen pencere mi (ornegin sistem gunlugu)?
    pub system: bool,
    /// Pencereyi acan gorev. Tampon yalnizca ONA eslenir.
    pub owner: usize,
    /// Tamponun sahibinin adres uzayindaki adresi (0 = eslenmedi).
    pub user_addr: usize,
}

impl Window {
    const fn empty() -> Self {
        Window {
            used: false,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            buffer: 0,
            title: [0; MAX_TITLE],
            title_len: 0,
            system: false,
            owner: usize::MAX,
            user_addr: 0,
        }
    }
}

/// Kullanici adres uzayinda pencere tamponlarina ayrilan bolge.
///
/// Kullanici PDE'si 0x00C00000..0x00FFFFFF'i kapsar; imaj ve yigin
/// bastaki 512 KiB'de (`USER_MAP_SIZE`) durur, tamponlar 13 MiB'den
/// itibaren. Her surec kendi adres uzayinda oldugu icin farkli surecler
/// ayni adresi kullanabilir -- catisma yalnizca **tek** surecin cok
/// pencere acmasi halinde olurdu, o yuzden surec basina slot verilir.
const WINDOW_MAP_BASE: usize = 0x00D0_0000;
const WINDOW_MAP_SLOT: usize = 0x0008_0000; // 512 KiB
const WINDOW_MAP_SLOTS: usize = 4;

static mut WINDOWS: [Window; MAX_WINDOWS] = [Window::empty(); MAX_WINDOWS];
/// z-sirasi: en arkadan en one. Deger = pencere indeksi.
static mut Z_ORDER: [usize; MAX_WINDOWS] = [0; MAX_WINDOWS];
static WINDOW_COUNT: AtomicUsize = AtomicUsize::new(0);
static FOCUSED: AtomicUsize = AtomicUsize::new(usize::MAX);
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Suruklenen pencere ve fare ile pencere kosesi arasindaki fark.
static DRAG_WINDOW: AtomicUsize = AtomicUsize::new(usize::MAX);
static DRAG_DX: AtomicUsize = AtomicUsize::new(0);
static DRAG_DY: AtomicUsize = AtomicUsize::new(0);
/// Kabuk penceresinin kimligi (icerigini kabuk modulu cizer).
static SHELL_WINDOW: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Bir gorevin halihazirda kac penceresi var (kullanici slot secimi icin).
fn owned_window_count(owner: usize) -> usize {
    let count = WINDOW_COUNT.load(Ordering::Relaxed);
    unsafe {
        let windows = core::ptr::addr_of!(WINDOWS) as *const Window;
        (0..count)
            .filter(|i| {
                let w = &*windows.add(*i);
                w.used && !w.system && w.owner == owner
            })
            .count()
    }
}

pub fn mark_shell(id: usize) {
    SHELL_WINDOW.store(id, Ordering::Relaxed);
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// GUI'yi baslatir: konsol ekrandan cekilir, WM devralir.
pub fn start() {
    if !gfx::available() {
        crate::println!("[LEVEL-0a] wm: framebuffer yok, GUI baslatilamiyor.");
        return;
    }
    console::wm_take_over();
    ACTIVE.store(true, Ordering::Relaxed);
    crate::println!("[LEVEL-0a] wm: pencere yoneticisi devrede.");
}

/// Yeni pencere olusturur; icerik tamponu Ring 3'e acilir.
pub fn create(title: &str, x: usize, y: usize, width: usize, height: usize, system: bool) -> Option<usize> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let count = WINDOW_COUNT.load(Ordering::Relaxed);
        if count >= MAX_WINDOWS || width == 0 || height == 0 {
            return None;
        }

        let bytes = width * height * 4;
        let buffer = kmalloc::kmalloc_aligned(bytes, 4096)?;
        // Tamponu beyaza boya.
        let pixels = buffer as *mut u32;
        for i in 0..(width * height) {
            pixels.add(i).write(gfx::WINDOW_BG);
        }

        // --- Tamponu YALNIZCA sahibine ac ---
        //
        // Eskiden `protect_user_range` tamponu identity haritasinda Ring 3'e
        // aciyordu; bu, her uygulamanin her pencerenin piksellerini
        // okuyabilmesi demekti. Artik tampon sahibinin adres uzayina, onun
        // kendi sanal adresine eslenir. Cekirdek ayni bellegi identity
        // adresinden gormeye devam eder (kompozitor oradan okur).
        let owner = scheduler::current_id();
        let space = scheduler::address_space_of(owner);
        let mut user_addr = 0usize;

        if space != 0 && !system {
            let slot = owned_window_count(owner);
            if slot >= WINDOW_MAP_SLOTS || bytes > WINDOW_MAP_SLOT {
                return None;
            }
            let vaddr = WINDOW_MAP_BASE + slot * WINDOW_MAP_SLOT;
            if !mmu::map_user_frames(space, vaddr, buffer as usize, bytes) {
                return None;
            }
            user_addr = vaddr;
        } else if !system {
            // Paylasimli model (x86_64) veya adres uzayi olmayan cagiran:
            // eski davranis.
            mmu::protect_user_range(buffer as usize, bytes);
            user_addr = buffer as usize;
        }

        let windows = core::ptr::addr_of_mut!(WINDOWS) as *mut Window;
        let mut win = Window {
            used: true,
            x,
            y,
            width,
            height,
            buffer: buffer as usize,
            title: [0; MAX_TITLE],
            title_len: 0,
            system,
            owner,
            user_addr,
        };
        for (i, b) in title.bytes().take(MAX_TITLE).enumerate() {
            win.title[i] = b;
            win.title_len = i + 1;
        }
        windows.add(count).write(win);

        let z = core::ptr::addr_of_mut!(Z_ORDER) as *mut usize;
        z.add(count).write(count);

        WINDOW_COUNT.store(count + 1, Ordering::Relaxed);
        // Yeni pencere odagi CALMAZ: aksi halde arka planda acilan bir
        // uygulama, kullanicinin yazdigi kabuktan odagi kapiyor. Odak
        // yalnizca tiklama ile (veya `focus()` ile acikca) degisir.
        if FOCUSED.load(Ordering::Relaxed) == usize::MAX {
            FOCUSED.store(count, Ordering::Relaxed);
        }
        Some(count)
    })
}

pub fn window_count() -> usize {
    WINDOW_COUNT.load(Ordering::Relaxed)
}

pub fn get(index: usize) -> Option<Window> {
    if index >= WINDOW_COUNT.load(Ordering::Relaxed) {
        return None;
    }
    unsafe {
        let windows = core::ptr::addr_of!(WINDOWS) as *const Window;
        Some(*windows.add(index))
    }
}

/// Bir pencereyi z-sirasinin en ustune tasir ve odagi verir.
fn raise(index: usize) {
    let count = WINDOW_COUNT.load(Ordering::Relaxed);
    unsafe {
        let z = core::ptr::addr_of_mut!(Z_ORDER) as *mut usize;
        let mut pos = None;
        for i in 0..count {
            if z.add(i).read() == index {
                pos = Some(i);
                break;
            }
        }
        if let Some(p) = pos {
            for i in p..count.saturating_sub(1) {
                let next = z.add(i + 1).read();
                z.add(i).write(next);
            }
            z.add(count - 1).write(index);
        }
    }
    FOCUSED.store(index, Ordering::Relaxed);
}

/// Odagi acikca bir pencereye verir (ornegin kabuk acilirken).
pub fn focus(index: usize) {
    if index < WINDOW_COUNT.load(Ordering::Relaxed) {
        raise(index);
    }
}

pub fn focused() -> Option<usize> {
    let f = FOCUSED.load(Ordering::Relaxed);
    if f == usize::MAX {
        None
    } else {
        Some(f)
    }
}

/// Fare/klavye olaylarini isler: pencere secme, baslik cubugundan surukleme.
pub fn handle_input() {
    while let Some(event) = input::poll() {
        match event {
            Event::Mouse { x, y, left } => handle_mouse(x, y, left),
            Event::Key(ascii) => match focused() {
                // Odak kabuktaysa tuslar kabuga; degilse uygulamaya gider.
                Some(f) if f == SHELL_WINDOW.load(Ordering::Relaxed) => {
                    crate::level0a::shell::on_key(ascii)
                }
                Some(f) => crate::level0a::gui_api::deliver_key(f, ascii),
                None => {}
            },
        }
    }
}

fn handle_mouse(mx: usize, my: usize, left: bool) {
    if !left {
        DRAG_WINDOW.store(usize::MAX, Ordering::Relaxed);
        return;
    }

    let dragging = DRAG_WINDOW.load(Ordering::Relaxed);
    if dragging != usize::MAX {
        // Suruklemeyi surdur.
        let dx = DRAG_DX.load(Ordering::Relaxed);
        let dy = DRAG_DY.load(Ordering::Relaxed);
        unsafe {
            let windows = core::ptr::addr_of_mut!(WINDOWS) as *mut Window;
            let w = &mut *windows.add(dragging);
            w.x = mx.saturating_sub(dx);
            w.y = my.saturating_sub(dy);
        }
        return;
    }

    // En ustteki pencereden basla (z-sirasi tersten).
    let count = WINDOW_COUNT.load(Ordering::Relaxed);
    unsafe {
        let z = core::ptr::addr_of!(Z_ORDER) as *const usize;
        let windows = core::ptr::addr_of!(WINDOWS) as *const Window;
        for i in (0..count).rev() {
            let idx = z.add(i).read();
            let w = *windows.add(idx);
            if !w.used {
                continue;
            }
            let full_h = w.height + TITLE_HEIGHT;
            if mx >= w.x && mx < w.x + w.width && my >= w.y && my < w.y + full_h {
                raise(idx);
                // Baslik cubuguna basildiysa suruklemeye basla.
                if my < w.y + TITLE_HEIGHT {
                    DRAG_WINDOW.store(idx, Ordering::Relaxed);
                    DRAG_DX.store(mx - w.x, Ordering::Relaxed);
                    DRAG_DY.store(my - w.y, Ordering::Relaxed);
                }
                return;
            }
        }
    }
}

/// Tum masaustunu yeniden cizer ve ekrana aktarir.
pub fn compose() {
    if !active() {
        return;
    }

    draw_desktop();

    let count = WINDOW_COUNT.load(Ordering::Relaxed);
    let focus = FOCUSED.load(Ordering::Relaxed);

    unsafe {
        let z = core::ptr::addr_of!(Z_ORDER) as *const usize;
        let windows = core::ptr::addr_of!(WINDOWS) as *const Window;

        for i in 0..count {
            let idx = z.add(i).read();
            let w = *windows.add(idx);
            if !w.used {
                continue;
            }

            // Sistem pencerelerinin icerigi her karede cekirdek tarafindan
            // uretilir (ornegin canli sistem gunlugu).
            if w.system {
                if SHELL_WINDOW.load(Ordering::Relaxed) == idx {
                    crate::level0a::shell::render(&w);
                } else {
                    render_system_window(&w);
                }
            }

            draw_frame(&w, idx == focus);

            let pixels = core::slice::from_raw_parts(w.buffer as *const u32, w.width * w.height);
            gfx::blit(w.x + BORDER, w.y + TITLE_HEIGHT, w.width, w.height, pixels);
        }
    }

    draw_cursor(input::mouse_x(), input::mouse_y());
    gfx::present();
}

fn draw_desktop() {
    gfx::clear(gfx::DESKTOP_BG);

    // Ust cubuk
    gfx::fill_rect(0, 0, gfx::width(), 24, 0x0016_1E2A);
    gfx::draw_text(8, 4, "The Cursed Moon Kernel", gfx::ACCENT);

    let stats = crate::level0a::gui_api::status_line();
    gfx::draw_text(320, 4, stats.as_str(), 0x0090_A0B0);
}

fn draw_frame(w: &Window, focused: bool) {
    let title_bg = if focused {
        gfx::TITLE_ACTIVE
    } else {
        gfx::TITLE_INACTIVE
    };

    // Baslik cubugu
    gfx::fill_rect(w.x, w.y, w.width + BORDER * 2, TITLE_HEIGHT, title_bg);

    let title = core::str::from_utf8(&w.title[..w.title_len]).unwrap_or("?");
    gfx::draw_text(w.x + 6, w.y + 2, title, gfx::WHITE);

    // Kenarlik
    gfx::stroke_rect(
        w.x,
        w.y,
        w.width + BORDER * 2,
        w.height + TITLE_HEIGHT + BORDER,
        if focused { gfx::ACCENT } else { 0x0040_4850 },
    );
}

/// Sistem gunlugu penceresi: konsol halka tamponunu her karede cizer.
fn render_system_window(w: &Window) {
    let pixels = w.buffer as *mut u32;
    let total = w.width * w.height;
    unsafe {
        for i in 0..total {
            pixels.add(i).write(0x0010_1418);
        }
    }

    let line_h = gfx::font_height();
    let rows = w.height / line_h;

    console::for_each_visible_line(rows, |row, text| {
        let y = row * line_h;
        let mut x = 2usize;
        for &byte in text {
            draw_char_into_window(w, x, y, byte, 0x00A8_C8A8);
            x += gfx::font_width();
            if x + gfx::font_width() > w.width {
                break;
            }
        }
    });
}

/// Pencere tamponuna karakter cizer (ekrana degil).
pub fn draw_char_into_window(w: &Window, x: usize, y: usize, ch: u8, color: u32) {
    use crate::level0a::drivers::font_data::{FONT, FONT_FIRST, FONT_LAST, FONT_WIDTH};

    let index = if (FONT_FIRST..=FONT_LAST).contains(&ch) {
        (ch - FONT_FIRST) as usize
    } else {
        return;
    };

    let pixels = w.buffer as *mut u32;
    for (row, bits) in FONT[index].iter().enumerate() {
        if *bits == 0 {
            continue;
        }
        let py = y + row;
        if py >= w.height {
            break;
        }
        for col in 0..FONT_WIDTH {
            if bits & (1 << (FONT_WIDTH - 1 - col)) != 0 {
                let px = x + col;
                if px < w.width {
                    unsafe { pixels.add(py * w.width + px).write(color) };
                }
            }
        }
    }
}

/// Basit ok seklinde fare imleci.
fn draw_cursor(x: usize, y: usize) {
    const CURSOR: [&[u8; 12]; 19] = [
        b"X           ",
        b"XX          ",
        b"XoX         ",
        b"XooX        ",
        b"XoooX       ",
        b"XooooX      ",
        b"XoooooX     ",
        b"XooooooX    ",
        b"XoooooooX   ",
        b"XooooooooX  ",
        b"XoooooooooX ",
        b"XooooooXXXXX",
        b"XoooXooX    ",
        b"XooX XooX   ",
        b"XoX  XooX   ",
        b"XX    XooX  ",
        b"X     XooX  ",
        b"       XX   ",
        b"            ",
    ];

    for (row, line) in CURSOR.iter().enumerate() {
        for (col, &ch) in line.iter().enumerate() {
            match ch {
                b'X' => gfx::put_pixel(x + col, y + row, gfx::BLACK),
                b'o' => gfx::put_pixel(x + col, y + row, gfx::WHITE),
                _ => {}
            }
        }
    }
}

/// Pencere tamponuna dolu dikdortgen cizer (imlec vb. icin).
pub fn fill_into_window(w: &Window, x: usize, y: usize, width: usize, height: usize, color: u32) {
    let pixels = w.buffer as *mut u32;
    for row in 0..height {
        let py = y + row;
        if py >= w.height {
            break;
        }
        for col in 0..width {
            let px = x + col;
            if px < w.width {
                unsafe { pixels.add(py * w.width + px).write(color) };
            }
        }
    }
}
