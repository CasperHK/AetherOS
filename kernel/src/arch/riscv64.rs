//! # RISC-V 64-bit Architecture Implementation
//!
//! Implements the kernel HAL for `riscv64gc-unknown-none-elf` targeting the
//! QEMU `virt` machine.  All peripheral base addresses are hard-coded for
//! the QEMU virtual board; a production port would read these from a
//! device-tree blob or board-specific configuration.
//!
//! ## Peripheral map (QEMU virt)
//! | Peripheral | Base address  | Description                        |
//! |------------|---------------|------------------------------------|
//! | CLINT      | `0x0200_0000` | Core-Local Interruptor (timer/swi)  |
//! | PLIC       | `0x0C00_0000` | Platform-Level Interrupt Controller |
//! | UART0      | `0x1000_0000` | NS16550A-compatible UART            |
//! | RAM        | `0x8000_0000` | 128 MiB system RAM                 |
//!
//! ## Interrupt / exception flow
//! `_trap_entry` (assembly) → saves all registers → calls `trap_dispatch`
//! (Rust) → restores registers → `mret`.

use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

// ── Hardware base addresses ───────────────────────────────────────────────────

/// NS16550A UART: transmit / receive data register
const UART0_BASE: usize = 0x1000_0000;
const UART_THR: usize   = UART0_BASE;      // Transmit Holding Register (W)
const UART_RBR: usize   = UART0_BASE;      // Receive Buffer Register   (R)
const UART_LSR: usize   = UART0_BASE + 5;  // Line Status Register

/// CLINT (Core Local Interruptor)
const CLINT_BASE: usize         = 0x0200_0000;
const CLINT_MTIME: usize        = CLINT_BASE + 0xBFF8;  // 64-bit RO timer
const CLINT_MTIMECMP0: usize    = CLINT_BASE + 0x4000;  // 64-bit R/W compare

/// Timer interrupt frequency: QEMU virt CLINT ticks at 10 MHz.
pub const TIMER_FREQ_HZ: u64 = 10_000_000;

/// Scheduler tick rate: 100 Hz → 10 ms quantum.
pub const SCHEDULER_HZ: u64 = 100;

/// Ticks between scheduler quanta.
pub const TICKS_PER_QUANTUM: u64 = TIMER_FREQ_HZ / SCHEDULER_HZ;

// ── Kernel tick counter ───────────────────────────────────────────────────────

/// Monotonic tick count incremented by the timer ISR.
/// Each tick = 1 / SCHEDULER_HZ seconds (10 ms at 100 Hz).
static KERNEL_TICKS: AtomicU64 = AtomicU64::new(0);

// ── UART helpers (no struct, no lock – used before memory init is complete) ──

/// Write a single byte to UART0, spinning until the transmit buffer is empty.
#[inline]
pub fn uart_write_byte(byte: u8) {
    unsafe {
        // Poll LSR.THRE (bit 5) – Transmit Holding Register Empty
        while (UART_LSR as *const u8).read_volatile() & (1 << 5) == 0 {
            core::hint::spin_loop();
        }
        (UART_THR as *mut u8).write_volatile(byte);
    }
}

/// Write a UTF-8 string slice to UART0, translating `\n` → `\r\n`.
pub fn uart_write_str(s: &str) {
    for byte in s.bytes() {
        if byte == b'\n' {
            uart_write_byte(b'\r');
        }
        uart_write_byte(byte);
    }
}

// ── Timer / CLINT ─────────────────────────────────────────────────────────────

/// Read the current value of the hardware `mtime` counter.
#[inline]
pub fn get_ticks() -> u64 {
    // mtime is memory-mapped and can be read with a volatile 64-bit load.
    unsafe { (CLINT_MTIME as *const u64).read_volatile() }
}

/// Programme the CLINT to deliver the next timer interrupt after `delta` ticks.
pub fn set_timer_delta(delta: u64) {
    let next = get_ticks().wrapping_add(delta);
    unsafe {
        (CLINT_MTIMECMP0 as *mut u64).write_volatile(next);
    }
}

/// Return the software monotonic tick counter (incremented in timer ISR).
#[inline]
pub fn kernel_ticks() -> u64 {
    KERNEL_TICKS.load(Ordering::Relaxed)
}

