//! Errors: syntax errors from the front end, Python exceptions at run time,
//! and the traceback snapshot attached to an escaping exception.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

/// A lexer or parser error. `line` and `column` are 1-based character
/// positions; `kind` is normally `"SyntaxError"` and can be
/// `"IndentationError"` for indentation-stack problems.
pub struct SyntaxError {
    pub kind: &'static str,
    pub message: String,
    pub line: u32,
    pub column: u32,
}

impl SyntaxError {
    pub fn new(message: String, line: u32, column: u32) -> SyntaxError {
        SyntaxError {
            kind: "SyntaxError",
            message,
            line,
            column,
        }
    }

    pub fn indentation(message: String, line: u32, column: u32) -> SyntaxError {
        SyntaxError {
            kind: "IndentationError",
            message,
            line,
            column,
        }
    }
}

/// The name of the exception class an error carries.
///
/// Builtin errors keep a static string (no allocation on the common path);
/// `raise MyError("...")` carries the class's own `Rc<str>` name.
#[derive(Clone)]
pub enum ErrorName {
    Builtin(&'static str),
    Dynamic(Rc<str>),
}

impl ErrorName {
    pub fn as_str(&self) -> &str {
        match self {
            ErrorName::Builtin(name) => name,
            ErrorName::Dynamic(name) => name,
        }
    }
}

/// One line of a Python traceback, outermost frame first.
#[derive(Clone)]
pub struct TraceFrame {
    pub filename: Rc<str>,
    pub name: Rc<str>,
    pub line: u32,
}

/// A Python exception travelling through the evaluator.
///
/// `frames` is a snapshot taken when the error is first raised, so unwinding
/// (and `try` handlers) cannot erase the traceback. `value` is the exception
/// instance when the user raised one; builtin errors construct a fresh
/// instance only if a handler binds it with `as`.
#[derive(Clone)]
pub struct PyError {
    pub name: ErrorName,
    pub message: String,
    pub frames: Vec<TraceFrame>,
    /// The exception value (class instance) when one exists.
    pub value: Option<crate::value::Value>,
    /// Re-raised marker used by a bare `raise` in a handler.
    #[allow(dead_code)]
    pub caught: bool,
}

impl PyError {
    pub fn new(name: ErrorName, message: String) -> PyError {
        PyError {
            name,
            message,
            frames: Vec::new(),
            value: None,
            caught: false,
        }
    }

    pub fn builtin(name: &'static str, message: String) -> PyError {
        PyError::new(ErrorName::Builtin(name), message)
    }

    /// The exception class name as it appears after the traceback.
    pub fn type_name(&self) -> &str {
        self.name.as_str()
    }
}

pub type PyResult<T> = Result<T, PyError>;

/// Build a builtin exception with a `&str` message.
#[allow(dead_code)]
pub fn error(name: &'static str, message: &str) -> PyError {
    PyError::builtin(name, String::from(message))
}

/// Build a builtin exception with an owned/formatted message.
#[allow(dead_code)]
pub fn error_string(name: &'static str, message: String) -> PyError {
    PyError::builtin(name, message)
}
