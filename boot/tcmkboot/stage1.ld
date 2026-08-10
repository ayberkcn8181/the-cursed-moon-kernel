/* 1. asama: BIOS 0x0000:0x7C00'e yukler. Duz ikili uretilir. */
ENTRY(_start)

SECTIONS
{
    . = 0x7C00;

    .text : {
        KEEP(*(.text.boot))
        *(.text*)
        *(.rodata*)
        *(.data*)
    }

    /* MBR'de bolum tablosu 446. bayttan basladigi icin kod+veri
       toplami 446 bayti gecemez; asagidaki kontrol bunu derleme
       zamaninda yakalar. */
    _stage1_end = .;
    ASSERT(_stage1_end - 0x7C00 <= 446,
           "1. asama 446 bayti asti (MBR bolum tablosu ezilirdi)")

    /DISCARD/ : {
        *(.eh_frame*)
        *(.comment)
        *(.note*)
        *(.bss*)
    }
}
