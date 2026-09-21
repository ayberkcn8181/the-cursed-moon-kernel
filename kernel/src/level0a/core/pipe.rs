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
//! ## Bloke eden okuma
//!
//! Bos bir borudan okumak, veri gelene ya da **yazan son uc kapanana**
//! kadar bekler -- POSIX'in sozlesmesi budur ve artik TCMK de onu
//! tutuyor. Ayrim onemli, cunku "veri yok" ile "bir daha veri gelmeyecek"
//! ayni sayiyla (`0`) ifade edilemez:
//!
//! ```text
//!   veri var            -> okunani dondur (kismi olabilir)
//!   veri yok, yazan var -> BEKLE
//!   veri yok, yazan yok -> 0 dondur (dosya sonu)
//! ```
//!
//! Onceden okuma bloke etmiyordu ve ikinci satir da `0` donduruyordu.
//! Bu, gercek bir Linux ikilisini kirmaya yeten bir farktir: `read`i
//! bloke sanan bir program, dosya sonu geldigini sanip erken cikar.
//!
//! Uyandirma iki yerden geliyor ve ikisi de sart: **yazma** (veri geldi)
//! ve **yazan ucun kapanmasi** (bir daha gelmeyecek). Ikincisi olmasa
//! borunun yazan ucunu kapatan bir ebeveyn, okuyan cocugu sonsuza kadar
//! uyutmus olurdu.
//!
//! Uyutulamayan baglamlar (masaustu/kabuk gorevi -- ekrani onlar ciziyor)
//! eski davranisa duser: bloke olmak yerine `0` doner. Bu, cagiran
//! tarafin `current_can_block` ile onceden anlamasi gereken bir durum.
//!
//! ## Bloke eden yazma
//!
//! Okumanin aynasi: tampon doluysa yazma, yer acilana ya da **okuyan
//! son uc kapanana** kadar bekler.
//!
//! ```text
//!   yer var             -> yaz (kismi olabilir)
//!   yer yok, okuyan var -> BEKLE
//!   yer yok, okuyan yok -> EPIPE + SIGPIPE
//! ```
//!
//! Son satir POSIX'in en sert varsayilani: sinyal yakalanmazsa surec
//! **oler**. Kaba gorunuyor ama kabuk boru hatlarinin calismasi buna
//! bagli -- `uretici | head` kaliginda `head` cikinca, uretici
//! durdurulmazsa sonsuza kadar kosardi.
//!
//! Iki anahtar kullaniliyor (`read_key` / `write_key`) ve ayri olmalari
//! sart: dolu boruda bekleyen yazici ile bos boruda bekleyen okuyucu
//! ayni nesneyi ama **zit kosullari** bekliyor. Tek anahtar olsaydi
//! birbirlerini bosa kaldirip dururlardi.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::level0a::core::scheduler;

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

    let written = crate::arch::cpu::without_interrupts(|| {
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
    });

    // Veri geldi: bekleyen okuyuculari kaldir. Uyandirma yazmanin
    // **disinda**, cunku `wake_kernel_key` kendi kritik bolgesini
    // aciyor ve ic ice girmesi gerekmiyor.
    if written > 0 {
        scheduler::wake_kernel_key(read_key(index), usize::MAX);
    }
    written
}

/// Bir borunun okuma bekleme anahtari.
///
/// Boru indeksi tek basina yetmezdi: anahtarlar gorev tablosunda tek bir
/// alanda tutuluyor ve baska cekirdek nesneleri de eklenebilir. Tur
/// etiketini anahtara katmak, ileride konsol ya da soket beklemesi
/// eklendiginde cakismayi derleme aninda degil ama **tasarim aninda**
/// engelliyor.
pub fn read_key(index: usize) -> usize {
    PIPE_READ_KEY_BASE + index
}

/// Bir borunun **yazma** bekleme anahtari.
///
/// Okuma anahtarindan ayri olmak zorunda: dolu bir boruda bekleyen
/// yazici ile bos bir boruda bekleyen okuyucu ayni nesneyi bekliyor ama
/// **zit kosullari** bekliyor. Tek anahtar olsaydi, bir okumanin
/// uyandirdigi yazici ile bir yazmanin uyandirdigi okuyucu birbirini
/// surekli bosa kaldirirdi.
pub fn write_key(index: usize) -> usize {
    PIPE_WRITE_KEY_BASE + index
}

/// Boru okuma anahtarlarinin tabani -- baska nesne turleriyle
/// cakismayacak bir aralik.
const PIPE_READ_KEY_BASE: usize = 0x0001_0000;
/// Yazma anahtarlarinin tabani.
const PIPE_WRITE_KEY_BASE: usize = 0x0002_0000;

/// Borunun tamponunda **bos yer** var mi?
pub fn has_room(index: usize) -> bool {
    match info(index) {
        Some((pending, _, _)) => pending < PIPE_CAPACITY,
        None => false,
    }
}

/// Okuyan uc sayisi (0 ise yazmanin alicisi yok).
pub fn readers(index: usize) -> usize {
    info(index).map(|(_, _, readers)| readers).unwrap_or(0)
}

/// Borudan okur; okunan bayt sayisini dondurur. Veri yoksa `0`.
pub fn read(index: usize, out: &mut [u8]) -> usize {
    if index >= MAX_PIPES || !PIPES[index].used.load(Ordering::Relaxed) {
        return 0;
    }

    let read = crate::arch::cpu::without_interrupts(|| {
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
    });

    // Yer acildi: dolu tamponda bekleyen yazicilari kaldir. Okumanin
    // aynasi -- yazma okuyuculari uyandiriyor, okuma yazicilari.
    if read > 0 {
        scheduler::wake_kernel_key(write_key(index), usize::MAX);
    }
    read
}

/// Borudan **tuketmeden** okur (`PeekNamedPipe`).
///
/// POSIX'te bunun karsiligi yok: orada "bakmak" istiyorsaniz `poll` ile
/// hazir mi diye sorar, sonra `read` ile alirsiniz -- ama aldiginiz an
/// tuketmis olursunuz. Win32 ikisini ayirmis, cunku adlandirilmis
/// borularda ileti sinirlarini gormek gerekebiliyor.
///
/// Doner: kopyalanan bayt sayisi. Imlec ilerlemez.
pub fn peek(index: usize, out: &mut [u8]) -> usize {
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

        // `tail` **geri yazilmiyor**: farkin tamami bu.
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

    // Yazan son uc kapandi: bekleyen okuyucular icin bu "dosya sonu"
    // haberidir. Uyandirmasak, bir daha veri gelmeyecek bir boruyu
    // sonsuza kadar beklerlerdi -- borunun kapanmasi, gelmeyecek verinin
    // tek isareti.
    if writer && pipe.writers.load(Ordering::Relaxed) == 0 {
        scheduler::wake_kernel_key(read_key(index), usize::MAX);
    }
    // Okuyan son uc kapandi: dolu tamponda bekleyen yazicilar artik
    // bosuna bekliyor -- kimse okumayacak. Uyandirilmalilar ki
    // `EPIPE`/`SIGPIPE` alabilsinler; aksi halde asla bosalmayacak bir
    // tamponu sonsuza kadar beklerlerdi.
    if !writer && pipe.readers.load(Ordering::Relaxed) == 0 {
        scheduler::wake_kernel_key(write_key(index), usize::MAX);
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
