//! `browse` -- `getdents` gosterimi: dosya sistemini gezmek.
//!
//! Bu cagriya kadar bir uygulama dosya sistemini **goremiyordu**. Adini
//! onceden bildigi bir dosyayi acabiliyordu (`open`), okuyabiliyordu
//! (`read`), boyutunu ogrenebiliyordu (`fstat`) -- ama "burada ne var?"
//! diye soramiyordu. Dizin listesini yalnizca cekirdegin kendi kabugu
//! biliyordu; Ring 3'te karsiligi yoktu.
//!
//! `getdents` o bosluktur. Bir dizin de dosya gibi `open` ile acilir,
//! ama `read` yerine `getdents` ile okunur: cekirdek tampona arka arkaya
//! kayitlar paketler, uygulama onlari cozer.
//!
//! ```text
//!   fd = open("/", 0)          -> dizin tanimlayicisi
//!   getdents(fd, buf, 512)     -> 96   (kayitlar)
//!   getdents(fd, buf, 512)     -> 0    (bitti)
//! ```
//!
//! Ekrandaki listenin her satiri boyle bir kayittir. Bir dizine girip
//! cikmak imleci sifirlar -- yani `open` yeniden cagrilir; POSIX'te de
//! bir dizin tanimlayicisi geriye sarilmaz (`rewinddir` ayri bir cagridir).
//!
//! Gezinmenin yaninda **yazma** da var: `n` yeni bir dizin acar
//! (`mkdir`), `d` secili girdiyi siler (`rmdir` / `unlink`). Olusturulan
//! dizinlerin adi `posixN`'dir; Win32 tarafindaki `winfiles` de `win32N`
//! yaratir. Iki uygulamayi yan yana calistirip birinin yarattigini
//! otekinde gormek, iki ABI'nin ayni dosya sistemine baktiginin
//! dogrudan kanitidir.
//!
//! Ad uretimi ayrica `EEXIST`i sinar: `posix1` varken tekrar denemek
//! hata doner, program bir sonraki numaraya gecer.
//!
//! Tuslar: `j`/`k` -> asagi/yukari, Enter -> dizine gir, `u` -> ust dizin,
//! `n` -> yeni dizin, `m` -> yeniden adlandir, `d` -> sil,
//! `r` -> yeniden oku, `q` -> cik

#![no_std]
#![no_main]

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

/// Ekranda tutulan en fazla girdi. Daha fazlasi varsa listelenmeyenlerin
/// sayisi alt satirda gosterilir -- sessizce kirpmak yaniltici olurdu.
const MAX_ROWS: usize = 14;
/// Bir ad icin ayrilan yer.
const MAX_NAME: usize = 28;
/// Gezilen yolun en fazla uzunlugu (sondaki NUL dahil).
const MAX_PATH: usize = 96;

/// Onbellege alinmis tek bir girdi.
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

/// Gezilen dizin: her zaman NUL ile sonlanan, mutlak bir yol.
struct Path {
    bytes: [u8; MAX_PATH],
    len: usize,
}

impl Path {
    fn root() -> Self {
        let mut path = Path {
            bytes: [0; MAX_PATH],
            len: 1,
        };
        path.bytes[0] = b'/';
        path
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("/")
    }

    /// Alt dizine iner. Sigmazsa hicbir sey yapmaz.
    fn push(&mut self, name: &str) {
        // Kokte zaten bir egik cizgi var; ikincisini eklemeyelim.
        let separator = if self.len > 1 { 1 } else { 0 };
        if self.len + separator + name.len() >= MAX_PATH {
            return;
        }
        if separator == 1 {
            self.bytes[self.len] = b'/';
            self.len += 1;
        }
        self.bytes[self.len..self.len + name.len()].copy_from_slice(name.as_bytes());
        self.len += name.len();
        self.bytes[self.len] = 0;
    }

    /// Ust dizine cikar. Kokten yukari cikilmaz.
    fn pop(&mut self) {
        if self.len <= 1 {
            return;
        }
        while self.len > 1 && self.bytes[self.len - 1] != b'/' {
            self.len -= 1;
        }
        // Bulunan egik cizgi de atilir; kok icin bir tane birakilir.
        if self.len > 1 {
            self.len -= 1;
        }
        self.bytes[self.len] = 0;
    }

    /// `dizin/ad` + NUL uretir; cekirdek cagrilari icin isaretci doner.
    ///
    /// Girdi kayitlari yalnizca **ad** tasir (POSIX `dirent`i de oyle),
    /// ama `mkdir`/`unlink` tam yol ister.
    fn child(&self, name: &str, buf: &mut [u8; MAX_PATH]) -> *const u8 {
        buf[..self.len].copy_from_slice(&self.bytes[..self.len]);
        let mut len = self.len;
        if self.bytes[self.len - 1] != b'/' {
            buf[len] = b'/';
            len += 1;
        }
        // Sondaki NUL icin bir bayt ayrilir.
        let taken = name.len().min(MAX_PATH - len - 1);
        buf[len..len + taken].copy_from_slice(&name.as_bytes()[..taken]);
        buf[len + taken] = 0;
        buf.as_ptr()
    }
}

