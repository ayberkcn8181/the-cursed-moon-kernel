/* TCMK i386 linker script — bkz. mimari dokumaninin 5. bolumu (Bellek Haritasi). */
ENTRY(_start)
OUTPUT_FORMAT(elf32-i386)

SECTIONS
{
    /* Cekirdek 1 MiB'e link edilir (GRUB Multiboot1 yukleme adresi). */
    . = 1M;

    .boot :
    {
        /* Multiboot1 basligi dosyanin ilk 8 KiB'i icinde olmali.
           KEEP: --gc-sections bu bolumu hicbir sey referans vermedigi icin
           (sadece GRUB bayt taramasiyla bulur) atmasin. */
        KEEP(*(.multiboot_header))
    }

    .text : ALIGN(4K)
    {
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
