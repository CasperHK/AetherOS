//! # Fixed-Priority Preemptive Scheduler
//!
//! ## Design
//! AetherOS uses a **fixed-priority, round-robin within a priority band**
//! scheduling policy, identical in spirit to ARINC 653 and FreeRTOS.
//!
//! ### Priority bands (highest → lowest)
//! | Band       | Index | Typical use                                      |
//! |------------|-------|--------------------------------------------------|
//! | `RealTime` |   0   | Emergency stop, obstacle-avoidance control loop  |
//! | `High`     |   1   | Sensor fusion, attitude/navigation control       |
//! | `Normal`   |   2   | Mission logic, inter-agent messaging             |
//! | `Low`      |   3   | Telemetry, logging, background maintenance       |
//!
//! The scheduler always runs the **lowest-indexed ready task**.  Within a
//! band, tasks are served in round-robin order.
//!
//! ## v0.1 limitations
//! Full hardware-assisted preemption (register save/restore at the timer ISR
//! level) requires an architecture-specific context-switch stub.  In v0.1
//! the scheduler is **cooperative**: tasks yield by calling
//! `arch::wait_for_interrupt()`, and the `on_timer_tick` callback marks the
//! current task as ready-to-yield so the next task in the same band can run
//! on the *next cooperative yield*.
//!
//! The data structures and API are already designed for full preemption;
//! upgrading to full context-switch only requires wiring the
//! `context_switch(from, to)` stub in the timer ISR.

use aether_common::{KernelError, Priority, TaskId, TaskState, MAX_TASKS, PRIORITY_LEVELS};

use crate::sync::SpinLock;
use crate::kprintln;

// ── Task Control Block ────────────────────────────────────────────────────────

/// A Task Control Block (TCB) stores everything the kernel needs to manage
/// and resume a task.
#[repr(C)]
pub struct TaskControlBlock {
    /// Globally unique task identifier (assigned at spawn; 0 = idle).
    pub id: TaskId,

    /// Hardware priority level.  Immutable after spawn in v0.1.
    pub priority: Priority,

    /// Current lifecycle state.
    pub state: TaskState,

    /// Saved stack pointer — updated by the context-switch stub before the
    /// task is de-scheduled.  In v0.1 this is only written at spawn time.
    pub stack_ptr: usize,

    /// Task entry-point function.  Must never return.
    pub entry: fn() -> !,

    /// Number of radiation-induced fault events detected for this task.
    /// Incremented by the TMR checker; triggers task restart above threshold.
    pub fault_count: u32,

    /// Accumulated tick count while this task has been in `Running` state.
    pub run_ticks: u64,
}

// ── Scheduler state ───────────────────────────────────────────────────────────

/// Convenience: `None`-filled TCB placeholder for array initialisation.
const EMPTY_TCB: Option<TaskControlBlock> = None;

/// Static task table.  MAX_TASKS slots total across all priority bands.
static TASK_TABLE: SpinLock<[Option<TaskControlBlock>; MAX_TASKS]> =
    SpinLock::new([EMPTY_TCB; MAX_TASKS]);

/// Per-priority-band "next round-robin index" cursor.
/// `RR_CURSOR[p]` is the slot index within TASK_TABLE to try next for band `p`.
static RR_CURSOR: SpinLock<[usize; PRIORITY_LEVELS]> =
    SpinLock::new([0usize; PRIORITY_LEVELS]);

/// ID counter for new tasks (never wraps in practice for embedded lifetimes).
static NEXT_TASK_ID: crate::sync::SpinLock<TaskId> = crate::sync::SpinLock::new(1);

/// Index of the task that is currently executing (or `MAX_TASKS` = none).
static CURRENT_TASK: SpinLock<usize> = SpinLock::new(MAX_TASKS);

// ── Public API ────────────────────────────────────────────────────────────────

/// Initialise the scheduler data structures.
///
/// Must be called once before any calls to `spawn` or `start`.
pub fn init() {
    // Everything is already zero-/None-initialised via BSS; just log.
    kprintln!("scheduler: initialized [{} priority levels, {} task slots]",
              PRIORITY_LEVELS, MAX_TASKS);
}

