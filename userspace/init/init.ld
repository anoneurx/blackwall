/* Flat binary linker script for the init process.
 * Base address: 0x400000 — well below kernel space, never conflicts.
 * The result is a simple static executable with no relocations.
 */
ENTRY(_start)

SECTIONS {
    . = 0x400000;

    .text : {
        *(.text .text.*)
    }

    . = ALIGN(4096);

    .rodata : {
        *(.rodata .rodata.*)
    }

    . = ALIGN(4096);

    .data : {
        *(.data .data.*)
    }

    . = ALIGN(4096);

    .bss : {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note*)
        *(.comment*)
        *(.dynamic*)
        *(.dynsym*)
        *(.dynstr*)
        *(.gnu.hash*)
        *(.hash*)
        *(.relro_padding*)
    }
}
