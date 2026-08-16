//! Klavye suruculugu (IRQ1): port 0x60'tan scancode set 1 okunur, event-driven
//! G/C ile ekrana yansitilir (doc S.4). US duzeni, shift ve caps lock.
//!
//! ## Neden buyuk harf artik zorunlu
//!
//! Ilk surumde yalnizca kucuk harf vardi ve bu uzun sure sorun olmadi:
//! komutlar (`ls`, `run`, `mkdir`) ve yollar hep kucuktu. Ortam
//! degiskenleri geldiginde is degisti -- `HOME`, `PATH`, `SHELL` gelenek
//! geregi **buyuk** harflidir, ve `set HOME=/tmp` yazilamayan bir kabuk
//! o degiskenleri kullanilamaz kilar. Yani eksik olan bir konfor degil,
//! bir yetenekti.
//!
//! Iki degistirici ayri calisir, cunku etki alanlari ayri:
//!
//!   * **Shift** her tusu etkiler (`1` -> `!`, `[` -> `{`) ve **basili
//!     oldugu surece** gecerlidir; birakma kodu (bit 7) ile duser.
//!   * **Caps lock** yalnizca **harfleri** etkiler ve bir **anahtardir**:
//!     basma aninda devrilir, birakma yok sayilir.
//!
//! Ikisi harflerde birbirini **gotururur** (`caps` acikken `shift+a`
//! yine kucuk `a` verir) -- gercek klavyelerin davranisi budur.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::cpu::inb;

const DATA_PORT: u16 = 0x60;

/// Sol/sag shift'in **make** kodlari (birakma = +0x80).
const LSHIFT: u8 = 0x2A;
const RSHIFT: u8 = 0x36;
/// Caps lock -- yalnizca basma kenari onemli.
const CAPS_LOCK: u8 = 0x3A;

static SHIFT: AtomicBool = AtomicBool::new(false);
static CAPS: AtomicBool = AtomicBool::new(false);

#[rustfmt::skip]
const SCANCODE_ASCII: [u8; 128] = [
    0, 0x1B, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'=', 0x08, b'\t',
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p', b'[', b']', b'\n', 0,
    b'a', b's', b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';', b'\'', b'`', 0, b'\\',
    b'z', b'x', b'c', b'v', b'b', b'n', b'm', b',', b'.', b'/', 0, b'*', 0, b' ',
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Shift basiliyken uretilen karakterler -- US duzeni.
///
/// Ayri bir tablo, `ch - 32` gibi bir aritmetikten daha dogru: noktalama
/// esleri (`1`->`!`, `/`->`?`) harf donusumune benzemez, duzenin kendi
/// sozlesmesidir. Bir gun TR duzeni eklenirse degisecek yer de burasi.
#[rustfmt::skip]
const SCANCODE_ASCII_SHIFT: [u8; 128] = [
    0, 0x1B, b'!', b'@', b'#', b'$', b'%', b'^', b'&', b'*', b'(', b')', b'_', b'+', 0x08, b'\t',
    b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I', b'O', b'P', b'{', b'}', b'\n', 0,
    b'A', b'S', b'D', b'F', b'G', b'H', b'J', b'K', b'L', b':', b'"', b'~', 0, b'|',
    b'Z', b'X', b'C', b'V', b'B', b'N', b'M', b'<', b'>', b'?', 0, b'*', 0, b' ',
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Caps lock su an acik mi -- kabugun `keys` komutu bunu gosteriyor.
pub fn caps_on() -> bool {
    CAPS.load(Ordering::Relaxed)
}

/// IRQ1 handler'i tarafindan cagrilir.
pub fn on_irq() {
    let scancode = unsafe { inb(DATA_PORT) };

    // Bit 7 = 1 -> tus birakma. Yalnizca degistiriciler icin anlamli:
    // shift'in **dusmesi** bir olaydir, harflerin degil.
    if scancode & 0x80 != 0 {
        let made = scancode & 0x7F;
        if made == LSHIFT || made == RSHIFT {
            SHIFT.store(false, Ordering::Relaxed);
        }
        return;
    }

    match scancode {
        LSHIFT | RSHIFT => {
            SHIFT.store(true, Ordering::Relaxed);
            return;
        }
        // Anahtar: her basmada devrilir, birakma yok sayilir.
        CAPS_LOCK => {
            CAPS.store(!CAPS.load(Ordering::Relaxed), Ordering::Relaxed);
            return;
        }
        _ => {}
    }

    let shift = SHIFT.load(Ordering::Relaxed);
    let mut ch = if shift {
        SCANCODE_ASCII_SHIFT[scancode as usize]
    } else {
        SCANCODE_ASCII[scancode as usize]
    };
    if ch == 0 {
        return;
    }

    // Caps yalnizca harfleri etkiler ve shift ile **birbirini goturur**:
    // ikisi de acikken sonuc yine kucuk harftir.
    if CAPS.load(Ordering::Relaxed) {
        if ch.is_ascii_lowercase() {
            ch = ch.to_ascii_uppercase();
        } else if ch.is_ascii_uppercase() {
            ch = ch.to_ascii_lowercase();
        }
    }

    // GUI aktifken tuslar olay kuyruguna gider (pencereler tuketir);
    // GUI yokken dogrudan konsola yankilanir.
    if crate::level0a::drivers::console::wm_owns_screen() {
        crate::level0a::input::on_key(ch);
    } else {
        crate::print!("{}", ch as char);
    }
}