/// Spawn a new task at the given priority.
///
/// The task entry function must **never return** (it loops internally or
/// calls `crate::safety::emergency_stop`).
///
/// Returns the assigned `TaskId` on success.
pub fn spawn(priority: Priority, entry: fn() -> !) -> Result<TaskId, KernelError> {
    let id = {
        let mut id_guard = NEXT_TASK_ID.lock();
        let id = *id_guard;
        *id_guard = id.wrapping_add(1);
        id
    };

    let mut table = TASK_TABLE.lock();
    for slot in table.iter_mut() {
        if slot.is_none() {
            *slot = Some(TaskControlBlock {
                id,
                priority,
                state: TaskState::Ready,
                stack_ptr: 0,   // set by context-switch stub when first run
                entry,
                fault_count: 0,
                run_ticks: 0,
            });
            kprintln!("scheduler: spawned task {} {} [{}]",
                      priority.tag(), id, priority.as_index());
            return Ok(id);
        }
    }
    Err(KernelError::OutOfResources)
}

/// Mark a task as blocked (waiting for IPC / timer).
pub fn block_task(id: TaskId) {
    let mut table = TASK_TABLE.lock();
    for slot in table.iter_mut().flatten() {
        if slot.id == id {
            slot.state = TaskState::Blocked;
            return;
        }
    }
}

/// Wake a previously blocked task, making it ready to run again.
pub fn wake_task(id: TaskId) {
    let mut table = TASK_TABLE.lock();
    for slot in table.iter_mut().flatten() {
        if slot.id == id && slot.state == TaskState::Blocked {
            slot.state = TaskState::Ready;
            return;
        }
    }
}

/// Called by the timer ISR on every tick.
///
/// In v0.1 this only updates accounting and prints a periodic heartbeat.
/// In a full implementation it would also check for time-slice expiry and
/// trigger a context switch to the next ready task.
pub fn on_timer_tick(tick: u64) {
    // Accumulate ticks for the currently running task.
    let cur_idx = *CURRENT_TASK.lock();
    if cur_idx < MAX_TASKS {
        if let Some(ref mut tcb) = TASK_TABLE.lock()[cur_idx] {
            tcb.run_ticks = tcb.run_ticks.wrapping_add(1);
        }
    }

    // Print a heartbeat every second (SCHEDULER_HZ ticks).
    if tick % crate::arch::SCHEDULER_HZ == 0 {
        let secs = tick / crate::arch::SCHEDULER_HZ;
        kprintln!("[tick {}] uptime: {} s", tick, secs);
    }
}

/// Start the scheduler: enable interrupts and enter the cooperative main loop.
///
/// This function never returns.
pub fn start() -> ! {
    kprintln!("scheduler: starting — enabling interrupts");
    crate::arch::enable_interrupts();

    // ── Cooperative main loop ────────────────────────────────────────────
    // Pick the highest-priority ready task, "run" it (call its entry fn),
    // then yield back here.  In a full preemptive implementation the timer
    // ISR would perform the context switch directly and this loop would
    // become a simple idle loop.
    loop {
        let next = find_next_ready_task();
        match next {
            Some(idx) => run_task(idx),
            None => {
                // No ready task — enter low-power wait.
                crate::arch::wait_for_interrupt();
            }
        }
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Find the index of the highest-priority ready task using round-robin
/// tie-breaking within each priority band.
fn find_next_ready_task() -> Option<usize> {
    let table = TASK_TABLE.lock();
    let mut cursors = RR_CURSOR.lock();

    for prio in 0..PRIORITY_LEVELS {
        let priority = Priority::from_u8(prio as u8);
        let start = cursors[prio];

        for i in 0..MAX_TASKS {
            let idx = (start + i) % MAX_TASKS;
            if let Some(ref tcb) = table[idx] {
                if tcb.priority == priority && tcb.state == TaskState::Ready {
                    // Advance the round-robin cursor for next call.
                    cursors[prio] = (idx + 1) % MAX_TASKS;
                    return Some(idx);
                }
            }
        }
    }
    None
}

/// "Run" a task: mark it as Running, call its entry function.
///
/// In v0.1 this is cooperative — the task runs until it calls
/// `wait_for_interrupt()` which returns after an ISR.  A full preemptive
/// implementation would restore the task's saved register context here
/// via an assembly `context_restore` stub.
fn run_task(idx: usize) {
    let entry_fn;
    {
        let mut table = TASK_TABLE.lock();
        let mut cur = CURRENT_TASK.lock();
        if let Some(ref mut tcb) = table[idx] {
            tcb.state = TaskState::Running;
            entry_fn = tcb.entry;
            *cur = idx;
        } else {
            return;
        }
    }
    // Call the task entry.  The task is expected to loop internally and
    // call `wait_for_interrupt()` to yield.
    // TODO(v0.2): replace this direct call with an assembly context-restore
    //             that switches the stack pointer and jumps to the saved PC.
    (entry_fn)();
}
