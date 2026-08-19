//! `probe` -- POSIX'in "sorma" cagrilari: `stat`, `access`, saat, `uname`.
//!
//! Bu cagrilardan onceki durum tek cumleyle: bir ELF **soramiyordu**.
//!
//!   * "bu yol var mi?" -> tek cevap `open` denemekti; dizinlerde o bile
//!     calismiyordu ve acmanin yan etkisi var (tanimlayici tuketiyor).
//!   * "saat kac?" -> POSIX tarafinda **hicbir saat yoktu**. Ayni
//!     cekirdekte kosan bir PE `NtQuerySystemTime` ile sorabiliyordu;
//!     asimetri kaynakta degil yalnizca ceviri katmanindaydi.
//!   * "hangi sistemdeyim?" -> karsiligi yoktu.
//!
//! ## Alti sinav
//!
//! ```text
//!   A  stat(/bin/browse)  -> var, dizin degil, salt okunur (RAMFS)
//!   B  stat(/bin)         -> dizin   (VFS'te dugumu yok, yollardan ima)
//!   C  stat(/yokboyle)    -> None
//!   D  access(W_OK)       -> RAMFS icin false, F_OK icin true
//!   E  uname              -> sysname "TCMK", machine mimariye gore
//!   F  CLOCK_MONOTONIC    -> uyuduktan sonra ILERLEMIS olmali
//!   G  nanosleep          -> ayni uyku, GERCEK Linux numarasi uzerinden
//!   H  writev             -> iki tampon tek cagride, donus toplam olmali
//!   I  getppid + exit_group -> cocuk ebeveynini tanimali ve gercek
//!                             Linux numarasiyla CIKABILMELI
//! ```
//!
//! G ve I ayni seyi baska acidan olcuyor: TCMK bazi yetenekleri kendi
//! numaralariyla zaten sunuyordu (`SYS_SLEEP`, `SYS_EXIT`), ama
//! derleyicinin urettigi bir Linux ikilisi o numaralari bilmez. Gercek
//! numaralar taninmadan "Linux uygulamalarini calistirir" cumlesi eksik
//! kalirdi -- glibc `exit` degil **`exit_group`** cagirir.
//!
//! F digerlerinden farkli: tek okuma bir sey kanitlamaz, **iki** okuma
//! arasindaki fark kanitlar. Saatin "calismasi" demek ilerlemesi demek.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0012_1A22;
const PANEL: u32 = 0x001E_2A34;
const FG: u32 = 0x00DC_E4EC;
const DIM: u32 = 0x0080_8C98;
const ACCENT: u32 = 0x0070_C8E0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

#[derive(Clone, Copy)]
struct Check {
    name: &'static str,
    detail: &'static str,
    passed: bool,
}

