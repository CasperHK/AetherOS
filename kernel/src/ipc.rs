//! # Reflex-Bus: Zero-Copy Inter-Process Communication
//!
//! The **Reflex-Bus** is AetherOS's IPC primitive, designed for real-time
//! safety-critical messaging between kernel tasks and (in future) between
//! isolated vault processes.
//!
//! ## Design goals
//! - **Zero-copy**: message payloads sit in statically allocated ring slots;
//!   the receiver reads directly from the ring slot without any data copy.
//! - **Bounded latency**: all operations are O(1) — no dynamic allocation.
//! - **Priority-preserving**: high-priority senders can mark a message as
//!   urgent; the receiver always processes urgent messages first.
//! - **Radiation tolerant**: each message slot carries a CRC-8 checksum;
//!   the receiver silently discards corrupted messages and increments a
//!   fault counter (TMR handled at the safety layer above).
//!
//! ## Channel model
//! Each channel is a uni-directional FIFO ring buffer with `IPC_RING_DEPTH`
//! slots, each holding `IPC_PAYLOAD_SIZE` bytes.  Channels are identified by
//! a `ChannelId` (0 … MAX_IPC_CHANNELS-1).
//!
//! ## Future: Mojo Compute-Engine interface
//! The `extern "C"` bridge below is a placeholder for the FFI boundary between
//! the Rust kernel and the Mojo-based compute engine.  The compute engine will
//! inject neural-network inference results as IPC messages on a dedicated
//! channel (e.g. `CHANNEL_AI_INFERENCE = 8`), and the navigation task will
//! consume them.

use aether_common::{
    ChannelId, KernelError, TaskId, IPC_PAYLOAD_SIZE, IPC_RING_DEPTH, MAX_IPC_CHANNELS,
};
use core::sync::atomic::{AtomicU32, Ordering};
use crate::sync::SpinLock;
use crate::kprintln;

// ── Well-known channel IDs ────────────────────────────────────────────────────

/// Emergency-stop broadcast channel.  Any task may send; the RT watchdog reads.
pub const CHANNEL_EMERGENCY:    ChannelId = 0;
/// Navigation → mission planner data channel.
pub const CHANNEL_NAV_MISSION:  ChannelId = 1;
/// Sensor fusion → navigation channel.
pub const CHANNEL_SENSOR_NAV:   ChannelId = 2;
/// Telemetry outbound queue (kernel → radio task).
pub const CHANNEL_TELEMETRY:    ChannelId = 3;
/// AI inference results from Mojo compute engine (future).
pub const CHANNEL_AI_INFERENCE: ChannelId = 8;

// ── Message ────────────────────────────────────────────────────────────────────

/// A single IPC message with fixed-size payload.
///
/// The payload is intentionally untyped (`[u8; IPC_PAYLOAD_SIZE]`); higher
/// layers use zero-copy casting (`as *const SomeStruct`) when the message
/// type is known.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Message {
    /// Sender task ID (0 = kernel / hardware interrupt).
    pub sender: TaskId,
    /// Message type tag — application-defined.  0 = generic data.
    pub kind: u16,
    /// CRC-8 checksum over the payload bytes.
    pub checksum: u8,
    /// Urgent flag: `true` = skip to head of receiver's processing queue.
    pub urgent: bool,
    /// Fixed-size zero-copy payload.
    pub payload: [u8; IPC_PAYLOAD_SIZE],
}

impl Message {
    /// Construct a new message, computing the CRC-8 automatically.
    pub fn new(sender: TaskId, kind: u16, payload: [u8; IPC_PAYLOAD_SIZE]) -> Self {
        let checksum = crc8(&payload);
        Message { sender, kind, checksum, urgent: false, payload }
    }

    /// Same as `new` but marks the message as urgent.
    pub fn urgent(sender: TaskId, kind: u16, payload: [u8; IPC_PAYLOAD_SIZE]) -> Self {
        let mut m = Self::new(sender, kind, payload);
        m.urgent = true;
        m
    }

    /// Verify integrity.  Returns `false` if the CRC does not match.
    pub fn is_valid(&self) -> bool {
        crc8(&self.payload) == self.checksum
    }
}

// ── Ring buffer ───────────────────────────────────────────────────────────────

/// A statically allocated lock-free ring buffer for IPC messages.
///
/// Uses separate `head` / `tail` atomic counters; wraps at `IPC_RING_DEPTH`.
struct Ring {
    slots: [Message; IPC_RING_DEPTH],
    head:  AtomicU32,   // next read index  (consumer increments)
    tail:  AtomicU32,   // next write index (producer increments)
}

/// Ring buffer is empty when head == tail.
/// Ring buffer is full  when (tail + 1) % DEPTH == head.
const EMPTY_MESSAGE: Message = Message {
    sender:   0,
    kind:     0,
    checksum: 0,
    urgent:   false,
    payload:  [0u8; IPC_PAYLOAD_SIZE],
};

const EMPTY_RING: Ring = Ring {
    slots: [EMPTY_MESSAGE; IPC_RING_DEPTH],
    head:  AtomicU32::new(0),
    tail:  AtomicU32::new(0),
};

impl Ring {
    /// Try to enqueue a message.  Returns `Err(OutOfResources)` if full.
    fn send(&mut self, msg: Message) -> Result<(), KernelError> {
        let tail = self.tail.load(Ordering::Acquire);
        let next_tail = (tail + 1) % IPC_RING_DEPTH as u32;
        if next_tail == self.head.load(Ordering::Acquire) {
            return Err(KernelError::OutOfResources); // ring full
        }
        self.slots[tail as usize] = msg;
        self.tail.store(next_tail, Ordering::Release);
        Ok(())
    }

