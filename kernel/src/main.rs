//! # AetherOS — Real-Time Neural Microkernel
//!
//! ```text
//! ╔══════════════════════════════════════════════════════════════╗
//! ║  AetherOS  —  Real-Time Neural Microkernel                   ║
//! ║  Target : RISC-V 64-bit (riscv64gc-unknown-none-elf)         ║
//! ║  Mission : Drones · Ground Robots · Satellites · Spacecraft  ║
//! ╚══════════════════════════════════════════════════════════════╝
//! ```
//!
//! ## Boot sequence
//! ```text
//! QEMU loads ELF → _start (arch/riscv64.rs asm)
//!   1. disable interrupts
//!   2. set up kernel stack (_stack_top from linker.ld)
//!   3. zero BSS
//!   4. call kmain()  ←── this file
//!        ├─ arch::early_init()    (UART, CLINT, trap vector)
//!        ├─ safety::init()        (TMR self-test, E-Stop arm)
//!        ├─ scheduler::init()     (task table clear)
//!        ├─ ipc::init()           (Reflex-Bus channel table)
//!        ├─ spawn initial tasks   (RT, High, Normal, Low)
//!        └─ scheduler::start()   (enable interrupts → never returns)
//! ```
//!
//! ## Module map
//! | Module        | Responsibility                                      |
//! |---------------|-----------------------------------------------------|
//! | `arch`        | Hardware abstraction (UART, timer, trap, halt)      |
//! | `scheduler`   | Fixed-priority round-robin task scheduler           |
//! | `ipc`         | Reflex-Bus zero-copy message passing                |
//! | `safety`      | Panic handler, TMR voter, emergency stop            |
//! | `sync`        | `SpinLock<T>` — the only synchronisation primitive  |
//! | `console`     | `kprint!` / `kprintln!` macros                      |

#![no_std]
#![no_main]
// Allow unused items during early development (they document future API).
#![allow(dead_code)]

// ── Module declarations ───────────────────────────────────────────────────────

pub mod arch;
pub mod console;
pub mod ipc;
pub mod safety;
pub mod scheduler;
pub mod sync;

// ── External crate ────────────────────────────────────────────────────────────

use aether_common::Priority;

// ── Kernel main ───────────────────────────────────────────────────────────────

/// Kernel C-ABI entry point — called by the `_start` assembly stub in
/// `arch/riscv64.rs` after BSS is zeroed and the stack is set up.
///
/// This function initialises every kernel subsystem and then starts the
/// scheduler, which **never returns**.
#[no_mangle]
pub extern "C" fn kmain() -> ! {
    // ── 1. Architecture initialisation ──────────────────────────────────
    // Sets up: UART baud, CLINT timer, machine trap vector.
    // After this call `kprintln!` is safe to use.
    arch::early_init();

    // ── 2. Boot banner ───────────────────────────────────────────────────
    print_banner();

    // ── 3. Safety subsystem ──────────────────────────────────────────────
    safety::init();

    // ── 4. Scheduler ─────────────────────────────────────────────────────
    scheduler::init();

    // ── 5. Reflex-Bus IPC ─────────────────────────────────────────────────
    ipc::init();

    kprintln!();
    kprintln!("Spawning initial kernel tasks...");

    // ── 6. Spawn tasks ───────────────────────────────────────────────────
    // Priority::RealTime — emergency watchdog (never preempted by lower tiers)
    scheduler::spawn(Priority::RealTime, emergency_watchdog_task)
        .expect("failed to spawn RT watchdog");

    // Priority::High — navigation / attitude control loop
    scheduler::spawn(Priority::High, navigation_task)
        .expect("failed to spawn navigation task");

    // Priority::Normal — mission planner / inter-agent comms
    scheduler::spawn(Priority::Normal, mission_manager_task)
        .expect("failed to spawn mission manager");

    // Priority::Low — telemetry downlink / logging
    scheduler::spawn(Priority::Low, telemetry_task)
        .expect("failed to spawn telemetry task");

    kprintln!();
    kprintln!("All systems nominal.  Transferring control to scheduler.");
    kprintln!("─────────────────────────────────────────────────────────");

    // ── 7. Start scheduler (never returns) ───────────────────────────────
    scheduler::start()
}

// ── Initial kernel tasks ──────────────────────────────────────────────────────

