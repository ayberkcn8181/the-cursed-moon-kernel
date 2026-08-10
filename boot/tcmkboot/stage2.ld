/* 2. asama: 1. asama tarafindan 0x0000:0x8000'e yuklenir. Duz ikili. */
ENTRY(_start)

SECTIONS
{
    . = 0x8000;

    .text : {
        KEEP(*(.text.boot))
        *(.text*)
        *(.rodata*)
        *(.data*)
    }

    _stage2_end = .;
    /* Kurulum 2. asama icin 32 sektor (16 KiB) ayirir. */
    ASSERT(_stage2_end - 0x8000 <= 16384, "2. asama 16 KiB'i asti")

    /DISCARD/ : {
        *(.eh_frame*)
        *(.comment)
        *(.note*)
        *(.bss*)
    }
}
