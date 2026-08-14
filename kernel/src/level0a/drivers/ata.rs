//! ATA PIO disk surucusu (doc S.2.2.C "Suruculer: Disk (ATA/AHCI)").
//!
//! Neden ATA PIO ile baslandi: port I/O disinda hicbir sey gerektirmez --
//! PCI taramasi, DMA, kesme yonetimi yok. Hem QEMU'da hem gercek (eski)
//! donanimda calisir. Yavastir (sektor basina 256 `in ax, dx`) ama dosya
//! sistemi ve kurulum icin fazlasiyla yeterlidir.
//!
//! ## Kesme (IRQ14) ile bekleme
//!
//! Surucu artik **kesmeyle** bekler: komut verildikten sonra gorev
//! `TaskState::IoWait`'e gecer ve IRQ14 gelene kadar zamanlanmaz. Disk
//! calisirken CPU baska sureclere ait olur -- onceki yoklamali surumde
//! buyuk bir yazma sistemi o sure boyunca durduruyordu.
//!
//! Bunun bugune kadar yapilmamis olmasinin sebebi bir **vektor
//! catismasiydi**: alisilmis PIC yerlesiminde IRQ14 vektor 46'ya duser,
//! ki orasi `int 0x2E`nin -- Windows NT sistem cagrisinin -- yeridir.
//! 0x2E bir ABI oldugu icin tasinabilir olan taraf PIC'ti; slave
//! denetleyici 0x70'e alindi (bkz. `pic.rs`) ve IRQ14 artik 118.
//!
//! Iki yer bilerek yoklamada kaldi:
//!
//! * **Acilis** (`init`): zamanlayici henuz yok, uyutulacak gorev de yok.
//! * **Idle gorevi (task 0)**: o gorev ayni zamanda masaustu dongusudur;
//!   uyutmak ekrani dondururdu. Kabuktan verilen disk komutlari bu yola
//!   duser.
//!
//! Kesme gelmezse (kayip IRQ, arizali aygit) bekleme suresi dolar ve
//! yoklama yoluna dusulur -- yani kesme bir **hizlandirici**dir, tek
//! dogruluk kaynagi degil.
//!
//! LBA28 kullanilir: 2^28 sektor = 128 GiB adresleme. LBA48 gerektiginde
//! ayni yapiya eklenebilir.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::arch::cpu::{inb, inw, outb, outw};
use crate::level0a::drivers::block::{BlockError, SECTOR_SIZE};

// Birincil ATA kanali.
const DATA: u16 = 0x1F0;
const FEATURES: u16 = 0x1F1;
const SECTOR_COUNT: u16 = 0x1F2;
const LBA_LOW: u16 = 0x1F3;
const LBA_MID: u16 = 0x1F4;
const LBA_HIGH: u16 = 0x1F5;
const DRIVE: u16 = 0x1F6;
const STATUS: u16 = 0x1F7;
const COMMAND: u16 = 0x1F7;
/// Alternatif durum: okumak kesme bayragini TEMIZLEMEZ; gecikme icin uygun.
const ALT_STATUS: u16 = 0x3F6;
/// Ayni adres yazildiginda aygit denetim registeridir (nIEN, SRST).
const DEVICE_CONTROL: u16 = 0x3F6;

const STATUS_ERR: u8 = 0x01;
const STATUS_DRQ: u8 = 0x08;
const STATUS_DF: u8 = 0x20;
const STATUS_DRDY: u8 = 0x40;
const STATUS_BSY: u8 = 0x80;

const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_FLUSH_CACHE: u8 = 0xE7;
const CMD_IDENTIFY: u8 = 0xEC;

/// Yoklama siniri. Gercek bir diskin en kotu durum yanit suresi uzundur ama
/// burada takilip kalmak yerine hata dondurmek yeglenir: disk arizasi
/// sistemi kilitlememelidir.
const SPIN_LIMIT: u32 = 5_000_000;

/// Kesme beklemesinin ust siniri (PIT tik'i, 100 Hz -> 2 saniye).
const IO_TIMEOUT_TICKS: u32 = 200;