/// Bos bir numara bulup `posixN` dizinini acar.
///
/// Numara arama `EEXIST`in gercekten dondugunu de sinar: `posix1` varken
/// cagri hata verir ve dongu bir sonrakine gecer.
fn make_dir(path: &Path) -> (&'static str, u32) {
    let mut buf = [0u8; MAX_PATH];
    let mut name = *b"posix0";
    for digit in b'1'..=b'9' {
        name[5] = digit;
        let text = match core::str::from_utf8(&name) {
            Ok(t) => t,
            Err(_) => break,
        };
        if unsafe { sys::mkdir(path.child(text, &mut buf)) } == 0 {
            return ("mkdir: olusturuldu", OK);
        }
    }
    ("mkdir: basarisiz", WARN)
}

/// Secili girdiyi `tasindiN` adiyla yeniden adlandirir.
///
/// `rename` veriyi **kopyalamaz**: TCMKFS'te ad ve ebeveyn ayni inode
/// alaninda oldugu icin islem tek bir alan degisikligidir. Bir dizini
/// yeniden adlandirmak da bu yuzden bedavadir -- icindeki dosyalara hic
/// dokunulmaz.
fn rename_entry(path: &Path, row: &Row) -> (&'static str, u32) {
    let mut from = [0u8; MAX_PATH];
    let mut to = [0u8; MAX_PATH];
    let source = path.child(row.name(), &mut from);
    let mut name = *b"tasindi0";
    for digit in b'1'..=b'9' {
        name[7] = digit;
        let text = match core::str::from_utf8(&name) {
            Ok(t) => t,
            Err(_) => break,
        };
        if unsafe { sys::rename(source, path.child(text, &mut to)) } == 0 {
            return ("rename: tasindi", OK);
        }
    }
    ("rename: basarisiz", WARN)
}

/// Secili girdiyi siler: dizinse `rmdir`, dosyaysa `unlink`.
fn remove(path: &Path, row: &Row) -> (&'static str, u32) {
    let mut buf = [0u8; MAX_PATH];
    let target = path.child(row.name(), &mut buf);
    let result = if row.is_dir {
        unsafe { sys::rmdir(target) }
    } else {
        unsafe { sys::unlink(target) }
    };
    match result {
        0 => ("silindi", OK),
        // Bos olmayan bir dizin silinmez -- POSIX de ENOTEMPTY doner.
        r if r == -ENOTEMPTY => ("dizin bos degil", WARN),
        // RAMFS dosyalari cekirdek imajinin parcasi; silinemezler.
        _ => ("silinemedi", WARN),
    }
}

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;

    let mut path = Path::root();
    let mut rows = [Row::empty(); MAX_ROWS];
    let mut count = 0usize;
    let mut selected = 0usize;

    let mut total = scan(&path, &mut rows, &mut count);
    let mut status = ("j/k sec  Enter gir  u ust", DIM);
    let _ = writeln!(out, "[browse] / icinde {} girdi.", total);

    let mut win = match Window::open("browse -- getdents", 250, 120, 420, 300) {
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
            // Enter: secili girdi bir dizinse icine gir.
            b'\n' | b'\r' => {
                if selected < count && rows[selected].is_dir {
                    path.push(rows[selected].name());
                    selected = 0;
                    total = scan(&path, &mut rows, &mut count);
                }
            }
            b'u' => {
                path.pop();
                selected = 0;
                total = scan(&path, &mut rows, &mut count);
            }
            b'r' => {
                total = scan(&path, &mut rows, &mut count);
                selected = selected.min(count.saturating_sub(1));
            }
            b'n' => {
                status = make_dir(&path);
                total = scan(&path, &mut rows, &mut count);
            }
            b'm' => {
                if selected < count {
                    status = rename_entry(&path, &rows[selected]);
                    total = scan(&path, &mut rows, &mut count);
                }
            }
            b'd' => {
                if selected < count {
                    status = remove(&path, &rows[selected]);
                    total = scan(&path, &mut rows, &mut count);
                    selected = selected.min(count.saturating_sub(1));
                }
            }
            _ => {}
        }

        draw(&mut win, &path, &rows[..count], selected, total, status);
        win.frame(60);
    }
}

/// Dizini bastan okur; onbellege sigan girdileri `rows`'a doldurur.
///
/// **Toplam** girdi sayisini doner -- onbellege sigmayanlar da sayilir,
/// cunku kullaniciya "12 girdi daha var" demek icin gereken sayi budur.
fn scan(path: &Path, rows: &mut [Row; MAX_ROWS], count: &mut usize) -> usize {
    *count = 0;
    // Tampon cagiranindir: `no_std`'de yigin ayirmasi yok. 512 bayt
    // yaklasik 25 kayit tasir, yani cogu dizin tek `getdents` ile biter.
    let mut buf = [0u8; 512];
    let mut dir = match unsafe { ReadDir::open(path.bytes.as_ptr(), &mut buf) } {
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
        row.len = entry.name.len().min(MAX_NAME);
        row.name[..row.len].copy_from_slice(&entry.name.as_bytes()[..row.len]);
        row.size = entry.size;
        row.is_dir = entry.is_dir();
        *count += 1;
    }
    total
}

fn draw(
    win: &mut Window,
    path: &Path,
    rows: &[Row],
    selected: usize,
    total: usize,
    status: (&str, u32),
) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, path.as_str(), ACCENT);

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
    win.text(6, h - 15, status.0, status.1);
    win.text(215, h - 15, "n yeni  m ad  d sil  q cik", DIM);
}
