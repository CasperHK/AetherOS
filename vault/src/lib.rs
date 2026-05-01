//! # aether-vault — WebAssembly Sandbox Isolation Layer
//!
//! The **Vault** provides capability-based isolation for untrusted mission
//! modules loaded at runtime.  A mission module (e.g. a new path-planning
//! algorithm) is compiled to WebAssembly, loaded into a Vault instance, and
//! given only the Reflex-Bus channels it has been explicitly granted.
//!
//! If the module panics, crashes, or exceeds its resource budget, the Vault
//! tears it down without affecting the rest of the kernel or other vaults.
//!
//! ## Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────────────┐
//! │  Mission module (untrusted, compiled to Wasm)           │
//! │    mission_planner.wasm                                 │
//! └─────────────────────┬──────────────────────────────────┘
//!                       │  Wasm ABI (imports / exports)
//! ┌─────────────────────▼──────────────────────────────────┐
//! │  aether-vault  (this crate)                             │
//! │  • Wasm runtime (wasmtime / micro-wasm future port)     │
//! │  • Capability filter (only allowed channels pass)       │
//! │  • Resource accounting (memory, ticks, message quota)   │
//! └─────────────────────┬──────────────────────────────────┘
//!                       │  Reflex-Bus IPC
//! ┌─────────────────────▼──────────────────────────────────┐
//! │  AetherOS Kernel                                        │
//! └────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Status: placeholder
//! The Wasm runtime integration is pending.  This crate defines the
//! data types and host-function signatures that the Wasm runtime will use.

extern crate std;

use aether_common::ChannelId;

// ── Capability set ────────────────────────────────────────────────────────────

/// The set of Reflex-Bus channels a Vault instance is permitted to access.
///
/// Stored as a bitmask: bit `n` = channel `n` is allowed.
#[derive(Clone, Copy, Debug, Default)]
pub struct CapabilitySet {
    allowed_channels: u64, // supports up to 64 channels
}

impl CapabilitySet {
    /// Create an empty (no permissions) capability set.
    pub const fn empty() -> Self {
        Self { allowed_channels: 0 }
    }

    /// Grant access to a specific channel.
    pub fn grant_channel(&mut self, ch: ChannelId) {
        if (ch as usize) < 64 {
            self.allowed_channels |= 1u64 << ch;
        }
    }

    /// Check whether a channel is allowed.
    pub fn is_allowed(&self, ch: ChannelId) -> bool {
        if (ch as usize) >= 64 { return false; }
        self.allowed_channels & (1u64 << ch) != 0
    }
}

// ── Vault instance ────────────────────────────────────────────────────────────

/// A Vault instance encapsulates one Wasm module and its resource limits.
pub struct Vault {
    /// Human-readable module name for diagnostics.
    pub name: &'static str,
    /// Capability set granted to this vault.
    pub caps: CapabilitySet,
    /// Maximum memory pages (1 page = 64 KiB) the module may allocate.
    pub max_memory_pages: u32,
    /// Maximum scheduler ticks this module may consume per quantum.
    pub tick_budget: u32,
    // TODO: wasm_module: WasmModule (wasmtime / custom micro-Wasm runtime)
}

impl Vault {
    /// Create a new Vault with the given capability set.
    pub fn new(name: &'static str, caps: CapabilitySet) -> Self {
        Self { name, caps, max_memory_pages: 16, tick_budget: 10 }
    }

    /// Load and execute a Wasm binary.  Returns an error string on failure.
    ///
    /// # Status
    /// Placeholder — returns an error until the Wasm runtime is linked in.
    ///
    /// Note: `eprintln!` is intentional here.  `aether-vault` is a host-side
    /// `std` crate (it runs on the host or in a privileged user-space process,
    /// not inside the bare-metal kernel).  When the vault eventually moves to
    /// `no_std`, this diagnostic output will be routed through the kernel's
    /// Reflex-Bus telemetry channel instead.
    pub fn load_and_run(&self, _wasm_bytes: &[u8]) -> Result<(), &'static str> {
        std::eprintln!("[aether-vault] '{}': Wasm runtime not yet implemented", self.name);
        Err("vault: wasm runtime not implemented")
    }
}