/// [RT] Emergency watchdog task.
///
/// Runs at the highest priority.  On every iteration it:
/// - Monitors the Reflex-Bus `CHANNEL_EMERGENCY` for stop signals.
/// - Checks the SEU counter; if it exceeds a threshold it triggers safe mode.
///
/// In a production system this task would also feed a hardware watchdog timer.
fn emergency_watchdog_task() -> ! {
    kprintln!("{} emergency watchdog: armed and running", Priority::RealTime.tag());

    loop {
        // Check for emergency-stop messages from other tasks.
        if let Ok(Some(msg)) = ipc::recv(ipc::CHANNEL_EMERGENCY) {
            if msg.urgent {
                safety::emergency_stop("E-Stop received on Reflex-Bus");
            }
        }

        // Monitor radiation fault counter.
        // Threshold: 10 SEU events → too many upsets to trust execution.
        if safety::seu_count() >= 10 {
            safety::emergency_stop("SEU threshold exceeded — unsafe to continue");
        }

        // Yield until the next timer tick.
        arch::wait_for_interrupt();
    }
}

/// [HI] Navigation / attitude control task.
///
/// In a real system this would run the IMU fusion loop and compute
/// attitude estimates at ≥ 500 Hz using sensor data from the Reflex-Bus.
fn navigation_task() -> ! {
    kprintln!("{} navigation: sensor fusion online", Priority::High.tag());

    loop {
        // TODO(v0.2): read IMU data from CHANNEL_SENSOR_NAV, run Kalman filter,
        //             publish attitude estimate to CHANNEL_NAV_MISSION.

        // Demonstrate Reflex-Bus usage: peek at AI inference channel.
        if ipc::has_pending(ipc::CHANNEL_AI_INFERENCE) {
            if let Ok(Some(_ai_msg)) = ipc::recv(ipc::CHANNEL_AI_INFERENCE) {
                // TODO: feed neural-net obstacle map into path planner
            }
        }

        arch::wait_for_interrupt();
    }
}

/// [NR] Mission manager task.
///
/// Handles high-level mission logic: waypoint sequencing, swarm coordination,
/// and command/control message processing.
fn mission_manager_task() -> ! {
    kprintln!("{} mission manager: online", Priority::Normal.tag());

    loop {
        // TODO(v0.2): process mission commands, advance waypoint sequence.

        arch::wait_for_interrupt();
    }
}

/// [LO] Telemetry task.
///
/// Periodically packages system status into a telemetry frame and sends it
/// on CHANNEL_TELEMETRY for the radio driver to downlink.
fn telemetry_task() -> ! {
    kprintln!("{} telemetry: downlink active", Priority::Low.tag());

    let mut last_tick = 0u64;

    loop {
        let now = arch::kernel_ticks();

        // Transmit a status frame roughly every 5 seconds.
        if now.wrapping_sub(last_tick) >= 5 * arch::SCHEDULER_HZ {
            last_tick = now;

            // Build a minimal telemetry payload.
            let mut payload = [0u8; aether_common::IPC_PAYLOAD_SIZE];
            let tick_bytes = now.to_le_bytes();
            payload[..8].copy_from_slice(&tick_bytes);
            let seu = safety::seu_count().to_le_bytes();
            payload[8..12].copy_from_slice(&seu);

            let msg = ipc::Message::new(0, 0x0001 /* STATUS_FRAME */, payload);
            let _ = ipc::send(ipc::CHANNEL_TELEMETRY, msg);
        }

        arch::wait_for_interrupt();
    }
}

// ── Banner ────────────────────────────────────────────────────────────────────

fn print_banner() {
    kprintln!();
    kprintln!("╔══════════════════════════════════════════════════════════╗");
    kprintln!("║          AetherOS  v{}                              ║", env!("CARGO_PKG_VERSION"));
    kprintln!("║       Real-Time Neural Microkernel                       ║");
    kprintln!("║  Drones · Ground Robots · Satellites · Deep-Space Craft  ║");
    kprintln!("╚══════════════════════════════════════════════════════════╝");
    kprintln!();
    kprintln!("  Target  : RISC-V 64-bit (riscv64gc-unknown-none-elf)");
    kprintln!("  Build   : {} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    kprintln!("  Timer   : {} Hz scheduler, {} Hz CLINT",
              arch::SCHEDULER_HZ, arch::TIMER_FREQ_HZ);
    kprintln!();
}
