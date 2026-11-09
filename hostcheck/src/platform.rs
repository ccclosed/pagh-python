//! Host `platform` for the interpreter core.
//!
//! This mirrors `src/platform.rs` (the libpagh implementation) with `std`, so
//! `hostcheck` can run exactly the same lexer, parser and evaluator code
//! without booting a kernel.

use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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

pub fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| error.to_string())
}

pub fn write_bytes(path: &str, data: &[u8]) -> Result<(), String> {
    std::fs::write(path, data).map_err(|error| error.to_string())
}

pub fn path_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

pub fn args() -> Vec<String> {
    std::env::args().collect()
}

// ── clock and sleeping (time module) ───────────────────────────────────────
//
// The host mirror of the ABI's two clocks: realtime is the wall clock,
// monotonic is an `Instant` (the same choice the kernel ABI makes). The
// process start time is the monotonic epoch.

/// The realtime clock in seconds since the Unix epoch.
pub fn realtime_seconds() -> Result<f64, String> {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => Ok(duration.as_secs_f64()),
        Err(error) => Err(format!("system clock before the epoch: {}", error)),
    }
}

static MONOTONIC_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// A monotonic clock in seconds. Only differences are meaningful.
pub fn monotonic_seconds() -> Result<f64, String> {
    let start = MONOTONIC_START.get_or_init(std::time::Instant::now);
    Ok(start.elapsed().as_secs_f64())
}

/// Sleep for a non-negative number of seconds.
pub fn sleep_seconds(seconds: f64) -> Result<(), String> {
    if !(seconds >= 0.0) {
        return Err(String::from("sleep length must be non-negative"));
    }
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    Ok(())
}

pub fn current_dir() -> String {
    std::env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| String::from("/"))
}

pub fn exit(code: i32) -> ! {
    std::process::exit(code)
}
fn entry_kind(metadata: &std::fs::Metadata) -> u32 {
    if metadata.file_type().is_dir() {
        1
    } else if metadata.file_type().is_symlink() {
        2
    } else if metadata.file_type().is_file() {
        0
    } else {
        3
    }
}

pub fn list_dir(path: &str) -> Result<Vec<(String, u32)>, String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry
            .file_type()
            .map(|file_type| {
                if file_type.is_dir() {
                    1
                } else if file_type.is_symlink() {
                    2
                } else if file_type.is_file() {
                    0
                } else {
                    3
                }
            })
            .unwrap_or(3);
        out.push((name, kind));
    }
    Ok(out)
}

pub fn stat_path(path: &str) -> Result<(u64, u32, u32), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let kind = entry_kind(&metadata);
    #[cfg(unix)]
    let mode = metadata.permissions().mode() & 0o7777;
    #[cfg(not(unix))]
    let mode = if metadata.permissions().readonly() { 0o444 } else { 0o644 };
    Ok((metadata.len(), mode, kind))
}

pub fn create_dir(path: &str) -> Result<(), String> {
    std::fs::create_dir(path).map_err(|error| error.to_string())
}

pub fn create_dir_all(path: &str) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| error.to_string())
}

pub fn remove_file(path: &str) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|error| error.to_string())
}

pub fn remove_dir(path: &str) -> Result<(), String> {
    std::fs::remove_dir(path).map_err(|error| error.to_string())
}

pub fn rename(from: &str, to: &str) -> Result<(), String> {
    std::fs::rename(from, to).map_err(|error| error.to_string())
}

pub fn vars() -> Vec<(String, String)> {
    std::env::vars().collect()
}

pub fn getpid() -> i64 {
    std::process::id() as i64
}

pub fn getppid() -> i64 {
    0
}

pub fn fork_process() -> Result<i64, String> {
    Err(String::from(
        "fork is only available in the pagh kernel (hostcheck has no libpagh)",
    ))
}

pub fn execv_replace(_path: &str, _arguments: &[String]) -> Result<(), String> {
    Err(String::from(
        "execv is only available in the pagh kernel (hostcheck has no libpagh)",
    ))
}

pub fn wait_child(_pid: i64, _nohang: bool) -> Result<(i64, i32), String> {
    Err(String::from(
        "waitpid is only available in the pagh kernel (hostcheck has no libpagh)",
    ))
}
