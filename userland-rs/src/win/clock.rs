//! `winclock` -- bir **Windows PE uygulamasi**.
//!
//! Ayni kaynak iki bicime derlenir: i386'da PE32 (`ImageBase
//! 0x00400000`), x86_64'te PE32+ (`ImageBase 0x140000000`). Ekranda
//! gorunen etiketler hangi bicimin kostugunu soyler.
//!
//! Bu program TCMK'nin varlik sebebini tek ekranda gosterir: Rust ile
//! yazilmis, `rust-lld` tarafindan gercek bir PE32 ikilisine linklenmis,
//! `ImageBase = 0x00400000` (Windows'un gelenegi) ile derlenmis ve
//! cekirdek tarafindan **taban yeniden yerlesimi** uygulanarak yuklenmis
//! bir Windows programidir. Tek bir sistem cagrisi bile POSIX degildir:
//! hepsi `int 0x2E` uzerinden NT'ye gider.
//!
//! Buna ragmen ayni ekranda, ayni pencere yoneticisinde, ayni
//! zamanlayicinin altinda ELF uygulamalariyla yan yana kosar. Wine'in
//! yapamadigi sey tam olarak budur: burada Windows ikilisi bir uyumluluk
//! katmaninin konugu degil, cekirdegin **birinci sinif** surecidir.
//!
//! Tuslar:  `s` -> saati diske yaz (NtCreateFile + NtWriteFile),  `q` -> cik

#![no_std]
#![no_main]

use tcmk::nt;
use tcmk::win32::{self, Hwnd};

tcmk::entry!(main);

const BG: u32 = 0x0009_1526;
const PANEL: u32 = 0x0012_2438;
const FG: u32 = 0x00D8_E4F0;
// Bicime bagli etiketler. Ayni kaynagin iki hedefte derlendigini
// ekranda da gostermek icin ayri tutulur.
#[cfg(target_pointer_width = "32")]
const FORMAT_TITLE: &str = "Saat -- PE32 (NT)";
#[cfg(target_pointer_width = "64")]
const FORMAT_TITLE: &str = "Saat -- PE32+ (NT)";

#[cfg(target_pointer_width = "32")]
const FORMAT_BANNER: &str = "[winclock] PE32 uygulamasi basladi, NT cagrilari kullaniliyor.\n";
#[cfg(target_pointer_width = "64")]
const FORMAT_BANNER: &str = "[winclock] PE32+ uygulamasi basladi, NT cagrilari kullaniliyor.\n";

#[cfg(target_pointer_width = "32")]
const FORMAT_LINE: &str = "PE32 . int 0x2E . NtUser*";
#[cfg(target_pointer_width = "64")]
const FORMAT_LINE: &str = "PE32+ . int 0x2E . NtUser*";

#[cfg(target_pointer_width = "32")]
const FORMAT_BASE: &str = "ImageBase 0x00400000 + reloc";
#[cfg(target_pointer_width = "64")]
const FORMAT_BASE: &str = "ImageBase 0x140000000 + DIR64";

const ACCENT: u32 = 0x0046_C8FF;
const WARN: u32 = 0x00FF_C24A;
const OK: u32 = 0x0072_E08A;

const SAVE_PATH: &str = "/clock.txt";

