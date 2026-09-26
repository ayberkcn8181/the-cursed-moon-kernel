//! Cekirdek ici ayirici -- sinir etiketli, birlestiren.
//!
//! Heap 8 MiB'e tasindi: framebuffer'in 3 MiB'lik arka tamponu cekirdek
//! `.bss`'ini ~5 MiB'e cikardi ve eski 2 MiB'lik heap konumu imajin
//! uzerine biniyordu (doc S.5'teki harita Faz 13 ile guncellendi).
//!
//! ## Neden bump yetmedi
//!
//! Uzun sure burasi bir **bump** ayiriciydi: isaretci ilerler, geri
//! donus yoktu. Gerekcesi de yaziliydi -- "tek tuketici scheduler'in
//! gorev yiginlari, onlar da yuvayla birlikte yeniden kullaniliyor".
//!
//! Gerekce bir dogruydu, ama zamanla yanlis oldu. Iki tuketici daha
//! geldi ve ikisi de **her cagride yeniden** tahsis ediyordu:
//!
//! ```text
//!   pencere tamponu   width * height * 4 bayt, her pencere acilista
//!   surec kernel yigini  16 KiB, her Ring 3 baslatmada
//! ```
//!
//! Bir pencere acip kapatmak 1 MiB'a varan bir sizinti demekti ve
//! hicbir yerde gorunmuyordu: heap dolana kadar her sey calisiyor,
//! sonra bir gun pencere acilmiyordu.
//!
//! ## Yerlesim
//!
//! Klasik **sinir etiketi** (boundary tag) duzeni. Her blogun basinda
//! ve sonunda boyu yaziyor; bu, komsuya iki yonde de yurumeyi ve
//! birlestirmeyi sabit zamanda yapmayi saglar:
//!
//! ```text
//!   +0    size   (usize)   blogun TOPLAM boyu (basliklar dahil)
//!   +W    used   (usize)   0 bos, 1 dolu
//!   ...   (16 bayta dolgu -- yuk boylece 16 hizali baslar)
//!   +16   yuk ...
//!   son-16  footer: size   (geri yurumek icin)
//! ```
//!
//! Bos bloklarin ayri bir listesi **yok**: bloklar bastan sona zaten
//! bitisik duruyor, yani "sonraki blok" `blok + size`. Ortulu liste
//! (implicit list) ayri isaretci tutmadigi icin bozulacak daha az sey
//! var; bedeli, tahsisin bloklar uzerinde yurumesi. Birkac yuz bloklu
//! bir cekirdekte bu bedel olculemez.
//!
//! ## Hizalama
//!
//! `align` 16'dan buyuk olabiliyor (pencere tamponlari sayfa hizali
//! isteniyor, cunku Ring 3'e eslenecekler). Uygun blok bulununca yuk
//! ileri kaydiriliyor ve **onde kalan parca ayri bir bos blok** olarak
//! birakiliyor. Parca kendi basina blok olamayacak kadar kucukse bir
//! sonraki hizali noktaya atlaniyor -- yoksa geri kazanilamayan bir
//! bosluk kalirdi.
//!
//! ## Es zamanlilik
//!
//! Tek islemci: butun islemler kesmeler kapaliyken yapiliyor.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub const HEAP_START: usize = 0x0080_0000; // 8 MiB
pub const HEAP_SIZE: usize = 4 * 1024 * 1024; // 4 MiB
pub const HEAP_END: usize = HEAP_START + HEAP_SIZE;

/// Yuk hizalamasi ve baslik boyu. Ikisi ayni: baslik 16 bayt oldugu
/// icin 16 hizali bir blogun yuku de 16 hizali basliyor.
const ALIGN: usize = 16;
const HEADER: usize = 16;
const FOOTER: usize = 16;
const OVERHEAD: usize = HEADER + FOOTER;
/// En kucuk anlamli blok: basliklar + bir hizalama birimi yuk.
const MIN_BLOCK: usize = OVERHEAD + ALIGN;

const FREE: usize = 0;
const USED: usize = 1;

static READY: AtomicBool = AtomicBool::new(false);
static USED_BYTES: AtomicUsize = AtomicUsize::new(0);

fn round_up(value: usize, to: usize) -> usize {
    (value + to - 1) & !(to - 1)
}

// --- Blok basliklari ---

unsafe fn size_of(block: usize) -> usize {
    (block as *const usize).read()
}

unsafe fn is_used(block: usize) -> bool {
    (block as *const usize).add(1).read() == USED
}

/// Basligi ve ayni boyu tasiyan footer'i birlikte yazar.
///
/// Ikisini ayri yazmak mumkun degil: footer bir onceki blogun boyunu
/// ogrenmenin **tek** yolu, yani ikisi tutarsiz kalirsa geri yurume
/// rastgele bir adrese dalar.
unsafe fn set_block(block: usize, size: usize, used: usize) {
    (block as *mut usize).write(size);
    (block as *mut usize).add(1).write(used);
    ((block + size - FOOTER) as *mut usize).write(size);
}

/// Bir onceki blogun basi -- footer'indan okunur.
unsafe fn prev_block(block: usize) -> Option<usize> {
    if block <= HEAP_START {
        return None;
    }
    let prev_size = ((block - FOOTER) as *const usize).read();
    if prev_size < MIN_BLOCK || prev_size > block - HEAP_START {
        // Bozuk footer: birlestirmeyi denemektense vazgec.
        return None;
    }
    Some(block - prev_size)
}

