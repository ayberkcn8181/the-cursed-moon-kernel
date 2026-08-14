//! `arena` -- `mmap` / `munmap` gosterimi: bellek al, geri ver.
//!
//! `brk` tek bir siniri iter ve geri cekildiginde cerceve **iade
//! etmez** -- adres hala surecin sayilir. `mmap` bagimsiz bloklar verir,
//! `munmap` ise cerceveleri gercekten havuza dondurur. Sistemdeki ilk
//! gercek "geri verme" yolu budur.
//!
//! Program bunu gozle gorulur hale getirir:
//!
//! ```text
//!   'a' -> 32 KiB blok ayir, DESENLE doldur
//!   'v' -> butun bloklari dogrula (desen bozulmus mu?)
//!   'f' -> son blogu birak (munmap)
//!   'z' -> hepsini birak
//! ```
//!
//! Kabuktan `mem` ve `ps` ile izlenir: blok ayrildikca `cerceve havuzu`
//! artar, `munmap`'ten sonra **duser**. `ps` satirindaki `+Nm` sayisi da
//! surecin `mmap` penceresinde tuttugu sayfa sayisidir.
//!
//! Ayrilan bellegin **sifirlanmis** geldigi de burada gorunur: blok
//! doldurulmadan once okunur ve sifir olmayan bayt sayilir. Cekirdek
//! cerceveyi vermeden once temizler; temizlemeseydi onceki surecin
//! verisi buraya sizardi.
//!
//! Tuslar: `a` ayir, `v` dogrula, `f` sonuncuyu birak, `z` hepsi, `q` cik

#![no_std]
#![no_main]

use tcmk::gui::Window;
use tcmk::io::Stdout;
use tcmk::sys;

tcmk::entry!(main);

const BG: u32 = 0x000E_1410;
const PANEL: u32 = 0x0018_2C22;
const FG: u32 = 0x00DC_ECE0;
const ACCENT: u32 = 0x0060_E0B0;
const OK: u32 = 0x0080_F090;
const WARN: u32 = 0x00FF_B060;

/// Blok boyutu. 32 KiB = 8 sayfa; pencere 512 KiB oldugu icin en fazla
/// 16 blok sigar.
const BLOCK: usize = 32 * 1024;
const MAX_BLOCKS: usize = 16;

struct Arena {
    blocks: [Option<*mut u8>; MAX_BLOCKS],
    count: usize,
    /// Ayrilan bloklarda bulunan sifir olmayan bayt sayisi -- cekirdek
    /// cerceveleri temizliyorsa sifir kalmali.
    dirty_bytes: usize,
    /// Dogrulamada bozuk bulunan bayt sayisi.
    corrupt: usize,
    verified: usize,
    failed: usize,
}

/// Bir blok ayirir, **once okuyup** sifirlanmis geldigini dogrular,
/// sonra desenle doldurur.
fn allocate(arena: &mut Arena, out: &mut Stdout) {
    use core::fmt::Write;
    match sys::mmap(BLOCK) {
        Some(ptr) if arena.count < MAX_BLOCKS => {
            let mut dirty = 0usize;
            unsafe {
                for i in 0..BLOCK {
                    if ptr.add(i).read_volatile() != 0 {
                        dirty += 1;
                    }
                }
                let base = ptr as usize;
                for i in 0..BLOCK {
                    ptr.add(i).write_volatile(pattern(base, i));
                }
            }
            arena.dirty_bytes += dirty;
            arena.blocks[arena.count] = Some(ptr);
            arena.count += 1;
            let _ = writeln!(
                out,
                "[arena] blok #{} @ {:#x} ({} kirli bayt)",
                arena.count - 1,
                ptr as usize,
                dirty
            );
        }
        Some(ptr) => {
            // Yer var ama defterimiz dolu: sizdirmadan geri ver.
            sys::munmap(ptr, BLOCK);
            let _ = writeln!(out, "[arena] defter dolu, blok geri verildi.");
        }
        None => {
            arena.failed += 1;
            let _ = writeln!(out, "[arena] mmap basarisiz: pencere doldu.");
        }
    }
}

/// Blok icerigini adresine bagli bir desenle doldurur: iki blok ayni
/// deseni tasimaz, yani bir blok digerinin uzerine yazarsa fark edilir.
fn pattern(base: usize, offset: usize) -> u8 {
    ((base >> 12) as u8) ^ (offset as u8) ^ 0x5A
}