fn main() {
    let mut win = match Hwnd::create(FORMAT_TITLE, 560, 430, 300, 150) {
        Some(w) => w,
        None => return,
    };

    // Konsola da yazalim: seri gunlukte PE'nin gercekten kostugu gorunur.
    let mut console = win32::Console;
    let _ = core::fmt::Write::write_str(
        &mut console,
        FORMAT_BANNER,
    );

    let mut status: &str = "s = kaydet, q = cik";
    let mut status_color = FG;

    loop {
        match win.get_message() {
            b'q' => break,
            b's' => {
                if save_time() {
                    status = "/clock.txt yazildi";
                    status_color = OK;
                } else {
                    status = "yazilamadi";
                    status_color = WARN;
                }
            }
            _ => {}
        }

        let now = win32::system_time();

        win.clear(BG);

        // Saat panosu.
        let w = win.width();
        win.fill(0, 0, w, 60, PANEL);

        let mut x = 12;
        x += draw_pair(&mut win, x, 14, now.hour);
        x += win.text_scaled(x, 14, ":", 3, ACCENT);
        x += draw_pair(&mut win, x, 14, now.minute);
        x += win.text_scaled(x, 14, ":", 3, ACCENT);
        let _ = draw_pair(&mut win, x, 14, now.second);

        // Tarih.
        let mut cx = 12;
        cx += win.number(cx, 70, now.year as usize, FG);
        win.glyph(cx, 70, b'-', FG);
        cx += 8;
        cx += draw_pair_small(&mut win, cx, 70, now.month);
        win.glyph(cx, 70, b'-', FG);
        cx += 8;
        let _ = draw_pair_small(&mut win, cx, 70, now.day);

        // Kimlik satiri: bu pencerenin arkasinda bir PE var.
        win.text(10, 90, FORMAT_LINE, ACCENT);
        win.text(10, 106, FORMAT_BASE, FG);
        win.text(10, 126, status, status_color);

        // Saniye cubugu -- saatin gercekten aktigini gorunur kilar.
        let bar = w.saturating_sub(20) * (now.second as usize) / 59;
        win.fill(10, 60, bar, 3, ACCENT);

        win.frame(200);
    }

    let _ = core::fmt::Write::write_str(&mut console, "[winclock] cikiliyor.\n");
}

/// Iki haneli buyuk sayi (basinda sifirla).
fn draw_pair(win: &mut Hwnd, x: usize, y: usize, value: u8) -> usize {
    let tens = (value / 10) as usize;
    let ones = (value % 10) as usize;
    win.glyph_scaled(x, y, b'0' + tens as u8, 3, FG);
    win.glyph_scaled(x + 24, y, b'0' + ones as u8, 3, FG);
    48
}

/// Iki haneli normal boyutlu sayi.
fn draw_pair_small(win: &mut Hwnd, x: usize, y: usize, value: u8) -> usize {
    win.glyph(x, y, b'0' + (value / 10), FG);
    win.glyph(x + 8, y, b'0' + (value % 10), FG);
    16
}

/// Iki haneli sayiyi tampona basar.
fn two(buf: &mut [u8], at: usize, value: u8) {
    buf[at] = b'0' + value / 10;
    buf[at + 1] = b'0' + value % 10;
}

/// Saati diske yazar -- yalnizca NT cagrilariyla.
///
/// `NtCreateFile(FILE_CREATE)` dosyayi olusturur, `NtWriteFile` icerigi
/// yazar, `NtClose` tutamaci birakir. Ayni TCMKFS bolumune ELF tarafi da
/// yazar; dosya sistemi hangi ABI'den gelindigini bilmez.
fn save_time() -> bool {
    let now = win32::system_time();

    // "YY-MM-DD HH:MM:SS\n" -- sabit uzunluklu, bicimleyici gerektirmez.
    let mut line = [b'0'; 18];
    two(&mut line, 0, (now.year % 100) as u8);
    line[2] = b'-';
    two(&mut line, 3, now.month);
    line[5] = b'-';
    two(&mut line, 6, now.day);
    line[8] = b' ';
    two(&mut line, 9, now.hour);
    line[11] = b':';
    two(&mut line, 12, now.minute);
    line[14] = b':';
    two(&mut line, 15, now.second);
    line[17] = b'\n';
    let n = line.len();

    let mut name = [0u8; 32];
    name[..SAVE_PATH.len()].copy_from_slice(SAVE_PATH.as_bytes());

    let (status, handle) = unsafe {
        nt::syscall3_out(
            nt::NT_CREATE_FILE,
            name.as_ptr() as usize,
            nt::FILE_CREATE,
            0,
        )
    };
    if status as u32 != nt::STATUS_SUCCESS {
        return false;
    }

    let wrote = nt::write_console(handle, &line[..n]) == nt::STATUS_SUCCESS;
    unsafe { nt::syscall1(nt::NT_CLOSE, handle) };
    wrote
}