static PRESENT: AtomicBool = AtomicBool::new(false);
static TOTAL_SECTORS: AtomicU32 = AtomicU32::new(0);
static READS: AtomicU32 = AtomicU32::new(0);
static WRITES: AtomicU32 = AtomicU32::new(0);
static ERRORS: AtomicU32 = AtomicU32::new(0);
/// IRQ14 geldi mi? Her komuttan **once** temizlenir.
static IRQ_SEEN: AtomicBool = AtomicBool::new(false);
static IRQ_COUNT: AtomicU32 = AtomicU32::new(0);
/// Aygit kilidi: ayni anda yalnizca bir gorev ATA komutu yurutebilir.
static LOCKED: AtomicBool = AtomicBool::new(false);

/// Model dizesi (IDENTIFY'dan, 40 karakter).
static mut MODEL: [u8; 40] = [b' '; 40];

/// Aygit kilidi. Bir komut bolunemez: ortasinda baska bir gorev ikinci
/// bir ATA komutu baslatirsa denetleyicinin durumu bozulur.
///
/// Onceki surum bunu `preempt_disable` ile saglardi -- yani "kimse
/// calismasin" diyerek. Artik yalnizca **ATA'ya dokunmak** yasak; baska
/// gorevler serbestce kosar, ki kesmeyle beklemenin butun anlami da
/// budur.
///
/// `Drop` ile birakilmasi sart: `?` ile erken donen her hata yolu da
/// kilidi birakmis olur.
struct DeviceLock;

impl DeviceLock {
    fn acquire() -> Self {
        loop {
            if LOCKED
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return DeviceLock;
            }
            crate::level0a::core::scheduler::yield_now();
        }
    }
}

impl Drop for DeviceLock {
    fn drop(&mut self) {
        LOCKED.store(false, Ordering::Release);
    }
}

/// IRQ14 isleyicisi (bkz. `idt`).
///
/// Durum registerini okumak aygitin kesme bayragini **temizler**; bu
/// yapilmazsa denetleyici bir daha kesme uretmez ve surucu ilk komuttan
/// sonra sessizce yoklamaya duser.
pub fn on_irq() {
    let _ = unsafe { inb(STATUS) };
    IRQ_SEEN.store(true, Ordering::SeqCst);
    IRQ_COUNT.fetch_add(1, Ordering::Relaxed);
    crate::level0a::core::scheduler::wake_io_waiters();
}

/// Komut oncesi kesme bayragini temizler.
fn arm_irq() {
    IRQ_SEEN.store(false, Ordering::SeqCst);
}

/// Kesmeyi bekler. Beklenemeyen baglamlarda (idle, masaustu, acilis)
/// hicbir sey yapmadan doner ve cagiran yoklamaya duser.
///
/// "Uyutulamayan gorev" sorusunun cevabi zamanlayicidadir
/// (`scheduler::can_block`), burada degil: eskiden burada "0 numara mi?"
/// diye bakiliyordu ve masaustu ayri bir goreve tasindiginda bu sessizce
/// yanlis cevap vermeye basladi.
fn await_irq() {
    if !crate::level0a::core::scheduler::current_can_block() {
        return;
    }
    // Uyanma kosulu **iki** sey: kesme geldi, YA DA aygit artik mesgul
    // degil.
    //
    // Ikincisi olmadan bekleme kesmeye bagimli hale geliyordu -- ve
    // kesme gelmedigi ortamlarda (olculdu: IRQ14 sayaci acilistaki
    // birinci kesmeden sonra hic artmiyor) her bekleme zaman asimi
    // suresince, iki saniye, bosuna suruyordu. Ring 3'ten yapilan bir
    // kaydetme boylece onlarca saniye aliyor, uygulama donmus
    // gorunuyordu.
    //
    // `ALT_STATUS` bilerek: onu okumak aygitin kesme bayragini
    // TEMIZLEMEZ, yani yoklama kesme yolunu bozmaz. Modul basligindaki
    // "kesme bir hizlandiricidir, tek dogruluk kaynagi degil" ifadesi
    // ancak bu satirla gercekten dogru oluyor.
    crate::level0a::core::scheduler::wait_for_io(
        || {
            IRQ_SEEN.load(Ordering::SeqCst)
                || unsafe { inb(ALT_STATUS) } & STATUS_BSY == 0
        },
        IO_TIMEOUT_TICKS,
    );
    IRQ_SEEN.store(false, Ordering::SeqCst);
}

/// Surucu/kanal secildikten sonra gereken ~400 ns bekleme: alternatif durum
/// dort kez okunur (her okuma bir ATA saat cevrimi surer).
fn io_delay() {
    for _ in 0..4 {
        unsafe { inb(ALT_STATUS) };
    }
}

