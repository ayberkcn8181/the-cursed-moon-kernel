//! Etkilesimli kabuk -- GUI icinde bir terminal penceresi.
//!
//! Kabugu cekirdek tarafinda tutmak bilincli bir tercihtir: Ring 3'te
//! calisan tam bir kabuk, satir duzenleme ve `fork/exec` gerektirir
//! (doc Faz 8/11-12). Buradaki kabuk cekirdek servislerine dogrudan
//! erisip sistemi gozlemlemeyi ve **Ring 3 uygulamalari baslatmayi**
//! saglar; boylece "uygulama calistirilabilir" iddiasi somutlasir.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::{fd, frames, init, kmalloc, mmu, pipe, scheduler, tcmkfs, vfs};
use crate::level0a::drivers::{ata, block, gfx, partition, rtc};
use crate::level0a::{exceptions, input, launcher, pit, wm};
use crate::level0b1::signal;
use crate::level0b2::{ipc, load_balancer, state_monitor};

const MAX_ROWS: usize = 24;
const MAX_COLS: usize = 78;
const PROMPT: &str = "tcmk> ";

static mut SCREEN: [[u8; MAX_COLS]; MAX_ROWS] = [[b' '; MAX_COLS]; MAX_ROWS];
static ROW: AtomicUsize = AtomicUsize::new(0);
static COL: AtomicUsize = AtomicUsize::new(0);

static mut INPUT: [u8; MAX_COLS] = [0; MAX_COLS];
static INPUT_LEN: AtomicUsize = AtomicUsize::new(0);

static WINDOW: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Kabuk penceresini olusturur.
pub fn start(x: usize, y: usize) {
    let w = MAX_COLS * gfx::font_width() + 8;
    let h = MAX_ROWS * gfx::font_height() + 6;

    match wm::create("TCMK Shell", x, y, w, h, true) {
        Some(id) => {
            WINDOW.store(id, Ordering::Relaxed);
            wm::mark_shell(id);
            wm::focus(id);
            write_line("The Cursed Moon Kernel -- etkilesimli kabuk");
            write_line("'help' yazip Enter'a basin.");
            write_line("");
            prompt();
        }
        None => crate::println!("[LEVEL-0a] shell: pencere olusturulamadi."),
    }
}

fn newline() {
    COL.store(0, Ordering::Relaxed);
    let row = ROW.load(Ordering::Relaxed);
    if row + 1 >= MAX_ROWS {
        // Yukari kaydir.
        unsafe {
            let screen = core::ptr::addr_of_mut!(SCREEN) as *mut [u8; MAX_COLS];
            for r in 1..MAX_ROWS {
                let src = screen.add(r).read();
                screen.add(r - 1).write(src);
            }
            screen.add(MAX_ROWS - 1).write([b' '; MAX_COLS]);
        }
    } else {
        ROW.store(row + 1, Ordering::Relaxed);
    }
}

fn put(ch: u8) {
    if ch == b'\n' {
        newline();
        return;
    }
    let col = COL.load(Ordering::Relaxed);
    if col >= MAX_COLS {
        newline();
    }
    let (row, col) = (ROW.load(Ordering::Relaxed), COL.load(Ordering::Relaxed));
    unsafe {
        let screen = core::ptr::addr_of_mut!(SCREEN) as *mut [u8; MAX_COLS];
        (*screen.add(row))[col] = ch;
    }
    COL.store(col + 1, Ordering::Relaxed);
}

pub fn write_str(s: &str) {
    for b in s.bytes() {
        put(b);
    }
}

pub fn write_line(s: &str) {
    write_str(s);
    newline();
}

fn prompt() {
    write_str(PROMPT);
}

/// WM'den gelen tus olayi.
pub fn on_key(ascii: u8) {
    match ascii {
        b'\n' => {
            newline();
            let len = INPUT_LEN.load(Ordering::Relaxed);
            let line = unsafe {
                let input = core::ptr::addr_of!(INPUT) as *const u8;
                core::slice::from_raw_parts(input, len)
            };
            let command = core::str::from_utf8(line).unwrap_or("");
            execute(command);
            INPUT_LEN.store(0, Ordering::Relaxed);
            prompt();
        }
        0x08 => {
            // Backspace
            let len = INPUT_LEN.load(Ordering::Relaxed);
            if len > 0 {
                INPUT_LEN.store(len - 1, Ordering::Relaxed);
                let col = COL.load(Ordering::Relaxed);
                if col > PROMPT.len() {
                    COL.store(col - 1, Ordering::Relaxed);
                    put(b' ');
                    COL.store(col - 1, Ordering::Relaxed);
                }
            }
        }
        0x20..=0x7E => {
            let len = INPUT_LEN.load(Ordering::Relaxed);
            if len < MAX_COLS - PROMPT.len() - 1 {
                unsafe {
                    let input = core::ptr::addr_of_mut!(INPUT) as *mut u8;
                    input.add(len).write(ascii);
                }
                INPUT_LEN.store(len + 1, Ordering::Relaxed);
                put(ascii);
            }
        }
        _ => {}
    }
}

