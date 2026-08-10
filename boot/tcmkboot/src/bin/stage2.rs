//! TCMK onyukleyici -- **2. asama**.
//!
//! 1. asama tarafindan 0x0000:0x8000'e yuklenir. Gorevleri:
//!
//!   1. VBE ile 1024x768x32 lineer framebuffer kurmak (BIOS gerektirir,
//!      bu yuzden gercek modda yapilir),
//!   2. cekirdegin bekledigi **Multiboot1 bilgi yapisini** kurmak,
//!   3. korumali moda gecip cekirdek imajini diskten 1 MiB'a okumak,
//!   4. `.bss`'i sifirlayip cekirdege atlamak.
//!
//! ## Neden cekirdek imaji "blob"
//!
//! Kurulum sirasinda ELF **coz ulmus** halde diske yazilir: tum PT_LOAD
//! segmentleri bellekteki yerlerine gore ardisik tek bir bloga yerlestirilir
//! (aradaki bosluklar sifirlanir). Boylece 2. asamanin ELF ayristirmasina
//! ihtiyaci kalmaz -- "su LBA'dan su kadar sektoru 0x100000'e oku" yeter.
//! Ayristirma isini zaten ELF yukleyicisi olan cekirdek yapar (kurulum
//! aninda, bkz. `level0a::installer`).
//!
//! ## Neden diski BIOS yerine ATA PIO ile okuyoruz
//!
//! Cekirdek imaji 1 MiB'i asiyor; BIOS int 0x13 ise gercek mod
//! segment:ofset ile yalnizca ilk 1 MiB'a yazabilir. Klasik cozum "unreal
//! mode" ile 32-bit adreslemedir; ama korumali moda bastan gecip diski
//! dogrudan ATA PIO ile okumak hem daha kisa hem de cekirdegin zaten
//! kullandigi ayni port dizisi (bkz. `drivers/ata.rs`). Bedeli: 2. asama
//! IDE disk varsayar.

#![no_std]
#![no_main]

use core::arch::global_asm;

