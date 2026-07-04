//! Build the native I/O shim and generate Rust bindings for it.
//!
//! Two steps run at build time, forming the FFI bridge this crate exists to
//! demonstrate:
//!
//! 1. [`cc`] compiles `src/hpc_io.c` into `libhpc_io.a` and arranges for it to
//!    be linked statically into the crate.
//! 2. [`bindgen`] reads `src/hpc_io.h` and emits typed `extern "C"`
//!    declarations into `$OUT_DIR/bindings.rs`, which `src/lib.rs` `include!`s.
//!
//! So the layering is: hand-written safe Rust (lib.rs) → machine-generated
//! declarations (bindings.rs) → raw POSIX syscalls in C (hpc_io.c).
//!
//! Panicking here is intentional and idiomatic: a build script has no useful
//! way to recover from a missing C toolchain or `libclang`, and a panic gives
//! Cargo a clean, actionable error.

use std::env;
use std::path::PathBuf;

fn main() {
    let src = "src/hpc_io.c";
    let header = "src/hpc_io.h";
    println!("cargo:rerun-if-changed={src}");
    println!("cargo:rerun-if-changed={header}");

    // 1. Compile the C shim into a static lib and link it into this crate.
    cc::Build::new()
        .file(src)
        .warnings(true)
        .flag_if_supported("-Wextra")
        .compile("hpc_io");

    // 2. Generate Rust bindings from the header. We allow-list only our own
    //    `hpc_*` symbols so the binding surface stays tiny and stable.
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is always set by Cargo"));
    let bindings = bindgen::Builder::default()
        .header(header)
        .allowlist_function("hpc_.*")
        .generate_comments(true)
        .layout_tests(false)
        .generate()
        .expect("bindgen failed to generate bindings for src/hpc_io.h");
    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("failed to write generated bindings to OUT_DIR");
}
