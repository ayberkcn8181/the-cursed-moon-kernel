//! `browse` -- `getdents` + `chdir`/`getcwd` gosterimi.
//!
//! Bu cagriya kadar bir uygulama dosya sistemini **goremiyordu**. Adini
//! onceden bildigi bir dosyayi acabiliyordu (`open`), okuyabiliyordu
//! (`read`), boyutunu ogrenebiliyordu (`fstat`) -- ama "burada ne var?"
//! diye soramiyordu. `getdents` o bosluktur: bir dizin de dosya gibi
//! `open` ile acilir, ama `read` yerine `getdents` ile okunur.
//!
//! ```text
//!   fd = open(".", 0)          -> dizin tanimlayicisi
//!   getdents(fd, buf, 512)     -> 96   (kayitlar)
//!   getdents(fd, buf, 512)     -> 0    (bitti)
//! ```
//!
//! ## Yolu artik program tasimiyor
//!
//! Ilk surumde bu program gezdigi yolu **kendi icinde** tutuyordu:
//! `Path` diye bir yapi, `push`/`pop`, ve her cagri icin `dizin + "/" +
//! ad` birlestirmesi. Sebep basitti -- Ring 3'te calisma dizini diye bir
//! sey yoktu, cekirdek yalnizca mutlak yol kabul ediyordu.
//!
//! `chdir`/`getcwd` geldikten sonra o kodun tamami **silindi**. Dizine
//! girmek `chdir(ad)`, cikmak `chdir("..")`, listelemek `open(".")`,
//! dizin acmak `mkdir("posix1")`. Hicbirinde yol birlestirmesi yok:
//! goreli adlari cekirdek surecin dizinine gore cozuyor.
//!
//! Ustteki yol satiri da tahmin degil, `getcwd`in cevabi. Yani ekranda
//! gorunen yol ile cekirdegin cozdugu yol **ayni kaynaktan** geliyor;
//! ayrisamazlar.
//!
//! Yazma da var: `n` yeni dizin (`mkdir`), `m` yeniden adlandir
//! (`rename`), `d` sil (`rmdir`/`unlink`). Olusturulan dizinlerin adi
//! `posixN`; Win32 tarafindaki `winfiles` de `win32N` yaratir. Iki
//! uygulamayi yan yana calistirip birinin yarattigini otekinde gormek,
//! iki ABI'nin ayni dosya sistemine baktiginin dogrudan kanitidir.
//!
//! Ad uretimi ayrica `EEXIST`i sinar: `posix1` varken tekrar denemek
//! hata doner, program bir sonraki numaraya gecer.
//!
//! ## Arguman
//!
//! `run browse /notlar` -- verilen dizinde acilir. Cekirdek argumanlari
//! yigina `argc`/`argv` olarak koyuyor; `tcmk::args` onlari okuyor.
//! Win32 tarafindaki `winfiles` ayni isi `GetCommandLineA` ile yapar --
//! ayni yetenek, iki ayri ABI sozlesmesi.
//!
//! Tuslar: `j`/`k` -> asagi/yukari, Enter -> dizine gir, `u` -> ust dizin,
//! `n` -> yeni dizin, `m` -> yeniden adlandir, `d` -> sil,
//! `r` -> yeniden oku, `q` -> cik

#![no_std]
#![no_main]

use tcmk::args;
use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys::{self, ReadDir};

tcmk::entry!(main);

const BG: u32 = 0x000E_1420;
const PANEL: u32 = 0x001A_2436;
const FG: u32 = 0x00D4_DCEC;
const DIM: u32 = 0x0078_8498;
const ACCENT: u32 = 0x0068_D0FF;
const DIRC: u32 = 0x00FF_C86A;
const SELECT: u32 = 0x0028_4460;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// POSIX `ENOTEMPTY` -- iki mimaride de 39.
const ENOTEMPTY: isize = 39;
/// POSIX `EROFS`: salt okunur dosya sistemi (RAMFS).
const EROFS: isize = 30;

/// Ekranda tutulan en fazla girdi. Daha fazlasi varsa listelenmeyenlerin
/// sayisi alt satirda gosterilir -- sessizce kirpmak yaniltici olurdu.
const MAX_ROWS: usize = 14;
/// Bir ad icin ayrilan yer (sondaki NUL dahil).
const MAX_NAME: usize = 32;
/// `getcwd` icin tampon.
const MAX_PATH: usize = 128;

