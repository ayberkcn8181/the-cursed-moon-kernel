//! Calisma dizini -- **surec basina**.
//!
//! Bugune kadar calisma dizini yalnizca **kabuga** aitti: kabuk yolu
//! cagirmadan once mutlaklastiriyor, uygulama her zaman mutlak yol
//! goruyordu. Ring 3'te `chdir`/`getcwd` karsiligi yoktu, yani bir
//! uygulama "bulundugum dizin" diye bir sey bilemiyordu -- `browse` ve
//! `winfiles` gezdikleri yolu bu yuzden kendi iclerinde tasiyor.
//!
//! POSIX'te calisma dizini surecin bir ozelligidir: `fork` ile
//! **devredilir**, `execve` ile **korunur**, ve goreli yollarin cozumu
//! ona gore yapilir. Uc kural da burada uygulaniyor:
//!
//! | olay | cwd |
//! |---|---|
//! | yeni gorev yuvasi | `/` (bkz. `scheduler::spawn_inner`) |
//! | `fork` | ebeveynden kopyalanir |
//! | `execve` | **degismez** -- yuva ayni kaldigi icin kendiliginden |
//!
//! Ucuncu satir tasarimin sonucu: sifirlama **imaj yuklenirken** degil
//! **yuva ayrilirken** yapiliyor. `execve` yuvayi yeniden kullandigi
//! icin cwd'ye dokunulmuyor; POSIX'in istedigi de bu.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::level0a::core::{scheduler, tcmkfs};

/// Bir yolun en fazla uzunlugu (TCMKFS ile ayni sinir).
pub const PATH_MAX: usize = tcmkfs::PATH_MAX;

static mut PATHS: [[u8; PATH_MAX]; scheduler::MAX_TASKS] =
    [[b'/'; PATH_MAX]; scheduler::MAX_TASKS];
static LENGTHS: [AtomicUsize; scheduler::MAX_TASKS] =
    [const { AtomicUsize::new(1) }; scheduler::MAX_TASKS];

fn slot(task: usize) -> *mut u8 {
    unsafe { (core::ptr::addr_of_mut!(PATHS) as *mut u8).add(task * PATH_MAX) }
}

/// Bir gorevin calisma dizini.
pub fn of(task: usize) -> &'static str {
    if task >= scheduler::MAX_TASKS {
        return "/";
    }
    unsafe {
        let len = LENGTHS[task].load(Ordering::Relaxed).min(PATH_MAX);
        core::str::from_utf8(core::slice::from_raw_parts(slot(task), len)).unwrap_or("/")
    }
}

/// Calisan gorevin calisma dizini.
pub fn current() -> &'static str {
    // Calisma dizini de gruba ait (`CLONE_FS`): bir is parcaciginin
    // `chdir`i kardeslerini de tasir.
    of(scheduler::current_group())
}

/// Bir gorevin calisma dizinini ayarlar.
///
/// Yolun gecerli bir dizin olup olmadigi **burada** sinanmaz; o denetim
/// `kernel_api::chdir`in isidir. Burasi yalnizca depolama.
pub fn set(task: usize, path: &str) -> bool {
    if task >= scheduler::MAX_TASKS || path.len() > PATH_MAX {
        return false;
    }
    crate::arch::cpu::without_interrupts(|| unsafe {
        let base = slot(task);
        for (i, byte) in path.bytes().enumerate() {
            base.add(i).write(byte);
        }
        LENGTHS[task].store(path.len(), Ordering::Relaxed);
    });
    true
}

/// `fork`: cocuk ebeveynin dizinini devralir.
pub fn clone_into(child: usize) {
    if child >= scheduler::MAX_TASKS {
        return;
    }
    let parent = current();
    set(child, parent);
}

/// Yeni bir gorev yuvasi kokten baslar.
///
/// Yuva geri kazanildigi icin sart: temizlenmeseydi yeni bir surec, ayni
/// yuvada calismis oncekinin dizininde acilirdi.
pub fn reset(task: usize) {
    set(task, "/");
}

/// Goreli yolu `base`e gore mutlaklastirir **ve sadelestirir**:
/// `.` atilir, `..` bir onceki bileseni siler.
///
/// ## Sadelestirme neden gerekli
///
/// `tcmkfs::resolve` `.`/`..` bilesenlerini zaten anliyor -- ama VFS'in
/// oteki ucu, cekirdek imajina gomulu **RAMFS**, duz bir isim tablosudur
/// ve yolu birebir karsilastirir. Yani `/./bin/hello` TCMKFS'te
/// calisirken RAMFS'te bulunamazdi. Yolu tek bir yerde sadelestirmek iki
/// dosya sistemini de ayni girdiyle besliyor.
///
/// Kabuk da, `kernel_api` de bu fonksiyonu kullanir; ikisinin ayri
/// kopyalari olsaydi "kabuktan calisan" ile "uygulamadan calisan" yol
/// ayrisabilirdi.
pub fn normalize<'a>(base: &str, path: &str, buf: &'a mut [u8; PATH_MAX]) -> Option<&'a str> {
    // Mutlak yol verildiyse taban hic karismaz.
    let base = if path.starts_with('/') { "" } else { base };

    buf[0] = b'/';
    let mut len = 1usize;
    // Her bilesenin **basladigi** uzunluk; `..` buraya geri sarar.
    let mut starts = [0usize; tcmkfs::MAX_DEPTH + 2];
    let mut depth = 0usize;

    for part in base.split('/').chain(path.split('/')) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            // Kokun ustune cikilmaz -- POSIX'te de `/..` koktur.
            if depth > 0 {
                depth -= 1;
                len = starts[depth];
            }
            continue;
        }
        if depth + 1 >= starts.len() {
            return None; // cok derin
        }
        starts[depth] = len;
        depth += 1;

        // Ilk bilesen zaten bastaki egik cizginin ardina gelir.
        if len > 1 {
            if len >= buf.len() {
                return None;
            }
            buf[len] = b'/';
            len += 1;
        }
        if len + part.len() >= buf.len() {
            return None;
        }
        buf[len..len + part.len()].copy_from_slice(part.as_bytes());
        len += part.len();
    }

    core::str::from_utf8(&buf[..len]).ok()
}

/// Calisan surecin dizinine gore mutlaklastirir.
pub fn resolve<'a>(path: &str, buf: &'a mut [u8; PATH_MAX]) -> Option<&'a str> {
    normalize(current(), path, buf)
}
