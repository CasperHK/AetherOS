# AetherOS — Real-Time Neural Microkernel

```
╔══════════════════════════════════════════════════════════════╗
║          AetherOS  v0.1.0                                    ║
║       Real-Time Neural Microkernel                           ║
║  Drones · Ground Robots · Satellites · Deep-Space Craft      ║
╚══════════════════════════════════════════════════════════════╝
```

> *"感知即行動，行動即生存。"*  
> *Perception is action. Action is survival.*

AetherOS is an open-source **Real-Time Neural Microkernel** designed for
extreme autonomous machines — drones, ground robots, satellites, and deep-space
spacecraft — where reliability, latency, and AI co-processing are non-negotiable.

## ✨ Core Philosophy

| Principle | Implementation |
|-----------|---------------|
| **Memory safety** | Written entirely in Rust (`no_std`, `no_main`) |
| **Hard real-time** | Fixed-priority preemptive scheduler; 4 priority bands |
| **AI-native** | Reflex-Bus IPC ↔ Mojo compute engine (inference results as first-class messages) |
| **Radiation tolerance** | Triple Modular Redundancy (TMR) voter + CRC-8 checksums |
| **Isolation** | WebAssembly Vault sandbox for untrusted mission modules |
| **Swarm-ready** | Reflex-Bus multi-channel architecture for drone swarm coordination |

## 🗂 Project Structure

```
AetherOS/
├── kernel/                  # Rust real-time microkernel (no_std + no_main)
│   ├── src/
│   │   ├── main.rs          # Entry point: _start (asm) → kmain (Rust)
│   │   ├── arch/
│   │   │   ├── mod.rs       # Hardware Abstraction Layer (HAL) interface
│   │   │   ├── riscv64.rs   # RISC-V 64 implementation (UART, CLINT, trap)
│   │   │   └── linker.ld    # Linker script for QEMU virt (0x8000_0000)
│   │   ├── scheduler.rs     # Fixed-priority round-robin scheduler
│   │   ├── ipc.rs           # Reflex-Bus zero-copy IPC
│   │   ├── safety.rs        # Panic handler, TMR, emergency stop
│   │   ├── sync.rs          # SpinLock<T> synchronisation primitive
│   │   └── console.rs       # kprint! / kprintln! macros
│   ├── Cargo.toml
│   └── build.rs             # Passes linker script to rustc
│
├── compute/                 # Mojo AI compute-engine bridge (placeholder)
├── vault/                   # WebAssembly sandbox isolation layer (placeholder)
├── common/                  # Shared types, constants, error codes
│
├── Cargo.toml               # Workspace root
├── rust-toolchain.toml      # Pins nightly + riscv64gc target
├── .cargo/config.toml       # Default target + QEMU runner
└── run.sh                   # Build & launch in QEMU
```

## 🚀 Quick Start

### Prerequisites

```bash
# Install Rust + nightly toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install nightly
rustup target add riscv64gc-unknown-none-elf --toolchain nightly
rustup component add rust-src --toolchain nightly

# Install QEMU (Ubuntu/Debian)
sudo apt install qemu-system-misc

# Install QEMU (macOS)
brew install qemu
```

### Build

```bash
# Debug build
cargo build --package aether-kernel

# Release build (size-optimised for embedded)
cargo build --package aether-kernel --release
```

### Run in QEMU

```bash
# Using the helper script (recommended)
./run.sh               # debug build
./run.sh --release     # release build
./run.sh --gdb         # start with GDB server on :1234

# Or manually
qemu-system-riscv64 \
    -machine virt \
    -nographic \
    -bios none \
    -m 128M \
    -kernel target/riscv64gc-unknown-none-elf/debug/aether-kernel
```

Press **Ctrl+A then X** to exit QEMU.

### Expected Output

```
╔══════════════════════════════════════════════════════════════╗
║          AetherOS  v0.1.0                                    ║
║       Real-Time Neural Microkernel                           ║
║  Drones · Ground Robots · Satellites · Deep-Space Craft      ║
╚══════════════════════════════════════════════════════════════╝

  Target  : RISC-V 64-bit (riscv64gc-unknown-none-elf)
  Build   : aether-kernel 0.1.0
  Timer   : 100 Hz scheduler, 10000000 Hz CLINT

safety: subsystem online [TMR enabled, CRC-8 checksums]
safety: TMR self-test PASS
scheduler: initialized [4 priority levels, 32 task slots]
reflex-bus: initialized [16 channels, depth 8, 64 B/slot]

Spawning initial kernel tasks...
scheduler: spawned task [RT] 1 [0]
scheduler: spawned task [HI] 2 [1]
scheduler: spawned task [NR] 3 [2]
scheduler: spawned task [LO] 4 [3]

All systems nominal.  Transferring control to scheduler.
─────────────────────────────────────────────────────────
scheduler: starting — enabling interrupts
[RT] emergency watchdog: armed and running
[HI] navigation: sensor fusion online
[NR] mission manager: online
[LO] telemetry: downlink active
[tick 100] uptime: 1 s
[tick 200] uptime: 2 s
...
```

### Debug with GDB

```bash
# Terminal 1: start QEMU with GDB server
./run.sh --gdb

# Terminal 2: attach GDB
riscv64-unknown-elf-gdb target/riscv64gc-unknown-none-elf/debug/aether-kernel
(gdb) target remote :1234
(gdb) break kmain
(gdb) continue
```

## 🏗 Architecture Overview

### Scheduler — Fixed Priority, 4 Bands

```
Priority 0 [RealTime] ─── Emergency stop, obstacle avoidance ISR  (≤ 100 µs)
Priority 1 [High]     ─── Sensor fusion, navigation, IMU loop      (≤ 1 ms)
Priority 2 [Normal]   ─── Mission logic, inter-agent communications (≤ 10 ms)
Priority 3 [Low]      ─── Telemetry, logging, background tasks      (best-effort)
```

### Reflex-Bus IPC

Zero-copy message passing with static ring buffers (no heap allocation).
Each message slot is one cache line (64 bytes) with a CRC-8 checksum.

```
CHANNEL_EMERGENCY    (0) — E-Stop broadcast
CHANNEL_NAV_MISSION  (1) — Navigation → Mission planner
CHANNEL_SENSOR_NAV   (2) — Sensor fusion → Navigation
CHANNEL_TELEMETRY    (3) — Status → Radio downlink
CHANNEL_AI_INFERENCE (8) — Mojo inference results → Navigation
```

### Radiation Tolerance (TMR)

Triple Modular Redundancy for safety-critical computations:

```rust
// Any function wrapped in tmr_vote() is executed 3× and the majority wins.
// Discrepancies increment the SEU (Single Event Upset) counter.
let altitude = safety::tmr_vote(|| read_altimeter_sensor());
```

## 🗺 Roadmap

- [x] v0.1 — Bare-metal boot, UART, timer interrupt, cooperative scheduler
- [ ] v0.2 — Full preemptive context switch (assembly register save/restore)
- [ ] v0.3 — Memory protection (RISC-V PMP regions per task)
- [ ] v0.4 — Mojo compute-engine FFI + AI inference pipeline
- [ ] v0.5 — WebAssembly Vault sandbox (wasmtime / micro-wasm)
- [ ] v0.6 — Multi-hart (SMP) support + inter-hart IPC
- [ ] v0.7 — Drone swarm mesh networking over Reflex-Bus

## 📜 License

Licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