/// Heap'i tek bir dev bos blok olarak kurar.
///
/// Tembel: ilk tahsiste calisiyor, boylece acilis sirasinda ayrica
/// cagrilmasi gerekmiyor.
unsafe fn init_once() {
    if READY.load(Ordering::Relaxed) {
        return;
    }
    set_block(HEAP_START, HEAP_SIZE, FREE);
    READY.store(true, Ordering::Relaxed);
}

/// Blogu `size` boyuna kucultup kalani bos blok olarak birakir.
///
/// Kalan parca kendi basina blok olamayacak kadar kucukse bolme
/// yapilmaz: kullanilamayan bir parcayi liste icinde tutmak, onu
/// blogun icinde birakmaktan daha kotudur (bir daha asla birlesemez).
unsafe fn split(block: usize, size: usize) {
    let total = size_of(block);
    if total < size + MIN_BLOCK {
        return;
    }
    set_block(block, size, USED);
    set_block(block + size, total - size, FREE);
}

/// `align` bayt hizali `size` baytlik blok ayirir; yer yoksa `None`.
pub fn kmalloc_aligned(size: usize, align: usize) -> Option<*mut u8> {
    if size == 0 || !align.is_power_of_two() {
        return None;
    }
    let align = align.max(ALIGN);
    let need = round_up(size, ALIGN) + OVERHEAD;

    crate::arch::cpu::without_interrupts(|| unsafe {
        init_once();

        let mut block = HEAP_START;
        while block < HEAP_END {
            let total = size_of(block);
            if total < MIN_BLOCK || block + total > HEAP_END {
                // Bozuk baslik: daha fazla yurumek tehlikeli.
                return None;
            }
            if !is_used(block) {
                // Yuku hizali noktaya kaydir; onde kalan parca kendi
                // basina bir blok olacak kadar buyuk olmali.
                let mut payload = round_up(block + HEADER, align);
                let mut lead = payload - HEADER - block;
                while lead != 0 && lead < MIN_BLOCK {
                    payload += align;
                    lead = payload - HEADER - block;
                }

                if lead + need <= total {
                    let start = payload - HEADER;
                    if lead != 0 {
                        // Onu ayri bir bos blok olarak birak.
                        set_block(block, lead, FREE);
                        set_block(start, total - lead, FREE);
                    }
                    set_block(start, total - lead, USED);
                    split(start, need);
                    USED_BYTES.fetch_add(size_of(start), Ordering::Relaxed);
                    return Some(payload as *mut u8);
                }
            }
            block += total;
        }
        None
    })
}

/// 16 bayt hizali varsayilan tahsis.
pub fn kmalloc(size: usize) -> Option<*mut u8> {
    kmalloc_aligned(size, ALIGN)
}

/// Tahsis edilmis bir blogu geri verir.
///
/// Komsulari bos ise onlarla **birlestirilir**; birlestirmeseydi heap
/// zamanla ayni toplam bos alanla ama hicbiri yeterince buyuk olmayan
/// parcalara bolunurdu -- ve bu, sizintinin yavas cekimde tekrari
/// olurdu.
///
/// # Safety
/// `ptr` daha once bu ayiricidan gelmis ve henuz geri verilmemis
/// olmali.
pub unsafe fn kfree(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    let payload = ptr as usize;
    if payload < HEAP_START + HEADER || payload >= HEAP_END {
        return;
    }
    let mut block = payload - HEADER;

    crate::arch::cpu::without_interrupts(|| {
        if !is_used(block) {
            // Iki kez birakma: sessizce yok saymak, birlestirme
            // muhasebesini bozmaktan iyidir.
            return;
        }
        let mut total = size_of(block);
        USED_BYTES.fetch_sub(total, Ordering::Relaxed);
        set_block(block, total, FREE);

        // Sonraki komsu.
        let next = block + total;
        if next < HEAP_END && !is_used(next) {
            total += size_of(next);
            set_block(block, total, FREE);
        }

        // Onceki komsu.
        if let Some(prev) = prev_block(block) {
            if !is_used(prev) {
                total += size_of(prev);
                block = prev;
                set_block(block, total, FREE);
            }
        }
    });
}

pub fn used_bytes() -> usize {
    USED_BYTES.load(Ordering::Relaxed)
}

pub fn free_bytes() -> usize {
    HEAP_SIZE - USED_BYTES.load(Ordering::Relaxed)
}

/// **Tek parca** halindeki en buyuk bos blok.
///
/// `free_bytes` ile arasindaki fark parcalanmanin kendisidir: toplam
/// bos alan boluk boluk ise buyuk bir tahsis yine de basarisiz olur.
/// Ayri bir sayi olmasi bilincli -- "bos alan var ama ayrilamiyor"
/// durumunu gorunur kilan tek olcu bu.
pub fn largest_free_block() -> usize {
    crate::arch::cpu::without_interrupts(|| unsafe {
        if !READY.load(Ordering::Relaxed) {
            return HEAP_SIZE - OVERHEAD;
        }
        let mut best = 0usize;
        let mut block = HEAP_START;
        while block < HEAP_END {
            let total = size_of(block);
            if total < MIN_BLOCK || block + total > HEAP_END {
                break;
            }
            if !is_used(block) && total - OVERHEAD > best {
                best = total - OVERHEAD;
            }
            block += total;
        }
        best
    })
}

/// Heap'teki blok sayisi (dolu + bos) -- parcalanmanin ikinci olcusu.
pub fn block_count() -> usize {
    crate::arch::cpu::without_interrupts(|| unsafe {
        if !READY.load(Ordering::Relaxed) {
            return 0;
        }
        let mut count = 0usize;
        let mut block = HEAP_START;
        while block < HEAP_END {
            let total = size_of(block);
            if total < MIN_BLOCK || block + total > HEAP_END {
                break;
            }
            count += 1;
            block += total;
        }
        count
    })
}
