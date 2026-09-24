//! `stdin` -- standart girdiden okumak **bekler**.
//!
//! Uzun sure TCMK'de `read(0, ...)` tus yoksa hemen `0` donuyordu. Sayi
//! masum gorunuyor ama POSIX'te `0` "dosya sonu" demektir: girdi bitti,
//! bir daha hic veri gelmeyecek. Gercek bir kabuk ya da `cat` bunu
//! gorunce cikardi -- yani klavye bagli oldugu halde program "girdi
//! kapandi" diye sonlanirdi.
//!
//! Dogrusu beklemektir:
//!
//! ```text
//!   read(0, ...)  -> tus yoksa gorev UYUR
//!                    tusa basilinca uyanir ve veriyle doner
//!                    girdi gercekten bittiyse 0 doner
//! ```
//!
//! ## "Ama pencere donar" itirazi
//!
//! Her karede cizen bir program bloke eden bir okuma yaparsa cizim
//! durur. Bu dogru, ama cevabi "cekirdek beklemesin" degil: gercek
//! Linux'ta da oyledir ve cozumu programin kendisindedir. Iki dogru
//! deyim var ve ikisi de bu sinavda olculuyor:
//!
//! ```text
//!   poll(stdin, 0)      -> hazir mi diye sor, degilse okuma
//!   O_NONBLOCK + read   -> oku, hazir degilse -EAGAIN al
//! ```
//!
//! `echo2` ilk deyime tasindi; ikincisi burada olculuyor.
//!
//! ## Uc bilincli kacis
//!
//! Cekirdek her kosulda uyumaz. Uc durumda eski davranis surer, ve
//! ucu de bilincli:
//!
//! ```text
//!   pencere yok        -> 0 (dosya sonu) -- kimse tus gonderemez
//!   uyutulamaz gorev   -> 0 -- masaustu/kabuk uyursa sistem donar
//!   O_NONBLOCK         -> -EAGAIN
//! ```
//!
//! Ilki onemli: penceresi olmayan bir surecin klavyesi yoktur, onu
//! uyutmak sonsuza kadar uyutmak olurdu. Bu yuzden orada `0` dogru
//! cevap -- gercekten girdi yok.
//!
//! ## Windows'ta karsiligi
//!
//! `ReadFile(GetStdHandle(STD_INPUT_HANDLE), ...)` da varsayilan olarak
//! bekler; Win32'de bloke etmeyen esdeger `PeekNamedPipe` ya da konsol
//! icin `WaitForSingleObject` + `ReadConsoleInput`. Yani "varsayilan
//! bekler" iki dunyada da ayni; ayrilan, beklemekten kacinma yolu.
//!
//! ## Bes sinav
//!
//! ```text
//!   A  O_NONBLOCK   -> tus yokken -EAGAIN doner, beklemez
//!   B  poll         -> tus yokken "hazir degil" der
//!   C  bloke etti   -> bekleyen okuma sinyalle bolundu, >=100 ms surdu
//!   D  penceresiz   -> pencere olmayan surecte 0 (dosya sonu)
//!   E  bayrak       -> fd 0'in bayragi yazilip geri okunabiliyor
//! ```
//!
//! C sinavin kalbi: `-EINTR` tek basina yetmez, cunku okuma hic
//! beklemeden de bolunmus gorunebilirdi. **Gecen sure** okumanin
//! gercekten uyudugunun kanitidir.
//!
//! D uc ayri sekilde kalabilir ve ucu ayri mesaj veriyor: cocuk uyuyup
//! kalmis, `EAGAIN` almis, ya da hic catallanamamis. Tek bir "kaldi"
//! mesaji uculdu ve yanlis yere baktiriyordu -- yuva tavani dolunca
//! `fork` basarisiz oluyor ve sinav cekirdegin stdin yolunu
//! sucluyormus gibi gorunuyordu. Ayni sebeple `gorev yuvasi` sayisi da
//! yaziliyor.
//!
//! E, cekirdekteki yeni tabloyu olcuyor: 0/1/2 numarali tanimlayicilar
//! bir `FileDescriptor` kaydina sahip degil (dogrudan konsola bagli),
//! bu yuzden bayraklari ayri bir grup tablosunda tutuluyor. Tablo
//! olmasaydi A sinavi kurulamazdi bile.
//!
//! Tuslar: `q` -> cik

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x0012_1A18;
const PANEL: u32 = 0x001E_2C2A;
const FG: u32 = 0x00E2_ECE8;
const DIM: u32 = 0x0088_9A96;
const ACCENT: u32 = 0x0060_E0C0;
const OK: u32 = 0x0070_E090;
const WARN: u32 = 0x00FF_8060;

