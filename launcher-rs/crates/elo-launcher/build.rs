//! Compile the Windows icon resource into this binary.
//!
//! # Why this is hand-rolled and not `winres`
//!
//! The workspace manifest makes the dependency tree a budget and an operator
//! sentence about supply-chain attacks (`tests/supply_chain.rs` counts it).
//! `winres`/`embed-resource` are good crates that do exactly this, and they
//! cost packages for work that is one subprocess: hand a `.rc` to the resource
//! compiler the target already needs, then hand the object to the linker.
//! Nothing below imports anything outside `std`.
//!
//! # ⚠ It never fails the build
//!
//! An icon is decoration and a launcher that refused to compile over one would
//! be trading the program for its picture — the same call `chrome`'s icon code
//! already makes at runtime. A missing resource compiler prints a warning and
//! produces a binary with the system default icon, which is exactly what
//! shipped before this file existed.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("assets");
    let rc = assets.join("elo.rc");
    let ico = assets.join("elo.ico");
    println!("cargo:rerun-if-changed={}", rc.display());
    println!("cargo:rerun-if-changed={}", ico.display());

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap())
        .join(if msvc { "elo.res" } else { "elo.o" });

    // windres emits a COFF object the GNU linker takes directly; rc.exe and
    // llvm-rc emit a .res the MSVC linker takes directly. Either way the
    // linker argument is just the output path.
    let tools: &[(&str, Vec<String>)] = &if msvc {
        vec![
            ("rc.exe", vec!["/nologo".into(), "/fo".into(), out.display().to_string(), rc.display().to_string()]),
            ("llvm-rc", vec![format!("/fo{}", out.display()), rc.display().to_string()]),
        ]
    } else {
        vec![
            ("x86_64-w64-mingw32-windres", vec![rc.display().to_string(), "-O".into(), "coff".into(), "-o".into(), out.display().to_string()]),
            ("windres", vec![rc.display().to_string(), "-O".into(), "coff".into(), "-o".into(), out.display().to_string()]),
        ]
    };

    for (tool, args) in tools {
        // The .rc names its icon by a bare filename, so the compiler has to be
        // run WITH assets/ as the working directory or it cannot find elo.ico.
        match Command::new(tool).args(args).current_dir(&assets).status() {
            Ok(s) if s.success() => {
                println!("cargo:rustc-link-arg-bins={}", out.display());
                return;
            }
            Ok(s) => println!("cargo:warning={tool} failed ({s}) — binary gets the default icon"),
            Err(_) => continue, // not installed; try the next one
        }
    }
    println!("cargo:warning=no resource compiler (rc.exe, llvm-rc or windres) — \
              the Windows binary gets the system default icon");
}
