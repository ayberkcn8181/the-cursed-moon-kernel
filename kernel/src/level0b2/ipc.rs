//! Katmanlar arasi iletisim (doc S.10).
//!
//! Doc S.10'un asamalari:
//!   Faz 1-2: dogrudan fonksiyon cagrisi
//!   Faz 3+ : syscall tablosu uzerinden dolayli cagri
//!   Faz 4+ : **mesaj kuyrugu (ring buffer) ile Level-0b2 <-> Level-0a
//!            iletisimi; heartbeat sayaci paylasimli bellek bolgesinde**
//!
//! Bu modul son asamayi gerceklestirir ve iki parcadan olusur:
//!
//! 1. [`SharedArea`] -- paylasimli durum bolgesi. Level-0a **yazar**
//!    (nabiz, tick, gorev sayisi), Level-0b2 **okur**. Boylece State
//!    Monitor "Level-0a yasiyor mu" sorusunu artik Level-0a'nin bir
//!    fonksiyonunu **cagirarak** cevaplamiyor: cagrilan taraf kilitliyse
//!    cagiran da kilitlenirdi. Paylasimli bellekten okumak bu bagimliligi
//!    kaldirir -- hata tolerans motorunun dogru calismasi icin sart.
//!
//! 2. Mesaj halkasi -- olaylar (hata, kisitlama, saglik degisimi, uygulama
//!    yasam dongusu) uretildikleri yerde kuyruga atilir, Level-0a'nin
//!    masaustu dongusunde tuketilir. Uretici tarafi **kesme baglaminda**
//!    olabilecegi icin (istisna handler'i) kuyruk kesmeler kapaliyken
//!    guncellenir; bu, tek CPU'da yeterli ve en ucuz karsilikli disliktir.

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// Paylasimli bolgenin gecerliligini dogrulayan imza ("TCMK").
pub const SHARED_MAGIC: u32 = 0x544D_4B00;

/// Level-0a'nin durumunu Level-0b2'ye tasiyan paylasimli bellek bolgesi.
#[repr(C, align(64))]
pub struct SharedArea {
    pub magic: AtomicU32,
    /// Scheduler dongusunun nabzi (doc S.11).
    pub heartbeat: AtomicU32,
    /// PIT tick sayaci.
    pub ticks: AtomicU32,
    /// Calisan gorev sayisi.
    pub tasks: AtomicU32,
    /// Baglam degisimi sayaci.
    pub switches: AtomicU32,
    /// Level-0a'nin kendi bildirdigi son saglik durumu (0=acilis,
    /// 1=servisler aktif, 2=servis hatasi).
    pub level0a_status: AtomicU32,
}

pub static SHARED: SharedArea = SharedArea {
    magic: AtomicU32::new(SHARED_MAGIC),
    heartbeat: AtomicU32::new(0),
    ticks: AtomicU32::new(0),
    tasks: AtomicU32::new(0),
    switches: AtomicU32::new(0),
    level0a_status: AtomicU32::new(0),
};

/// Paylasimli bolgenin fiziksel/sanal adresi (kabuktan gorulebilsin diye).
pub fn shared_addr() -> usize {
    &SHARED as *const SharedArea as usize
}

pub fn shared_is_valid() -> bool {
    SHARED.magic.load(Ordering::Relaxed) == SHARED_MAGIC
}

// --- Mesaj halkasi ------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Level-0a acilisi tamamlandi (a = servis sayisi).
    BootComplete,
    /// Ring 3 uygulamasi baslatildi (a = gorev no).
    AppStart,
    /// Surec sonlandi (a = cikis kodu).
    AppExit,
    /// CPU istisnasi yakalandi (a = vektor, b = hata adresi).
    Fault,
    /// Yuk dengeleyici bir gorevi kisitladi (a = pencere ici cagri sayisi).
    Throttled,
    /// Level-0a saglik durumu degisti (a = yeni durum).
    HealthChange,
    /// Serbest metin/log olayi.
    Note,
}

#[derive(Clone, Copy)]
pub struct Message {
    pub kind: Kind,
    pub task: usize,
    pub a: usize,
    pub b: usize,
    /// Mesajin uretildigi PIT tick'i.
    pub tick: u32,
    /// Kisa aciklama (sabit dizeler; kuyrukta ayirma yapilmaz).
    pub text: &'static str,
}

impl Message {
    const fn empty() -> Self {
        Message {
            kind: Kind::Note,
            task: 0,
            a: 0,
            b: 0,
            tick: 0,
            text: "",
        }
    }
}

const RING_SIZE: usize = 32;
/// Kabugun `ipc` komutunda gosterilen son mesajlar.
const HISTORY_SIZE: usize = 8;

