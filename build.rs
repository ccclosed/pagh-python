//! Point the linker at the API repository's layout script by absolute path.
//!
//! The pattern is copied from `pagh-programs/programs/build.rs`: passing
//! `-T../pagh-api/pagh-user.ld` through `.cargo/config.toml` would depend on
//! rustc's working directory, which is not a stable contract. A build script
//! knows `CARGO_MANIFEST_DIR`, so the path is correct no matter where the build
//! is started from, and Cargo re-links when the script changes.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let script = manifest.join("../pagh-api/pagh-user.ld");
    // `canonicalize` only cleans up `..`; if the file is missing the linker's
    // own "cannot find linker script" error is clearer than a panic here.
    let script = script.canonicalize().unwrap_or(script);
    println!("cargo:rerun-if-changed={}", script.display());
    println!("cargo:rustc-link-arg=-T{}", script.display());
    println!("cargo:rerun-if-changed=build.rs");
}