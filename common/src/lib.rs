//! # aether-common
//!
//! Shared types, constants, and primitives used by every AetherOS crate.
//! Must compile under both `no_std` (kernel) and `std` (host tools).
//!
//! ## Design Principles
//! - Zero-cost abstractions: all types are `repr(C)` or `repr(u8)` for
//!   predictable layout across FFI and IPC boundaries.
//! - `Copy` wherever the value is small (fits in a register).
//! - Every variant of every enum is documented with its real-time context.

#![no_std]

// ── Kernel-wide capacity constants ──────────────────────────────────────────

/// Maximum number of task control blocks in the static task table.
/// Keep this a power-of-two to allow cheap modular indexing.
pub const MAX_TASKS: usize = 32;

/// Maximum number of Reflex-Bus IPC channels.
pub const MAX_IPC_CHANNELS: usize = 16;

/// Number of hardware priority levels.
/// Matches the scheduler's priority enum variants below.
pub const PRIORITY_LEVELS: usize = 4;

/// Fixed payload size (bytes) for a single zero-copy IPC message slot.
/// Chosen to be one cache line (64 B) on most embedded targets.
pub const IPC_PAYLOAD_SIZE: usize = 64;

/// Depth of each channel's message ring buffer (must be power-of-two).
pub const IPC_RING_DEPTH: usize = 8;

/// Kernel stack size per hart, in bytes (64 KiB).
pub const KERNEL_STACK_SIZE: usize = 64 * 1024;

// ── Primitive type aliases ───────────────────────────────────────────────────

/// Unique task identifier.  0 is reserved for the idle task.
pub type TaskId = u32;

/// IPC channel identifier.
pub type ChannelId = u32;

// ── Priority ─────────────────────────────────────────────────────────────────

/// Hardware-enforced task priority level.
///
/// Lower numeric value == **higher** priority.  The scheduler always runs
/// the lowest-numbered ready task and never yields to a higher number while
/// a lower number is runnable.
///
/// | Level       | Use-case examples                                      |
/// |-------------|--------------------------------------------------------|
/// | `RealTime`  | Emergency-stop ISR, obstacle-avoidance control loop    |
/// | `High`      | Sensor fusion, navigation, attitude control            |
/// | `Normal`    | Mission logic, inter-agent communications              |
/// | `Low`       | Telemetry downlink, logging, background maintenance    |
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Highest: hard real-time safety-critical paths.
    RealTime = 0,
    /// High: time-sensitive control loops (≤ 1 ms deadline).
    High     = 1,
    /// Normal: mission-management and communications (≤ 10 ms deadline).
    Normal   = 2,
    /// Low: background tasks with no hard deadline.
    Low      = 3,
}

impl Priority {
    /// Convert a raw `u8` to a `Priority`, clamping to `Low` on out-of-range.
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Priority::RealTime,
            1 => Priority::High,
            2 => Priority::Normal,
            _ => Priority::Low,
        }
    }

    /// Return the priority as a `usize` index (used for priority-queue indexing).
    #[inline]
    pub const fn as_index(self) -> usize {
        self as usize
    }

    /// Human-readable prefix used in log messages.
    pub const fn tag(self) -> &'static str {
        match self {
            Priority::RealTime => "[RT]",
            Priority::High     => "[HI]",
            Priority::Normal   => "[NR]",
            Priority::Low      => "[LO]",
        }
    }
}

// ── TaskState ────────────────────────────────────────────────────────────────

/// State machine for a task control block.
///
/// ```text
///   Idle ──spawn──► Ready ──schedule──► Running
///                     ▲                    │
///                     │   yield / block    ▼
///                   Ready ◄──────────── Blocked
///                     │
///                   exit ──────────────► Terminated
/// ```
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// Slot is unused / not yet allocated.
    Idle       = 0,
    /// Task is runnable and waiting to be scheduled.
    Ready      = 1,
    /// Task is the currently executing task on this hart.
    Running    = 2,
    /// Task is waiting for an IPC message or timer event.
    Blocked    = 3,
    /// Task has returned / been killed; slot can be recycled.
    Terminated = 4,
}

// ── KernelError ──────────────────────────────────────────────────────────────

/// Compact error codes returned by kernel syscalls and internal APIs.
///
/// Stored as a single byte to keep IPC messages small.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernelError {
    /// No free slots left in a static table.
    OutOfResources  = 1,
    /// `TaskId` does not correspond to a live task.
    InvalidTask     = 2,
    /// `ChannelId` is out-of-range or not yet opened.
    InvalidChannel  = 3,
    /// Caller does not have the required capability.
    PermissionDenied = 4,
    /// Operation timed out waiting for a resource.
    Timeout         = 5,
    /// Data integrity check failed (checksum / TMR mismatch).
    Corrupted       = 6,
}

impl KernelError {
    /// A short human-readable description (no heap allocation).
    pub const fn description(self) -> &'static str {
        match self {
            KernelError::OutOfResources   => "out of resources",
            KernelError::InvalidTask      => "invalid task id",
            KernelError::InvalidChannel   => "invalid channel id",
            KernelError::PermissionDenied => "permission denied",
            KernelError::Timeout          => "timeout",
            KernelError::Corrupted        => "data integrity error",
        }
    }
}