/// Kucuk bir sayi bicimleyici (no_std, ayirma yok).
fn write_num(mut n: usize) {
    if n == 0 {
        put(b'0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut i = 0;
    while n > 0 {
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        put(digits[i]);
    }
}

/// Onaltilik yazdirma (sabit genislik, `0x` onekli).
fn write_hex(value: usize, digits: usize) {
    write_str("0x");
    let mut i = digits;
    while i > 0 {
        i -= 1;
        let nibble = ((value >> (i * 4)) & 0xF) as u8;
        put(if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        });
    }
}

/// Sayiyi sagdan hizali yazar (tablolar icin).
fn write_num_right(n: usize, width: usize) {
    let mut digits = 1;
    let mut probe = n;
    while probe >= 10 {
        probe /= 10;
        digits += 1;
    }
    for _ in digits..width {
        put(b' ');
    }
    write_num(n);
}

/// Iki haneli, sifir dolgulu (tarih/saat alanlari icin).
fn write_pad2(n: usize) {
    if n < 10 {
        put(b'0');
    }
    write_num(n);
}

fn write_padded(s: &str, width: usize) {
    write_str(s);
    for _ in s.len()..width {
        put(b' ');
    }
}

/// "12 34" bicimindeki iki sayiyi ayirir (`signal <gorev> <sinyal>`).
fn two_numbers(arg: &str) -> Option<(usize, usize)> {
    let arg = arg.trim();
    let space = arg.find(' ')?;
    let first = arg[..space].trim().parse_usize()?;
    let second = arg[space + 1..].trim().parse_usize()?;
    Some((first, second))
}

/// `str::parse` `core`'da var ama hata tipi bicimleme gerektirir; kucuk
/// bir ayristirici tutmak hem daha ucuz hem de bos/kirli girdide net.
trait ParseUsize {
    fn parse_usize(&self) -> Option<usize>;
}

impl ParseUsize for str {
    fn parse_usize(&self) -> Option<usize> {
        if self.is_empty() {
            return None;
        }
        let mut value = 0usize;
        for b in self.bytes() {
            if !b.is_ascii_digit() {
                return None;
            }
            value = value * 10 + (b - b'0') as usize;
        }
        Some(value)
    }
}

/// Bir sinyal maskesini "SIGUSR1 SIGTERM" gibi yazar; bos maskede "-".
fn write_sig_list(mask: u32, width: usize) {
    let start = COL.load(Ordering::Relaxed);
    if mask == 0 {
        put(b'-');
    } else {
        let mut first = true;
        for signo in 1..=signal::MAX_SIGNAL {
            if mask & (1 << signo) == 0 {
                continue;
            }
            if !first {
                put(b',');
            }
            first = false;
            write_str(signal::name_of(signo));
        }
    }
    let written = COL.load(Ordering::Relaxed).saturating_sub(start);
    for _ in written..width {
        put(b' ');
    }
}

/// `path`, `prefix` dizininin altinda mi?
///
/// Salt metin karsilastirmasi yeterli: VFS yollari `path_of` ile
/// kanonik uretiliyor, yani ".." ya da tekrarli egik cizgi icermiyorlar.
fn path_under(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() || prefix == "/" {
        return true;
    }
    match path.strip_prefix(prefix) {
        // "/notlar" ile "/notlar2/x" karismasin diye ayirici sart.
        Some(rest) => rest.starts_with('/'),
        None => false,
    }
}

fn state_name(state: scheduler::TaskState) -> &'static str {
    match state {
        scheduler::TaskState::Unused => "bos",
        scheduler::TaskState::Ready => "hazir",
        scheduler::TaskState::Running => "calisiyor",
        scheduler::TaskState::Blocked => "uyuyor",
        scheduler::TaskState::Waiting => "bekliyor",
        scheduler::TaskState::IoWait => "disk",
        scheduler::TaskState::Terminated => "bitti",
    }
}

