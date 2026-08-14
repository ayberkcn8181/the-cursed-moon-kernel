//! `winpad` -- **ithal tablosu uzerinden** kosan bir Windows uygulamasi.
//!
//! `winclock` NT cagrilarini elle (`int 0x2E`) yapiyordu. Bu program ise
//! tek bir sistem cagrisi yazmaz: her sey `KERNEL32.dll` ve
//! `TCMKGUI.dll`'den **ithal edilir**, cagrilar IAT uzerinden gider.
//! Yani derleyicinin urettigi siradan bir Windows ikilisiyle ayni
//! yapidadir -- ithal tablosunu cozmeyen bir cekirdek bunu calistiramaz.
//!
//! Yaptigi is: kucuk bir metin defteri. Yazilanlar `CreateFileA` +
//! `WriteFile` ile TCMKFS'e yazilir, acilista `CreateFileA` +
//! `ReadFile` ile geri okunur -- Windows'ta bir dosyayi kaydetmenin ve
//! geri okumanin tam dizisi.
//!
//! Tuslar:  yazi,  BACKSPACE,  `~` -> kaydet,  ESC -> cik

#![no_std]
#![no_main]

use tcmk::winapi::{self, Window};

tcmk::entry!(main);

const BG: u32 = 0x000C_1A2A;
const PANEL: u32 = 0x0016_2A40;
const FG: u32 = 0x00DC_E8F4;
const ACCENT: u32 = 0x0046_C8FF;
const OK: u32 = 0x0072_E08A;
const WARN: u32 = 0x00FF_C24A;

const PATH: &str = "/winpad.txt";
const CELL_W: usize = 8;
const CELL_H: usize = 16;
const CAPACITY: usize = 512;

struct Buffer {
    bytes: [u8; CAPACITY],
    len: usize,
}

fn main() {
    let mut win = match Window::create("Not Defteri -- IAT", 300, 300, 360, 220) {
        Some(w) => w,
        None => return,
    };

    let mut console = winapi::Console;
    let _ = core::fmt::Write::write_str(
        &mut console,
        "[winpad] IAT uzerinden KERNEL32.dll + TCMKGUI.dll kullaniliyor.\n",
    );

    let mut buffer = Buffer {
        bytes: [0; CAPACITY],
        len: 0,
    };

    let mut status: &str = "~ kaydet   ESC cik";
    let mut status_color = FG;

    // Onceki oturumdan kalani oku.
    if load(&mut buffer) {
        status = "onceki not okundu";
        status_color = OK;
    }

    loop {
        match win.get_message() {
            0 => {}
            0x1B => break, // ESC
            b'~' | b'`' => {
                // Kaydetme denemesi konsola da yazilir: kaydin
                // gerceklesip gerceklesmedigi ekrandan bagimsiz olarak
                // seri gunlukten dogrulanabilsin.
                let ok = save(&buffer);
                let _ = core::fmt::Write::write_str(
                    &mut console,
                    if ok {
                        "[winpad] WriteFile: kaydedildi.\n"
                    } else {
                        "[winpad] WriteFile: YAZILAMADI.\n"
                    },
                );
                if ok {
                    status = "kaydedildi";
                    status_color = OK;
                } else {
                    status = "yazilamadi";
                    status_color = WARN;
                }
            }
            8 => {
                if buffer.len > 0 {
                    buffer.len -= 1;
                }
            }
            ch => {
                if buffer.len < CAPACITY && (ch == b'\n' || ch >= 32) {
                    buffer.bytes[buffer.len] = ch;
                    buffer.len += 1;
                }
            }
        }

        draw(&mut win, &buffer, status, status_color);
        win.frame(30);
    }

    let _ = core::fmt::Write::write_str(&mut console, "[winpad] cikiliyor.\n");

    // `entry!` dondugunde zaten cikardi, ama o yol ham `int 0x2E`
    // kullanir. Burada bilerek ithal edilmis `ExitProcess` cagriliyor:
    // boylece bu ikili tek bir elle yazilmis sistem cagrisi icermez.
    unsafe { winapi::ExitProcess(0) }
}

