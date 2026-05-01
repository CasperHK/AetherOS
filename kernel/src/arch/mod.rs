//! # Hardware Abstraction Layer (HAL)
//!
//! This module re-exports the platform-specific implementation so that every
//! other kernel module can simply write `crate::arch::uart_write_str(…)` and
//! remain architecture-agnostic.
//!
//! ## Adding a new architecture
//! 1. Create `kernel/src/arch/<arch>.rs`.
//! 2. Implement **all** the public symbols listed in the "Required interface"
//!    comment below.
//! 3. Add a `#[cfg]` / `pub use` pair in this file.

// ── Platform selection ───────────────────────────────────────────────────────

#[cfg(target_arch = "riscv64")]
mod riscv64;
#[cfg(target_arch = "riscv64")]
pub use riscv64::*;

// AArch64 stub – ground-robot / Raspberry Pi future port.
// Uncomment and implement riscv64.rs's interface in aarch64.rs to enable.
// #[cfg(target_arch = "aarch64")]
// mod aarch64;
// #[cfg(target_arch = "aarch64")]
// pub use aarch64::*;

// ── Required interface (every arch module must expose these) ─────────────────
//
// /// One-time hardware initialisation: UART baud-rate, CLINT, trap vector.
// pub fn early_init();
//
// /// Write a single byte to the debug UART (blocking, no lock required).
// pub fn uart_write_byte(byte: u8);
//
// /// Write a UTF-8 string slice to the debug UART.
// pub fn uart_write_str(s: &str);
//
// /// Return the current value of the hardware monotonic tick counter.
// pub fn get_ticks() -> u64;
//
// /// Program the next timer interrupt to fire `delta` ticks from now.
// pub fn set_timer_delta(delta: u64);
//
// /// Enable machine-mode interrupts globally (set mstatus.MIE).
// pub fn enable_interrupts();
//
// /// Disable machine-mode interrupts globally (clear mstatus.MIE).
// /// Returns the previous interrupt-enable state so callers can restore it.
// pub fn disable_interrupts() -> bool;
//
// /// Restore the interrupt-enable state returned by `disable_interrupts`.
// pub fn restore_interrupts(was_enabled: bool);
//
// /// Enter a low-power wait state until the next interrupt fires.
// pub fn wait_for_interrupt();
//
// /// Immediately halt the hart (used by the safety subsystem).
// pub fn halt() -> !;
