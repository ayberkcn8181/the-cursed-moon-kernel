//! GUI cekirdek API'si -- pencere olusturma, cizim tamponu, olay kuyrugu.
//!
//! Level-0b1'in cevirmenleri bu API'yi TCMK'ye ozgu syscall'lara baglar
//! (POSIX/NT'de karsiligi olmayan islevler icin ayrilmis numara araligi).
//!
//! Tasarim: uygulama pencere tamponunun **adresini** alir ve oraya
//! dogrudan yazar. Cekirdek her cizim icin araya girmez; yalnizca
//! kompozisyon ve olay dagitimi yapar.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::wm;

/// Pencere basina klavye olay kuyrugu.
const KEY_QUEUE: usize = 16;
static mut KEYS: [[u8; KEY_QUEUE]; wm::MAX_WINDOWS] = [[0; KEY_QUEUE]; wm::MAX_WINDOWS];
static KEY_HEAD: [AtomicUsize; wm::MAX_WINDOWS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
static KEY_TAIL: [AtomicUsize; wm::MAX_WINDOWS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiError {
    NoDisplay,
    TooManyWindows,
    BadWindow,
}

/// Yeni pencere acar; pencere kimligini dondurur.
pub fn create_window(
    title: &str,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Result<usize, GuiError> {
    if !wm::active() {
        return Err(GuiError::NoDisplay);
    }
    wm::create(title, x, y, width, height, false).ok_or(GuiError::TooManyWindows)
}

/// Pencerenin piksel tamponunun **cagiran surecin** adres uzayindaki adresi.
///
/// Sahiplik denetimi sart: tampon artik yalnizca sahibinin adres uzayina
/// eslendigi icin baska bir surece adresi vermek zaten ise yaramazdi, ama
/// acikca reddetmek hatayi sessiz bir page fault yerine anlasilir bir
/// donus degerine cevirir.
pub fn window_buffer(id: usize) -> Result<usize, GuiError> {
    let caller = crate::level0a::core::scheduler::current_id();
    wm::get(id)
        .filter(|w| w.used && w.owner == caller && w.user_addr != 0)
        .map(|w| w.user_addr)
        .ok_or(GuiError::BadWindow)
}

/// Pencerenin (genislik, yukseklik) bilgisi -- tek bir kelimede paketli.
pub fn window_size(id: usize) -> Result<usize, GuiError> {
    wm::get(id)
        .filter(|w| w.used)
        .map(|w| (w.width << 16) | (w.height & 0xFFFF))
        .ok_or(GuiError::BadWindow)
}

/// Pencerenin ekrandaki sol ust kosesi -- tek kelimede paketli.
///
/// Kullanici uygulamalari fare koordinatlarini kendi ic koordinatlarina
/// cevirebilsin diye gerekir: pencere baslik cubugundan suruklendiginde
/// olusturma anindaki konum artik gecerli degildir.
pub fn window_pos(id: usize) -> Result<usize, GuiError> {
    wm::get(id)
        .filter(|w| w.used)
        .map(|w| (w.x << 16) | (w.y & 0xFFFF))
        .ok_or(GuiError::BadWindow)
}

/// Pencereye bir tus olayi teslim eder (WM cagirir).
pub fn deliver_key(id: usize, ascii: u8) {
    if id >= wm::MAX_WINDOWS {
        return;
    }
    let head = KEY_HEAD[id].load(Ordering::Relaxed);
    let next = (head + 1) % KEY_QUEUE;
    if next == KEY_TAIL[id].load(Ordering::Relaxed) {
        return; // kuyruk dolu
    }
    unsafe {
        let keys = core::ptr::addr_of_mut!(KEYS) as *mut [u8; KEY_QUEUE];
        (*keys.add(id))[head] = ascii;
    }
    KEY_HEAD[id].store(next, Ordering::Relaxed);
}

/// Pencerenin bekleyen tus olayini alir; yoksa 0 doner.
pub fn poll_key(id: usize) -> u8 {
    if id >= wm::MAX_WINDOWS {
        return 0;
    }
    let tail = KEY_TAIL[id].load(Ordering::Relaxed);
    if tail == KEY_HEAD[id].load(Ordering::Relaxed) {
        return 0;
    }
    let ascii = unsafe {
        let keys = core::ptr::addr_of!(KEYS) as *const [u8; KEY_QUEUE];
        (*keys.add(id))[tail]
    };
    KEY_TAIL[id].store((tail + 1) % KEY_QUEUE, Ordering::Relaxed);
    ascii
}

/// Fare durumu: x (bit 20+), y (bit 4+), sol tus (bit 0).
pub fn mouse_state() -> usize {
    let x = crate::level0a::input::mouse_x() & 0xFFF;
    let y = crate::level0a::input::mouse_y() & 0xFFF;
    let left = crate::level0a::input::mouse_left() as usize;
    (x << 20) | (y << 4) | left
}

/// Ust cubukta gosterilen kisa durum satiri.
pub struct StatusLine {
    buf: [u8; 96],
    len: usize,
}

impl StatusLine {
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl core::fmt::Write for StatusLine {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for b in s.bytes() {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

pub fn status_line() -> StatusLine {
    use core::fmt::Write;
    let mut line = StatusLine {
        buf: [0; 96],
        len: 0,
    };
    let _ = write!(
        line,
        "gorev:{}  pencere:{}  tick:{}  nabiz:{}",
        crate::level0a::core::scheduler::task_count(),
        wm::window_count(),
        crate::level0a::pit::ticks(),
        crate::level0a::pit::heartbeat(),
    );
    line
}
