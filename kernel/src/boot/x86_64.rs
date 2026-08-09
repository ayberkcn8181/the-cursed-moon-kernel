//! Multiboot2 basligi + Long Mode gecisi (doc S.15: x86_64 -> Multiboot 2).
//!
//! GRUB, ELF64 imajini yukler ama CPU'yu **32-bit korumali modda** birakir
//! (paging kapali, EAX=0x36d76289, EBX=multiboot2 info). Long Mode'a gecis
//! bize aittir ve sirasi katidir:
//!
//!   1. Gecici sayfa tablolarini kur (PML4 -> PDPT -> PD, 2 MiB'lik sayfalar)
//!   2. CR3 = PML4
//!   3. CR4.PAE = 1                (PAE olmadan Long Mode yok)
//!   4. EFER.LME = 1               (MSR 0xC0000080)
//!   5. CR0.PG = 1                 (bu anda CPU Long Mode'a girer)
//!   6. 64-bit GDT yukle ve uzak atlama ile CS'i 64-bit koda cevir
//!
//! Ilk 1 GiB **2 MiB'lik sayfalarla** birebir eslenir; bu hem cekirdegi
//! (1 MiB), hem heap'i (2 MiB), hem kullanici bolgesini (3 MiB), hem de
//! VGA'yi (0xB8000) kapsar. LAPIC (0xFEE00000) icin ayri bir 2 MiB girdi
//! kullanilir (doc S.7 Faz 4).

use core::arch::global_asm;

global_asm!(
    r#"
/* ---------------- Multiboot2 basligi ---------------- */
.section .multiboot_header, "a"
.align 8
mb2_start:
    .long 0xE85250D6                                  /* magic            */
    .long 0                                           /* mimari: i386     */
    .long mb2_end - mb2_start                         /* baslik uzunlugu  */
    .long -(0xE85250D6 + 0 + (mb2_end - mb2_start))   /* checksum         */

    /* Framebuffer etiketi (tip 5): GUI icin lineer framebuffer iste. */
    .align 8
    .short 5
    .short 0
    .long 20
    .long 1024      /* genislik  */
    .long 768       /* yukseklik */
    .long 32        /* bit/piksel */

    /* bitis etiketi (tip 0, boyut 8) */
    .align 8
    .short 0
    .short 0
    .long 8
mb2_end:

/* ---------------- 32-bit giris ---------------- */
.section .text.boot, "ax"
.code32
.global _start
.type _start, @function
_start:
    /* GRUB'un verdigi degerleri sakla: EAX=magic, EBX=info adresi */
    mov edi, eax
    mov esi, ebx

    lea esp, [boot_stack_top]

    /* --- Sayfa tablolarini kur --- */
    /* PML4[0] -> PDPT   (Present | Writable | User) */
    lea eax, [pdpt_table]
    or eax, 0b111
    mov [pml4_table], eax
    mov dword ptr [pml4_table + 4], 0

    /* PDPT[0] -> PD  (ilk 1 GiB) */
    lea eax, [pd_table]
    or eax, 0b111
    mov [pdpt_table], eax
    mov dword ptr [pdpt_table + 4], 0

    /* PDPT[3] -> PD_LAPIC  (0xC0000000-0xFFFFFFFF araligi; LAPIC burada) */
    lea eax, [pd_lapic_table]
    or eax, 0b011
    mov [pdpt_table + 24], eax
    mov dword ptr [pdpt_table + 28], 0

    /* PD: 512 adet 2 MiB'lik birebir esleme -> ilk 1 GiB
       DIKKAT: User biti BURADA VERILMEZ. Verilseydi cekirdegin tamami
       (1 MiB'deki kod, 2 MiB'deki heap) Ring 3'e acik olurdu. Kullanici
       bolgesi sonradan `mmu::protect_user_range` tarafindan, ilgili 2 MiB
       girdisi 4 KiB'lik bir tabloya bolunerek sayfa sayfa acilir. */
    xor ecx, ecx
2:
    mov eax, 0x200000       /* 2 MiB */
    mul ecx                 /* edx:eax = ecx * 2 MiB */
    or eax, 0b10000011      /* Present | Writable | PageSize(2MiB) */
    mov [pd_table + ecx * 8], eax
    mov [pd_table + ecx * 8 + 4], edx
    inc ecx
    cmp ecx, 512
    jne 2b

    /* PD_LAPIC: 0xC0000000-0xFFFFFFFF araliginin TAMAMINI 2 MiB'lik
       sayfalarla esle. Bu aralikta hem LAPIC (0xFEE00000) hem de QEMU'nun
       framebuffer'i (~0xFD000000) bulunur; tek tek esleyip adres
       tahmininde bulunmaktansa 1 GiB'i birden acmak hem basit hem
       dayanikli. */
    xor ecx, ecx
3:
    mov eax, 0x200000
    mul ecx
    add eax, 0xC0000000
    adc edx, 0
    or eax, 0b10000011      /* Present | Writable | PageSize */
    mov [pd_lapic_table + ecx * 8], eax
    mov [pd_lapic_table + ecx * 8 + 4], edx
    inc ecx
    cmp ecx, 512
    jne 3b

    /* --- CR3 = PML4 --- */
    lea eax, [pml4_table]
    mov cr3, eax

    /* --- CR4.PAE = 1 --- */
    mov eax, cr4
    or eax, 1 << 5
    mov cr4, eax

    /* --- EFER.LME = 1 (MSR 0xC0000080) --- */
    mov ecx, 0xC0000080
    rdmsr
    or eax, 1 << 8
    wrmsr

    /* --- CR0.PG = 1 (ve PE zaten acik) --- */
    mov eax, cr0
    or eax, 1 << 31
    mov cr0, eax

    /* --- 64-bit GDT ve uzak atlama --- */
    lgdt [gdt64_pointer]
    jmp 0x08, offset long_mode_entry

/* ---------------- 64-bit giris ---------------- */
.code64
long_mode_entry:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax

    lea rsp, [boot_stack_top]

    /* GRUB degerlerini System V AMD64 ABI'sine gore aktar:
       rdi = magic, rsi = multiboot info adresi. edi/esi zaten dolu;
       ust 32 bit'i temizlemek icin 32-bit mov yeterlidir. */
    mov edi, edi
    mov esi, esi
    call kernel_main

    cli
3:
    hlt
    jmp 3b

/* ---------------- Veri ---------------- */
.section .rodata
.align 16
gdt64:
    .quad 0                             /* null                          */
    .quad 0x00AF9A000000FFFF            /* 0x08: 64-bit kod (L=1)        */
    .quad 0x00CF92000000FFFF            /* 0x10: veri                    */
gdt64_pointer:
    .short gdt64_pointer - gdt64 - 1
    .quad gdt64

.section .bss
.align 4096
pml4_table:
    .skip 4096
pdpt_table:
    .skip 4096
pd_table:
    .skip 4096
pd_lapic_table:
    .skip 4096
boot_stack_bottom:
    .skip 32768
boot_stack_top:
"#
);