fn main() {
    use core::fmt::Write;
    let mut out = Stdout;

    let mut arena = Arena {
        blocks: [None; MAX_BLOCKS],
        count: 0,
        dirty_bytes: 0,
        corrupt: 0,
        verified: 0,
        failed: 0,
    };

    // Iki blok BASLANGICTA ayrilir: kabuktan `ps` yazan biri, tusa
    // basmadan da `+Nm` sutununda surecin mmap sayfalarini gorsun.
    for _ in 0..2 {
        allocate(&mut arena, &mut out);
    }

    let mut win = match Window::open("arena -- mmap / munmap", 300, 130, 400, 260) {
        Some(w) => w,
        None => return,
    };

    loop {
        match win.poll_key() {
            b'q' => break,
            b'a' => allocate(&mut arena, &mut out),
            b'v' => {
                arena.corrupt = 0;
                for slot in arena.blocks.iter().take(arena.count) {
                    if let Some(ptr) = slot {
                        let base = *ptr as usize;
                        unsafe {
                            for i in 0..BLOCK {
                                if ptr.add(i).read_volatile() != pattern(base, i) {
                                    arena.corrupt += 1;
                                }
                            }
                        }
                    }
                }
                arena.verified += 1;
                let _ = writeln!(
                    out,
                    "[arena] dogrulama: {} blok, {} bozuk bayt.",
                    arena.count, arena.corrupt
                );
            }
            b'f' => {
                if arena.count > 0 {
                    arena.count -= 1;
                    if let Some(ptr) = arena.blocks[arena.count].take() {
                        sys::munmap(ptr, BLOCK);
                        let _ = writeln!(out, "[arena] blok birakildi @ {:#x}", ptr as usize);
                    }
                }
            }
            b'z' => {
                while arena.count > 0 {
                    arena.count -= 1;
                    if let Some(ptr) = arena.blocks[arena.count].take() {
                        sys::munmap(ptr, BLOCK);
                    }
                }
                let _ = writeln!(out, "[arena] hepsi birakildi.");
            }
            _ => {}
        }

        draw(&mut win, &arena);
        win.frame(60);
    }

    // Cikarken temizle -- surec olurken cekirdek zaten birakir, ama
    // bunu uygulamanin yapmasi dogru olan.
    for slot in arena.blocks.iter_mut() {
        if let Some(ptr) = slot.take() {
            sys::munmap(ptr, BLOCK);
        }
    }
    let _ = writeln!(out, "[arena] cikis: {} dogrulama, {} bozuk.", arena.verified, arena.corrupt);
}

fn draw(win: &mut Window, arena: &Arena) {
    let (w, h) = (win.width(), win.height());
    win.clear(BG);

    win.fill(0, 0, w, 22, PANEL);
    win.text(6, 3, "mmap(32 KiB) / munmap", ACCENT);

    win.text(6, 30, "blok:", FG);
    win.number(60, 30, arena.count, ACCENT);
    win.text(96, 30, "/", FG);
    win.number(112, 30, MAX_BLOCKS, FG);
    win.text(170, 30, "toplam:", FG);
    win.number(240, 30, arena.count * BLOCK / 1024, ACCENT);
    win.text(280, 30, "KiB", FG);

    // Blok haritasi: her kare bir blok.
    for i in 0..MAX_BLOCKS {
        let x = 8 + i * 24;
        let filled = i < arena.count;
        win.fill(x, 52, 20, 20, if filled { ACCENT } else { PANEL });
    }

    win.text(6, 84, "sifir gelmeyen bayt:", FG);
    win.number(
        188,
        84,
        arena.dirty_bytes,
        if arena.dirty_bytes == 0 { OK } else { WARN },
    );
    win.text(6, 102, "dogrulama sayisi:", FG);
    win.number(188, 102, arena.verified, FG);
    win.text(6, 120, "bozuk bayt:", FG);
    win.number(
        188,
        120,
        arena.corrupt,
        if arena.corrupt == 0 { OK } else { WARN },
    );
    win.text(6, 138, "reddedilen mmap:", FG);
    win.number(188, 138, arena.failed, if arena.failed == 0 { FG } else { WARN });

    win.text(
        6,
        166,
        if arena.dirty_bytes == 0 {
            "cerceveler sifirlanmis geliyor"
        } else {
            "DIKKAT: sifirlanmamis cerceve!"
        },
        if arena.dirty_bytes == 0 { OK } else { WARN },
    );

    win.text(6, h - 46, "a ayir   v dogrula", FG);
    win.text(6, h - 30, "f sonuncuyu birak   z hepsi", FG);
    win.text(6, h - 14, "q = cik", FG);
}