const EAGAIN: isize = 11;
const EINTR: isize = 4;

/// Kardesin sinyali gondermeden once bekleyecegi sure. Ana akisin
/// gercekten uyumus olmasi icin yeterince uzun.
const DELAY_MS: usize = 120;
/// C sinavinin gecmesi icin gereken en az sure, PIT tiki cinsinden
/// (10 ms cozunurluk). `DELAY_MS`in biraz altinda: olcum tik sinirina
/// denk gelirse bir tik kaybedilebilir.
const MIN_TICKS: usize = 10;

/// Sinyalin gonderilecegi gorev.
static TARGET: AtomicUsize = AtomicUsize::new(0);
/// Isleyici kac kez calisti.
static HANDLER_RUNS: AtomicUsize = AtomicUsize::new(0);

/// D sinavinda cocugun cevabi icin beklenecek en uzun sure. Sinavin
/// kendisi asili kalmamali: cekirdek yanlissa cocuk uyur ve ancak bu
/// sure dolunca olduruluyor.
const ANSWER_MS: isize = 1500;
/// D sinavinda cocuktan hic cevap gelmedigini gosteren deger.
const NO_ANSWER: isize = isize::MIN;
/// D sinavinin hic kurulamadigini gosteren deger (boru acilamadi).
const NO_PIPE: isize = isize::MIN + 1;
/// D sinavi icin cocuk catallanamadi (gorev yuvasi kalmadi).
const NO_FORK: isize = isize::MIN + 2;

/// C sinavinin kardesi icin katilma yuvasi.
static mut JOIN_C: u32 = 0;

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

extern "C" fn on_signal(_signo: u32) {
    HANDLER_RUNS.fetch_add(1, Ordering::SeqCst);
}