static mut RING: [Message; RING_SIZE] = [Message::empty(); RING_SIZE];
static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);

static mut HISTORY: [Message; HISTORY_SIZE] = [Message::empty(); HISTORY_SIZE];
static HISTORY_LEN: AtomicUsize = AtomicUsize::new(0);
static HISTORY_NEXT: AtomicUsize = AtomicUsize::new(0);

static POSTED: AtomicU32 = AtomicU32::new(0);
static CONSUMED: AtomicU32 = AtomicU32::new(0);
static DROPPED: AtomicU32 = AtomicU32::new(0);
static HIGH_WATER: AtomicUsize = AtomicUsize::new(0);

/// Kuyruga mesaj birakir. Kuyruk doluysa mesaj **dusurulur** ve sayaca
/// islenir: uretici hicbir kosulda beklemez, cunku uretici kesme
/// baglaminda olabilir.
pub fn post(kind: Kind, task: usize, a: usize, b: usize, text: &'static str) -> bool {
    let message = Message {
        kind,
        task,
        a,
        b,
        tick: crate::level0a::pit::ticks(),
        text,
    };

    crate::arch::cpu::without_interrupts(|| unsafe {
        let head = HEAD.load(Ordering::Relaxed);
        let next = (head + 1) % RING_SIZE;
        if next == TAIL.load(Ordering::Relaxed) {
            DROPPED.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let ring = core::ptr::addr_of_mut!(RING) as *mut Message;
        ring.add(head).write(message);
        HEAD.store(next, Ordering::Relaxed);
        POSTED.fetch_add(1, Ordering::Relaxed);

        let depth = (next + RING_SIZE - TAIL.load(Ordering::Relaxed)) % RING_SIZE;
        HIGH_WATER.fetch_max(depth, Ordering::Relaxed);
        true
    })
}

/// Bir mesaj alir (Level-0a tuketicisi).
pub fn poll() -> Option<Message> {
    crate::arch::cpu::without_interrupts(|| unsafe {
        let tail = TAIL.load(Ordering::Relaxed);
        if tail == HEAD.load(Ordering::Relaxed) {
            return None;
        }
        let ring = core::ptr::addr_of!(RING) as *const Message;
        let message = ring.add(tail).read();
        TAIL.store((tail + 1) % RING_SIZE, Ordering::Relaxed);
        CONSUMED.fetch_add(1, Ordering::Relaxed);
        remember(message);
        Some(message)
    })
}

/// Tuketilen mesaji gecmise yazar (kabuk `ipc` komutu icin).
///
/// # Safety
/// Kesmeler kapaliyken cagrilmalidir.
unsafe fn remember(message: Message) {
    let slot = HISTORY_NEXT.load(Ordering::Relaxed);
    let history = core::ptr::addr_of_mut!(HISTORY) as *mut Message;
    history.add(slot).write(message);
    HISTORY_NEXT.store((slot + 1) % HISTORY_SIZE, Ordering::Relaxed);
    let len = HISTORY_LEN.load(Ordering::Relaxed);
    if len < HISTORY_SIZE {
        HISTORY_LEN.store(len + 1, Ordering::Relaxed);
    }
}

pub fn history_len() -> usize {
    HISTORY_LEN.load(Ordering::Relaxed)
}

/// Gecmisteki `index`. 0 = en yeni.
pub fn history(index: usize) -> Option<Message> {
    let len = HISTORY_LEN.load(Ordering::Relaxed);
    if index >= len {
        return None;
    }
    let next = HISTORY_NEXT.load(Ordering::Relaxed);
    let slot = (next + HISTORY_SIZE - 1 - index) % HISTORY_SIZE;
    unsafe {
        let history = core::ptr::addr_of!(HISTORY) as *const Message;
        Some(history.add(slot).read())
    }
}

pub fn depth() -> usize {
    let head = HEAD.load(Ordering::Relaxed);
    let tail = TAIL.load(Ordering::Relaxed);
    (head + RING_SIZE - tail) % RING_SIZE
}

pub fn capacity() -> usize {
    RING_SIZE - 1
}

pub fn posted() -> u32 {
    POSTED.load(Ordering::Relaxed)
}

pub fn consumed() -> u32 {
    CONSUMED.load(Ordering::Relaxed)
}

pub fn dropped() -> u32 {
    DROPPED.load(Ordering::Relaxed)
}

pub fn high_water() -> usize {
    HIGH_WATER.load(Ordering::Relaxed)
}

pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::BootComplete => "acilis",
        Kind::AppStart => "uygulama-basladi",
        Kind::AppExit => "uygulama-bitti",
        Kind::Fault => "istisna",
        Kind::Throttled => "kisitlama",
        Kind::HealthChange => "saglik",
        Kind::Note => "not",
    }
}
