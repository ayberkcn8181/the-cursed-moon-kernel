//! CMOS gercek zaman saati (RTC) surucusu.
//!
//! PIT sistemin **ne kadar suredir calistigini** olcer; duvardaki saati
//! bilmez. Dosya zaman damgalari, gunluk kayitlari ve kullaniciya
//! gosterilen saat icin gercek tarih gerekir -- CMOS RTC'yi okumak bunun
//! en ucuz yolu: iki port, birkac register.
//!
//! ## Okuma sirasindaki tuzak
//!
//! RTC saniyede bir kendini gunceller ve guncelleme **atomik degildir**:
//! tam o anda okunan degerler tutarsiz olabilir (ornegin 01:59:59 ile
//! 02:00:00 arasinda dakika yeni, saniye eski). Iki savunma birlikte
//! kullanilir:
//!   1. "guncelleme suruyor" bayragi (Status A, bit 7) bekleniyor,
//!   2. okuma iki kez yapilip **ayni sonucu verene kadar** tekrarlanir.
//!
//! ## Bicim
//!
//! BIOS'a gore degerler BCD (0x59 = 59) veya ikili olabilir; saat 12 ya
//! da 24 saatlik olabilir. Status B bunu soyler ve donusum burada yapilir,
//! boylece ust katmanlar tek bir bicim gorur.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::arch::cpu::{inb, outb};

const CMOS_INDEX: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

const REG_SECOND: u8 = 0x00;
const REG_MINUTE: u8 = 0x02;
const REG_HOUR: u8 = 0x04;
const REG_DAY: u8 = 0x07;
const REG_MONTH: u8 = 0x08;
const REG_YEAR: u8 = 0x09;
const REG_STATUS_A: u8 = 0x0A;
const REG_STATUS_B: u8 = 0x0B;

const STATUS_A_UPDATING: u8 = 0x80;
const STATUS_B_BINARY: u8 = 0x04;
const STATUS_B_24H: u8 = 0x02;

/// Okuma sirasinda takilip kalmamak icin ust sinir.
const SPIN_LIMIT: u32 = 1_000_000;

static AVAILABLE: AtomicBool = AtomicBool::new(false);
/// Acilis ani (Unix zamani) -- `date` ve `uptime` icin.
static BOOT_EPOCH: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

fn read_register(register: u8) -> u8 {
    unsafe {
        // Bit 7 = NMI'yi kapat; CMOS erisimi sirasinda gelenek budur.
        outb(CMOS_INDEX, register | 0x80);
        inb(CMOS_DATA)
    }
}

fn update_in_progress() -> bool {
    read_register(REG_STATUS_A) & STATUS_A_UPDATING != 0
}

fn from_bcd(value: u8) -> u8 {
    (value & 0x0F) + ((value >> 4) * 10)
}

fn read_raw() -> DateTime {
    DateTime {
        second: read_register(REG_SECOND),
        minute: read_register(REG_MINUTE),
        hour: read_register(REG_HOUR),
        day: read_register(REG_DAY),
        month: read_register(REG_MONTH),
        year: read_register(REG_YEAR) as u16,
    }
}

/// RTC'yi okur. Cihaz yoksa/okunamazsa `None`.
pub fn now() -> Option<DateTime> {
    if !AVAILABLE.load(Ordering::Relaxed) {
        return None;
    }

    let mut spins = 0u32;
    while update_in_progress() {
        spins += 1;
        if spins > SPIN_LIMIT {
            return None;
        }
    }

    let mut previous = read_raw();
    // Ust ustę ayni sonucu verene kadar tekrarla: guncelleme tam okuma
    // sirasinda gerceklesirse alanlar farkli anlara ait olurdu.
    loop {
        while update_in_progress() {
            spins += 1;
            if spins > SPIN_LIMIT {
                return None;
            }
        }
        let current = read_raw();
        if current == previous {
            break;
        }
        previous = current;
        spins += 1;
        if spins > SPIN_LIMIT {
            return None;
        }
    }

    let status_b = read_register(REG_STATUS_B);
    let mut time = previous;

    // 12 saatlik bicimde ogleden sonra bayragi saatin en ust bitindedir.
    let pm = status_b & STATUS_B_24H == 0 && time.hour & 0x80 != 0;
    time.hour &= 0x7F;

    if status_b & STATUS_B_BINARY == 0 {
        time.second = from_bcd(time.second);
        time.minute = from_bcd(time.minute);
        time.hour = from_bcd(time.hour);
        time.day = from_bcd(time.day);
        time.month = from_bcd(time.month);
        time.year = from_bcd(time.year as u8) as u16;
    }

    if pm && time.hour < 12 {
        time.hour += 12;
    }
    if status_b & STATUS_B_24H == 0 && !pm && time.hour == 12 {
        time.hour = 0;
    }

    // CMOS yalnizca iki haneli yil tutar; yuzyil register'i her BIOS'ta
    // yok. 2000-2099 varsaymak, bu cekirdegin omru icin yeterli.
    time.year += 2000;

    Some(time)
}

/// Gun sayisini Unix epoch'tan (1970-01-01) itibaren hesaplar.
fn days_from_civil(year: u16, month: u8, day: u8) -> u32 {
    // Howard Hinnant'in civil_from_days tersi; artik yil kurallarini
    // tablosuz ve dogru sekilde isler.
    let y = if month <= 2 { year as i32 - 1 } else { year as i32 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32; // 0..399
    let m = month as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as u32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146097 + doe as i32 - 719468) as u32
}

/// Unix zamani (1970'ten beri gecen saniye).
pub fn unix_time() -> u32 {
    match now() {
        Some(t) => {
            days_from_civil(t.year, t.month, t.day) * 86400
                + t.hour as u32 * 3600
                + t.minute as u32 * 60
                + t.second as u32
        }
        None => 0,
    }
}

/// RTC'yi tanir; makul bir tarih okuyabiliyorsa etkinlestirir.
///
/// # Safety
/// Acilista, kesmeler kapaliyken cagrilmalidir.
pub unsafe fn init() -> bool {
    AVAILABLE.store(true, Ordering::Relaxed);

    // Saglik kontrolu: cihaz yoksa portlar 0xFF doner ve tarih sacmalar.
    let ok = match now() {
        Some(t) => t.month >= 1 && t.month <= 12 && t.day >= 1 && t.day <= 31 && t.hour < 24,
        None => false,
    };

    AVAILABLE.store(ok, Ordering::Relaxed);
    if ok {
        BOOT_EPOCH.store(unix_time(), Ordering::Relaxed);
    }
    ok
}

pub fn available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

pub fn boot_epoch() -> u32 {
    BOOT_EPOCH.load(Ordering::Relaxed)
}

/// Unix zamanini takvim tarihine cevirir (dosya listeleri icin).
pub fn from_unix(epoch: u32) -> DateTime {
    let days = (epoch / 86400) as i32;
    let seconds = epoch % 86400;

    // civil_from_days (Hinnant).
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;

    DateTime {
        year: (if month <= 2 { y + 1 } else { y }) as u16,
        month,
        day,
        hour: (seconds / 3600) as u8,
        minute: ((seconds % 3600) / 60) as u8,
        second: (seconds % 60) as u8,
    }
}