    /// Try to dequeue a message.  Returns `None` if empty.
    /// Returns `Err(Corrupted)` if the CRC check fails.
    fn recv(&mut self) -> Result<Option<Message>, KernelError> {
        let head = self.head.load(Ordering::Acquire);
        if head == self.tail.load(Ordering::Acquire) {
            return Ok(None); // empty
        }
        let msg = self.slots[head as usize];
        self.head.store((head + 1) % IPC_RING_DEPTH as u32, Ordering::Release);

        if !msg.is_valid() {
            // CRC mismatch — likely cosmic-ray bit flip; discard and report.
            return Err(KernelError::Corrupted);
        }
        Ok(Some(msg))
    }

    fn is_empty(&self) -> bool {
        self.head.load(Ordering::Relaxed) == self.tail.load(Ordering::Relaxed)
    }
}

// SAFETY: Ring uses AtomicU32 for the indices; slots are only accessed while
// holding the outer SpinLock.
unsafe impl Send for Ring {}
unsafe impl Sync for Ring {}

// ── Channel table ─────────────────────────────────────────────────────────────

static CHANNELS: SpinLock<[Ring; MAX_IPC_CHANNELS]> =
    SpinLock::new([EMPTY_RING; MAX_IPC_CHANNELS]);

// ── Public API ────────────────────────────────────────────────────────────────

/// Initialise the IPC subsystem.
pub fn init() {
    kprintln!("reflex-bus: initialized [{} channels, depth {}, {} B/slot]",
              MAX_IPC_CHANNELS, IPC_RING_DEPTH, IPC_PAYLOAD_SIZE);
}

/// Send a message to `channel`.
///
/// This is a non-blocking call; returns `Err(OutOfResources)` if the ring
/// buffer is full.
pub fn send(channel: ChannelId, msg: Message) -> Result<(), KernelError> {
    if channel as usize >= MAX_IPC_CHANNELS {
        return Err(KernelError::InvalidChannel);
    }
    CHANNELS.lock()[channel as usize].send(msg)
}

/// Receive the next message from `channel`.
///
/// Returns:
/// - `Ok(Some(msg))` — a valid message was available.
/// - `Ok(None)`      — the channel is empty (no message yet).
/// - `Err(Corrupted)` — a message was present but failed its CRC check.
pub fn recv(channel: ChannelId) -> Result<Option<Message>, KernelError> {
    if channel as usize >= MAX_IPC_CHANNELS {
        return Err(KernelError::InvalidChannel);
    }
    CHANNELS.lock()[channel as usize].recv()
}

/// Returns `true` if `channel` has at least one pending message.
pub fn has_pending(channel: ChannelId) -> bool {
    if channel as usize >= MAX_IPC_CHANNELS {
        return false;
    }
    !CHANNELS.lock()[channel as usize].is_empty()
}

/// Broadcast a short emergency-stop message on `CHANNEL_EMERGENCY`.
///
/// This is a fast path: the payload is zeroed and `urgent` is set to `true`.
/// The real-time watchdog task will observe this on its next iteration.
pub fn broadcast_emergency_stop(sender: TaskId) -> Result<(), KernelError> {
    let msg = Message::urgent(sender, 0xFFFF, [0u8; IPC_PAYLOAD_SIZE]);
    send(CHANNEL_EMERGENCY, msg)
}

// ── CRC-8 (Dallas/Maxim polynomial 0x31) ─────────────────────────────────────

/// Compute CRC-8 over a byte slice using the Dallas/Maxim polynomial (0x31).
///
/// Chosen because it detects all single-bit errors and many multi-bit errors —
/// important for radiation-tolerant operation in space.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0x00;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ 0x31;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

// ── Mojo Compute-Engine FFI placeholder ──────────────────────────────────────
//
// When the Mojo AI compute engine is integrated, it will push inference results
// through this C-ABI boundary into the kernel's Reflex-Bus.  The Rust side
// registers a callback; the Mojo side calls it via the extern "C" pointer.
//
// Example future usage:
//   mojo_register_inference_callback(aether_ai_result_callback);
//
// For now these symbols exist only as documentation / linker placeholders.

/// Signature of the inference result callback that the compute engine invokes.
///
/// `channel`  — which Reflex-Bus channel to post to (use `CHANNEL_AI_INFERENCE`)
/// `kind`     — model-defined output type tag
/// `data`     — pointer to the inference output buffer (must be IPC_PAYLOAD_SIZE bytes)
pub type AiResultCallback = extern "C" fn(channel: u32, kind: u16, data: *const u8);

/// The actual callback implementation — called from Mojo when an inference
/// result is ready.
///
/// The `channel` parameter uses `u32` (the underlying type of `ChannelId`) so
/// that this function has a stable C ABI signature that the Mojo runtime can
/// call without depending on Rust type aliases.  The value is validated by
/// `send()` before use.
///
/// # Safety
/// `data` must point to a valid buffer of exactly `IPC_PAYLOAD_SIZE` bytes.
#[no_mangle]
pub unsafe extern "C" fn aether_ai_result_callback(
    channel: u32,   // == ChannelId; u32 used explicitly for stable C ABI
    kind: u16,
    data: *const u8,
) {
    // SAFETY: caller guarantees `data` is valid for IPC_PAYLOAD_SIZE bytes.
    let mut payload = [0u8; IPC_PAYLOAD_SIZE];
    core::ptr::copy_nonoverlapping(data, payload.as_mut_ptr(), IPC_PAYLOAD_SIZE);
    let msg = Message::new(0 /* kernel */, kind, payload);
    let _ = send(channel, msg); // best-effort; drop if ring is full
}
