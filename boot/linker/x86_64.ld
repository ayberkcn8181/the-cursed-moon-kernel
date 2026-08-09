/* TCMK x86_64 linker script.
 *
 * Cekirdek yine 1 MiB'e link edilir (doc S.5). GRUB, Multiboot2 ile ELF64
 * imajini yukler ve 32-bit korumali modda `_start`'a girer; Long Mode'a
 * gecis cekirdegin kendi boot stub'inda yapilir.
 */
ENTRY(_start)
OUTPUT_FORMAT(elf64-x86-64)

SECTIONS
{
    . = 1M;

    .boot :
    {
        /* Multiboot2 basligi imajin ilk 32 KiB'inde ve 8 bayt hizali olmali. */
        KEEP(*(.multiboot_header))
    }

    .text : ALIGN(4K)
    {
        *(.text.boot)       /* 32-bit giris kodu once gelsin */
        *(.text .text.*)
    }

    .rodata : ALIGN(4K)
    {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K)
    {
        *(.data .data.*)
    }

    .bss : ALIGN(4K)
    {
        *(.bss .bss.*)
        *(COMMON)
    }

    /DISCARD/ :
    {
        *(.comment)
        *(.eh_frame)
        *(.note.*)
    }
}
