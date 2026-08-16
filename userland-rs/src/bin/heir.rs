//! `heir` -- calisma dizini `fork` ile devrediliyor, `execve` ile korunuyor mu?
//!
//! `chdir`/`getcwd` geldiginde iki POSIX kurali daha soz verilmisti:
//!
//! | olay | cwd |
//! |---|---|
//! | `fork` | cocuk ebeveynin dizininde **dogar** |
//! | `execve` | yeni imaj ayni dizinde **baslar** |
//!
//! Ikisi de cekirdekte tek bir tasarim kararindan cikiyor: sifirlama
//! **gorev yuvasi ayrilirken** yapiliyor, imaj yuklenirken degil.
//! `execve` yuvayi yeniden kullandigi icin cwd'ye dokunulmuyor; `fork`
//! ayri yuva aldigi icin ebeveyninkini ayrica kopyaliyor.
//!
//! Ama "tasarim boyle" demek olcum degil. Bu program ikisini de sinar.
//!
//! ## Birinci sinav: `fork`
//!
//! ```text
//!   mkdir /miras ; chdir("/miras")
//!   beklenen = getcwd()            // ebeveynin GERCEK dizini
//!   fork()
//!     cocuk : getcwd() == beklenen ?  -> cikis kodu 0 (evet) / 1 (hayir)
//!     ebeveyn: waitpid -> cikis kodunu okur
//! ```
//!
//! Karsilastirma sabit bir yola degil **ebeveynin gercek dizinine**
//! yapilir. Ilk surumde `"/miras"` sabitiyle karsilastiriliyordu ve
//! disk bagli olmayan bir kosuda `chdir` sessizce basarisiz olunca sinav
//! "KALDI" diyordu -- oysa devralma dogru calisiyor, yalnizca gidilecek
//! dizin yoktu. Sinanan sey "cocuk ebeveynle ayni yerde mi", bir yol
//! adinin kendisi degil.
//!
//! Beklenen deger fork'tan **once** bir tampona yazilir; cocuk adres
//! uzayinin kopyasini aldigi icin onu oldugu gibi gorur.
//!
//! Cocugun cevabi **cikis koduyla** tasiniyor, ekrana yazarak degil:
//! boylece sinav ebeveyn tarafindan makine gibi okunuyor, insan gozune
//! bakmiyor.
//!
//! ## Ikinci sinav: `execve`
//!
//! `x` tusuna basilinca program `/bin/browse`i **kendi yerine** yukler.
//! `browse` acilirken ust satirda `getcwd`in cevabini gosterir. Orada
//! `/miras` goruluyorsa exec dizini korumus demektir -- ve bu, ekranda
//! dogrudan okunan bir kanit.
//!
//! Tuslar: `x` -> execve sinavi, `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0010_1A16;
const PANEL: u32 = 0x001A_2C26;
const FG: u32 = 0x00DC_ECE4;
const DIM: u32 = 0x0078_9890;
const ACCENT: u32 = 0x0070_E0C0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

/// Sinavin yapildigi dizin.
const HOME: &[u8] = b"/miras\0";

/// Ebeveynin fork aninda bulundugu dizin.
///
/// `static mut`: fork'tan once yazilir, cocuk adres uzayinin kopyasini
/// aldigi icin ayni degeri gorur. Karsilastirma buna yapilir -- sabit
/// bir yola degil.
static mut EXPECTED: [u8; 128] = [0; 128];
static mut EXPECTED_LEN: usize = 0;

