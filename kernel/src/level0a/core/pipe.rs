//! Boru (pipe) -- iki surec arasinda tek yonlu bayt akisi.
//!
//! `fork` bir sureci ikiye ayirir ama ikisinin **konusabilmesi** icin
//! ortak bir kanal gerekir; adres uzaylari artik ayri oldugu icin
//! (bkz. `mmu::clone_user_space`) paylasilan bir degisken yoktur. Boru,
//! UNIX'in bu soruna verdigi cevaptir: cekirdekte duran bir halka
//! tamponunun iki ucu iki dosya tanimlayicisi olarak goruntulenir.
//!
//! ```text
//!   yazan surec  --write(fd[1])-->  [ halka tampon ]  --read(fd[0])-->  okuyan surec
//! ```
//!
//! Tampon **cekirdektedir**, yani `fork`'ta kopyalanmaz: iki surec de
//! ayni boruyu gorur. Zaten isin puf noktasi budur.
//!
//! Tanimlayicilar ise **kopyalanir**: `fork` ebeveynin fd tablosunu
//! cocuga klonlar (bkz. `core::fd::clone_into`) ve boru uclarinin
//! sayaclarini artirir. Kopyalanmasalardi UNIX'in klasik kalibi
//! calismazdi -- her taraf kullanmadigi ucu kapatir, ve paylasilan bir
//! tabloda cocugun kapattigi uc ebeveyninkini de yok ederdi.
//!
//! ## Bloke etmeyen okuma
//!
//! Gercek POSIX'te bos bir borudan okumak, veri gelene ya da yazan uc
//! kapanana kadar **bloke olur**. TCMK'de okuma bloke olmaz; veri yoksa
//! `0` doner. Nedeni GUI'dir: bloke olan bir surec penceresini de
//! dondurur, oysa buradaki uygulamalar kendi cizim dongulerini surer.
//! Yazan uc kapandiginda `is_closed` ile "dosya sonu" ayirt edilebilir,
//! yani protokol yine kurulabilir.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Ayni anda acik olabilecek boru sayisi.
pub const MAX_PIPES: usize = 4;
/// Boru basina tampon boyu. Halka tampon oldugu icin dolu tampona yazmak
/// veri kaybetmez, yazma kisa doner (POSIX'te de oyle).
pub const PIPE_CAPACITY: usize = 1024;

static mut BUFFERS: [[u8; PIPE_CAPACITY]; MAX_PIPES] = [[0; PIPE_CAPACITY]; MAX_PIPES];

struct Pipe {
    used: AtomicBool,
    head: AtomicUsize,
    tail: AtomicUsize,
    /// Yazan uclarin sayisi. Sifira duserse okuyan taraf icin "dosya
    /// sonu" demektir.
    writers: AtomicUsize,
    readers: AtomicUsize,
}

impl Pipe {
    const fn new() -> Self {
        Pipe {
            used: AtomicBool::new(false),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            writers: AtomicUsize::new(0),
            readers: AtomicUsize::new(0),
        }
    }
}

static PIPES: [Pipe; MAX_PIPES] = [Pipe::new(), Pipe::new(), Pipe::new(), Pipe::new()];

/// Yeni bir boru ayirir; indeksini dondurur.
pub fn create() -> Option<usize> {
    for (i, pipe) in PIPES.iter().enumerate() {
        if pipe
            .used
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            pipe.head.store(0, Ordering::Relaxed);
            pipe.tail.store(0, Ordering::Relaxed);
            pipe.writers.store(1, Ordering::Relaxed);
            pipe.readers.store(1, Ordering::Relaxed);
            return Some(i);
        }
    }
    None
}

/// Bir borunun durumu: (bekleyen bayt, yazan uc, okuyan uc).
/// Boru acik degilse `None`.
pub fn info(index: usize) -> Option<(usize, usize, usize)> {
    if index >= MAX_PIPES {
        return None;
    }
    let pipe = &PIPES[index];
    if !pipe.used.load(Ordering::Relaxed) {
        return None;
    }
    let head = pipe.head.load(Ordering::Relaxed);
    let tail = pipe.tail.load(Ordering::Relaxed);
    Some((
        head.wrapping_sub(tail) % (PIPE_CAPACITY + 1),
        pipe.writers.load(Ordering::Relaxed),
        pipe.readers.load(Ordering::Relaxed),
    ))
}