fn draw(win: &mut Window, buffer: &Buffer, status: &str, status_color: u32) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    // Baslik seridi.
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, PATH, ACCENT);

    // Metin alani: basit sarma, satir sonu karakterine saygi.
    let columns = (w.saturating_sub(12)) / CELL_W;
    let mut cx = 0usize;
    let mut cy = 0usize;
    for i in 0..buffer.len {
        let ch = buffer.bytes[i];
        if ch == b'\n' {
            cx = 0;
            cy += 1;
            continue;
        }
        if columns > 0 && cx >= columns {
            cx = 0;
            cy += 1;
        }
        win.glyph(6 + cx * CELL_W, 28 + cy * CELL_H, ch, FG);
        cx += 1;
    }

    // Imlec.
    win.fill(6 + cx * CELL_W, 28 + cy * CELL_H + CELL_H - 2, CELL_W, 2, ACCENT);

    // Alt bilgi cubugu.
    win.fill(0, h - 20, w, 20, PANEL);
    win.text(6, h - 18, status, status_color);

    // Sag altta bayt sayaci ve calisma suresi. `GetTickCount` de
    // `KERNEL32.dll`'den ithal edilmis bir cagridir; milisaniye dondurur.
    let seconds = unsafe { winapi::GetTickCount() } as usize / 1000;
    let mut rx = w.saturating_sub(112);
    rx += win.number(rx, h - 18, buffer.len, FG);
    rx += win.text_scaled(rx, h - 18, "B ", 1, FG);
    rx += win.number(rx, h - 18, seconds, ACCENT);
    win.glyph(rx, h - 18, b's', ACCENT);
}

/// Notu diske yazar -- `CreateFileA` + `WriteFile` + `CloseHandle`.
///
/// Bu, Windows'ta bir dosyayi kaydetmenin **tam** dizisidir. Onceden
/// yazma `WriteConsoleA` ile yapiliyordu: tutamac bir dosya olabildigi ve
/// TCMK'de ikisi de ayni `kernel_api::write`'a indigi icin calisiyordu,
/// ama sozlesme yanlisti -- `WriteConsoleA`nin dorduncu parametresi
/// "yazilan **karakter** sayisi"dir ve konsol icin tanimlidir.
fn save(buffer: &Buffer) -> bool {
    let mut name = [0u8; 32];
    name[..PATH.len()].copy_from_slice(PATH.as_bytes());

    let handle = unsafe {
        winapi::CreateFileA(
            name.as_ptr(),
            winapi::GENERIC_WRITE,
            0,
            core::ptr::null_mut(),
            winapi::CREATE_ALWAYS,
            0,
            0,
        )
    };
    if handle == winapi::INVALID_HANDLE_VALUE {
        return false;
    }

    let mut written: winapi::Dword = 0;
    let ok = unsafe {
        winapi::WriteFile(
            handle,
            buffer.bytes.as_ptr(),
            buffer.len as winapi::Dword,
            &mut written,
            core::ptr::null_mut(),
        )
    } != 0;

    unsafe { winapi::CloseHandle(handle) };

    // Cikti parametresi gercekten dolduruldu mu? Win32 sozlesmesinin
    // sinandigi yer burasi.
    ok && written as usize == buffer.len
}

/// Notu geri okur -- `CreateFileA` + `ReadFile` + `CloseHandle`.
fn load(buffer: &mut Buffer) -> bool {
    let mut name = [0u8; 32];
    name[..PATH.len()].copy_from_slice(PATH.as_bytes());

    let handle = unsafe {
        winapi::CreateFileA(
            name.as_ptr(),
            winapi::GENERIC_READ,
            0,
            core::ptr::null_mut(),
            winapi::OPEN_EXISTING,
            0,
            0,
        )
    };
    if handle == winapi::INVALID_HANDLE_VALUE {
        return false;
    }

    let mut read: winapi::Dword = 0;
    let ok = unsafe {
        winapi::ReadFile(
            handle,
            buffer.bytes.as_mut_ptr(),
            CAPACITY as winapi::Dword,
            &mut read,
            core::ptr::null_mut(),
        )
    } != 0;

    unsafe { winapi::CloseHandle(handle) };

    if ok && read > 0 {
        buffer.len = read as usize;
        true
    } else {
        false
    }
}
