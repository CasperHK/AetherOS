//! # Safety Subsystem: Fault Tolerance, Panic Handling, Emergency Stop
//!
//! This module implements the safety-critical layer of AetherOS.  It covers:
//!
//! 1. **Panic handler** — the `#[panic_handler]` required by `no_std` Rust.
//!    Instead of a bare infinite loop, AetherOS enters *safe mode* and
//!    attempts to transmit a last-resort diagnostic message over UART before
//!    halting.
//!
//! 2. **Emergency stop (E-Stop)** — a high-priority function that can be
//!    invoked from any task or ISR to immediately cut actuator power and
//!    broadcast an emergency message over the Reflex-Bus.  This mirrors the
//!    hardware E-Stop signal found on industrial robots and drones.
//!
//! 3. **Triple Modular Redundancy (TMR)** — a software implementation of
//!    voter logic for safety-critical computations.  A value is computed
//!    three times and the result is taken by majority vote.  Any discrepancy
//!    increments a SEU (Single Event Upset) counter.  This is a key
//!    mitigation strategy for cosmic-ray-induced bit flips in space
//!    environments.
//!
//! 4. **Checksum verification** — lightweight CRC-8 integrity check used to
//!    detect memory corruption before acting on safety-critical data.
//!
//! 5. **Exception handler** — called by the arch trap dispatcher for
//!    synchronous CPU exceptions (illegal instruction, misaligned access, …).
//!
//! ## Real-time constraints
//! `emergency_stop` and `tmr_vote` **must** complete in bounded time with no
//! dynamic memory allocation.  All data structures are static.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::panic::PanicInfo;
use crate::kprintln;

// ── Global safety state ───────────────────────────────────────────────────────

/// Set to `true` when the system has entered emergency / safe mode.
static EMERGENCY_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Cumulative count of SEU events detected by TMR voters across all tasks.
static SEU_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Count of exceptions handled (for telemetry).
static EXCEPTION_COUNTER: AtomicU32 = AtomicU32::new(0);

// ── Initialisation ────────────────────────────────────────────────────────────

/// Initialise the safety subsystem.
pub fn init() {
    EMERGENCY_ACTIVE.store(false, Ordering::Relaxed);
    SEU_COUNTER.store(0, Ordering::Relaxed);
    kprintln!("safety: subsystem online [TMR enabled, CRC-8 checksums]");

    // Self-test: run a TMR vote on a known value to verify voter logic.
    let result = tmr_vote(|| 0xABCD_1234u32);
    if result != 0xABCD_1234 {
        // This should never happen on correct hardware; if it does it means
        // the TMR implementation itself is broken — halt immediately.
        panic!("safety: TMR self-test FAILED — aborting boot");
    }
    kprintln!("safety: TMR self-test PASS");
}

// ── Emergency Stop ────────────────────────────────────────────────────────────

/// **Emergency Stop** — the highest-priority safety action in AetherOS.
///
/// This function:
/// 1. Disables interrupts (ensures atomicity of E-Stop sequence).
/// 2. Broadcasts an emergency message on the Reflex-Bus.
/// 3. Signals actuator-disable GPIOs (TODO: arch-specific GPIO driver).
/// 4. Logs the event over UART.
/// 5. Halts the system in a safe, low-power state.
///
/// ## Real-time requirement
/// This function **must** complete within the hard deadline specified by the
/// airframe / mission safety plan (typically ≤ 500 µs for drones).
/// All operations are O(1); no locks that can block.
///
/// ## Calling contexts
/// May be called from any context including ISRs.
#[inline(never)]
pub fn emergency_stop(reason: &str) -> ! {
    // 1. Disable interrupts to prevent re-entrancy.
    crate::arch::disable_interrupts();

    // 2. Flip the global E-Stop flag (used by all task loops to detect stop).
    EMERGENCY_ACTIVE.store(true, Ordering::SeqCst);

    // 3. Broadcast emergency message over Reflex-Bus (best-effort).
    let _ = crate::ipc::broadcast_emergency_stop(0);

    // 4. TODO(hw): drive actuator-disable GPIO low via arch HAL.
    //    crate::arch::gpio_emergency_disable();

    // 5. Log and halt.
    crate::arch::uart_write_str("\n[EMERGENCY STOP] ");
    crate::arch::uart_write_str(reason);
    crate::arch::uart_write_str("\nSystem halted.\n");

    crate::arch::halt()
}

/// Returns `true` if an emergency stop has been activated.
#[inline]
pub fn is_emergency_active() -> bool {
    EMERGENCY_ACTIVE.load(Ordering::Acquire)
}

// ── Triple Modular Redundancy (TMR) ──────────────────────────────────────────

