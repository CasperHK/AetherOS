//! # AetherOS kernel build script
//!
//! Passes the architecture-specific linker script to `rustc` so that the
//! final ELF has the correct memory layout for bare-metal execution on the
//! QEMU RISC-V `virt` machine.

fn main() {
    // CARGO_MANIFEST_DIR is the absolute path to kernel/
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    // Inject linker script – path must be absolute so it works regardless of
    // the working directory from which `cargo build` is invoked.
    println!("cargo:rustc-link-arg=-T{manifest}/src/arch/linker.ld");

    // Re-run this build script when the linker script changes.
    println!("cargo:rerun-if-changed=src/arch/linker.ld");
    println!("cargo:rerun-if-changed=build.rs");
}