fn expected() -> &'static str {
    unsafe {
        let base = core::ptr::addr_of!(EXPECTED) as *const u8;
        core::str::from_utf8(core::slice::from_raw_parts(base, EXPECTED_LEN)).unwrap_or("/")
    }
}

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;

    // Sinav dizinini kur. Zaten varsa `EEXIST` doner; onemli degil.
    unsafe { sys::mkdir(HOME.as_ptr()) };
    let moved = unsafe { sys::chdir(HOME.as_ptr()) } == 0;

    let mut buf = [0u8; 128];
    let parent_cwd = cwd(&mut buf);
    unsafe {
        let base = core::ptr::addr_of_mut!(EXPECTED) as *mut u8;
        for (i, byte) in parent_cwd.bytes().enumerate() {
            base.add(i).write(byte);
        }
        EXPECTED_LEN = parent_cwd.len();
    }
    let _ = writeln!(out, "[heir] ebeveyn cwd: {}", parent_cwd);

    if !moved {
        // Disk bagli degilse `/miras` yaratilamaz. Sinav yine yapilir --
        // karsilastirma ebeveynin gercek dizinine oldugu icin anlamli --
        // ama sonucun kokten mi yoksa alt dizinden mi olctugu bildirilir.
        let _ = writeln!(
            out,
            "[heir] chdir basarisiz (disk bagli degil); sinav kokten yapiliyor."
        );
    }

    // --- Birinci sinav: fork ---
    //
    // Cocuk kendi `getcwd`ini ebeveynin dizinine karsi sinar ve cevabi
    // **cikis koduyla** birakir. Ebeveyn `waitpid` ile okur.
    let mut child_ok = false;
    let mut child_seen = false;
    match sys::fork() {
        0 => {
            // Cocuk: ayri gorev yuvasi, ayri adres uzayi.
            let mut child_buf = [0u8; 128];
            let mine = cwd(&mut child_buf);
            let matches = mine == expected();
            let mut child_out = Stdout;
            let _ = writeln!(
                child_out,
                "[heir] cocuk cwd: {} ({})",
                mine,
                if matches { "devralindi" } else { "DEVRALINMADI" }
            );
            sys::exit(if matches { 0 } else { 1 });
        }
        pid if pid > 0 => {
            let mut status = 0u32;
            if sys::waitpid(pid as usize, &mut status, 0) >= 0 {
                child_seen = true;
                child_ok = sys::exit_status(status) == 0;
            }
        }
        _ => {
            let _ = writeln!(out, "[heir] fork basarisiz (gorev tablosu dolu?)");
        }
    }

    let _ = writeln!(
        out,
        "[heir] fork sinavi: {}",
        if !child_seen {
            "cocuk toplanamadi"
        } else if child_ok {
            "gecti"
        } else {
            "KALDI"
        }
    );

    let mut win = match Window::open("heir -- fork / execve mirasi", 270, 150, 420, 220) {
        Some(w) => w,
        None => return,
    };

    loop {
        match win.poll_key() {
            b'q' => break,
            b'x' => {
                // Ikinci sinav: kendi yerine `browse`i yukle. Ayni gorev
                // yuvasinda kaldigi icin cwd korunmali; `browse` acilirken
                // ust satirda `getcwd`i gosteriyor.
                let _ = writeln!(
                    out,
                    "[heir] execve /bin/browse -- cwd korunmali: {}",
                    expected()
                );
                sys::execve("/bin/browse");
                // Buraya dusulduyse exec basarisiz olmus demektir.
                let _ = writeln!(out, "[heir] execve basarisiz.");
            }
            _ => {}
        }

        let mut frame_buf = [0u8; 128];
        draw(&mut win, cwd(&mut frame_buf), child_seen, child_ok);
        win.frame(60);
    }
}

/// `getcwd`in cevabi. Program kendi tahminini gostermiyor.
fn cwd(buf: &mut [u8; 128]) -> &str {
    let n = sys::getcwd(buf);
    if n <= 1 {
        return "/";
    }
    // Donus NUL'u da sayar; metin onun oncesinde biter.
    core::str::from_utf8(&buf[..n as usize - 1]).unwrap_or("/")
}

fn draw(win: &mut Window, path: &str, child_seen: bool, child_ok: bool) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "getcwd:", DIM);
    win.text(62, 3, path, ACCENT);

    let mut y = 34;
    win.text(6, y, "fork sinavi", FG);
    let (text, color) = if !child_seen {
        ("cocuk toplanamadi", WARN)
    } else if child_ok {
        ("gecti -- cocuk ayni dizinde dogdu", OK)
    } else {
        ("KALDI -- cocugun dizini farkli", WARN)
    };
    win.text(120, y, text, color);

    y += 24;
    win.text(6, y, "execve sinavi", FG);
    win.text(120, y, "'x' -> browse'u kendi yerine yukler", DIM);
    y += 16;
    win.text(120, y, "acilan pencerede getcwd bu yolu gostermeli", DIM);
    y += 16;
    win.text(120, y, path, ACCENT);

    win.fill(6, h - 44, w - 12, 20, PANEL);
    win.text(12, h - 41, "cikis kodu tasiyici: cocuk 0 = devraldi", DIM);
    win.text(6, h - 16, "x execve sinavi   q cik", DIM);
}
