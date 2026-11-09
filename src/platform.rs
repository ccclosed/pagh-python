//! The only layer that touches the operating system.
//!
//! The interpreter itself (`lexer`..`cli`) is plain `alloc` code with no
//! `libpagh` calls. Keeping the boundary this small has two payoffs: the same
//! modules compile on the host for fast tests (`hostcheck` supplies a `std`
//! implementation of this module), and every kernel interaction is one of the
//! functions below instead of being sprinkled through the evaluator.
//!
//! Text is UTF-8; the paigh ABI has no locale and no binary stdio mode. The
//! only errors surfaced here are short human-readable strings; callers wrap
//! them in `OSError`/`FileNotFoundError`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Write a string to standard output (fd 1), ignoring a write error: there is
/// nowhere sensible to report it from inside `print`.
pub fn write_stdout(text: &str) {
    libpagh::print::_print(format_args!("{}", text));
}

/// Write a string to standard error (fd 2).
pub fn write_stderr(text: &str) {
    libpagh::print::_eprint(format_args!("{}", text));
}

/// Read a whole file as UTF-8. The error is the OS error's display text.
pub fn read_text(path: &str) -> Result<String, String> {
    libpagh::fs::read_to_string(path).map_err(|error| format!("{}", error))
}

/// Read a whole file as raw bytes.
pub fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    libpagh::fs::read_to_end(path).map_err(|error| format!("{}", error))
}

/// Create or truncate `path` and write raw bytes.
pub fn write_bytes(path: &str, data: &[u8]) -> Result<(), String> {
    libpagh::fs::write(path, data).map_err(|error| format!("{}", error))
}

/// Whether `path` exists. `stat` opens its target read-only and this kernel
/// rejects that for directories, so a directory is probed with `getdents64`.
pub fn path_exists(path: &str) -> bool {
    libpagh::fs::stat(path).is_ok() || list_dir(path).is_ok()
}

/// The process arguments, `argv[0]` first.
pub fn args() -> Vec<String> {
    libpagh::env::args().collect()
}

/// The current working directory, or `/` when the kernel did not provide one.
pub fn current_dir() -> String {
    libpagh::env::current_dir()
}

/// Terminate the process with `code`.
pub fn exit(code: i32) -> ! {
    libpagh::sys::exit(code)
}
// ── filesystem metadata and mutation (os module) ───────────────────────────

/// Directory entries as `(name, ABI kind)`, excluding `.` and `..` the way
/// `os.listdir` does.
pub fn list_dir(path: &str) -> Result<Vec<(String, u32)>, String> {
    let directory = libpagh::fs::Dir::open(path).map_err(|error| format!("{}", error))?;
    let mut out = Vec::new();
    for entry in directory {
        let entry = entry.map_err(|error| format!("{}", error))?;
        let name = entry.name();
        if name == "." || name == ".." {
            continue;
        }
        out.push((String::from(name), entry.kind().raw()));
    }
    Ok(out)
}

/// `(size, mode, ABI kind)` for `os.stat`. A directory cannot be `fstat`ed
/// through `libpagh::fs::stat` here, so it is detected with a directory probe
/// and reported as kind 1 with size 0 (the ABI has no directory size).
pub fn stat_path(path: &str) -> Result<(u64, u32, u32), String> {
    match libpagh::fs::stat(path) {
        Ok(stat) => Ok((stat.size, stat.mode, stat.kind)),
        Err(error) => {
            if list_dir(path).is_ok() {
                Ok((0, 0o755, 1))
            } else {
                Err(format!("{}", error))
            }
        }
    }
}

pub fn create_dir(path: &str) -> Result<(), String> {
    libpagh::fs::create_dir(path).map_err(|error| format!("{}", error))
}

/// Create `path` and every missing parent.
///
/// `libpagh::fs::create_dir_all` stats each existing component, and this
/// kernel's `stat` opens the target read-only, which fails for directories
/// (`-ENOTSUP`). The listdir probe below asks the one question that matters —
/// "is this already a directory?" — through `getdents64`, which the kernel
/// does support for directories.
pub fn create_dir_all(path: &str) -> Result<(), String> {
    let absolute = libpagh::path::resolve(path);
    if absolute == "/" {
        return Ok(());
    }
    let mut prefix = String::new();
    for component in absolute.split('/').filter(|part| !part.is_empty()) {
        prefix.push('/');
        prefix.push_str(component);
        match libpagh::fs::create_dir(&prefix) {
            Ok(()) => continue,
            Err(error) => {
                if list_dir(&prefix).is_ok() {
                    continue;
                }
                return Err(format!("cannot create '{}': {}", prefix, error));
            }
        }
    }
    Ok(())
}

