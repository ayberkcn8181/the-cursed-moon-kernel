//! TCMK onyukleyici -- **1. asama** (MBR acilis sektoru).
//!
//! BIOS bu sektorun ilk 446 baytini 0x0000:0x7C00'e yukler ve DL'de acilis
//! surucusunu vererek buraya atlar. Bolum tablosu (446..510) ve imza
//! (510..512) kurulum sirasinda **korunur** -- yani bu ikili 446 bayti
//! gecemez (bkz. `stage1.ld` icindeki ASSERT).
//!
//! Gorevi tek sey: 2. asamayi diskten okuyup ona atlamak. LBA ve sektor
//! sayisi kurulum aninda `boot_params` blokuna yamalanir (bkz.
//! `level0a::installer`), boylece onyukleyici diskin duzenine gomulu
//! degildir.
//!
//! BIOS'un "extended read" cagrisi (int 0x13, AH=0x42) kullanilir: CHS
//! aritmetigi gerektirmez ve 1996'dan beri her BIOS'ta vardir.
//!
//! ## 16-bit assembly tuzaklari (LLVM)
//!
//! LLVM'in Intel sozdiziminde **ciplak sembol bir BELLEK operandidir**:
//! `mov si, msg` sembolun ADRESINI degil, o adresteki degeri yukler.
//! Adres icin `offset` sart. (Ayni tuzak cekirdegin `_start`'inda
//! `mov esp, stack_top` olarak yasandi.)
//!
//! Ayrica `.code16` icinde `call`/`ret`/`retf` LLVM tarafindan **32-bit**
//! olarak uretilir (0x66 onekiyle); gercek mod yigininda bu 4 baytlik
//! itme/cekme demektir ve donus adresi bozulur. LLVM'in Intel sozdizimi
//! `callw`/`retw`'yi de kabul etmiyor -- bu yuzden altyordam hic yok:
//! yazdirma dongusu iki yere de satir ici acildi ve uzak atlama ham
//! baytla kodlandi. 446 baytlik butcede bu birkac bayt sorun degil.

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
    .ascii  "TCMKB1"        /* ofset  3: imza                      */
stage2_lba:
    .long   0               /* ofset  9: 2. asamanin mutlak LBA'si */
stage2_count:
    .word   0               /* ofset 13: kac sektor okunacak       */

start:
    cli
    xor     ax, ax
    mov     ds, ax
    mov     es, ax
    mov     ss, ax
    mov     sp, 0x7c00      /* yigin kodun hemen altinda buyur */
    sti

    mov     [drive], dl     /* BIOS acilis surucusunu DL'de verir */

    mov     si, offset msg_boot
1:  lodsb
    test    al, al
    jz      2f
    mov     ah, 0x0e
    mov     bx, 7
    int     0x10
    jmp     1b
2:

    /* Disk Address Packet'i doldur ve 2. asamayi 0x0000:0x8000'e oku. */
    mov     ax, [stage2_count]
    mov     [dap_count], ax
    mov     eax, [stage2_lba]
    mov     [dap_lba], eax

    mov     si, offset dap
    mov     dl, [drive]
    mov     ah, 0x42
    int     0x13
    jc      disk_error

    /* DL korunur: 2. asama da ayni surucuden okuyacak. */
    mov     dl, [drive]
    /* ljmp 0x0000:0x8000 -- ham kodlama (LLVM'in far-jump sozdizimine
       guvenmemek icin). EA = jmp ptr16:16, once ofset sonra segment. */
    .byte   0xEA
    .word   0x8000
    .word   0x0000

disk_error:
    mov     si, offset msg_err
3:  lodsb
    test    al, al
    jz      halt
    mov     ah, 0x0e
    mov     bx, 7
    int     0x10
    jmp     3b
halt:
    hlt
    jmp     halt

/* --- veri --- */
drive:
    .byte   0

dap:
    .byte   0x10            /* paket boyu    */
    .byte   0
dap_count:
    .word   0               /* sektor sayisi */
    .word   0x8000          /* hedef ofset   */
    .word   0x0000          /* hedef segment */
dap_lba:
    .quad   0               /* baslangic LBA */

msg_boot:
    .asciz  "TCMK boot..."
msg_err:
    .asciz  " disk hatasi"
"#
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