const EMPTY: Check = Check {
    name: "",
    detail: "",
    passed: false,
};

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;
    let mut checks = [EMPTY; 9];

    // --- A: RAMFS dosyasi ---
    let file = sys::stat("/bin/browse");
    let a = matches!(file, Some(info) if !info.is_dir && info.read_only && info.size > 0);
    checks[0] = Check {
        name: "A stat dosya",
        detail: match file {
            None => "/bin/browse BULUNAMADI",
            Some(info) if info.is_dir => "dizin sanildi",
            Some(info) if !info.read_only => "salt okunur degil sanildi",
            Some(_) => "var, salt okunur, boyu dolu",
        },
        passed: a,
    };

    // --- B: dizin ---
    //
    // `/bin`in VFS'te dugumu yok: varligi yollardan ima ediliyor
    // (bkz. `kernel_api::is_dir_path`). Yine de `stat` onu gormeli.
    let dir = sys::stat("/bin");
    checks[1] = Check {
        name: "B stat dizin",
        detail: match dir {
            None => "/bin BULUNAMADI",
            Some(info) if info.is_dir => "ima edilen dizin gorundu",
            Some(_) => "dizin DEGIL sanildi",
        },
        passed: matches!(dir, Some(info) if info.is_dir),
    };

    // --- C: olmayan yol ---
    let missing = sys::stat("/yokboyle");
    checks[2] = Check {
        name: "C stat yok",
        detail: if missing.is_none() {
            "olmayan yol None dondu"
        } else {
            "OLMAYAN yol bulundu"
        },
        passed: missing.is_none(),
    };

    // --- D: access ---
    //
    // `R_OK`/`X_OK` TCMK'de her zaman gecer (izin biti yok); `W_OK`
    // gercek bir cevap verir, cunku RAMFS gercekten yazilamaz.
    let exists = sys::access("/bin/browse", sys::F_OK);
    let writable = sys::access("/bin/browse", sys::W_OK);
    checks[3] = Check {
        name: "D access",
        detail: if !exists {
            "F_OK BASARISIZ"
        } else if writable {
            "W_OK RAMFS icin gecti (gecmemeliydi)"
        } else {
            "F_OK gecti, W_OK RAMFS icin gecmedi"
        },
        passed: exists && !writable,
    };

    // --- E: uname ---
    let system = sys::uname();
    let expected_machine = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "i686"
    };
    let e = match &system {
        Some(info) => info.sysname() == "TCMK" && info.machine() == expected_machine,
        None => false,
    };
    checks[4] = Check {
        name: "E uname",
        detail: match &system {
            None => "cagri BASARISIZ",
            Some(info) if info.sysname() != "TCMK" => "sysname yanlis",
            Some(info) if info.machine() != expected_machine => "machine yanlis",
            Some(_) => "sysname ve machine dogru",
        },
        passed: e,
    };

    // --- F: monotonik saat ilerliyor mu? ---
    //
    // Tek okuma bir sey kanitlamaz; iki okuma arasindaki fark kanitlar.
    let before = sys::clock_gettime(sys::CLOCK_MONOTONIC);
    sys::sleep_ms(400);
    let after = sys::clock_gettime(sys::CLOCK_MONOTONIC);
    let elapsed = millis(after).saturating_sub(millis(before));
    checks[5] = Check {
        name: "F monotonik saat",
        detail: if elapsed >= 300 && elapsed < 2000 {
            "400 ms uykudan sonra ilerlemis"
        } else if elapsed == 0 {
            "saat HIC ilerlemedi"
        } else {
            "gecen sure beklenenden uzak"
        },
        passed: elapsed >= 300 && elapsed < 2000,
    };

    // --- G: ayni uyku, gercek Linux numarasi ---
    let before_nano = sys::clock_gettime(sys::CLOCK_MONOTONIC);
    let slept = sys::nanosleep(0, 400_000_000);
    let after_nano = sys::clock_gettime(sys::CLOCK_MONOTONIC);
    let nano_elapsed = millis(after_nano).saturating_sub(millis(before_nano));
    checks[6] = Check {
        name: "G nanosleep",
        detail: if slept != 0 {
            "cagri BASARISIZ (numara taninmadi?)"
        } else if nano_elapsed >= 300 && nano_elapsed < 2000 {
            "gercek Linux numarasiyla uyudu"
        } else {
            "uyudu ama sure beklenenden uzak"
        },
        passed: slept == 0 && nano_elapsed >= 300 && nano_elapsed < 2000,
    };

    // --- H: iki tampon, tek cagri ---
    //
    // Donus **toplam** olmali; tek parca yazip donen bir uygulama
    // ciktinin yarisini kaybederdi.
    let parts = [
        sys::IoVec::new(b"[probe] H writev: "),
        sys::IoVec::new(b"iki parca tek cagride
"),
    ];
    let total: usize = parts.iter().map(|p| p.len).sum();
    let written = sys::writev(sys::STDOUT, &parts);
    checks[7] = Check {
        name: "H writev",
        detail: if written < 0 {
            "cagri BASARISIZ (numara taninmadi?)"
        } else if written as usize == total {
            "iki tampon yazildi, donus toplam"
        } else {
            "donus toplamla uyusmuyor"
        },
        passed: written >= 0 && written as usize == total,
    };

    // --- I: getppid, ve cocugun exit_group ile cikisi ---
    //
    // Cocuk `exit_group`u **dogrudan** cagiriyor: glibc'nin yaptigi da
    // budur. Numara taninmasaydi cocuk hic cikamaz, `waitpid` donmezdi.
    let me = tcmk::signal::getpid();
    let child = sys::fork();
    if child == 0 {
        let seen = sys::getppid();
        unsafe {
            sys::syscall1(
                sys::SYS_EXIT_GROUP,
                if seen == me { 1 } else { 0 },
            )
        };
        // Buraya dusulurse `exit_group` donmus demektir -- ki donmemeli.
        sys::exit(0);
    }
    let mut status = 0u32;
    let mut knows_parent = false;
    if child > 0 {
        sys::waitpid(child as usize, &mut status, 0);
        knows_parent = sys::exit_status(status) == 1;
    }
    checks[8] = Check {
        name: "I getppid/exit_group",
        detail: if child <= 0 {
            "fork basarisiz"
        } else if knows_parent {
            "cocuk ebeveynini tanidi ve exit_group ile cikti"
        } else {
            "cocuk ebeveynini TANIMADI"
        },
        passed: knows_parent,
    };

    for check in &checks {
        let _ = writeln!(
            out,
            "[probe] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let _ = writeln!(
        out,
        "[probe] time()={} monotonik={} ms",
        sys::time(),
        millis(after)
    );

    let mut win = match Window::open("probe -- stat / access / uname / saat", 240, 130, 440, 230) {
        Some(w) => w,
        None => return,
    };
    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks, &system);
        win.frame(60);
    }
}

fn millis(spec: (usize, usize)) -> usize {
    spec.0 * 1000 + spec.1 / 1_000_000
}

fn draw(win: &mut Window, checks: &[Check; 9], system: &Option<sys::UtsName>) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "POSIX: sormak icin acmak gerekmiyor", ACCENT);

    // Pencerede yalnizca **ozet**: dokuz satir ve sonuclari. Ayrintilar
    // seri gunlukte; pencere tamponu kmalloc'tan geliyor ve onu
    // buyutmek, ayni anda acilabilecek pencere sayisini dusururdu.
    let mut y = 28;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            300,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 15;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.fill(6, h - 42, w - 12, 20, PANEL);
    if let Some(info) = system {
        win.text(12, h - 39, info.sysname(), FG);
        win.text(70, h - 39, info.release(), DIM);
        win.text(130, h - 39, info.machine(), DIM);
    }
    win.text(
        6,
        h - 14,
        if passed == checks.len() {
            "dokuz sinav da gecti   q cik"
        } else {
            "BIR SINAV KALDI   q cik"
        },
        if passed == checks.len() { OK } else { WARN },
    );
}