global_asm!(
    r#"
.section .text.boot
.code16
.global _start

_start:
    jmp     short start
    nop

/* --- Kurulum sirasinda yamalanan parametre bloku (ofset 3) --- */
.global boot_params
boot_params:
    .ascii  "TCMKB2"        /* ofset  3: imza                          */
blob_lba:
    .long   0               /* ofset  9: cekirdek blogunun mutlak LBA  */
blob_sectors:
    .long   0               /* ofset 13: kac sektor                    */
load_addr:
    .long   0x00100000      /* ofset 17: bellege nereye                */
kernel_entry:
    .long   0               /* ofset 21: giris noktasi                 */
bss_start:
    .long   0               /* ofset 25: sifirlanacak alanin basi      */
bss_end:
    .long   0               /* ofset 29: sifirlanacak alanin sonu      */

start:
    cli
    xor     ax, ax
    mov     ds, ax
    mov     es, ax
    mov     ss, ax
    mov     sp, 0x7c00
    sti

    mov     si, offset msg_stage2
1:  lodsb
    test    al, al
    jz      2f
    mov     ah, 0x0e
    mov     bx, 7
    int     0x10
    jmp     1b
2:

    /* --- A20 kapisi (hizli yol: port 0x92 bit 1) ---
       A20 kapaliyken 1 MiB uzeri adresler 0'a sarar; cekirdegi
       0x100000'e yazmak imkansiz olurdu. */
    in      al, 0x92
    or      al, 2
    and     al, 0xFE        /* bit 0 = fast reset; kazara set etmeyelim */
    out     0x92, al

    /* --- VBE: 1024x768x32 lineer framebuffer --- */
    mov     ax, 0x4f00
    mov     di, 0x0600      /* VbeInfoBlock */
    mov     dword ptr [di], 0x32454256   /* "VBE2" */
    int     0x10
    cmp     ax, 0x004f
    jne     no_vbe

    /* Mod listesi isaretcisi: VbeInfoBlock+14 (seg:ofs) */
    mov     si, [0x0600 + 14]
    mov     ax, [0x0600 + 16]
    mov     fs, ax

next_mode:
    mov     cx, fs:[si]
    add     si, 2
    cmp     cx, 0xffff
    je      no_vbe

    push    si
    mov     ax, 0x4f01
    mov     di, 0x0500      /* ModeInfoBlock */
    int     0x10
    pop     si
    cmp     ax, 0x004f
    jne     next_mode

    /* attributes bit 7 = lineer framebuffer destegi */
    test    byte ptr [0x0500 + 0], 0x80
    jz      next_mode
    cmp     word ptr [0x0500 + 18], 1024    /* XResolution */
    jne     next_mode
    cmp     word ptr [0x0500 + 20], 768     /* YResolution */
    jne     next_mode
    cmp     byte ptr [0x0500 + 25], 32      /* BitsPerPixel */
    jne     next_mode

    /* Modu kur (0x4000 = lineer framebuffer kullan) */
    mov     bx, cx
    or      bx, 0x4000
    mov     ax, 0x4f02
    int     0x10
    cmp     ax, 0x004f
    jne     no_vbe

    /* --- Multiboot1 bilgi yapisi (0x5000) ---
       Cekirdek yalnizca flags bit 12'yi ve framebuffer alanlarini okur
       (bkz. boot/multiboot.rs). Diger alanlar sifir birakilir. */
    mov     dword ptr [0x5000 + 0], 0x1000      /* flags: bit 12 */

    mov     eax, [0x0500 + 40]                  /* PhysBasePtr */
    mov     [0x5000 + 88], eax
    mov     dword ptr [0x5000 + 92], 0          /* adresin ust yarisi */

    movzx   eax, word ptr [0x0500 + 16]         /* BytesPerScanLine */
    mov     [0x5000 + 96], eax
    mov     dword ptr [0x5000 + 100], 1024
    mov     dword ptr [0x5000 + 104], 768
    mov     byte ptr [0x5000 + 108], 32
    mov     byte ptr [0x5000 + 109], 1          /* tip 1 = dogrudan RGB */
    jmp     to_pm

no_vbe:
    /* Framebuffer kurulamadi: cekirdek metin moduna duser (flags=0). */
    mov     dword ptr [0x5000 + 0], 0

to_pm:
    cli
    lgdt    [gdt_desc]
    mov     eax, cr0
    or      eax, 1
    mov     cr0, eax
    /* ljmp 0x08:pm_entry -- 16-bit moddan 32-bit segmente atlamak icin
       operand-boyu oneki (0x66) sart; LLVM'in sozdizimine guvenmemek
       icin ham kodlanir. */
    .byte   0x66, 0xEA
    .long   pm_entry
    .word   0x08

/* ------------------------------------------------------------------ */
.code32
pm_entry:
    mov     ax, 0x10
    mov     ds, ax
    mov     es, ax
    mov     ss, ax
    mov     fs, ax
    mov     gs, ax
    mov     esp, 0x7000

    /* Cekirdek blogunu diskten oku. */
    mov     edi, [load_addr]
    mov     esi, [blob_lba]
    mov     ecx, [blob_sectors]

read_loop:
    test    ecx, ecx
    jz      read_done
    mov     eax, ecx
    cmp     eax, 64
    jbe     1f
    mov     eax, 64         /* komut basina en fazla 64 sektor */
1:
    push    ecx
    push    eax             /* bu turda okunacak sektor sayisi */
    call    ata_read
    pop     eax
    pop     ecx
    sub     ecx, eax
    add     esi, eax
    jmp     read_loop
read_done:

    /* .bss'i sifirla (memsz > filesz olan segmentin kalani). */
    mov     edi, [bss_start]
    mov     ecx, [bss_end]
    sub     ecx, edi
    jbe     bss_done
    xor     eax, eax
    shr     ecx, 2
    rep     stosd
bss_done:

    /* Cekirdege gec: Multiboot1 sozlesmesi EAX=magic, EBX=info. */
    mov     eax, 0x2BADB002
    mov     ebx, 0x5000
    mov     ecx, [kernel_entry]
    jmp     ecx

/* ATA PIO okuma (birincil master, LBA28).
   Giris: esi = LBA, edi = hedef, yigin[4] = sektor sayisi (<=64).
   Cikis: edi ilerletilmis. */
ata_read:
    push    eax
    push    ecx
    push    edx
    mov     eax, [esp + 16]     /* sektor sayisi (3 push + donus adresi) */
    push    eax                 /* dongu sayaci olarak sakla */

    /* BSY dususunu bekle */
    mov     dx, 0x1F7
1:  in      al, dx
    test    al, 0x80
    jnz     1b

    /* Surucu/LBA[27:24] */
    mov     dx, 0x1F6
    mov     eax, esi
    shr     eax, 24
    and     al, 0x0F
    or      al, 0xE0
    out     dx, al

    mov     dx, 0x1F1
    xor     al, al
    out     dx, al

    mov     dx, 0x1F2
    mov     eax, [esp]
    out     dx, al

    mov     dx, 0x1F3
    mov     eax, esi
    out     dx, al
    mov     dx, 0x1F4
    mov     eax, esi
    shr     eax, 8
    out     dx, al
    mov     dx, 0x1F5
    mov     eax, esi
    shr     eax, 16
    out     dx, al

    mov     dx, 0x1F7
    mov     al, 0x20            /* READ SECTORS */
    out     dx, al

next_sector:
    mov     dx, 0x1F7
2:  in      al, dx
    test    al, 0x80            /* BSY */
    jnz     2b
    test    al, 0x08            /* DRQ */
    jz      2b

    mov     dx, 0x1F0
    mov     ecx, 256
    rep     insw                /* es:edi <- veri portu */

    dec     dword ptr [esp]
    jnz     next_sector

    pop     eax
    pop     edx
    pop     ecx
    pop     eax
    ret

/* --- GDT: duz 4 GiB kod/veri --- */
.align 8
gdt:
    .quad   0x0000000000000000  /* null                       */
    .quad   0x00CF9A000000FFFF  /* 0x08: kod, taban 0, 4 GiB  */
    .quad   0x00CF92000000FFFF  /* 0x10: veri, taban 0, 4 GiB */
gdt_end:

gdt_desc:
    .word   gdt_end - gdt - 1
    .long   gdt

.code16
msg_stage2:
    .asciz  " yukleniyor\r\n"
"#
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