/// Boruya yazar; yazilan bayt sayisini dondurur (tampon dolduysa kisa
/// donebilir).
pub fn write(index: usize, bytes: &[u8]) -> usize {
    if index >= MAX_PIPES || !PIPES[index].used.load(Ordering::Relaxed) {
        return 0;
    }
    // Okuyan kimse kalmadiysa yazmanin anlami yok (POSIX'te SIGPIPE).
    if PIPES[index].readers.load(Ordering::Relaxed) == 0 {
        return 0;
    }

    crate::arch::cpu::without_interrupts(|| {
        let pipe = &PIPES[index];
        let mut head = pipe.head.load(Ordering::Relaxed);
        let tail = pipe.tail.load(Ordering::Relaxed);
        let mut written = 0usize;

        for &byte in bytes {
            let next = (head + 1) % (PIPE_CAPACITY + 1);
            if next == tail {
                break; // tampon dolu
            }
            unsafe {
                let buffers = core::ptr::addr_of_mut!(BUFFERS) as *mut u8;
                buffers.add(index * PIPE_CAPACITY + head % PIPE_CAPACITY).write(byte);
            }
            head = next;
            written += 1;
        }

        pipe.head.store(head, Ordering::Relaxed);
        written
    })
}

/// Borudan okur; okunan bayt sayisini dondurur. Veri yoksa `0`.
pub fn read(index: usize, out: &mut [u8]) -> usize {
    if index >= MAX_PIPES || !PIPES[index].used.load(Ordering::Relaxed) {
        return 0;
    }

    crate::arch::cpu::without_interrupts(|| {
        let pipe = &PIPES[index];
        let head = pipe.head.load(Ordering::Relaxed);
        let mut tail = pipe.tail.load(Ordering::Relaxed);
        let mut read = 0usize;

        while read < out.len() && tail != head {
            out[read] = unsafe {
                let buffers = core::ptr::addr_of!(BUFFERS) as *const u8;
                buffers.add(index * PIPE_CAPACITY + tail % PIPE_CAPACITY).read()
            };
            tail = (tail + 1) % (PIPE_CAPACITY + 1);
            read += 1;
        }

        pipe.tail.store(tail, Ordering::Relaxed);
        read
    })
}

/// Bir ucu kapatir. Son uc de kapaninca boru serbest kalir.
pub fn close_end(index: usize, writer: bool) {
    if index >= MAX_PIPES || !PIPES[index].used.load(Ordering::Relaxed) {
        return;
    }
    let pipe = &PIPES[index];
    let counter = if writer { &pipe.writers } else { &pipe.readers };
    let previous = counter.load(Ordering::Relaxed);
    if previous > 0 {
        counter.store(previous - 1, Ordering::Relaxed);
    }

    if pipe.writers.load(Ordering::Relaxed) == 0 && pipe.readers.load(Ordering::Relaxed) == 0 {
        pipe.used.store(false, Ordering::Relaxed);
    }
}

/// Bir ucun sahibi cogaldi (`fork`: cocuk tanimlayicilari devralir).
///
/// Sayaclar olmasa cocugun kapattigi uc boruyu tumden oldururdu; iki
/// taraf da kapatana kadar acik kalmasi gerekir.
pub fn add_ref(index: usize, writer: bool) {
    if index >= MAX_PIPES || !PIPES[index].used.load(Ordering::Relaxed) {
        return;
    }
    let pipe = &PIPES[index];
    let counter = if writer { &pipe.writers } else { &pipe.readers };
    counter.store(counter.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
}

/// Kabuk raporu: kac boru acik.
pub fn open_count() -> usize {
    PIPES
        .iter()
        .filter(|p| p.used.load(Ordering::Relaxed))
        .count()
}