pub fn remove_file(path: &str) -> Result<(), String> {
    libpagh::fs::remove_file(path).map_err(|error| format!("{}", error))
}

pub fn remove_dir(path: &str) -> Result<(), String> {
    libpagh::fs::remove_dir(path).map_err(|error| format!("{}", error))
}

pub fn rename(from: &str, to: &str) -> Result<(), String> {
    libpagh::fs::rename(from, to).map_err(|error| format!("{}", error))
}

/// The process environment as `(key, value)` pairs.
pub fn vars() -> Vec<(String, String)> {
    libpagh::env::vars().collect()
}

// ── clock and sleeping (time module) ───────────────────────────────────────
//
// ABI clocks: 0 is the realtime/uptime clock, 1 is monotonic. Both report
// `(seconds, nanoseconds)`; the interpreter converts to a float second count.

/// The realtime clock in seconds since its epoch.
pub fn realtime_seconds() -> Result<f64, String> {
    let mut ts = libpagh::abi::Timespec::default();
    libpagh::sys::clock_gettime(libpagh::sys::CLOCK_REALTIME, &mut ts)
        .map_err(|error| format!("clock_gettime: {}", error))?;
    Ok(ts.seconds as f64 + ts.nanos as f64 / 1_000_000_000.0)
}

/// The monotonic clock in seconds.
pub fn monotonic_seconds() -> Result<f64, String> {
    let mut ts = libpagh::abi::Timespec::default();
    libpagh::sys::clock_gettime(libpagh::sys::CLOCK_MONOTONIC, &mut ts)
        .map_err(|error| format!("clock_gettime: {}", error))?;
    Ok(ts.seconds as f64 + ts.nanos as f64 / 1_000_000_000.0)
}

/// `nanosleep` for a non-negative number of seconds.
pub fn sleep_seconds(seconds: f64) -> Result<(), String> {
    if !(seconds >= 0.0) {
        return Err(String::from("sleep length must be non-negative"));
    }
    let total = seconds * 1_000_000_000.0;
    // Saturate rather than wrap: an absurd sleep is still a sleep, and the
    // kernel clamps what it can.
    let nanos = if total >= 9_000_000_000_000_000_000.0 {
        9_000_000_000_000_000_000u64
    } else {
        total as u64
    };
    let request = libpagh::abi::Timespec {
        seconds: (nanos / 1_000_000_000) as i64,
        nanos: (nanos % 1_000_000_000) as i64,
    };
    libpagh::sys::nanosleep(&request).map_err(|error| format!("nanosleep: {}", error))
}

// ── processes (os module) ──────────────────────────────────────────────────

pub fn getpid() -> i64 {
    libpagh::sys::getpid().map(|pid| pid as i64).unwrap_or(-1)
}

pub fn getppid() -> i64 {
    libpagh::sys::getppid().map(|pid| pid as i64).unwrap_or(-1)
}

/// `fork(2)`: 0 in the child, the child pid in the parent.
pub fn fork_process() -> Result<i64, String> {
    libpagh::sys::fork()
        .map(|pid| pid as i64)
        .map_err(|error| format!("{}", error))
}

/// Replace this process image. Returns only on error (the kernel `execve`
/// contract: success never returns).
pub fn execv_replace(path: &str, arguments: &[String]) -> Result<(), String> {
    let resolved = libpagh::path::resolve(path);
    let mut argv: Vec<&str> = Vec::with_capacity(arguments.len());
    for argument in arguments {
        argv.push(argument.as_str());
    }
    let mut environment: Vec<(String, String)> = vars();
    if !environment.iter().any(|(key, _)| key == "PWD") {
        environment.push((String::from("PWD"), current_dir()));
    }
    let mut envp: Vec<String> = Vec::with_capacity(environment.len());
    for (key, value) in environment {
        envp.push(format!("{}={}", key, value));
    }
    let mut env_refs: Vec<&str> = Vec::with_capacity(envp.len());
    for entry in &envp {
        env_refs.push(entry.as_str());
    }
    libpagh::sys::execve(&resolved, &argv, &env_refs).map_err(|error| format!("{}", error))
}

/// `wait4`: `(pid, raw_status)`, or `(0, 0)` with `nohang` and no zombie.
pub fn wait_child(pid: i64, nohang: bool) -> Result<(i64, i32), String> {
    let options = if nohang { libpagh::sys::WNOHANG } else { 0 };
    libpagh::sys::wait4(pid as i32, options)
        .map(|(child, status)| (child as i64, status))
        .map_err(|error| format!("{}", error))
}