/// Onbellege alinmis tek bir girdi.
///
/// Ad **NUL ile kapatilir**: dogrudan `mkdir`/`unlink`e verilebilsin
/// diye. Yol birlestirmesi artik yok, cunku cekirdek goreli adi surecin
/// dizinine gore cozuyor.
#[derive(Clone, Copy)]
struct Row {
    name: [u8; MAX_NAME],
    len: usize,
    size: usize,
    is_dir: bool,
}

impl Row {
    const fn empty() -> Self {
        Row {
            name: [0; MAX_NAME],
            len: 0,
            size: 0,
            is_dir: false,
        }
    }

    fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.len]).unwrap_or("?")
    }
}

/// Bos bir numara bulup `posixN` dizinini acar -- **goreli adla**.
///
/// Numara arama `EEXIST`in gercekten dondugunu de sinar: `posix1` varken
/// cagri hata verir ve dongu bir sonrakine gecer.
fn make_dir() -> (&'static str, u32) {
    let mut name = *b"posix0\0";
    for digit in b'1'..=b'9' {
        name[5] = digit;
        if unsafe { sys::mkdir(name.as_ptr()) } == 0 {
            return ("mkdir: olusturuldu", OK);
        }
    }
    ("mkdir: basarisiz", WARN)
}

/// Secili girdiyi `tasindiN` adiyla yeniden adlandirir.
///
/// `rename` veriyi **kopyalamaz**: TCMKFS'te ad ve ebeveyn ayni inode
/// alaninda oldugu icin islem tek bir alan degisikligidir.
fn rename_entry(row: &Row) -> (&'static str, u32) {
    let mut name = *b"tasindi0\0";
    for digit in b'1'..=b'9' {
        name[7] = digit;
        if unsafe { sys::rename(row.name.as_ptr(), name.as_ptr()) } == 0 {
            return ("rename: tasindi", OK);
        }
    }
    ("rename: basarisiz", WARN)
}

/// Secili girdiyi siler: dizinse `rmdir`, dosyaysa `unlink`.
fn remove(row: &Row) -> (&'static str, u32) {
    let result = if row.is_dir {
        unsafe { sys::rmdir(row.name.as_ptr()) }
    } else {
        unsafe { sys::unlink(row.name.as_ptr()) }
    };
    match result {
        0 => ("silindi", OK),
        // Bos olmayan bir dizin silinmez -- POSIX de ENOTEMPTY doner.
        r if r == -ENOTEMPTY => ("silinemedi: dizin bos degil", WARN),
        // RAMFS dosyasi: duruyor ama cekirdek imajinin parcasi.
        r if r == -EROFS => ("silinemedi: salt okunur (RAMFS)", WARN),
        _ => ("silinemedi", WARN),
    }
}

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;

    let mut rows = [Row::empty(); MAX_ROWS];
    let mut count = 0usize;
    let mut selected = 0usize;
    let mut path = [0u8; MAX_PATH];
    let mut status = ("j/k sec  Enter gir  u ust", DIM);

    // Arguman verildiyse orada acilir. Yol birlestirmesi yok: goreli ad
    // da olabilir, cekirdek surecin dizinine gore cozer.
    if let Some(start) = args::first() {
        let mut target = [0u8; MAX_PATH];
        let taken = start.len().min(MAX_PATH - 1);
        target[..taken].copy_from_slice(&start.as_bytes()[..taken]);
        if unsafe { sys::chdir(target.as_ptr()) } != 0 {
            let _ = writeln!(out, "[browse] '{}' acilamadi, kokte kaliniyor.", start);
        }
    }

    let mut total = scan(&mut rows, &mut count);
    let _ = writeln!(
        out,
        "[browse] {} icinde {} girdi ({} arguman).",
        cwd(&mut path),
        total,
        args::count()
    );

    let mut win = match Window::open("browse -- getdents / chdir", 250, 120, 420, 300) {
        Some(w) => w,
        None => return,
    };

    loop {
        match win.poll_key() {
            b'q' => break,
            b'j' => {
                if count > 0 && selected + 1 < count {
                    selected += 1;
                }
            }
            b'k' => selected = selected.saturating_sub(1),
            // Enter: secili girdi bir dizinse **icine gir**. Yol
            // birlestirmesi yok -- cekirdek goreli adi cozuyor.
            b'\n' | b'\r' => {
                if selected < count && rows[selected].is_dir {
                    unsafe { sys::chdir(rows[selected].name.as_ptr()) };
                    selected = 0;
                    total = scan(&mut rows, &mut count);
                }
            }
            b'u' => {
                unsafe { sys::chdir(b"..\0".as_ptr()) };
                selected = 0;
                total = scan(&mut rows, &mut count);
            }
            b'r' => {
                total = scan(&mut rows, &mut count);
                selected = selected.min(count.saturating_sub(1));
            }
            b'n' => {
                status = make_dir();
                total = scan(&mut rows, &mut count);
            }
            b'm' => {
                if selected < count {
                    status = rename_entry(&rows[selected]);
                    total = scan(&mut rows, &mut count);
                }
            }
            b'd' => {
                if selected < count {
                    status = remove(&rows[selected]);
                    total = scan(&mut rows, &mut count);
                    selected = selected.min(count.saturating_sub(1));
                }
            }
            _ => {}
        }

        draw(&mut win, cwd(&mut path), &rows[..count], selected, total, status);
        win.frame(60);
    }
}

