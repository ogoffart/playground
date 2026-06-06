//! Build script: provide a stub `libxdo.so` so the binary links in environments where the real
//! libxdo is unavailable.
//!
//! `dioxus-desktop` pulls in `muda` / `tray-icon` / `global-hotkey`, which link against libxdo
//! unconditionally. This app uses none of those facilities (the menubar is disabled), so the
//! symbols are never called — we just need *something* named `libxdo.so` to satisfy the linker.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let src = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("stubs")
        .join("xdo_stub.c");
    println!("cargo:rerun-if-changed={}", src.display());

    let lib = out_dir.join("libxdo.so");

    // Compile the stub into a shared object named exactly `libxdo.so`.
    let status = Command::new(env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args(["-shared", "-fPIC", "-o"])
        .arg(&lib)
        .arg(&src)
        .status();

    match status {
        Ok(s) if s.success() => {
            // Put the stub first on the search path so `-lxdo` resolves to it...
            println!("cargo:rustc-link-search=native={}", out_dir.display());
            // ...and bake OUT_DIR into the binary's rpath so the dynamic loader finds the stub
            // `libxdo.so` at runtime too (it is not on the system library path).
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", out_dir.display());
        }
        _ => {
            eprintln!(
                "warning: failed to build libxdo stub; link may fail if real libxdo is absent"
            );
        }
    }
}