// ── Interrupt control ────────────────────────────────────────────────────────

/// Enable machine-mode interrupts (set `mstatus.MIE`).
///
/// # Safety
/// Calling this in a critical section without restoring the previous state
/// can introduce data races.
#[inline]
pub fn enable_interrupts() {
    unsafe {
        core::arch::asm!("csrsi mstatus, 0x8", options(nomem, nostack));
    }
}

/// Disable machine-mode interrupts (clear `mstatus.MIE`).
///
/// Returns `true` if interrupts were enabled before this call, so the caller
/// can restore the previous state with [`restore_interrupts`].
#[inline]
pub fn disable_interrupts() -> bool {
    let mstatus: usize;
    unsafe {
        core::arch::asm!(
            "csrrci {}, mstatus, 0x8",
            out(reg) mstatus,
            options(nomem, nostack)
        );
    }
    // bit 3 of mstatus = MIE (Machine Interrupt Enable)
    mstatus & 0x8 != 0
}

/// Restore the interrupt-enable state returned by [`disable_interrupts`].
#[inline]
pub fn restore_interrupts(was_enabled: bool) {
    if was_enabled {
        enable_interrupts();
    }
}

/// Enter low-power wait-for-interrupt state.
#[inline]
pub fn wait_for_interrupt() {
    unsafe {
        core::arch::asm!("wfi", options(nomem, nostack));
    }
}

/// Hard halt – spin forever with interrupts disabled.
///
/// Used by the safety subsystem when recovery is impossible.
pub fn halt() -> ! {
    disable_interrupts();
    loop {
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)) };
    }
}

// ── One-time hardware initialisation ────────────────────────────────────────

/// Perform early hardware initialisation:
/// 1. Disable all interrupts.
/// 2. Set the trap vector to our assembly trap entry.
/// 3. Arm the first timer interrupt.
/// 4. Enable the machine timer interrupt enable bit.
pub fn early_init() {
    unsafe {
        // 1. Disable global interrupts while we set up the trap vector.
        core::arch::asm!("csrw mstatus, zero", options(nomem, nostack));

        // 2. Clear all pending interrupt-enable bits.
        core::arch::asm!("csrw mie, zero", options(nomem, nostack));

        // 3. Programme the trap vector.
        //    mtvec[1:0] = 0b00 → "direct" mode (one handler for all traps).
        let trap_addr: usize;
        core::arch::asm!(
            "la {}, _trap_entry",
            out(reg) trap_addr,
            options(nomem, nostack)
        );
        core::arch::asm!(
            "csrw mtvec, {}",
            in(reg) trap_addr,
            options(nomem, nostack)
        );

        // 4. Arm the first timer interrupt (fires after one quantum).
        set_timer_delta(TICKS_PER_QUANTUM);

        // 5. Enable machine-timer-interrupt (mie.MTIE, bit 7 = 0x80).
        //    csrsi only accepts 0..31; use a register-based csrs instead.
        core::arch::asm!(
            "csrs mie, {0}",
            in(reg) 0x80usize,
            options(nomem, nostack)
        );
    }
}

// ── Trap dispatch (called from assembly _trap_entry) ────────────────────────

/// Rust-side trap handler.  Called by `_trap_entry` with interrupts disabled.
///
/// `mcause` layout:
///   - bit 63 set   → interrupt
///   - bit 63 clear → synchronous exception
///   - bits 62:0    → interrupt / exception code
///
/// # Safety
/// Must only be called from the `_trap_entry` assembly stub with a valid
/// trap frame on the stack.
#[no_mangle]
pub extern "C" fn trap_dispatch(mcause: usize, mepc: usize) {
    const INTERRUPT_BIT: usize = 1 << (usize::BITS - 1);
    const MACHINE_TIMER: usize = INTERRUPT_BIT | 7; // mip.MTIP

    if mcause == MACHINE_TIMER {
        // ── Timer interrupt ──────────────────────────────────────────────
        // 1. Rearm the timer for the next quantum.
        set_timer_delta(TICKS_PER_QUANTUM);

        // 2. Increment the monotonic tick counter.
        let tick = KERNEL_TICKS.fetch_add(1, Ordering::Relaxed) + 1;

        // 3. Notify the scheduler (will preempt if a higher-priority task
        //    became ready – full context switch is a TODO in v0.1).
        crate::scheduler::on_timer_tick(tick);
    } else if mcause & INTERRUPT_BIT == 0 {
        // ── Synchronous exception ────────────────────────────────────────
        // For now, treat any unexpected exception as a fatal fault.
        // A production kernel would decode mcause and dispatch to specific
        // handlers (e.g. page-fault → memory manager, ecall → syscall).
        crate::safety::handle_exception(mcause, mepc);
    }
    // All other interrupt sources (external, software) are silently ignored
    // in v0.1; the PLIC driver and software-IPI handler are future work.
}