fn wait_while_busy() -> Result<u8, BlockError> {
    for _ in 0..SPIN_LIMIT {
        let status = unsafe { inb(STATUS) };
        if status & STATUS_BSY == 0 {
            return Ok(status);
        }
    }
    Err(BlockError::Timeout)
}

/// BSY dususunu ve DRQ yukselisini bekler; ERR/DF gorurse hata doner.
///
/// Once kesme beklenir; kesme geldiginde (ya da beklenemeyen bir
/// baglamdaysak hemen) durum bitleri yine de dogrulanir -- kesme "bir sey
/// oldu" der, "ne oldugunu" durum registeri soyler.
fn wait_for_data() -> Result<(), BlockError> {
    await_irq();
    for _ in 0..SPIN_LIMIT {
        let status = unsafe { inb(STATUS) };
        if status & STATUS_BSY != 0 {
            continue;
        }
        if status & (STATUS_ERR | STATUS_DF) != 0 {
            ERRORS.fetch_add(1, Ordering::Relaxed);
            return Err(BlockError::DeviceError);
        }
        if status & STATUS_DRQ != 0 {
            return Ok(());
        }
    }
    Err(BlockError::Timeout)
}

/// Diski tanir; bulunursa sektor sayisini ve modelini kaydeder.
///
/// # Safety
/// Yalnizca acilista, kesmeler kapaliyken cagrilmalidir.
pub unsafe fn init() -> bool {
    // Aygit denetim registeri: nIEN (bit 1) = 0 -> kesme URETILSIN.
    // Varsayilan zaten budur, ama acikca yazmak BIOS'un kapatmis olma
    // ihtimalini ortadan kaldirir -- kapaliysa IRQ14 hic gelmez ve
    // surucu sessizce yoklamaya duserdi.
    outb(DEVICE_CONTROL, 0x00);

    // Birincil master'i sec.
    outb(DRIVE, 0xA0);
    io_delay();
    outb(SECTOR_COUNT, 0);
    outb(LBA_LOW, 0);
    outb(LBA_MID, 0);
    outb(LBA_HIGH, 0);
    outb(COMMAND, CMD_IDENTIFY);
    io_delay();

    // Durum 0 ise bu kanalda surucu yok.
    if inb(STATUS) == 0 {
        return false;
    }

    if wait_while_busy().is_err() {
        return false;
    }

    // LBA_MID/HIGH sifir degilse cihaz ATA degil (ATAPI/SATA imzasi).
    if inb(LBA_MID) != 0 || inb(LBA_HIGH) != 0 {
        return false;
    }

    if wait_for_data().is_err() {
        return false;
    }

    let mut identify = [0u16; 256];
    for word in identify.iter_mut() {
        *word = inw(DATA);
    }

    // Kelime 60-61: LBA28 toplam sektor sayisi.
    let sectors = (identify[60] as u32) | ((identify[61] as u32) << 16);
    TOTAL_SECTORS.store(sectors, Ordering::Relaxed);

    // Kelime 27-46: model dizesi, her kelimede iki karakter TERS sirada.
    let model = core::ptr::addr_of_mut!(MODEL) as *mut u8;
    for i in 0..20 {
        let word = identify[27 + i];
        model.add(i * 2).write((word >> 8) as u8);
        model.add(i * 2 + 1).write((word & 0xFF) as u8);
    }

    PRESENT.store(true, Ordering::Relaxed);
    true
}