/// Bekleyip ana akisa sinyal gonderen kardes.
extern "C" fn late_signaller(_param: usize) -> usize {
    sys::sleep_ms(DELAY_MS);
    tcmk::signal::kill(TARGET.load(Ordering::SeqCst), tcmk::signal::SIGUSR1);
    0
}

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;
    let mut checks = [EMPTY; 5];

    // Pencere **once** aciliyor, sinavlardan sonra degil.
    //
    // Sirasi rastgele degil: cekirdek "bu surecin klavyesi var mi"
    // sorusunu pencere sahipligiyle cevapliyor. Pencere sonra acilsaydi
    // butun okumalar penceresiz yoldan `0` donerdi ve A/B/C sinavlari
    // olculmek istenen seyi hic calistirmazdi.
    let mut win = match Window::open("stdin -- okumak bekler", 280, 170, 470, 180) {
        Some(w) => w,
        None => {
            let _ = writeln!(out, "[stdin] pencere acilamadi, sinav kosulamaz.");
            return;
        }
    };

    TARGET.store(sys::gettid(), Ordering::SeqCst);

    // --- A: O_NONBLOCK -> -EAGAIN ---
    //
    // Hicbir tusa basilmadi, yani girdi hazir degil. Bloke etmeyen
    // okuma bunu beklemeden soylemeli.
    let set = sys::set_flags(sys::STDIN, sys::O_NONBLOCK);
    let before_a = sys::ticks();
    let mut buf = [0u8; 16];
    let nonblock_result = sys::read(sys::STDIN, &mut buf);
    let waited_a = sys::ticks().saturating_sub(before_a);
    let a = set >= 0 && nonblock_result == -EAGAIN && waited_a <= 2;
    checks[0] = Check {
        name: "A O_NONBLOCK",
        detail: if set < 0 {
            "fcntl bayragi kabul etmedi"
        } else if nonblock_result == 0 {
            "0 dondu -- dosya sonu gibi davrandi"
        } else if nonblock_result != -EAGAIN {
            "yanlis hata kodu"
        } else if waited_a > 2 {
            "-EAGAIN dondu ama BEKLEDI"
        } else {
            "tus yokken beklemeden -EAGAIN"
        },
        passed: a,
    };

    // --- E: bayrak geri okunabiliyor ---
    //
    // Ayni tabloyu iki yonden sinar: yazilan bayrak okunuyor mu, ve
    // temizlenince gercekten gidiyor mu. Ikincisi C icin sart --
    // bayrak kalsaydi bloke eden yol hic calismazdi.
    let readback = sys::get_flags(sys::STDIN);
    let cleared = sys::set_flags(sys::STDIN, 0);
    let after_clear = sys::get_flags(sys::STDIN);
    let e = readback >= 0
        && readback as usize & sys::O_NONBLOCK != 0
        && cleared >= 0
        && after_clear == 0;
    checks[4] = Check {
        name: "E bayrak",
        detail: if readback < 0 {
            "fcntl F_GETFL calismadi"
        } else if readback as usize & sys::O_NONBLOCK == 0 {
            "yazilan bayrak geri okunamadi"
        } else if after_clear != 0 {
            "bayrak temizlenmedi"
        } else {
            "fd 0 bayragi yazilip geri okundu"
        },
        passed: e,
    };

    // --- B: poll hazir degil der ---
    //
    // Bloke etmeden sormanin POSIX yolu. `echo2` tam olarak bunu
    // kullaniyor; calismasaydi o program ya donar ya tus kacirirdi.
    let mut watch = [sys::PollFd::new(sys::STDIN, sys::POLLIN)];
    let ready = sys::poll(&mut watch, 0);
    let b = ready == 0 && !watch[0].ready(sys::POLLIN);
    checks[1] = Check {
        name: "B poll",
        detail: if b {
            "tus yokken hazir degil dedi"
        } else {
            "tus yokken HAZIR dedi"
        },
        passed: b,
    };

    // --- C: gercekten bekledi mi ---
    //
    // Bayrak temizlendi, yani okuma artik bloke etmeli. Kardes
    // `DELAY_MS` sonra sinyal gonderiyor; isleyici `SA_RESTART`siz
    // kuruldugu icin okuma `-EINTR` ile donmeli.
    //
    // Gecen sureyi olcmek sart: cekirdek yine eskisi gibi hemen `0`
    // donseydi ya da sinyali beklemeden bolunmus gibi yapsaydi hata
    // kodu dogru gorunup **bekleme** hic olmayabilirdi.
    tcmk::signal::install(tcmk::signal::SIGUSR1, on_signal);
    let slot = core::ptr::addr_of_mut!(JOIN_C);
    let helper = unsafe { sys::clone_joinable(late_signaller, 0, slot) };
    let before_c = sys::ticks();
    let blocking_result = sys::read(sys::STDIN, &mut buf);
    let waited_c = sys::ticks().saturating_sub(before_c);
    let handler_ran = HANDLER_RUNS.load(Ordering::SeqCst) > 0;
    unsafe { sys::join_thread(slot, 3000) };
    let c = helper > 0 && blocking_result == -EINTR && handler_ran && waited_c >= MIN_TICKS;
    checks[2] = Check {
        name: "C bloke etti",
        detail: if blocking_result == 0 {
            "0 dondu -- HIC beklemedi"
        } else if blocking_result != -EINTR {
            "yanlis hata kodu"
        } else if !handler_ran {
            "isleyici hic calismadi"
        } else if waited_c < MIN_TICKS {
            "-EINTR dondu ama beklemedi"
        } else {
            "bekledi ve sinyalle bolundu"
        },
        passed: c,
    };

    // --- D: penceresiz surecte dosya sonu ---
    //
    // Cocuk `fork` ile yaratildigi icin kendi gorev numarasina sahip ve
    // hicbir pencerenin sahibi degil. Klavyesi olmayan bir surec
    // uyutulsaydi sonsuza kadar uyurdu; dogru cevap `0`.
    //
    // Sonuc boru uzerinden geliyor ve ebeveyn `poll` ile **zaman
    // asimli** bekliyor: cekirdek yanlissa cocuk asili kalir, ve bu
    // sinav asili kalan bir sinav olmamali.
    // Cocugun okumasinin **ham** sonucu; `NO_ANSWER` "hic cevap gelmedi"
    // demek. Tek bir "kaldi" mesaji uc ayri arizayi ortecekti: uyuyup
    // kalmak, `EAGAIN` almak ve veri gormek ayri sebepler.
    let mut child_raw = NO_ANSWER;
    let d = match sys::pipe() {
        Some((read_end, write_end)) => match sys::fork() {
            0 => {
                sys::close(read_end);
                let mut child_buf = [0u8; 8];
                let n = sys::read(sys::STDIN, &mut child_buf);
                // Tek bayta sigsin diye 128 kaydirmali: 128 = 0,
                // altindakiler hata, ustundekiler okunan bayt sayisi.
                let shifted = n.clamp(-120, 120) + 128;
                sys::write(write_end, &[shifted as u8]);
                sys::exit(0);
            }
            id if id > 0 => {
                sys::close(write_end);
                let mut watch = [sys::PollFd::new(read_end, sys::POLLIN)];
                let answered = sys::poll(&mut watch, ANSWER_MS) > 0 && watch[0].ready(sys::POLLIN);
                let mut mark = [0u8; 1];
                if answered && sys::read(read_end, &mut mark) == 1 {
                    child_raw = mark[0] as isize - 128;
                } else {
                    // Cocuk asili kaldi: birakilirsa sonraki cizim
                    // dongusu boyunca uyur.
                    tcmk::signal::kill(id as usize, tcmk::signal::SIGKILL);
                }
                let mut status = 0u32;
                sys::waitpid(id as usize, &mut status, 0);
                sys::close(read_end);
                child_raw == 0
            }
            // Catallanamadi. Neredeyse her zaman gorev yuvasinin
            // bitmesidir ve bu, cekirdegin stdin yoluyla ilgisiz bir
            // ariza -- ayri bir mesaj olmazsa oraya yamanirdi.
            _ => {
                child_raw = NO_FORK;
                false
            }
        },
        // Boru acilamadi: sinav kurulamadi. Ayri bir mesaji hak ediyor,
        // yoksa cekirdegin stdin yolu sucluymus gibi gorunur.
        None => {
            child_raw = NO_PIPE;
            false
        }
    };
    checks[3] = Check {
        name: "D penceresiz",
        detail: if d {
            "penceresiz surecte 0 (dosya sonu)"
        } else if child_raw == NO_FORK {
            "catallanamadi -- gorev yuvasi kalmadi"
        } else if child_raw == NO_PIPE {
            "boru acilamadi -- sinav kurulamadi"
        } else if child_raw == NO_ANSWER {
            "penceresiz surec ASILI kaldi"
        } else if child_raw < 0 {
            "penceresiz surecte okuma HATA dondu"
        } else {
            "penceresiz surec VERI gordu"
        },
        passed: d,
    };

    for check in &checks {
        let _ = writeln!(
            out,
            "[stdin] {}: {} ({})",
            check.name,
            if check.passed { "gecti" } else { "KALDI" },
            check.detail
        );
    }
    let _ = writeln!(
        out,
        "[stdin] gorev yuvasi: {}/{}",
        sys::task_count(),
        sys::task_slots()
    );
    let _ = writeln!(
        out,
        "[stdin] bekleme: {} tik (en az {}), penceresiz okuma: {}",
        waited_c,
        MIN_TICKS,
        match child_raw {
            NO_ANSWER => -999,
            NO_PIPE => -998,
            NO_FORK => -997,
            other => other,
        }
    );

    loop {
        if win.poll_key() == b'q' {
            break;
        }
        draw(&mut win, &checks, waited_c);
        // Tablo durgun; her karede yeniden cizmek CPU'yu bosuna yakar.
        win.frame(30);
    }
}

fn draw(win: &mut Window, checks: &[Check; 5], waited: usize) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);
    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "read(0) bekler; kacis yolu programda", ACCENT);

    let mut y = 30;
    for check in checks {
        win.text(6, y, check.name, FG);
        win.text(
            350,
            y,
            if check.passed { "gecti" } else { "KALDI" },
            if check.passed { OK } else { WARN },
        );
        y += 16;
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    win.text(6, h - 30, "bekleme (tik):", DIM);
    win.number(140, h - 30, waited, FG);
    win.text(
        6,
        h - 14,
        if passed == checks.len() {
            "hepsi gecti   q cik"
        } else {
            "BIR SINAV KALDI   q cik"
        },
        if passed == checks.len() { OK } else { WARN },
    );
}
