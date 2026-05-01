#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# run.sh — Build AetherOS and launch it inside QEMU
#
# Usage:
#   ./run.sh           # build debug, run in QEMU
#   ./run.sh --release # build release, run in QEMU
#   ./run.sh --gdb     # build debug, start QEMU with GDB server on :1234
#   ./run.sh --help    # show this help
#
# Requirements:
#   rustup (nightly + riscv64gc-unknown-none-elf target installed)
#   qemu-system-riscv64
#
# Install QEMU on Ubuntu/Debian:
#   sudo apt install qemu-system-misc
# Install QEMU on macOS:
#   brew install qemu
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

KERNEL_ELF=""
BUILD_MODE="debug"
GDB_MODE=0

# ── Parse arguments ────────────────────────────────────────────────────────────
for arg in "$@"; do
    case "$arg" in
        --release) BUILD_MODE="release" ;;
        --gdb)     GDB_MODE=1 ;;
        --help|-h)
            sed -n '2,20p' "$0"   # print the header comment
            exit 0
            ;;
        *) echo "Unknown argument: $arg" >&2; exit 1 ;;
    esac
done

# ── Build ──────────────────────────────────────────────────────────────────────
echo ">>> Building AetherOS kernel [${BUILD_MODE}]..."

if [ "$BUILD_MODE" = "release" ]; then
    cargo build --package aether-kernel --release
    KERNEL_ELF="target/riscv64gc-unknown-none-elf/release/aether-kernel"
else
    cargo build --package aether-kernel
    KERNEL_ELF="target/riscv64gc-unknown-none-elf/debug/aether-kernel"
fi

echo ">>> Kernel ELF: ${KERNEL_ELF}"
echo ""

# ── QEMU flags ─────────────────────────────────────────────────────────────────
QEMU_ARGS=(
    -machine virt           # Generic RISC-V virtual board
    -nographic              # Redirect serial to stdout (Ctrl+A X to exit)
    -bios none              # Skip OpenSBI; jump directly to our kernel
    -kernel "${KERNEL_ELF}" # Load kernel ELF

    # Memory: 128 MiB (matches linker script RAM region)
    -m 128M

    # Serial: map UART0 to stdio (default for -nographic, but explicit here)
    -serial stdio

    # CPU: single hart, RV64GC
    -cpu rv64
)

if [ "$GDB_MODE" = "1" ]; then
    QEMU_ARGS+=( -s -S )   # -s = GDB server on :1234, -S = freeze at start
    echo ">>> GDB mode: start GDB with:"
    echo "    riscv64-unknown-elf-gdb ${KERNEL_ELF}"
    echo "    (gdb) target remote :1234"
    echo "    (gdb) continue"
    echo ""
fi

echo ">>> Launching QEMU... (press Ctrl+A X to exit)"
echo "─────────────────────────────────────────────────────────────────"
exec qemu-system-riscv64 "${QEMU_ARGS[@]}"
