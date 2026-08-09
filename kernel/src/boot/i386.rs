//! Multiboot1 basligi ve `_start` giris noktasi.
//!
//! NASM'in yerini alan tek Rust-native parca: `global_asm!` ile gomulu.
//! GRUB, korumali modda (paging kapali, A20 acik, kesmeler kapali) EAX'e
//! multiboot magic (0x2BADB002), EBX'e multiboot info adresini koyup buraya
//! zipliyor.

use core::arch::global_asm;

global_asm!(
    r#"
.section .multiboot_header, "a"
.align 4
multiboot_header:
    .long 0x1BADB002
    .long 0x00000003
    .long -(0x1BADB002 + 0x00000003)

.section .bss
.align 16
stack_bottom:
    .skip 16384
stack_top:

.section .text
.global _start
.type _start, @function
_start:
    /* DIKKAT: "mov esp, stack_top" GAS/LLVM Intel-syntax'ta bir BELLEK
       DEREFERANSI olarak assemble edilir (stack_top ADRESINDEKI degeri
       yukler, adresin KENDISINI degil) -- bu, esp'yi rastgele/ROM-shadow
       bolgesine (0xFFFFFFxx) isaret eder ve ilk push/pop'ta sessizce
       her seyi bozar. Adresi almak icin `lea` kullanilmali. */
    lea esp, [stack_top]
    push ebx
    push eax
    call kernel_main
    cli
.hang:
    hlt
    jmp .hang
"#
);
