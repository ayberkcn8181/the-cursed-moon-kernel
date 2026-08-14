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
//! Tuslar: `j`/`k` -> sec, Enter -> dizine gir, `u` -> ust dizin,
//! `r` -> yenile, ESC -> cik

#![no_std]
#![no_main]

use tcmk::winapi::{self, Win32FindData, Window};

tcmk::entry!(main);

const BG: u32 = 0x000A_1626;
const PANEL: u32 = 0x0014_2A44;
const FG: u32 = 0x00DC_E8F4;
const DIM: u32 = 0x0072_8A9E;
const ACCENT: u32 = 0x0046_C8FF;
const DIRC: u32 = 0x00FF_C24A;
const SELECT: u32 = 0x0022_4468;

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

/// Gezilen dizin. Windows tarzinda tutulur (`\` ile), cunku cagri
/// Win32'dir; cekirdek ayirici cevrimini kendisi yapar.
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
        path.bytes[0] = b'\\';
        path
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("\\")
    }

    /// Yola `\*` ekleyip NUL ile kapatan bir desen uretir.
    ///
    /// `FindFirstFileA` yol degil **desen** alir; TCMK'de desen esleme
    /// olmadigi icin `*` "hepsi" demektir.
    fn pattern(&self, buf: &mut [u8; MAX_PATH + 4]) -> *const u8 {
        let mut len = 0usize;
        buf[..self.len].copy_from_slice(&self.bytes[..self.len]);
        len += self.len;
        if self.bytes[self.len - 1] != b'\\' {
            buf[len] = b'\\';
            len += 1;
        }
        buf[len] = b'*';
        buf[len + 1] = 0;
        buf.as_ptr()
    }

    fn push(&mut self, name: &str) {
        let separator = if self.bytes[self.len - 1] == b'\\' { 0 } else { 1 };
        if self.len + separator + name.len() >= MAX_PATH {
            return;
        }
        if separator == 1 {
            self.bytes[self.len] = b'\\';
            self.len += 1;
        }
        self.bytes[self.len..self.len + name.len()].copy_from_slice(name.as_bytes());
        self.len += name.len();
    }

    fn pop(&mut self) {
        if self.len <= 1 {
            return;
        }
        while self.len > 1 && self.bytes[self.len - 1] != b'\\' {
            self.len -= 1;
        }
        if self.len > 1 {
            self.len -= 1;
        }
    }
}

fn main() {
    let mut console = winapi::Console;

    let mut path = Path::root();
    let mut rows = [Row::empty(); MAX_ROWS];
    let mut count = scan(&path, &mut rows);
    let mut selected = 0usize;

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
                    path.push(rows[selected].name());
                    count = scan(&path, &mut rows);
                    selected = 0;
                }
            }
            b'u' => {
                path.pop();
                count = scan(&path, &mut rows);
                selected = 0;
            }
            b'r' => {
                count = scan(&path, &mut rows);
                selected = selected.min(count.saturating_sub(1));
            }
            _ => {}
        }

        draw(&mut win, &path, &rows[..count], selected);
        win.frame(40);
    }

    unsafe { winapi::ExitProcess(0) }
}

/// Dizini `FindFirstFileA`/`FindNextFileA` ile dolasir.
fn scan(path: &Path, rows: &mut [Row; MAX_ROWS]) -> usize {
    let mut pattern = [0u8; MAX_PATH + 4];
    let mut data = Win32FindData::zeroed();

    let find = unsafe { winapi::FindFirstFileA(path.pattern(&mut pattern), &mut data) };
    if find == winapi::INVALID_HANDLE_VALUE {
        return 0;
    }

    let mut count = 0usize;
    loop {
        if count < rows.len() {
            let name = data.name();
            let row = &mut rows[count];
            row.len = name.len().min(MAX_NAME);
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

fn draw(win: &mut Window, path: &Path, rows: &[Row], selected: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "C:", DIM);
    win.text(24, 3, path.as_str(), ACCENT);

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
    win.text(6, h - 15, "j/k sec  Enter gir  u ust  r yenile  ESC cik", DIM);
}