fn execute(line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }

    // Komut ve argumani ayir.
    let (cmd, arg) = match line.find(' ') {
        Some(i) => (&line[..i], line[i + 1..].trim()),
        None => (line, ""),
    };

    match cmd {
        "help" => {
            write_line("sistem:");
            write_line("  ps  top  kill <id>  svc  health  uptime  date  ver");
            write_line("  signal <id> <sinyal>   sinyal gonder (9=KILL 10=USR1 15=TERM)");
            write_line("  sigs                   isleyicileri ve bekleyen sinyalleri goster");
            write_line("level-0b2 (merkezi denetleyici):");
            write_line("  load          yuk dengeleyici istatistikleri");
            write_line("  ipc           mesaj kuyrugu + paylasimli bolge");
            write_line("  faults        istisna/hata istatistikleri");
            write_line("  stall <sn>    nabzi bastir (fallback testi)");
            write_line("bellek / disk:");
            write_line("  mem  disk  df  format onayla  sync  install");
            write_line("dosya:");
            write_line("  ls [dizin]  cat <yol>  save <yol> <metin>  cp <kaynak> <hedef>  rm <yol>");
            write_line("  mkdir <yol>  rmdir <yol>");
            write_line("uygulama / pencere:");
            write_line("  apps  run <ad>  win  focus <id>  mouse");
            write_line("diger:");
            write_line("  echo <metin>  pipes  clear  help");
        }
        "ps" => {
            write_line("  id durum      ad            cagri  adres-uzayi");
            for i in 0..scheduler::task_count() {
                let state = scheduler::state_of(i);
                write_str(if i == scheduler::current_id() {
                    "  *"
                } else {
                    "   "
                });
                write_num_right(i, 2);
                put(b' ');
                write_padded(state_name(state), 10);
                put(b' ');
                write_padded(scheduler::name_of(i), 11);
                write_num_right(load_balancer::task_total(i) as usize, 7);
                let space = scheduler::address_space_of(i);
                if space == 0 {
                    write_str("     cekirdek");
                } else {
                    write_str("  ");
                    write_hex(space, 8);
                    write_str(" (");
                    write_num(mmu::user_pages(space));
                    write_str(" sayfa)");
                }
                newline();
            }
            write_str("baglam degisimi: ");
            write_num(scheduler::switch_count());
            write_str("  (zorla: ");
            write_num(scheduler::preemptions());
            write_str(")  uyuyan: ");
            write_num(scheduler::sleeping_count());
            newline();
        }
        "top" => {
            write_str("son saniye -- kota ");
            write_num(load_balancer::quota() as usize);
            write_line(" cagri/gorev");
            write_line("  id ad            cagri/sn   kisitlama");
            for i in 0..scheduler::task_count() {
                write_str("  ");
                write_num_right(i, 2);
                put(b' ');
                write_padded(scheduler::name_of(i), 13);
                write_num_right(load_balancer::task_rate(i) as usize, 9);
                write_num_right(load_balancer::task_throttles(i) as usize, 12);
                newline();
            }
            write_str("en yogun gorev: #");
            write_num(load_balancer::busiest_task());
            write_str(" ");
            write_line(scheduler::name_of(load_balancer::busiest_task()));
        }
        "load" => {
            write_line("  kanal          toplam    /sn     tepe  durum");
            for i in 0..load_balancer::CHANNELS {
                write_str("  ");
                write_padded(load_balancer::CHANNEL_NAMES[i], 13);
                write_num_right(load_balancer::total(i) as usize, 8);
                write_num_right(load_balancer::rate(i) as usize, 7);
                write_num_right(load_balancer::peak(i) as usize, 9);
                write_str(if load_balancer::is_saturated(i) {
                    "  DOYGUN"
                } else {
                    "  normal"
                });
                newline();
            }
            write_str("toplam cagri: ");
            write_num(load_balancer::call_count() as usize);
            write_str("  kisitlama: ");
            write_num(load_balancer::throttles() as usize);
            newline();
            write_str("olcum penceresi: ");
            write_num(load_balancer::windows_rolled() as usize);
            write_str(" saniye  darbogaz: ");
            write_line(if load_balancer::any_saturated() {
                "VAR"
            } else {
                "yok"
            });
        }
        "ipc" => {
            write_str("paylasimli bolge: ");
            write_hex(ipc::shared_addr(), 8);
            write_str(if ipc::shared_is_valid() {
                "  (gecerli)"
            } else {
                "  (BOZUK)"
            });
            newline();
            write_str("kuyruk: ");
            write_num(ipc::depth());
            write_str("/");
            write_num(ipc::capacity());
            write_str("  tepe: ");
            write_num(ipc::high_water());
            newline();
            write_str("gonderilen: ");
            write_num(ipc::posted() as usize);
            write_str("  tuketilen: ");
            write_num(ipc::consumed() as usize);
            write_str("  dusen: ");
            write_num(ipc::dropped() as usize);
            newline();
            write_line("son olaylar:");
            for i in 0..ipc::history_len() {
                if let Some(m) = ipc::history(i) {
                    write_str("  t=");
                    write_num_right(m.tick as usize, 6);
                    put(b' ');
                    write_padded(ipc::kind_name(m.kind), 17);
                    write_line(m.text);
                }
            }
        }
        "svc" => {
            for i in 0..init::service_count() {
                if let Some(s) = init::service(i) {
                    write_str("  ");
                    write_padded(s.name, 14);
                    write_line(match s.state {
                        init::ServiceState::Active => "[ACTIVE]",
                        init::ServiceState::Failed => "[FAILED]",
                        init::ServiceState::Inactive => "[INACTIVE]",
                    });
                }
            }
            write_str("hepsi aktif: ");
            write_line(if init::all_services_active() {
                "evet"
            } else {
                "HAYIR"
            });
        }
        "pipes" => {
            write_str("acik boru: ");
            write_num(pipe::open_count());
            write_str(" / ");
            write_num(pipe::MAX_PIPES);
            write_str("  (tampon ");
            write_num(pipe::PIPE_CAPACITY);
            write_line(" bayt)");
            for i in 0..pipe::MAX_PIPES {
                if let Some((bytes, writers, readers)) = pipe::info(i) {
                    write_str("  #");
                    write_num(i);
                    write_str("  bekleyen:");
                    write_num_right(bytes, 5);
                    write_str("  yazan:");
                    write_num(writers);
                    write_str("  okuyan:");
                    write_num(readers);
                    newline();
                }
            }
        }
        "health" => {
            write_str("Level-0a durumu: ");
            write_line(match state_monitor::health() {
                crate::level0b2::state_monitor::Level0aHealth::Healthy => "Saglikli",
                crate::level0b2::state_monitor::Level0aHealth::Busy => "Mesgul",
                crate::level0b2::state_monitor::Level0aHealth::Dead => "OLU",
            });
            write_str("nabiz: ");
            write_num(pit::heartbeat() as usize);
            write_str("  tick: ");
            write_num(pit::ticks() as usize);
            newline();
            write_str("paylasimli nabiz: ");
            write_num(
                ipc::SHARED
                    .heartbeat
                    .load(core::sync::atomic::Ordering::Relaxed) as usize,
            );
            newline();
            if pit::heartbeat_suppressed() {
                write_line("nabiz SIMULASYON GEREGI bastirilmis durumda ('stall').");
            }
            write_str("nabiz sessizligi: ");
            write_num(state_monitor::stalled_ticks() as usize);
            write_str(" tick (olu esigi ");
            write_num(state_monitor::dead_threshold_ticks() as usize);
            write_line(")");
            write_str("co-service: ");
            write_line(if state_monitor::level0a_is_dead() {
                "DEVREDE (Level-0a olu)"
            } else {
                "beklemede"
            });
            write_str("olu olayi: ");
            write_num(state_monitor::dead_events() as usize);
            write_str("  toparlanma: ");
            write_num(state_monitor::recoveries() as usize);
            newline();
            write_str("minimal modda reddedilen cagri: ");
            write_num(crate::level0b2::fallback::unsupported_calls() as usize);
            newline();
            write_str("gercek zaman saati: ");
            if rtc::available() {
                write_line("var (CMOS RTC)");
            } else {
                write_line("YOK -- zaman damgalari 0");
            }
        }
        "stall" => {
            // Doc S.12 test maddesi: "Heartbeat timeout simulasyonunda
            // fallback devreye girer". Nabiz bastirilir; Level-0a aslinda
            // calismaya devam eder ama disaridan kilitlenmis gorunur.
            let seconds = arg
                .bytes()
                .fold(None::<usize>, |acc, b| {
                    if b.is_ascii_digit() {
                        Some(acc.unwrap_or(0) * 10 + (b - b'0') as usize)
                    } else {
                        acc
                    }
                })
                .unwrap_or(5)
                .clamp(1, 30);
            pit::suppress_heartbeat(seconds as u32 * 100);
            write_str("nabiz ");
            write_num(seconds);
            write_line(" saniye bastiriliyor -- Fallback devreye girmeli.");
            write_line("('health' ve 'ipc' ile izleyin)");
        }
        "ver" => {
            write_line("The Cursed Moon Kernel (TCMK) -- Rust portu");
            write_str("mimari: ");
            write_line(if cfg!(target_arch = "x86_64") {
                "x86_64 (Long Mode, syscall)"
            } else {
                "i386 (korumali mod, int 0x80)"
            });
            write_line("katmanlar: Level-0b2 / Level-0b1 / Level-0a / Level-1");
            write_line("ikili formatlar: ELF32 + PE32 (ayni cekirdek, iki dunya)");
        }
        "kill" => {
            let id = arg.bytes().fold(None::<usize>, |acc, b| {
                if b.is_ascii_digit() {
                    Some(acc.unwrap_or(0) * 10 + (b - b'0') as usize)
                } else {
                    acc
                }
            });
            // `kill` POSIX'te "SIGKILL gonder"in kisaltmasidir; TCMK'de de
            // oyle. Fark su ki artik `signal` ile baska bir sinyal de
            // gonderilebilir ve surec onu yakalayabilir -- SIGKILL ise
            // yakalanamaz, hedefi kosulsuz durdurur.
            match id {
                Some(i) => match signal::raise(i, signal::SIGKILL) {
                    Ok(()) => {
                        write_str("SIGKILL gonderildi: gorev #");
                        write_num(i);
                        newline();
                    }
                    Err(_) => write_line("boyle bir gorev yok ('ps' ile listeleyin)"),
                },
                None => write_line("kullanim: kill <gorev-no>  ('ps' ile listeleyin)"),
            }
        }
        "signal" => {
            // signal <gorev-no> <sinyal>
            match two_numbers(arg) {
                Some((task, signo)) => match signal::raise(task, signo as u32) {
                    Ok(()) => {
                        write_str(signal::name_of(signo as u32));
                        write_str(" gonderildi: gorev #");
                        write_num(task);
                        write_line(" (teslim, surec bir sonraki syscall'da Ring 3'e donerken)");
                    }
                    Err(signal::SignalError::InvalidSignal) => {
                        write_line("gecersiz sinyal (1..31)")
                    }
                    Err(signal::SignalError::Uncatchable) => write_line("bu sinyal yakalanamaz"),
                    Err(signal::SignalError::NoSuchTask) => {
                        write_line("boyle bir gorev yok ('ps' ile listeleyin)")
                    }
                },
                None => {
                    write_line("kullanim: signal <gorev-no> <sinyal-no>");
                    write_line("  9=SIGKILL (yakalanamaz)  10=SIGUSR1  12=SIGUSR2  15=SIGTERM");
                }
            }
        }
        "sigs" => {
            write_str("gonderilen: ");
            write_num(signal::sent_count() as usize);
            write_str("   teslim edilen: ");
            write_num(signal::delivered_count() as usize);
            newline();
            write_line("  id  gorev        isleyicili        bekleyen");
            for i in 0..scheduler::MAX_TASKS {
                if scheduler::state_of(i) == scheduler::TaskState::Unused {
                    continue;
                }
                let handled = signal::handled_mask(i);
                let pending = signal::pending_mask(i);
                if handled == 0 && pending == 0 {
                    continue;
                }
                write_num_right(i, 4);
                write_str("  ");
                write_padded(scheduler::name_of(i), 12);
                write_str(" ");
                write_sig_list(handled, 17);
                write_sig_list(pending, 0);
                newline();
            }
        }
        "echo" => write_line(arg),
        "mouse" => {
            write_str("fare: x=");
            write_num(input::mouse_x());
            write_str(" y=");
            write_num(input::mouse_y());
            write_str(" sol=");
            write_line(if input::mouse_left() { "basili" } else { "serbest" });
            write_str("ekran: ");
            write_num(gfx::width());
            write_str("x");
            write_num(gfx::height());
            newline();
        }
        "mem" => {
            write_str("heap kullanilan: ");
            write_num(kmalloc::used_bytes());
            write_line(" bayt");
            write_str("heap bos: ");
            write_num(kmalloc::free_bytes() / 1024);
            write_line(" KiB");
            write_str("kullanici bolgesi: ");
            write_hex(mmu::USER_MEM_START, 8);
            write_str(" + ");
            write_num(mmu::USER_MAP_SIZE / 1024);
            write_line(" KiB / surec");
            write_str("cerceve havuzu: ");
            write_num(frames::used());
            write_str(" / ");
            write_num(frames::total());
            write_str(" kullanimda (tepe ");
            write_num(frames::peak());
            write_str(", bos ");
            write_num(frames::free_count() * 4 / 1024);
            write_line(" MiB)");
            write_str("identity esleme: ");
            write_num(mmu::identity_mapped_bytes() / (1024 * 1024));
            write_line(" MiB");
            write_str("acik dosya: ");
            write_num(fd::open_count());
            newline();
        }
        "ls" => {
            // Argumansiz: her sey. Argumanla: yalnizca o dizinin altindaki
            // yollar. Once diskteki dizinler listelenir -- VFS'in dizin
            // kavrami olmadigi icin onlari TCMKFS'e sormak gerekir.
            let prefix = if arg.is_empty() { "/" } else { arg.trim_end_matches('/') };
            let mut dirs = 0usize;
            if tcmkfs::mounted() {
                if let Some(dir) = tcmkfs::resolve(if prefix.is_empty() { "/" } else { prefix }) {
                    let mut buf = [0u8; tcmkfs::PATH_MAX];
                    for i in 0..tcmkfs::MAX_INODES {
                        if tcmkfs::entry_kind(i) != Some(tcmkfs::KIND_DIR)
                            || i == tcmkfs::ROOT_INODE
                            || tcmkfs::parent_of(i) != Some(dir)
                        {
                            continue;
                        }
                        if let Some(path) = tcmkfs::path_of(i, &mut buf) {
                            write_str("  ");
                            write_padded("dizin", 6);
                            write_padded(path, 22);
                            newline();
                            dirs += 1;
                        }
                    }
                }
            }
            let mut files = 0usize;
            for i in 0..vfs::node_count() {
                if let Some(path) = vfs::path_of(i) {
                    if !arg.is_empty() && !path_under(path, prefix) {
                        continue;
                    }
                    files += 1;
                    write_str("  ");
                    write_padded(
                        match vfs::source(i) {
                            Some(vfs::Source::Disk) => "disk",
                            _ => "ram",
                        },
                        6,
                    );
                    write_padded(path, 22);
                    write_num_right(vfs::size(i).unwrap_or(0), 7);
                    write_str(" bayt");
                    // Yalnizca diskteki dosyalarin zaman damgasi vardir;
                    // RAMFS dosyalari cekirdek imajiyla birlikte gelir.
                    if let Some(epoch) = vfs::mtime(i) {
                        if epoch != 0 {
                            let t = rtc::from_unix(epoch);
                            write_str("  ");
                            write_num_right(t.year as usize, 4);
                            put(b'-');
                            write_pad2(t.month as usize);
                            put(b'-');
                            write_pad2(t.day as usize);
                            put(b' ');
                            write_pad2(t.hour as usize);
                            put(b':');
                            write_pad2(t.minute as usize);
                        }
                    }
                    newline();
                }
            }
            write_str(" toplam ");
            write_num(files);
            write_str(" dosya");
            if dirs > 0 {
                write_str(", ");
                write_num(dirs);
                write_str(" dizin");
            }
            newline();
        }
        "mkdir" => {
            if arg.is_empty() {
                write_line("kullanim: mkdir <yol>   (ust dizin onceden var olmali)");
            } else {
                match vfs::mkdir(arg) {
                    Ok(()) => {
                        write_str("dizin olusturuldu: ");
                        write_line(arg);
                    }
                    Err(e) => write_line(tcmkfs::error_name(e)),
                }
            }
        }
        "rmdir" => {
            if arg.is_empty() {
                write_line("kullanim: rmdir <yol>   (dizin bos olmali)");
            } else {
                match vfs::rmdir(arg) {
                    Ok(()) => {
                        write_str("dizin silindi: ");
                        write_line(arg);
                    }
                    Err(e) => write_line(tcmkfs::error_name(e)),
                }
            }
        }
        "disk" => {
            if !block::available() {
                write_line("blok aygiti yok (ISO'dan disksiz acildi).");
            } else {
                write_str("aygit: ");
                write_str(block::name());
                write_str("  model: ");
                write_line(ata::model());
                write_str("kapasite: ");
                write_num(block::sector_count() as usize);
                write_str(" sektor (");
                write_num(block::capacity_mib() as usize);
                write_line(" MiB)");
                let (r, w) = block::stats();
                write_str("okunan: ");
                write_num(r);
                write_str(" sektor  yazilan: ");
                write_num(w);
                newline();
                let (_, _, errors) = ata::stats();
                write_str("hata: ");
                write_num(errors as usize);
                write_str("  surucu hazir: ");
                write_line(if ata::drive_ready() { "evet" } else { "hayir" });
                write_line("bolumler:");
                for i in 0..partition::count() {
                    if let Some(p) = partition::get(i) {
                        write_str("  ");
                        write_num(p.index + 1);
                        write_str("  tur ");
                        write_hex(p.kind as usize, 2);
                        write_str(" ");
                        write_padded(partition::type_name(p.kind), 12);
                        write_str("lba ");
                        write_num_right(p.start_lba as usize, 8);
                        write_num_right(p.sectors as usize / 2048, 6);
                        write_str(" MiB");
                        if p.bootable {
                            write_str("  [acilis]");
                        }
                        newline();
                    }
                }
                if partition::count() == 0 {
                    write_line("  (bolum tablosu bos veya MBR imzasi yok)");
                }
            }
        }
        "df" => {
            if !tcmkfs::mounted() {
                write_line("kalici dosya sistemi bagli degil.");
                write_line("('disk' ile bolume, 'format onayla' ile bicimlendirmeye bakin)");
            } else {
                write_str("tcmkfs: ");
                write_num(tcmkfs::used_kib() as usize);
                write_str(" / ");
                write_num(tcmkfs::total_kib() as usize);
                write_line(" KiB kullanimda");
                write_str("bos blok: ");
                write_num(tcmkfs::free_blocks() as usize);
                write_str(" / ");
                write_num(tcmkfs::total_blocks() as usize);
                write_str("  (blok ");
                write_num(tcmkfs::BLOCK_SIZE);
                write_line(" bayt)");
                write_str("dosya: ");
                write_num(tcmkfs::file_count());
                write_str(" / ");
                write_num(tcmkfs::MAX_INODES);
                write_str("  (isim uzayinda ");
                write_num(vfs::file_count());
                write_str(")");
                write_str("  blok yazimi: ");
                write_num(tcmkfs::block_writes() as usize);
                write_str("  IRQ14: ");
                write_num(ata::irq_count() as usize);
                write_str("  io beklemesi: ");
                write_num(scheduler::io_waits());
                newline();
            }
        }
        "format" => {
            if arg != "onayla" {
                write_line("DIKKAT: TCMKFS bolumundeki TUM veri silinir.");
                write_line("onaylamak icin: format onayla");
            } else {
                match tcmkfs::format("TCMK") {
                    Ok(()) => {
                        write_str("bicimlendirildi: ");
                        write_num(tcmkfs::total_kib() as usize / 1024);
                        write_line(" MiB kullanilabilir");
                    }
                    Err(e) => write_line(tcmkfs::error_name(e)),
                }
            }
        }
        "save" => {
            let (path, text) = match arg.find(' ') {
                Some(i) => (&arg[..i], arg[i + 1..].trim_start()),
                None => (arg, ""),
            };
            if path.is_empty() {
                write_line("kullanim: save <yol> <metin>");
            } else {
                let mut buf = [0u8; MAX_COLS + 1];
                let len = text.len().min(MAX_COLS);
                buf[..len].copy_from_slice(&text.as_bytes()[..len]);
                buf[len] = b'\n';
                match vfs::write_file(path, &buf[..len + 1]) {
                    Ok(n) => {
                        write_num(n);
                        write_str(" bayt yazildi: ");
                        write_line(path);
                    }
                    Err(e) => write_line(tcmkfs::error_name(e)),
                }
            }
        }
        "cp" => {
            // Bir dosyayi diske kopyalar -- "uygulama kurmanin" en yalin
            // hali: /bin/paint'i /home/paint'e kopyalayip oradan calistirmak.
            let (src, dst) = match arg.find(' ') {
                Some(i) => (&arg[..i], arg[i + 1..].trim()),
                None => (arg, ""),
            };
            if src.is_empty() || dst.is_empty() {
                write_line("kullanim: cp <kaynak> <hedef>");
            } else {
                match vfs::lookup(src).and_then(vfs::load) {
                    Some(data) => match vfs::write_file(dst, data) {
                        Ok(n) => {
                            write_num(n);
                            write_str(" bayt kopyalandi -> ");
                            write_line(dst);
                        }
                        Err(e) => write_line(tcmkfs::error_name(e)),
                    },
                    None => write_line("kaynak dosya okunamadi"),
                }
            }
        }
        "rm" => {
            if arg.is_empty() {
                write_line("kullanim: rm <yol>");
            } else {
                match vfs::remove_file(arg) {
                    Ok(()) => {
                        write_str("silindi: ");
                        write_line(arg);
                    }
                    Err(e) => write_line(tcmkfs::error_name(e)),
                }
            }
        }
        "install" => {
            // Diskin acilis sektorunu TCMK'nin kendi onyukleyicisiyle
            // degistirir; bundan sonra GRUB devrede olmaz.
            #[cfg(target_arch = "x86")]
            {
                use crate::level0a::installer;
                if arg != "onayla" {
                    write_line("TCMK onyukleyicisi diskin acilis sektorune yazilacak.");
                    write_line("(bolum tablosu korunur; GRUB devreden cikar)");
                    write_str("2. asama diskte: ");
                    write_line(if installer::stage2_present() {
                        "hazir"
                    } else {
                        "YOK -- kurulum reddedilecek"
                    });
                    if let Some(lba) = installer::kernel_blob_lba() {
                        write_str("cekirdek imaji: lba ");
                        write_num(lba as usize);
                        newline();
                    }
                    write_line("onaylamak icin: install onayla");
                } else {
                    match installer::install() {
                        Ok(r) => {
                            write_str("1. asama yazildi (");
                            write_num(r.stage1_bytes);
                            write_line(" bayt, MBR 0..446).");
                            write_str("2. asama: lba ");
                            write_num(r.stage2_lba as usize);
                            write_str(", ");
                            write_num(r.stage2_sectors as usize);
                            write_line(" sektor.");
                            write_str("cekirdek imaji: lba ");
                            write_num(r.blob_lba as usize);
                            newline();
                            write_line("Kurulum tamam -- makineyi diskten yeniden baslatin.");
                        }
                        Err(e) => write_line(installer::error_name(e)),
                    }
                }
            }
            #[cfg(not(target_arch = "x86"))]
            write_line("kurulum su an yalnizca i386'da destekleniyor.");
        }
        "sync" => match tcmkfs::sync() {
            Ok(()) => write_line("tcmkfs: onbellek diske yazildi."),
            Err(e) => write_line(tcmkfs::error_name(e)),
        },
        "cat" => match vfs::lookup(arg) {
            Some(node) => {
                let mut buf = [0u8; 256];
                let n = vfs::read(node, 0, &mut buf).unwrap_or(0);
                for &b in &buf[..n] {
                    put(if b == b'\n' { b'\n' } else { b });
                }
                if n > 0 && buf[n - 1] != b'\n' {
                    newline();
                }
            }
            None => write_line("dosya bulunamadi"),
        },
        "run" => {
            if arg.is_empty() {
                write_line("kullanim: run <yol>");
            } else {
                match crate::level0a::launcher::spawn_user_app(arg) {
                    Ok(()) => {
                        write_str("baslatildi: ");
                        write_line(arg);
                    }
                    Err(msg) => write_line(msg),
                }
            }
        }
        "apps" => {
            write_line("calistirilabilir uygulamalar ('run <ad>'):");
            for (short, full, _) in launcher::available() {
                write_str("  ");
                write_str(short);
                write_str("  ->  ");
                write_line(full);
            }
        }
        "faults" => {
            write_str("yakalanan istisna: ");
            write_num(exceptions::total());
            newline();
            write_str("sonlandirilan surec: ");
            write_num(exceptions::killed_processes());
            newline();
            write_str("son istisna: ");
            match exceptions::last_vector() {
                Some(v) => {
                    write_str("#");
                    write_num(v);
                    write_str(" ");
                    write_line(exceptions::NAMES.get(v).copied().unwrap_or("?"));
                }
                None => write_line("yok"),
            }
        }
        "win" => {
            write_line("  id  boyut     sahip     tampon (surecin adresi)");
            for i in 0..wm::window_count() {
                if let Some(w) = wm::get(i) {
                    write_str("  ");
                    write_num_right(i, 2);
                    write_str("  ");
                    let mut dims = 0;
                    write_num(w.width);
                    write_str("x");
                    write_num(w.height);
                    dims += 1;
                    let _ = dims;
                    write_str("  ");
                    if w.system {
                        write_padded("cekirdek", 10);
                        write_line("(kernel tamponu)");
                    } else {
                        write_padded(scheduler::name_of(w.owner), 10);
                        write_hex(w.user_addr, 8);
                        newline();
                    }
                }
            }
            write_line("(her tampon YALNIZCA sahibinin adres uzayina eslidir)");
            write_line("odak degistirmek icin: focus <id>");
        }
        "focus" => {
            // Klavye odagini kabuktan baska bir pencereye verir. Fareyle
            // tiklamak da ayni isi yapar; komut, uzaktan/betikle surulen
            // oturumlar icin gerekli.
            let id = arg.bytes().fold(None::<usize>, |acc, b| {
                if b.is_ascii_digit() {
                    Some(acc.unwrap_or(0) * 10 + (b - b'0') as usize)
                } else {
                    acc
                }
            });
            match id {
                Some(i) if i < wm::window_count() => {
                    wm::focus(i);
                    write_str("odak: pencere #");
                    write_num(i);
                    newline();
                }
                Some(_) => write_line("boyle bir pencere yok ('win' ile listeleyin)"),
                None => write_line("kullanim: focus <pencere-no>"),
            }
        }
        "date" => {
            match rtc::now() {
                Some(t) => {
                    write_num_right(t.year as usize, 4);
                    put(b'-');
                    write_pad2(t.month as usize);
                    put(b'-');
                    write_pad2(t.day as usize);
                    put(b' ');
                    write_pad2(t.hour as usize);
                    put(b':');
                    write_pad2(t.minute as usize);
                    put(b':');
                    write_pad2(t.second as usize);
                    write_line("  (UTC, CMOS RTC)");
                    write_str("unix zamani: ");
                    write_num(rtc::unix_time() as usize);
                    newline();
                }
                None => write_line("gercek zaman saati yok."),
            }
        }
        "uptime" => {
            write_str("tick: ");
            write_num(pit::ticks() as usize);
            write_str("  (~");
            write_num(pit::ticks() as usize / 100);
            write_line(" saniye)");

            // PIT tick'i sayar; RTC varsa duvardaki saate gore de olculur.
            // Ikisinin farki, kesmelerin kacirildigini gosterir.
            let boot = rtc::boot_epoch();
            if boot != 0 {
                let seconds = rtc::unix_time().saturating_sub(boot) as usize;
                write_str("duvar saati: ");
                write_num(seconds / 3600);
                put(b':');
                write_pad2((seconds / 60) % 60);
                put(b':');
                write_pad2(seconds % 60);
                write_line("  (RTC)");

                write_str("acilis: ");
                let t = rtc::from_unix(boot);
                write_num(t.year as usize);
                put(b'-');
                write_pad2(t.month as usize);
                put(b'-');
                write_pad2(t.day as usize);
                put(b' ');
                write_pad2(t.hour as usize);
                put(b':');
                write_pad2(t.minute as usize);
                put(b':');
                write_pad2(t.second as usize);
                write_line(" (UTC)");
            }
        }
        "clear" => {
            unsafe {
                let screen = core::ptr::addr_of_mut!(SCREEN) as *mut [u8; MAX_COLS];
                for r in 0..MAX_ROWS {
                    screen.add(r).write([b' '; MAX_COLS]);
                }
            }
            ROW.store(0, Ordering::Relaxed);
            COL.store(0, Ordering::Relaxed);
        }
        _ => {
            write_str("bilinmeyen komut: ");
            write_line(cmd);
        }
    }
}

/// Kabuk penceresinin icerigini piksel tamponuna cizer (WM cagirir).
pub fn render(w: &wm::Window) {
    let pixels = w.buffer as *mut u32;
    unsafe {
        for i in 0..(w.width * w.height) {
            pixels.add(i).write(0x0008_0C10);
        }
    }

    let fh = gfx::font_height();
    let fw = gfx::font_width();

    unsafe {
        let screen = core::ptr::addr_of!(SCREEN) as *const [u8; MAX_COLS];
        for r in 0..MAX_ROWS {
            let row = &*screen.add(r);
            for c in 0..MAX_COLS {
                let ch = row[c];
                if ch != b' ' {
                    wm::draw_char_into_window(w, 4 + c * fw, 3 + r * fh, ch, 0x0080_F080);
                }
            }
        }
    }

    // Imlec
    let (r, c) = (ROW.load(Ordering::Relaxed), COL.load(Ordering::Relaxed));
    if c < MAX_COLS && r < MAX_ROWS && (pit::ticks() / 50) % 2 == 0 {
        wm::fill_into_window(w, 4 + c * fw, 3 + r * fh, fw, fh, 0x0040_8040);
    }
}