/// Execute `compute` three times and return the majority result.
///
/// If all three results agree, the result is returned unchanged.
/// If exactly two agree (one SEU), the majority is returned and the SEU
/// counter is incremented.
/// If all three disagree (double SEU or systemic error), the system enters
/// safe mode.
///
/// ## Type parameter
/// `T` must be `Copy + Eq`.  For floating-point values, use a fixed-point
/// representation or a bitcast to `u32`/`u64` for the comparison.
///
/// ## Example
/// ```no_run
/// let altitude_cm = tmr_vote(|| read_altimeter());
/// ```
pub fn tmr_vote<T, F>(compute: F) -> T
where
    T: Copy + Eq,
    F: Fn() -> T,
{
    let a = compute();
    let b = compute();
    let c = compute();

    if a == b {
        // Fast path (normal, no SEU): a == b, return a.
        if a != c {
            // c is the outlier — single-bit upset in result c.
            SEU_COUNTER.fetch_add(1, Ordering::Relaxed);
        }
        return a;
    }
    if a == c {
        // b is the outlier.
        SEU_COUNTER.fetch_add(1, Ordering::Relaxed);
        return a;
    }
    if b == c {
        // a is the outlier.
        SEU_COUNTER.fetch_add(1, Ordering::Relaxed);
        return b;
    }

    // All three disagree — unrecoverable data error.
    emergency_stop("TMR triple disagreement — unrecoverable SEU");
}

/// Return the cumulative SEU event count since boot.
#[inline]
pub fn seu_count() -> u32 {
    SEU_COUNTER.load(Ordering::Relaxed)
}

// ── Checksum verification ─────────────────────────────────────────────────────

/// Verify a data buffer against an expected CRC-8 checksum.
///
/// Returns `true` if the data is intact, `false` if corruption is detected.
/// Delegates to `ipc::crc8` for consistent checksum computation.
#[inline]
pub fn checksum_verify(data: &[u8], expected: u8) -> bool {
    crate::ipc::crc8(data) == expected
}

/// Compute and return the CRC-8 checksum of a buffer.
#[inline]
pub fn checksum(data: &[u8]) -> u8 {
    crate::ipc::crc8(data)
}

// ── Exception handler ─────────────────────────────────────────────────────────

/// Handle a synchronous CPU exception (called from `trap_dispatch`).
///
/// For now this is a fatal handler; a production kernel would decode the
/// exception type and attempt recovery (e.g. kill the offending task and
/// restart it if it is restartable).
pub fn handle_exception(mcause: usize, mepc: usize) {
    EXCEPTION_COUNTER.fetch_add(1, Ordering::Relaxed);

    // Print diagnostic information.
    crate::arch::uart_write_str("\n[EXCEPTION] mcause=0x");
    print_hex_uart(mcause as u64);
    crate::arch::uart_write_str(" mepc=0x");
    print_hex_uart(mepc as u64);
    crate::arch::uart_write_str("\n");

    // TODO(v0.2): decode mcause, attempt per-task recovery:
    //   - Illegal instruction / misaligned → kill & restart task
    //   - PMP fault → sandbox violation → terminate vault process
    emergency_stop("unhandled CPU exception");
}

// ── Panic handler ─────────────────────────────────────────────────────────────

/// Kernel panic handler.
///
/// `no_std` requires exactly one `#[panic_handler]` in the binary.
/// AetherOS's handler differs from a simple `loop {}`:
///   1. Disables interrupts to freeze the system state.
///   2. Writes a diagnostic message to UART.
///   3. Calls `arch::halt()` — a WFI loop — rather than a busy spin.
///
/// This ensures the last-resort message is always transmitted even if the
/// scheduler or memory subsystem is in an inconsistent state.
#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    // Disable interrupts immediately — we do not want to be preempted while
    // printing the panic message.
    crate::arch::disable_interrupts();

    crate::arch::uart_write_str("\n\n╔══════════════════════════════╗\n");
    crate::arch::uart_write_str(  "║      KERNEL PANIC            ║\n");
    crate::arch::uart_write_str(  "╚══════════════════════════════╝\n");

    if let Some(location) = info.location() {
        crate::arch::uart_write_str("Location : ");
        crate::arch::uart_write_str(location.file());
        crate::arch::uart_write_str(":");
        // Print line number without alloc
        print_u32_uart(location.line());
        crate::arch::uart_write_str("\n");
    }

    if let Some(msg) = info.message().as_str() {
        crate::arch::uart_write_str("Message  : ");
        crate::arch::uart_write_str(msg);
        crate::arch::uart_write_str("\n");
    }

    crate::arch::uart_write_str("SEU count: ");
    print_u32_uart(SEU_COUNTER.load(Ordering::Relaxed));
    crate::arch::uart_write_str("\n");
    crate::arch::uart_write_str("Entering safe mode (WFI halt).\n\n");

    crate::arch::halt()
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Print a `u64` as 16 hex digits to UART without using fmt or alloc.
fn print_hex_uart(val: u64) {
    for i in (0..16).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;
        let c = if nibble < 10 { b'0' + nibble } else { b'a' - 10 + nibble };
        crate::arch::uart_write_byte(c);
    }
}

/// Print a `u32` as decimal digits to UART without using fmt or alloc.
fn print_u32_uart(mut val: u32) {
    if val == 0 {
        crate::arch::uart_write_byte(b'0');
        return;
    }
    let mut buf = [0u8; 10]; // max 10 decimal digits for u32
    let mut i = 0usize;
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    // buf is filled in reverse order
    for j in (0..i).rev() {
        crate::arch::uart_write_byte(buf[j]);
    }
}
