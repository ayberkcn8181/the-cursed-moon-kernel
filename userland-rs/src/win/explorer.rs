//! `winfiles` -- Win32 tarafinin dizin gezgini.
//!
//! POSIX'teki `browse` ile **ayni** dizinleri, **ayni** cekirdek koduyla
//! listeler; degisen tek sey ABI'dir:
//!
//! ```text
//!   browse (ELF)     open("/") -> getdents(fd, buf, 512)   -> N kayit
//!   winfiles (PE)    FindFirstFileA("C:\\*") -> FindNextFileA -> 1 kayit
//! ```
//!
//! Ikisi de Level-0b1'de `kernel_api::next_entry`e iner. Iki listenin
//! ayrisabilmesi icin cekirdekte ikinci bir gezinme kodu olmasi gerekirdi
//! -- yok; Level-0b1'in varlik sebebi de zaten bu.
//!
//! Hata bildirimi de Win32'nin kendi bicimi: cagrilar `BOOL` doner,
//! **sebep** `GetLastError`da durur. POSIX ayni bilgiyi negatif errno
//! olarak dogrudan donus degerinde tasir; iki ABI'nin yapisal farki bu
//! ekranda dogrudan gorunuyor -- alt satirdaki mesajlar tahmin degil,
//! gercek `ERROR_*` kodlarindan geliyor.
//!
//! Windows'a ozgu ne var:
//!
//!   * `WIN32_FIND_DATAA` **birebir** Windows yerlesiminde (320 bayt,
//!     `cFileName` +44'te). Sadelestirilmis bir kayit ikili uyumu bozardi.
//!   * `dwFileAttributes` gercek `FILE_ATTRIBUTE_DIRECTORY` bitini tasir.
//!   * `ftLastWriteTime` gercek bir `FILETIME`dir: 1601'den beri gecen
//!     100 ns'lik araliklar. Ekrandaki "epoch" sutunu onu Unix zamanina
//!     geri cevirir; sayi tutuyorsa cevrim iki yonde de dogru demektir.
//!   * Butun cagrilar ithal tablosu (IAT) uzerinden gider -- bu ikili tek
//!     bir elle yazilmis sistem cagrisi icermez.
//!
//! ## Yolu artik program tasimiyor
//!
//! Ilk surumde bu program gezdigi yolu kendi icinde tutuyordu: `Path`
//! diye bir yapi, `push`/`pop`, ve her cagri icin yol birlestirmesi.
//! `SetCurrentDirectoryA` geldikten sonra o kodun tamami silindi --
//! dizine girmek `SetCurrentDirectoryA(ad)`, listelemek
//! `FindFirstFileA("*")`, dizin acmak `CreateDirectoryA("win321")`.
//! Ustteki yol satiri da tahmin degil, `GetCurrentDirectoryA`nin cevabi.
//!
//! Gezinmenin yaninda **yazma** da var: `n` `CreateDirectoryA` ile yeni
//! bir dizin acar, `d` secili girdiyi `RemoveDirectoryA`/`DeleteFileA`
//! ile siler. Olusturulan dizinlerin adi `win32N`'dir; POSIX tarafindaki
//! `browse` de `posixN` yaratir. Iki uygulamayi yan yana calistirip
//! birinin yarattigini otekinde gormek, iki ABI'nin ayni dosya sistemine
//! baktiginin dogrudan kanitidir.
//!
//! ## Arguman
//!
//! `run winfiles /notlar` -- verilen dizinde acilir. Argumanlar buraya
//! `GetCommandLineA` ile **tek bir dize** olarak geliyor ve bolmek
//! cagirana kaliyor (gercek Windows'ta CRT'nin isi). POSIX tarafindaki
//! `browse` ayni argumani yiginda `argv` **dizisi** olarak aliyor --
//! ayni yetenek, iki ayri ABI sozlesmesi.
//!
//! Tuslar: `j`/`k` -> sec, Enter -> dizine gir, `u` -> ust dizin,
//! `n` -> yeni dizin, `m` -> yeniden adlandir (`MoveFileA`), `d` -> sil,
//! `r` -> yenile, ESC -> cik

#![no_std]
#![no_main]

use tcmk::args;
use tcmk::winapi::{self, Win32FindData, Window};

tcmk::entry!(main);

const BG: u32 = 0x000A_1626;
const PANEL: u32 = 0x0014_2A44;
const FG: u32 = 0x00DC_E8F4;
const DIM: u32 = 0x0072_8A9E;
const ACCENT: u32 = 0x0046_C8FF;
const DIRC: u32 = 0x00FF_C24A;
const SELECT: u32 = 0x0022_4468;
const OK: u32 = 0x0072_E08A;
const WARN: u32 = 0x00FF_C24A;

const MAX_ROWS: usize = 12;
const MAX_NAME: usize = 24;
const MAX_PATH: usize = 80;

