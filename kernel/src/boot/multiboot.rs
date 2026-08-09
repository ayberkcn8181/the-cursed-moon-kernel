//! Bootloader'in birakti gi bilgi yapisini ayristirir.
//!
//! Iki farkli format vardir ve tek ihtiyacimiz olan sey ikisinde de ayni:
//! **lineer framebuffer'in adresi ve geometrisi** (GUI'nin temeli).
//!
//!   i386   -> Multiboot1 info yapisi, flags bit 12 framebuffer alanlarini
//!             gecerli kilar.
//!   x86_64 -> Multiboot2 etiket listesi, tip 8 = framebuffer.

#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    pub addr: usize,
    // bpp yalnizca dogrulama sirasinda (32 mi?) kontrol edilir.
    pub pitch: usize,
    pub width: usize,
    pub height: usize,
    #[allow(dead_code)]
    pub bpp: u8,
}

/// Bootloader bilgisinden framebuffer tanimini cikarir.
///
/// # Safety
/// `info_addr` bootloader'in verdigi gecerli adres olmalidir.
#[cfg(target_arch = "x86")]
pub unsafe fn framebuffer(info_addr: usize) -> Option<FramebufferInfo> {
    if info_addr == 0 {
        return None;
    }

    let flags = (info_addr as *const u32).read();
    // bit 12: framebuffer alanlari gecerli
    if flags & (1 << 12) == 0 {
        return None;
    }

    let base = info_addr as *const u8;
    let fb_addr = (base.add(88) as *const u64).read_unaligned() as usize;
    let pitch = (base.add(96) as *const u32).read_unaligned() as usize;
    let width = (base.add(100) as *const u32).read_unaligned() as usize;
    let height = (base.add(104) as *const u32).read_unaligned() as usize;
    let bpp = base.add(108).read();
    let fb_type = base.add(109).read();

    // tip 1 = dogrudan RGB. Metin modu (tip 2) GUI icin kullanilamaz.
    if fb_type != 1 || bpp != 32 || fb_addr == 0 {
        return None;
    }

    Some(FramebufferInfo {
        addr: fb_addr,
        pitch,
        width,
        height,
        bpp,
    })
}

#[cfg(target_arch = "x86_64")]
pub unsafe fn framebuffer(info_addr: usize) -> Option<FramebufferInfo> {
    if info_addr == 0 {
        return None;
    }

    let total_size = (info_addr as *const u32).read() as usize;
    let mut offset = 8usize; // total_size + reserved

    while offset + 8 <= total_size {
        let tag = (info_addr + offset) as *const u8;
        let tag_type = (tag as *const u32).read_unaligned();
        let tag_size = (tag.add(4) as *const u32).read_unaligned() as usize;

        if tag_type == 0 {
            break; // bitis etiketi
        }

        if tag_type == 8 && tag_size >= 32 {
            let fb_addr = (tag.add(8) as *const u64).read_unaligned() as usize;
            let pitch = (tag.add(16) as *const u32).read_unaligned() as usize;
            let width = (tag.add(20) as *const u32).read_unaligned() as usize;
            let height = (tag.add(24) as *const u32).read_unaligned() as usize;
            let bpp = tag.add(28).read();
            let fb_type = tag.add(29).read();

            if fb_type == 1 && bpp == 32 && fb_addr != 0 {
                return Some(FramebufferInfo {
                    addr: fb_addr,
                    pitch,
                    width,
                    height,
                    bpp,
                });
            }
        }

        // Etiketler 8 bayt hizali ilerler.
        offset += (tag_size + 7) & !7;
    }

    None
}