fn setup_lba(lba: u32, count: u8) -> Result<(), BlockError> {
    if !PRESENT.load(Ordering::Relaxed) {
        return Err(BlockError::NoDevice);
    }
    let total = TOTAL_SECTORS.load(Ordering::Relaxed);
    if lba.checked_add(count as u32).map_or(true, |end| end > total) {
        return Err(BlockError::OutOfRange);
    }
    if lba >= 1 << 28 {
        return Err(BlockError::OutOfRange);
    }

    wait_while_busy()?;
    unsafe {
        // 0xE0 = LBA modu, master; alt dort bit LBA'nin 27-24. bitleri.
        outb(DRIVE, 0xE0 | ((lba >> 24) & 0x0F) as u8);
        io_delay();
        outb(FEATURES, 0);
        outb(SECTOR_COUNT, count);
        outb(LBA_LOW, (lba & 0xFF) as u8);
        outb(LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(LBA_HIGH, ((lba >> 16) & 0xFF) as u8);
    }
    Ok(())
}

/// `count` sektoru `lba`'dan okur. `buf` en az `count * 512` bayt olmalidir.
pub fn read(lba: u32, count: u8, buf: &mut [u8]) -> Result<(), BlockError> {
    if count == 0 {
        return Ok(());
    }
    if buf.len() < count as usize * SECTOR_SIZE {
        return Err(BlockError::BufferTooSmall);
    }

    let _guard = DeviceLock::acquire();
    setup_lba(lba, count)?;
    arm_irq();
    unsafe { outb(COMMAND, CMD_READ_SECTORS) };

    for sector in 0..count as usize {
        wait_for_data()?;
        let base = sector * SECTOR_SIZE;
        for word in 0..SECTOR_SIZE / 2 {
            let value = unsafe { inw(DATA) };
            buf[base + word * 2] = (value & 0xFF) as u8;
            buf[base + word * 2 + 1] = (value >> 8) as u8;
        }
    }

    READS.fetch_add(count as u32, Ordering::Relaxed);
    Ok(())
}

/// `count` sektoru `lba`'ya yazar ve disk onbellegini bosaltir.
pub fn write(lba: u32, count: u8, buf: &[u8]) -> Result<(), BlockError> {
    if count == 0 {
        return Ok(());
    }
    if buf.len() < count as usize * SECTOR_SIZE {
        return Err(BlockError::BufferTooSmall);
    }

    let _guard = DeviceLock::acquire();
    setup_lba(lba, count)?;
    // Yazmada ilk sektor icin kesme BEKLENMEZ: aygit komuttan hemen sonra
    // DRQ'yu kaldirir ve veriyi ister. Kesme, her sektor yazildiktan
    // SONRA gelir. Bu yuzden bayrak burada kurulur ama ilk `wait_for_data`
    // cagrisi cogunlukla kesme beklemeden durumu hazir bulur.
    arm_irq();
    unsafe { outb(COMMAND, CMD_WRITE_SECTORS) };

    for sector in 0..count as usize {
        wait_for_data()?;
        let base = sector * SECTOR_SIZE;
        for word in 0..SECTOR_SIZE / 2 {
            let value = (buf[base + word * 2] as u16) | ((buf[base + word * 2 + 1] as u16) << 8);
            unsafe { outw(DATA, value) };
        }
    }

    // Onbellek bosaltma olmadan yazilanlar guc kesintisinde kaybolabilir.
    wait_while_busy()?;
    arm_irq();
    unsafe { outb(COMMAND, CMD_FLUSH_CACHE) };
    // Flush uzun surebilir (aygit gercekten diske yaziyor); beklemenin en
    // cok ise yaradigi yer burasidir.
    await_irq();
    let status = wait_while_busy()?;
    if status & (STATUS_ERR | STATUS_DF) != 0 {
        ERRORS.fetch_add(1, Ordering::Relaxed);
        return Err(BlockError::DeviceError);
    }

    WRITES.fetch_add(count as u32, Ordering::Relaxed);
    Ok(())
}

pub fn total_sectors() -> u32 {
    TOTAL_SECTORS.load(Ordering::Relaxed)
}

pub fn model() -> &'static str {
    unsafe {
        let model = core::ptr::addr_of!(MODEL) as *const u8;
        let bytes = core::slice::from_raw_parts(model, 40);
        let end = bytes
            .iter()
            .rposition(|b| *b != b' ' && *b != 0)
            .map_or(0, |i| i + 1);
        core::str::from_utf8(&bytes[..end]).unwrap_or("?")
    }
}

/// Gelen IRQ14 sayisi -- kesme yolunun gercekten kullanildiginin kaniti.
pub fn irq_count() -> u32 {
    IRQ_COUNT.load(Ordering::Relaxed)
}

pub fn stats() -> (u32, u32, u32) {
    (
        READS.load(Ordering::Relaxed),
        WRITES.load(Ordering::Relaxed),
        ERRORS.load(Ordering::Relaxed),
    )
}

/// Surucunun hazir olup olmadigini kontrol eder (kabuk `disk` komutu).
pub fn drive_ready() -> bool {
    PRESENT.load(Ordering::Relaxed) && unsafe { inb(STATUS) } & STATUS_DRDY != 0
}
