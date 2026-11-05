//! Host `platform` for the interpreter core.
//!
//! This mirrors `src/platform.rs` (the libpagh implementation) with `std`, so
//! `hostcheck` can run exactly the same lexer, parser and evaluator code
//! without booting a kernel.

use std::io::Write;

pub fn write_stdout(text: &str) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(text.as_bytes());
    let _ = lock.flush();
}

pub fn write_stderr(text: &str) {
    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    let _ = lock.write_all(text.as_bytes());
    let _ = lock.flush();
}

pub fn read_text(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| error.to_string())
}

pub fn write_text(path: &str, text: &str) -> Result<(), String> {
    std::fs::write(path, text.as_bytes()).map_err(|error| error.to_string())
}

pub fn path_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

pub fn args() -> Vec<String> {
    std::env::args().collect()
}

pub fn current_dir() -> String {
    std::env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| String::from("/"))
}

pub fn exit(code: i32) -> ! {
    std::process::exit(code)
}