/// Calisma dizinini `getcwd` ile sorar.
///
/// Ekranda gorunen yol ile cekirdegin cozdugu yol boylece **ayni
/// kaynaktan** geliyor; program kendi tahminini gostermiyor.
fn cwd(buf: &mut [u8; MAX_PATH]) -> &str {
    let n = sys::getcwd(buf);
    if n <= 1 {
        return "/";
    }
    // Donus NUL'u da sayar; metin onun oncesinde biter.
    core::str::from_utf8(&buf[..n as usize - 1]).unwrap_or("/")
}

/// Calisma dizinini bastan okur; onbellege sigan girdileri doldurur.
///
/// **Toplam** girdi sayisini doner -- onbellege sigmayanlar da sayilir,
/// cunku kullaniciya "12 girdi daha var" demek icin gereken sayi budur.
fn scan(rows: &mut [Row; MAX_ROWS], count: &mut usize) -> usize {
    *count = 0;
    // Tampon cagiranindir: `no_std`'de yigin ayirmasi yok. 512 bayt
    // yaklasik 25 kayit tasir, yani cogu dizin tek `getdents` ile biter.
    let mut buf = [0u8; 512];
    // `.` -- calisma dizini. Mutlak yol tasimaya gerek yok.
    let mut dir = match unsafe { ReadDir::open(b".\0".as_ptr(), &mut buf) } {
        Some(d) => d,
        None => return 0,
    };

    let mut total = 0usize;
    while let Some(entry) = dir.next() {
        total += 1;
        if *count >= rows.len() {
            continue;
        }
        let row = &mut rows[*count];
        // Sondaki NUL icin bir bayt ayrilir.
        row.len = entry.name.len().min(MAX_NAME - 1);
        row.name = [0; MAX_NAME];
        row.name[..row.len].copy_from_slice(&entry.name.as_bytes()[..row.len]);
        row.size = entry.size;
        row.is_dir = entry.is_dir();
        *count += 1;
    }
    total
}

fn draw(
    win: &mut Window,
    path: &str,
    rows: &[Row],
    selected: usize,
    total: usize,
    status: (&str, u32),
) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "getcwd:", DIM);
    win.text(62, 3, path, ACCENT);

    if rows.is_empty() {
        win.text(6, 40, "(bos dizin)", DIM);
    }

    let mut y = 30;
    for (i, row) in rows.iter().enumerate() {
        if i == selected {
            win.fill(2, y - 2, w - 4, 15, SELECT);
        }
        // Dizinler once goze carpsin: hem renk hem sondaki egik cizgi.
        if row.is_dir {
            win.text(8, y, row.name(), DIRC);
            win.text(8 + row.name().len() * 8, y, "/", DIRC);
        } else {
            win.text(8, y, row.name(), FG);
            win.number(280, y, row.size, DIM);
            win.text(330, y, "bayt", DIM);
        }
        y += 16;
    }

    win.fill(0, h - 36, w, 36, PANEL);
    win.text(6, h - 32, "girdi:", DIM);
    win.number(56, h - 32, total, FG);
    if total > rows.len() {
        win.text(90, h - 32, "(listede", DIM);
        win.number(160, h - 32, rows.len(), FG);
        win.text(190, h - 32, ")", DIM);
    }
    win.text(215, h - 32, status.0, status.1);
    win.text(6, h - 15, "j/k  Enter gir  u ust  n yeni  m ad  d sil  q cik", DIM);
}
