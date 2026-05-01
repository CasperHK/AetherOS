//! # Kernel Console
//!
//! Provides `kprint!` / `kprintln!` macros for formatted kernel output.
//!
//! Internally, these macros use `core::fmt::Write` and route all bytes
//! through `crate::arch::uart_write_str`, which is available immediately
//! after `arch::early_init()`.
//!
//! ## Usage
//! ```no_run
//! kprintln!("Task {} started at tick {}", task_id, tick);
//! kprint!("progress: {}/100\r", pct);
//! ```
//!
//! ## Thread safety
//! In v0.1 there is no per-call locking: two concurrent callers may produce
//! interleaved output.  A future version will wrap the UART in a SpinLock
//! and hold it for the duration of each `kprint!` call.

use core::fmt;

// ── KernelConsole writer ─────────────────────────────────────────────────────

/// Zero-sized write target that routes `fmt::Write` calls to the UART.
pub struct KernelConsole;

impl fmt::Write for KernelConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        crate::arch::uart_write_str(s);
        Ok(())
    }
}

// ── Macros ───────────────────────────────────────────────────────────────────

/// Print a formatted string to the kernel UART without a trailing newline.
///
/// Syntax is identical to `std::print!`.
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::console::KernelConsole, $($arg)*);
    }};
}

/// Print a formatted string to the kernel UART with a trailing newline.
///
/// Syntax is identical to `std::println!`.
#[macro_export]
macro_rules! kprintln {
    ()              => ($crate::kprint!("\n"));
    ($($arg:tt)*)   => {{
        $crate::kprint!($($arg)*);
        $crate::kprint!("\n");
    }};
}
