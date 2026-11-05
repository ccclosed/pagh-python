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

use alloc::string::String;
use alloc::vec::Vec;

use alloc::format;

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

/// Create or truncate `path` and write `text` as UTF-8.
pub fn write_text(path: &str, text: &str) -> Result<(), String> {
    libpagh::fs::write(path, text.as_bytes()).map_err(|error| format!("{}", error))
}

/// Whether `path` can be stat'ed. Used to decide whether a module file exists
/// without turning a probe into an error in the caller.
pub fn path_exists(path: &str) -> bool {
    libpagh::fs::stat(path).is_ok()
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