#[derive(Clone, Copy)]
struct Row {
    name: [u8; MAX_NAME],
    len: usize,
    size: u32,
    /// `ftLastWriteTime`den geri cevrilmis Unix zamani.
    unix_time: u32,
    is_dir: bool,
}

impl Row {
    const fn empty() -> Self {
        Row {
            name: [0; MAX_NAME],
            len: 0,
            size: 0,
            unix_time: 0,
            is_dir: false,
        }
    }

    fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.len]).unwrap_or("?")
    }
}

/// Calisma dizinini `GetCurrentDirectoryA` ile sorar.
///
/// Ilk surumde bu program gezdigi yolu **kendi icinde** tutuyordu:
/// `Path` diye bir yapi, `push`/`pop`, ve her cagri icin
/// `dizin + "\\" + ad` birlestirmesi. `SetCurrentDirectoryA` geldikten
/// sonra o kodun tamami silindi -- ekrandaki yol da artik tahmin degil,
/// cekirdegin cevabi.
fn cwd(buf: &mut [u8; MAX_PATH]) -> &str {
    let n = unsafe { winapi::GetCurrentDirectoryA(MAX_PATH as u32, buf.as_mut_ptr()) } as usize;
    // Sifir gercek bir hata; tampon yetmediyse **gereken** boyut doner
    // ve o da MAX_PATH'ten buyuk olur.
    if n == 0 || n >= MAX_PATH {
        return "\\";
    }
    core::str::from_utf8(&buf[..n]).unwrap_or("\\")
}

/// Bos bir numara bulup `win32N` dizinini acar.
fn make_dir() -> (&'static str, u32) {
    let mut name = *b"win320\0";
    for digit in b'1'..=b'9' {
        name[5] = digit;
        // Win32 sozlesmesi: basari TRUE, hata FALSE. Zaten varsa FALSE
        // doner ve dongu bir sonraki numaraya gecer. Yol birlestirmesi
        // yok -- goreli adi cekirdek calisma dizinine gore cozuyor.
        if unsafe { winapi::CreateDirectoryA(name.as_ptr(), core::ptr::null_mut()) } != 0 {
            return ("CreateDirectoryA: olusturuldu", OK);
        }
    }
    (
        match unsafe { winapi::GetLastError() } {
            winapi::ERROR_ALREADY_EXISTS => "mkdir: hepsi dolu (EEXIST)",
            winapi::ERROR_DISK_FULL => "mkdir: disk yok/dolu",
            _ => "CreateDirectoryA: basarisiz",
        },
        WARN,
    )
}

/// Secili girdiyi `MoveFileA` ile `tasindiN` adina tasir.
///
/// Win32'de "yeniden adlandir" ve "tasi" ayni cagridir; TCMK'de de oyle,
/// cunku ikisi de tek bir inode alani degisikligi.
fn rename_entry(row: &Row) -> (&'static str, u32) {
    let mut name = *b"tasindi0\0";
    for digit in b'1'..=b'9' {
        name[7] = digit;
        if unsafe { winapi::MoveFileA(row.name.as_ptr(), name.as_ptr()) } != 0 {
            return ("MoveFileA: tasindi", OK);
        }
    }
    (
        match unsafe { winapi::GetLastError() } {
            winapi::ERROR_ALREADY_EXISTS => "mv: hedef zaten var",
            winapi::ERROR_ACCESS_DENIED => "mv: salt okunur (RAMFS)",
            _ => "MoveFileA: basarisiz",
        },
        WARN,
    )
}

/// Secili girdiyi siler: dizinse `RemoveDirectoryA`, dosyaysa `DeleteFileA`.
fn remove(row: &Row) -> (&'static str, u32) {
    let ok = if row.is_dir {
        unsafe { winapi::RemoveDirectoryA(row.name.as_ptr()) }
    } else {
        unsafe { winapi::DeleteFileA(row.name.as_ptr()) }
    };
    if ok != 0 {
        return ("silindi", OK);
    }
    // Sebep artik tahmin degil: `GetLastError` gercek kodu veriyor.
    (
        match unsafe { winapi::GetLastError() } {
            winapi::ERROR_DIR_NOT_EMPTY => "silinemedi: dizin bos degil",
            winapi::ERROR_FILE_NOT_FOUND => "silinemedi: bulunamadi",
            // RAMFS: dosya duruyor ama cekirdek imajinin parcasi.
            winapi::ERROR_ACCESS_DENIED => "silinemedi: salt okunur (RAMFS)",
            winapi::ERROR_NOT_SUPPORTED => "silinemedi: desteklenmiyor",
            _ => "silinemedi",
        },
        WARN,
    )
}

