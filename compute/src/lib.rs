//! # aether-compute — Mojo AI Compute-Engine Bridge
//!
//! This crate will host the FFI and orchestration layer between the AetherOS
//! kernel and the [Mojo](https://www.modular.com/mojo) neural-network
//! inference runtime.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  Mojo Compute Engine (user-space / co-processor)         │
//! │  ┌──────────────────────────────────────────────────┐   │
//! │  │  model.mojo  — neural net (object detection,     │   │
//! │  │                path planning, sensor fusion)      │   │
//! │  └──────────────┬───────────────────────────────────┘   │
//! │                 │ C-ABI / shared memory                  │
//! │  ┌──────────────▼───────────────────────────────────┐   │
//! │  │  aether-compute  (this crate)                     │   │
//! │  │  • schedule inference jobs                        │   │
//! │  │  • marshal results → aether_ai_result_callback    │   │
//! │  │  • power / thermal budgeting                      │   │
//! │  └──────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────┘
//!          ↕  Reflex-Bus IPC (CHANNEL_AI_INFERENCE)
//! ┌─────────────────────────────────────────────────────────┐
//! │  AetherOS Kernel (aether-kernel)                         │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Status: placeholder
//! The Mojo runtime integration is pending.  This crate currently only
//! defines the Rust-side data types and function signatures that the
//! compute engine will implement.

// Allow std for this host-side crate (not a bare-metal component).
extern crate std;

use aether_common::{ChannelId, IPC_PAYLOAD_SIZE};

// ── Inference job descriptor ──────────────────────────────────────────────────

/// Describes a single AI inference job to be submitted to the Mojo engine.
#[repr(C)]
pub struct InferenceJob {
    /// Which Reflex-Bus channel to post the result to.
    pub result_channel: ChannelId,
    /// Model identifier (application-defined).
    pub model_id: u32,
    /// Input tensor data pointer (must remain valid until the job completes).
    pub input_ptr: *const u8,
    /// Input tensor byte length.
    pub input_len: usize,
}

// SAFETY: InferenceJob contains raw pointers.  The user of this API must
//         ensure the pointed-to data outlives the job.
unsafe impl Send for InferenceJob {}

/// Result of a completed inference job (posted to Reflex-Bus).
#[repr(C)]
pub struct InferenceResult {
    pub model_id:  u32,
    pub kind:      u16,
    pub payload:   [u8; IPC_PAYLOAD_SIZE],
}

// ── Mojo engine FFI stubs ─────────────────────────────────────────────────────

// These extern declarations will be satisfied by the Mojo-compiled shared
// library when it is linked in.  Until then they are declared but not called.

extern "C" {
    /// Submit an inference job to the Mojo engine (non-blocking).
    /// Returns 0 on success, non-zero on error.
    #[allow(dead_code)]
    fn mojo_submit_inference(job: *const InferenceJob) -> i32;

    /// Register the Rust callback that the Mojo engine will invoke when an
    /// inference job completes.
    #[allow(dead_code)]
    fn mojo_register_result_callback(
        cb: unsafe extern "C" fn(channel: u32, kind: u16, data: *const u8),
    );
}

// ── Public Rust API (stubs) ───────────────────────────────────────────────────

/// Initialise the compute engine.
///
/// Must be called once from the host before submitting any jobs.
/// (No-op until the Mojo runtime is linked in.)
pub fn init() {
    // TODO: call mojo_register_result_callback(aether_kernel::ipc::aether_ai_result_callback)
    std::eprintln!("[aether-compute] init: Mojo compute engine not yet linked");
}