// ── Assembly: kernel entry point and trap handler ───────────────────────────

global_asm!(r#"
# ═══════════════════════════════════════════════════════════════════════════
# _start — AetherOS kernel entry point
#
# QEMU with `-bios none -kernel <elf>` jumps here after loading the ELF.
# At this point:
#   - We are in machine mode (M-mode), highest RISC-V privilege level.
#   - All CSRs are in their reset state; interrupts are disabled.
#   - The ELF is loaded at 0x8000_0000; our linker script ensures _start
#     sits at the very first byte of that region.
#
# Responsibilities:
#   1. Disable interrupts (they should already be off, belt-and-suspenders).
#   2. Set the stack pointer to _stack_top (defined in linker.ld).
#   3. Zero-initialise the BSS segment.
#   4. Call kmain() — this never returns.
# ═══════════════════════════════════════════════════════════════════════════
    .section .text.start
    .global  _start
    .align   2

_start:
    # ── 1. Belt-and-suspenders: disable all interrupts ──────────────────
    csrw  mie,     zero
    csrw  mstatus, zero

    # ── 2. Set up the kernel stack ───────────────────────────────────────
    # _stack_top is the highest address of the stack region (grows down).
    la    sp, _stack_top

    # ── 3. Zero-initialise BSS ───────────────────────────────────────────
    # Uses doubleword stores (8 bytes per iteration) for speed.
    # The linker script guarantees both _bss_start and _bss_end are
    # 8-byte aligned.
    la    t0, _bss_start
    la    t1, _bss_end
.Lbss_loop:
    bgeu  t0, t1, .Lbss_done
    sd    zero, 0(t0)
    addi  t0, t0, 8
    j     .Lbss_loop
.Lbss_done:

    # ── 4. Jump to the Rust kernel main ──────────────────────────────────
    call  kmain

    # Should never reach here — halt with wfi loop as a safety net.
.Lhalt:
    wfi
    j     .Lhalt


# ═══════════════════════════════════════════════════════════════════════════
# _trap_entry — machine-mode trap / interrupt handler entry stub
#
# All 31 general-purpose registers (excluding x0, which is hardwired to 0)
# are saved to the current stack.  The Rust `trap_dispatch` function is then
# called with mcause and mepc.  On return, registers are restored and mret
# jumps back to the interrupted instruction (or the next one for interrupts).
#
# Register save layout on stack (each slot is 8 bytes / 1 doubleword):
#   sp+  0 : ra  (x1)       sp+ 88 : a5  (x15)
#   sp+  8 : sp  (x2) *      sp+ 96 : a6  (x16)
#   sp+ 16 : gp  (x3)       sp+104 : a7  (x17)
#   sp+ 24 : tp  (x4)       sp+112 : s2  (x18)
#   sp+ 32 : t0  (x5)       sp+120 : s3  (x19)
#   sp+ 40 : t1  (x6)       sp+128 : s4  (x20)
#   sp+ 48 : t2  (x7)       sp+136 : s5  (x21)
#   sp+ 56 : s0  (x8)       sp+144 : s6  (x22)
#   sp+ 64 : s1  (x9)       sp+152 : s7  (x23)
#   sp+ 72 : a0  (x10)      sp+160 : s8  (x24)
#   sp+ 80 : a1  (x11)      sp+168 : s9  (x25)
#                            sp+176 : s10 (x26)
#   sp+ 88 cont.             sp+184 : s11 (x27)
#   sp+ 88 : a2  (x12) *     sp+192 : t3  (x28)
#   (recalculated below)     sp+200 : t4  (x29)
#                            sp+208 : t5  (x30)
#                            sp+216 : t6  (x31)
# * The saved sp value at sp+8 is the stack pointer *before* allocation.
# ═══════════════════════════════════════════════════════════════════════════
    .section .text.trap
    .global  _trap_entry
    .align   4          # mtvec requires 4-byte alignment

_trap_entry:
    # Allocate 31*8 = 248 bytes for the trap frame (skip x0, always zero).
    addi  sp, sp, -248

    # Save caller-saved and callee-saved registers.
    sd    x1,   0*8(sp)   # ra
    # x2 (sp): save the pre-trap value = current sp + 248
    addi  t6, sp, 248
    sd    t6,   1*8(sp)   # original sp
    sd    x3,   2*8(sp)   # gp
    sd    x4,   3*8(sp)   # tp
    sd    x5,   4*8(sp)   # t0
    sd    x6,   5*8(sp)   # t1
    sd    x7,   6*8(sp)   # t2
    sd    x8,   7*8(sp)   # s0 / fp
    sd    x9,   8*8(sp)   # s1
    sd    x10,  9*8(sp)   # a0
    sd    x11, 10*8(sp)   # a1
    sd    x12, 11*8(sp)   # a2
    sd    x13, 12*8(sp)   # a3
    sd    x14, 13*8(sp)   # a4
    sd    x15, 14*8(sp)   # a5
    sd    x16, 15*8(sp)   # a6
    sd    x17, 16*8(sp)   # a7
    sd    x18, 17*8(sp)   # s2
    sd    x19, 18*8(sp)   # s3
    sd    x20, 19*8(sp)   # s4
    sd    x21, 20*8(sp)   # s5
    sd    x22, 21*8(sp)   # s6
    sd    x23, 22*8(sp)   # s7
    sd    x24, 23*8(sp)   # s8
    sd    x25, 24*8(sp)   # s9
    sd    x26, 25*8(sp)   # s10
    sd    x27, 26*8(sp)   # s11
    sd    x28, 27*8(sp)   # t3
    sd    x29, 28*8(sp)   # t4
    sd    x30, 29*8(sp)   # t5
    # t6 was already used above; its original value is in mscratch (see note).
    # For v0.1 (no user-mode) we don't swap mscratch, so t6 may be clobbered.
    # A future version will save/restore t6 via mscratch swap.
    sd    x31, 30*8(sp)   # t6 (may reflect modified value in v0.1)

    # ── Call the Rust trap dispatcher ────────────────────────────────────
    csrr  a0, mcause
    csrr  a1, mepc
    call  trap_dispatch

    # ── Restore all registers ────────────────────────────────────────────
    ld    x1,   0*8(sp)
    # x2 (sp) is restored last
    ld    x3,   2*8(sp)
    ld    x4,   3*8(sp)
    ld    x5,   4*8(sp)
    ld    x6,   5*8(sp)
    ld    x7,   6*8(sp)
    ld    x8,   7*8(sp)
    ld    x9,   8*8(sp)
    ld    x10,  9*8(sp)
    ld    x11, 10*8(sp)
    ld    x12, 11*8(sp)
    ld    x13, 12*8(sp)
    ld    x14, 13*8(sp)
    ld    x15, 14*8(sp)
    ld    x16, 15*8(sp)
    ld    x17, 16*8(sp)
    ld    x18, 17*8(sp)
    ld    x19, 18*8(sp)
    ld    x20, 19*8(sp)
    ld    x21, 20*8(sp)
    ld    x22, 21*8(sp)
    ld    x23, 22*8(sp)
    ld    x24, 23*8(sp)
    ld    x25, 24*8(sp)
    ld    x26, 25*8(sp)
    ld    x27, 26*8(sp)
    ld    x28, 27*8(sp)
    ld    x29, 28*8(sp)
    ld    x30, 29*8(sp)
    ld    x31, 30*8(sp)

    # Restore sp (x2) — this also deallocates the trap frame.
    ld    sp,   1*8(sp)

    # Return from machine-mode trap (restores pc from mepc, priv from mstatus).
    mret
"#);