fn main() {
    let mut console = winapi::Console;

    // Arguman verildiyse orada acilir.
    if let Some(start) = args::first() {
        let mut target = [0u8; MAX_PATH];
        let taken = start.len().min(MAX_PATH - 1);
        target[..taken].copy_from_slice(&start.as_bytes()[..taken]);
        unsafe { winapi::SetCurrentDirectoryA(target.as_ptr()) };
    }

    let mut path = [0u8; MAX_PATH];
    let mut rows = [Row::empty(); MAX_ROWS];
    let mut count = scan(&mut rows);
    let mut selected = 0usize;
    let mut status = ("j/k sec  Enter gir  u ust", DIM);

    let _ = core::fmt::Write::write_str(
        &mut console,
        "[winfiles] FindFirstFileA/FindNextFileA -- IAT uzerinden.\n",
    );

    let mut win = match Window::create("Dosya Gezgini -- FindFirstFile", 320, 160, 420, 280) {
        Some(w) => w,
        None => return,
    };

    loop {
        match win.get_message() {
            0 => {}
            0x1B => break, // ESC
            b'j' => {
                if count > 0 && selected + 1 < count {
                    selected += 1;
                }
            }
            b'k' => selected = selected.saturating_sub(1),
            b'\n' | b'\r' => {
                if selected < count && rows[selected].is_dir {
                    // Yol birlestirmesi yok: goreli adi cekirdek cozuyor.
                    unsafe { winapi::SetCurrentDirectoryA(rows[selected].name.as_ptr()) };
                    count = scan(&mut rows);
                    selected = 0;
                }
            }
            b'u' => {
                unsafe { winapi::SetCurrentDirectoryA(b"..\0".as_ptr()) };
                count = scan(&mut rows);
                selected = 0;
            }
            b'r' => {
                count = scan(&mut rows);
                selected = selected.min(count.saturating_sub(1));
            }
            b'n' => {
                status = make_dir();
                count = scan(&mut rows);
            }
            b'm' => {
                if selected < count {
                    status = rename_entry(&rows[selected]);
                    count = scan(&mut rows);
                }
            }
            b'd' => {
                if selected < count {
                    status = remove(&rows[selected]);
                    count = scan(&mut rows);
                    selected = selected.min(count.saturating_sub(1));
                }
            }
            _ => {}
        }

        draw(&mut win, cwd(&mut path), &rows[..count], selected, status);
        win.frame(40);
    }

    unsafe { winapi::ExitProcess(0) }
}

/// Dizini `FindFirstFileA`/`FindNextFileA` ile dolasir.
fn scan(rows: &mut [Row; MAX_ROWS]) -> usize {
    let mut data = Win32FindData::zeroed();

    // `*` -- calisma dizininin tamami. Mutlak yol tasimaya gerek yok.
    let find = unsafe { winapi::FindFirstFileA(b"*\0".as_ptr(), &mut data) };
    if find == winapi::INVALID_HANDLE_VALUE {
        return 0;
    }

    let mut count = 0usize;
    loop {
        if count < rows.len() {
            let name = data.name();
            let row = &mut rows[count];
            // Sondaki NUL icin bir bayt ayrilir: ad dogrudan
            // `CreateDirectoryA`/`DeleteFileA`ya verilebilsin diye.
            row.len = name.len().min(MAX_NAME - 1);
            row.name = [0; MAX_NAME];
            row.name[..row.len].copy_from_slice(&name.as_bytes()[..row.len]);
            row.size = data.size_low;
            row.unix_time = data.last_write_time.to_unix();
            row.is_dir = data.is_directory();
            count += 1;
        }
        // FALSE = dizin bitti. Windows'ta da dongu boyle sonlanir.
        if unsafe { winapi::FindNextFileA(find, &mut data) } == 0 {
            break;
        }
    }

    unsafe { winapi::FindClose(find) };
    count
}

fn draw(win: &mut Window, path: &str, rows: &[Row], selected: usize, status: (&str, u32)) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "C:", DIM);
    win.text(24, 3, path, ACCENT);

    if rows.is_empty() {
        win.text(6, 40, "(bos dizin)", DIM);
    }

    let mut y = 30;
    for (i, row) in rows.iter().enumerate() {
        if i == selected {
            win.fill(2, y - 2, w - 4, 15, SELECT);
        }
        if row.is_dir {
            win.text(8, y, "[dizin]", DIRC);
            win.text(72, y, row.name(), DIRC);
        } else {
            win.text(72, y, row.name(), FG);
            win.number(280, y, row.size as usize, DIM);
            // Zaman damgasi FILETIME'dan geri cevrildi; sifir "bilgi yok"
            // demektir (RAMFS dosyalari cekirdek imajiyla gelir).
            if row.unix_time != 0 {
                win.number(330, y, row.unix_time as usize, DIM);
            }
        }
        y += 16;
    }

    win.fill(0, h - 36, w, 36, PANEL);
    win.text(6, h - 32, "girdi:", DIM);
    win.number(56, h - 32, rows.len(), FG);
    win.text(96, h - 32, "sutunlar: bayt / FILETIME->epoch", DIM);
    win.text(6, h - 15, status.0, status.1);
    win.text(230, h - 15, "n yeni  m ad  d sil  ESC cik", DIM);
}
