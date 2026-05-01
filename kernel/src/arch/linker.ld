/* ═══════════════════════════════════════════════════════════════════════════
 * AetherOS — Linker Script for RISC-V 64-bit (QEMU virt machine)
 *
 * Memory map (QEMU virt):
 *   0x0000_0000 – 0x0000_FFFF  Boot-ROM (QEMU jump-trampoline, read-only)
 *   0x0200_0000                CLINT base (timer / software interrupts)
 *   0x0C00_0000                PLIC base  (external interrupts)
 *   0x1000_0000                UART0 (NS16550A)
 *   0x8000_0000 – 0x8800_0000  RAM (128 MiB, configurable via -m)
 *
 * With `-bios none -kernel <elf>`, QEMU loads the ELF and jumps to the
 * entry-point (which our linker script sets to `_start`).
 * ═══════════════════════════════════════════════════════════════════════════ */

OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY {
    /* 128 MiB RAM, starting at the QEMU virt machine's RAM base */
    RAM : ORIGIN = 0x80000000, LENGTH = 128M
}

SECTIONS {
    /* Kernel is linked to run at 0x8000_0000 */
    . = ORIGIN(RAM);

    /* ── .text ─────────────────────────────────────────────────────────── */
    .text : ALIGN(4K) {
        /* _start *must* land at the very first byte – QEMU jumps here */
        KEEP(*(.text.start))
        /* Trap vector is 4-byte aligned per RISC-V spec (mtvec[1:0] = 0) */
        KEEP(*(.text.trap))
        *(.text .text.*)
    } > RAM

    /* ── .rodata ────────────────────────────────────────────────────────── */
    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    } > RAM

    /* ── .data ──────────────────────────────────────────────────────────── */
    .data : ALIGN(4K) {
        _data_start = .;
        *(.data .data.*)
        *(.sdata .sdata.*)
        _data_end = .;
    } > RAM

    /* ── .bss ───────────────────────────────────────────────────────────── */
    /* Cleared to zero by _start before any Rust code runs */
    .bss : ALIGN(8) {
        _bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        . = ALIGN(8);
        _bss_end = .;
    } > RAM

    /* ── Kernel stack ───────────────────────────────────────────────────── */
    /* 64 KiB; grows downward from _stack_top toward _stack_bottom.
     * A guard page (unmapped) immediately before _stack_bottom is a TODO
     * once we have an MMU / PMP configured.                                */
    . = ALIGN(16);
    _stack_bottom = .;
    . += 0x10000;   /* 64 KiB */
    _stack_top = .;

    /* Convenience symbol: end of the static kernel image */
    _kernel_end = .;

    /* Discard DWARF unwind tables – not needed without a C++ runtime */
    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
    }
}
