//! Runtime values and the object model.
//!
//! Values are cheap to clone: scalars are inline and every heap object is
//! behind `Rc` (with `RefCell` only where Python semantics need mutation).
//! This matters on pagh: the kernel gives a process a fixed 1 MiB bump heap
//! that is never reclaimed, so frame recycling in `interp.rs` and shared
//! strings in the AST keep small programs well inside it.
//!
//! Dicts are insertion-ordered `Vec<(key, value)>` pairs. A linear lookup is
//! O(n), which is the right tradeoff here: it preserves Python's iteration
//! order with no hash function, no collision handling and no extra allocation,
//! and scripts on this ABI are small.

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt::Write;

use crate::ast::{CompGen, Expr, Stmt};

/// A lexical environment. Function calls get one of these; the module scope
/// and class bodies get their own.
pub struct Env {
    pub vars: Vec<(Rc<str>, Value)>,
    pub parent: Option<EnvRef>,
    /// Names declared `global` in this function frame.
    pub globals: Vec<Rc<str>>,
    /// Names declared `nonlocal` in this function frame.
    pub nonlocals: Vec<Rc<str>>,
}

pub type EnvRef = Rc<RefCell<Env>>;

impl Env {
    pub fn new(parent: Option<EnvRef>) -> EnvRef {
        Rc::new(RefCell::new(Env {
            vars: Vec::new(),
            parent,
            globals: Vec::new(),
            nonlocals: Vec::new(),
        }))
    }

    pub fn get_local(&self, name: &str) -> Option<Value> {
        self.vars
            .iter()
            .rev()
            .find(|(key, _)| &**key == name)
            .map(|(_, value)| value.clone())
    }

    /// Set an existing slot or append a new one. Returns whether the name was
    /// already present (used to decide whether to grow the Vec).
    pub fn set_local(&mut self, name: Rc<str>, value: Value) -> bool {
        for (key, slot) in self.vars.iter_mut().rev() {
            if *key == name {
                *slot = value;
                return true;
            }
        }
        self.vars.push((name, value));
        false
    }

    pub fn clear_for_pool(&mut self) {
        self.vars.clear();
        self.globals.clear();
        self.nonlocals.clear();
        self.parent = None;
    }
}

/// Look a name up through the lexical chain. Builtins are consulted by the
/// interpreter after this returns `None`; the module environment is the root
/// of the chain.
pub fn env_lookup(env: &EnvRef, name: &str) -> Option<Value> {
    let mut current = Some(env.clone());
    while let Some(frame) = current {
        let borrowed = frame.borrow();
        if let Some(value) = borrowed.get_local(name) {
            return Some(value);
        }
        current = borrowed.parent.clone();
    }
    None
}

/// Find the frame that owns `name` for a `nonlocal` assignment: the nearest
/// enclosing function frame that already has the variable. The module and
/// class scopes are not valid `nonlocal` targets in Python.
pub fn env_find_nonlocal(env: &EnvRef, name: &str) -> Option<EnvRef> {
    let mut current = env.borrow().parent.clone();
    while let Some(frame) = current {
        let borrowed = frame.borrow();
        if borrowed.get_local(name).is_some() {
            return Some(frame.clone());
        }
        current = borrowed.parent.clone();
    }
    None
}

// ── functions, classes, instances ──────────────────────────────────────────

/// What a callable evaluates. A `def` owns a statement block; a `lambda`
/// owns one expression, which keeps the AST borrowable for the module's
/// lifetime without cloning it.
pub enum FunctionBody {
    Block(Rc<Vec<Stmt>>),
    Expression(Rc<Expr>),
    /// A generator expression's body: walk the comprehension clauses, yield
    /// each `element`. Creating this body is what `(x for x in y)` does; it
    /// has no parameters and its closure is the surrounding scope.
    GeneratorExpr {
        element: Rc<Expr>,
        generators: Rc<Vec<CompGen>>,
    },
}

pub struct Function {
    pub name: Rc<str>,
    /// Positional parameters with defaults already evaluated at `def` time.
    pub params: Vec<(Rc<str>, Option<Value>)>,
    /// Keyword-only parameters (`def f(*, x)`, `def f(*args, x=1)`), in
    /// declaration order; they are never filled positionally.
    pub kwonly: Vec<(Rc<str>, Option<Value>)>,
    /// The `*args` parameter, if the signature has one.
    pub varargs: Option<Rc<str>>,
    /// The `**kwargs` parameter, if the signature has one.
    pub kwargs: Option<Rc<str>>,
    pub body: FunctionBody,
    pub closure: EnvRef,
    /// The body contains `yield`: calling this function builds a generator
    /// (binding the parameters) instead of executing anything.
    pub is_generator: bool,
    /// The module the function was defined in, for traceback lines.
    pub filename: Rc<str>,
    pub def_line: u32,
}

pub struct BoundMethod {
    pub function: Rc<Function>,
    pub receiver: Value,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BuiltinType {
    Object,
    NoneType,
    Bool,
    Int,
    Float,
    Str,
    List,
    Tuple,
    Dict,
    Set,
    Bytes,
    Function,
    Type,
    Module,
    File,
    Range,
    Iterator,
    Generator,
    DictKeys,
    DictValues,
    DictItems,
}

impl BuiltinType {
    pub fn name(self) -> &'static str {
        match self {
            BuiltinType::Object => "object",
            BuiltinType::NoneType => "NoneType",
            BuiltinType::Bool => "bool",
            BuiltinType::Int => "int",
            BuiltinType::Float => "float",
            BuiltinType::Str => "str",
            BuiltinType::List => "list",
            BuiltinType::Tuple => "tuple",
            BuiltinType::Dict => "dict",
            BuiltinType::Set => "set",
            BuiltinType::Bytes => "bytes",
            BuiltinType::Function => "function",
            BuiltinType::Type => "type",
            BuiltinType::Module => "module",
            BuiltinType::File => "file",
            BuiltinType::Range => "range",
            BuiltinType::Iterator => "iterator",
            BuiltinType::DictKeys => "dict_keys",
            BuiltinType::DictValues => "dict_values",
            BuiltinType::DictItems => "dict_items",
            BuiltinType::Generator => "generator",
        }
    }
}

pub type Attrs = RefCell<Vec<(Rc<str>, Value)>>;

pub struct Class {
    pub name: Rc<str>,
    pub bases: Vec<Rc<Class>>,
    pub attrs: Attrs,
    /// `Some(tag)` for the interpreter's builtin type objects (`int`, `str`,
    /// `object`, ...), which are callable classes too.
    pub builtin: Option<BuiltinType>,
    /// True for `Exception` and every exception subclass, including user
    /// classes that inherit from one.
    pub is_exception: bool,
}

pub struct Instance {
    pub class: Rc<Class>,
    pub attrs: Attrs,
}

pub struct ModuleValue {
    pub name: Rc<str>,
    pub attrs: Attrs,
}

pub struct FileValue {
    pub name: Rc<str>,
    pub mode: Rc<str>,
    /// File contents are bytes. Text mode only ever stores valid UTF-8 (the
    /// constructor validates it), while binary mode is unrestricted.
    pub data: Vec<u8>,
    pub pos: usize,
    pub readable: bool,
    pub writable: bool,
    pub closed: bool,
    pub dirty: bool,
    pub binary: bool,
}

impl Drop for FileValue {
    fn drop(&mut self) {
        // Python closes the file when the last reference dies, and a buffered
        // writer flushes. Our `open(..., 'w')` buffers in memory, so this is
        // where an unclosed file's bytes reach the filesystem. A write error
        // here cannot be reported anywhere (the value is being dropped), so it
        // is ignored exactly like a close error in CPython's `__del__`.
        if self.writable && self.dirty && !self.closed {
            let _ = crate::platform::write_bytes(&self.name, &self.data);
        }
    }
}

impl FileValue {
    fn check_read(&self) -> Result<(), String> {
        if self.closed {
            return Err(String::from("I/O operation on closed file"));
        }
        if !self.readable {
            return Err(String::from("not readable"));
        }
        Ok(())
    }

    fn check_write(&self) -> Result<(), String> {
        if self.closed {
            return Err(String::from("I/O operation on closed file"));
        }
        if !self.writable {
            return Err(String::from("not writable"));
        }
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        if self.writable && self.dirty {
            crate::platform::write_bytes(&self.name, &self.data)?;
            self.dirty = false;
        }
        self.closed = true;
        Ok(())
    }

    /// One UTF-8 character at `offset`, if the bytes start with a valid one.
    fn char_at(&self, offset: usize) -> Option<(char, usize)> {
        let text = core::str::from_utf8(&self.data[offset..]).ok()?;
        let ch = text.chars().next()?;
        Some((ch, ch.len_utf8()))
    }

    pub fn read(&mut self, count: Option<i64>) -> Result<String, String> {
        self.check_read()?;
        if self.binary {
            return Err(String::from("binary file read() returns bytes"));
        }
        match count {
            None => {
                let text = core::str::from_utf8(&self.data[self.pos..])
                    .map_err(|_| String::from("file contains invalid UTF-8"))?;
                let out = String::from(text);
                self.pos = self.data.len();
                Ok(out)
            }
            Some(count) => {
                if count < 0 {
                    return Err(String::from("read length must be non-negative"));
                }
                let mut end = self.pos;
                let mut taken = 0i64;
                while taken < count {
                    match self.char_at(end) {
                        Some((_, width)) => {
                            end += width;
                            taken += 1;
                        }
                        None => break,
                    }
                }
                let text = core::str::from_utf8(&self.data[self.pos..end])
                    .map_err(|_| String::from("file contains invalid UTF-8"))?;
                self.pos = end;
                Ok(String::from(text))
            }
        }
    }

    pub fn read_bytes(&mut self, count: Option<i64>) -> Result<Vec<u8>, String> {
        self.check_read()?;
        if !self.binary {
            return Err(String::from("text file read() returns str"));
        }
        match count {
            None => {
                let out = self.data[self.pos..].to_vec();
                self.pos = self.data.len();
                Ok(out)
            }
            Some(count) => {
                if count < 0 {
                    return Err(String::from("read length must be non-negative"));
                }
                let end = self
                    .pos
                    .saturating_add(count as usize)
                    .min(self.data.len());
                let out = self.data[self.pos..end].to_vec();
                self.pos = end;
                Ok(out)
            }
        }
    }

    pub fn readline(&mut self) -> Result<String, String> {
        self.check_read()?;
        if self.binary {
            return Err(String::from("binary file readline() returns bytes"));
        }
        let bytes = self.readline_bytes_raw()?;
        String::from_utf8(bytes).map_err(|_| String::from("file contains invalid UTF-8"))
    }

    pub fn readline_bytes(&mut self) -> Result<Vec<u8>, String> {
        self.check_read()?;
        if !self.binary {
            return Err(String::from("text file readline() returns str"));
        }
        self.readline_bytes_raw()
    }

    fn readline_bytes_raw(&mut self) -> Result<Vec<u8>, String> {
        if self.pos >= self.data.len() {
            return Ok(Vec::new());
        }
        let rest = &self.data[self.pos..];
        let end = match rest.iter().position(|byte| *byte == b'\n') {
            Some(offset) => self.pos + offset + 1,
            None => self.data.len(),
        };
        let out = self.data[self.pos..end].to_vec();
        self.pos = end;
        Ok(out)
    }

    fn write_raw(&mut self, bytes: &[u8]) -> Result<usize, String> {
        if self.pos >= self.data.len() {
            self.data.extend_from_slice(bytes);
            self.pos = self.data.len();
        } else {
            // Overwrite the bytes under the cursor, like a file in `r+`/`w+`
            // mode; when the new text is longer than the rest of the file this
            // also extends it. In `a` mode `pos == len`, so the branch above is
            // the append path.
            let end = self
                .pos
                .saturating_add(bytes.len())
                .min(self.data.len());
            self.data.splice(self.pos..end, bytes.iter().copied());
            self.pos += bytes.len();
        }
        self.dirty = true;
        Ok(bytes.len())
    }

    pub fn write(&mut self, text: &str) -> Result<usize, String> {
        self.check_write()?;
        if self.binary {
            return Err(String::from(
                "write() argument must be bytes, not str (binary file)",
            ));
        }
        let bytes = text.as_bytes().to_vec();
        self.write_raw(&bytes)
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, String> {
        self.check_write()?;
        if !self.binary {
            return Err(String::from(
                "write() argument must be str, not bytes (text file)",
            ));
        }
        self.write_raw(bytes)
    }
}

/// A suspended generator: the function it runs, the frame its parameters were
/// bound into, and the fiber holding its interpreter state.
///
/// The interpreter resumes it through the fiber and never holds a `RefCell`
/// borrow across the switch — the fields below are written by the resumer
/// before the switch and by the running fiber at `yield`, never concurrently.
pub struct Generator {
    /// The generator function, or the synthetic `<genexpr>` function of a
    /// generator expression.
    pub function: Rc<Function>,
    /// The frame the body runs in, with the parameters already bound.
    pub env: EnvRef,
    /// Display name: the function name, or `<genexpr>`.
    pub name: Rc<str>,
    /// The stack the body runs on, allocated by the first `next()`: a
    /// generator that is created and never iterated costs no fiber slot.
    pub fiber: Option<Box<crate::fiber::Fiber>>,
    /// True between `next()` and the matching `yield`.
    pub running: bool,
    /// True once the body has actually started on its fiber. `send()` must
    /// refuse a non-None value before that, and `throw()`/`close()` must not
    /// run the body at all: CPython raises/returns in the caller instead.
    pub started: bool,
    /// True once the body returned or raised; the fiber is never resumed.
    pub finished: bool,
    /// The exception that ended the body, re-raised to the caller once.
    pub failed: Option<crate::error::PyError>,
    /// The value the body last yielded. The body's `return` value is kept
    /// apart in `returned`.
    pub yielded: Option<Value>,
    /// The value a `yield` expression evaluates to when the generator is
    /// resumed: `None` for `next()`, the argument for `send(value)`.
    pub sent: Value,
    /// What the resumer injected at the suspended `yield`: `throw()` and
    /// `close()` write it before the switch, `fiber_yield` reads it after the
    /// switch and raises it inside the body.
    pub pending: Option<GeneratorPending>,
    /// The body's `return` value. `yield from` exposes it as the value of the
    /// delegated expression, the way CPython exposes it through
    /// `StopIteration.value` (the interpreter itself has no exception
    /// attributes, so it is kept here instead of on the StopIteration object).
    pub returned: Option<Value>,
}

/// What the resumer asked the suspended generator to do when it resumes.
///
/// Kept apart from [`Generator::sent`] because `throw`/`close` do not deliver
/// a value: they raise an exception at the `yield` point.
pub enum GeneratorPending {
    /// `throw(error)`: raise `error` at the suspended `yield`.
    Throw(crate::error::PyError),
    /// `close()`: raise `GeneratorExit` at the suspended `yield`, closing a
    /// delegated generator first (CPython calls `delegate.close()` in
    /// `gen_close`, which is not the same as `throw(GeneratorExit)`).
    Close,
}

/// Which dict method produced a view.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DictViewKind {
    Keys,
    Values,
    Items,
}

impl DictViewKind {
    pub fn name(self) -> &'static str {
        match self {
            DictViewKind::Keys => "dict_keys",
            DictViewKind::Values => "dict_values",
            DictViewKind::Items => "dict_items",
        }
    }
}

/// A live view over a dict, as returned by `d.keys()`, `d.values()` and
/// `d.items()`.
///
/// It is not a snapshot: `len`, iteration and membership read the dict when
/// they are used, so a view obtained before a mutation sees it — exactly like
/// CPython's view objects. Being a separate `Rc` also gives the view object
/// identity, which is what `d.values() == d.values()` is false about.
pub struct DictView {
    pub entries: Rc<RefCell<Vec<(Value, Value)>>>,
    pub kind: DictViewKind,
}

pub struct RangeValue {
    pub start: i64,
    pub stop: i64,
    pub step: i64,
    pub len: i64,
}

pub enum IterState {
    Range {
        current: i64,
        stop: i64,
        step: i64,
        remaining: i64,
    },
    Str {
        text: Rc<str>,
        byte_index: usize,
    },
    List {
        items: Rc<RefCell<Vec<Value>>>,
        index: usize,
    },
    Tuple {
        items: Rc<Vec<Value>>,
        index: usize,
    },
    DictKeys {
        entries: Rc<RefCell<Vec<(Value, Value)>>>,
        index: usize,
    },
    DictValues {
        entries: Rc<RefCell<Vec<(Value, Value)>>>,
        index: usize,
    },
    DictItems {
        entries: Rc<RefCell<Vec<(Value, Value)>>>,
        index: usize,
    },
    SetElems {
        items: Rc<RefCell<Vec<Value>>>,
        index: usize,
    },
    Bytes {
        data: Rc<Vec<u8>>,
        index: usize,
    },
    Enumerate {
        inner: Value,
        index: i64,
    },
    Zip {
        inners: Vec<Value>,
        done: bool,
    },
    /// A text file iterated line by line (`for line in f`). The file owns the
    /// cursor, so every iterator over the same file shares it, exactly like
    /// CPython's file object.
    File {
        file: Rc<RefCell<FileValue>>,
    },
}

#[allow(dead_code)] // free-class variants are reachable through `class_for` too
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BuiltinId {
    Print,
    Len,
    Range,
    Int,
    Float,
    Str,
    Abs,
    Min,
    Max,
    Sum,
    Sorted,
    Enumerate,
    Zip,
    List,
    Dict,
    Tuple,
    Set,
    Bytes,
    Type,
    IsInstance,
    Open,
    Chr,
    Ord,
    Repr,
    Bool,
    Iter,
    Next,
    Any,
    All,
    Divmod,
    Pow,
    Round,
    Hex,
    Id,
    // math module functions
    MathSqrt,
    MathFloor,
    MathCeil,
    MathTrunc,
    MathFabs,
    MathPow,
    MathFmod,
    MathSin,
    MathCos,
    MathTan,
    MathLog,
    MathLog10,
    MathExp,
    MathIsnan,
    MathIsinf,
    // str methods
    StrSplit,
    StrJoin,
    StrStrip,
    StrLstrip,
    StrRstrip,
    StrUpper,
    StrLower,
    StrReplace,
    StrFind,
    StrStartswith,
    StrEndswith,
    StrCount,
    StrIndex,
    StrIsdigit,
    StrTitle,
    StrSplitlines,
    StrFormat,
    // list methods
    ListAppend,
    ListExtend,
    ListPop,
    ListSort,
    ListInsert,
    ListRemove,
    ListIndex,
    ListReverse,
    ListClear,
    ListCopy,
    // dict methods
    DictGet,
    DictKeys,
    DictValues,
    DictItems,
    DictUpdate,
    DictPop,
    DictSetdefault,
    DictClear,
    DictCopy,
    // set methods
    SetAdd,
    SetRemove,
    SetDiscard,
    SetUnion,
    SetIntersection,
    SetDifference,
    SetIssubset,
    SetSuperset,
    SetCopy,
    SetClear,
    SetPop,
    SetUpdate,
    SetDifferenceUpdate,
    SetIntersectionUpdate,
    SetSymmetricDifference,
    SetSymmetricDifferenceUpdate,
    SetIsDisjoint,
    // bytes methods
    BytesDecode,
    BytesHex,
    StrEncode,
    // map
    Map,
    /// `staticmethod(f)`: wraps a function so instance attribute lookup does
    /// not bind it as a method (the wrapper carries the function as receiver).
    StaticMethod,
    /// Calling such a wrapper: forwards to the wrapped function unchanged.
    StaticMethodCall,
    // generator methods
    GenSend,
    GenThrow,
    GenClose,
    // os module
    OsGetpid,
    OsGetppid,
    OsGetcwd,
    OsListdir,
    OsMkdir,
    OsMakedirs,
    OsRemove,
    OsRmdir,
    OsRename,
    OsStat,
    OsFork,
    OsExecv,
    OsWaitpid,
    OsWait,
    OsSystem,
    // json module
    JsonDumps,
    JsonLoads,
    // os.path module
    PathJoin,
    PathBasename,
    PathDirname,
    PathExists,
    PathIsfile,
    PathIsdir,
    // time module
    TimeTime,
    TimeMonotonic,
    TimeSleep,
    // random module
    RandomRandom,
    RandomSeed,
    RandomRandint,
    RandomChoice,
    RandomShuffle,
    RandomUniform,
    // file methods
    FileRead,
    FileReadline,
    FileReadlines,
    FileWrite,
    FileWritelines,
    FileClose,
    FileFlush,
    FileEnter,
    FileExit,
}

pub struct BuiltinValue {
    pub id: BuiltinId,
    /// Methods carry their receiver; free functions do not.
    pub receiver: Option<Value>,
}

pub fn builtin_name(id: BuiltinId) -> &'static str {
    match id {
        BuiltinId::Print => "print",
        BuiltinId::Len => "len",
        BuiltinId::Range => "range",
        BuiltinId::Int => "int",
        BuiltinId::Float => "float",
        BuiltinId::Str => "str",
        BuiltinId::Abs => "abs",
        BuiltinId::Min => "min",
        BuiltinId::Max => "max",
        BuiltinId::Sum => "sum",
        BuiltinId::Sorted => "sorted",
        BuiltinId::Enumerate => "enumerate",
        BuiltinId::Zip => "zip",
        BuiltinId::List => "list",
        BuiltinId::Dict => "dict",
        BuiltinId::Tuple => "tuple",
        BuiltinId::Set => "set",
        BuiltinId::Bytes => "bytes",
        BuiltinId::Type => "type",
        BuiltinId::IsInstance => "isinstance",
        BuiltinId::Open => "open",
        BuiltinId::Chr => "chr",
        BuiltinId::Ord => "ord",
        BuiltinId::Repr => "repr",
        BuiltinId::Bool => "bool",
        BuiltinId::Iter => "iter",
        BuiltinId::Next => "next",
        BuiltinId::Any => "any",
        BuiltinId::All => "all",
        BuiltinId::Divmod => "divmod",
        BuiltinId::Pow => "pow",
        BuiltinId::Round => "round",
        BuiltinId::Hex => "hex",
        BuiltinId::Id => "id",
        BuiltinId::StaticMethod | BuiltinId::StaticMethodCall => "staticmethod",
        BuiltinId::GenSend => "send",
        BuiltinId::GenThrow => "throw",
        BuiltinId::GenClose => "close",
        BuiltinId::MathSqrt => "sqrt",
        BuiltinId::MathFloor => "floor",
        BuiltinId::MathCeil => "ceil",
        BuiltinId::MathTrunc => "trunc",
        BuiltinId::MathFabs => "fabs",
        BuiltinId::MathPow => "pow",
        BuiltinId::MathFmod => "fmod",
        BuiltinId::MathSin => "sin",
        BuiltinId::MathCos => "cos",
        BuiltinId::MathTan => "tan",
        BuiltinId::MathLog => "log",
        BuiltinId::MathLog10 => "log10",
        BuiltinId::MathExp => "exp",
        BuiltinId::MathIsnan => "isnan",
        BuiltinId::MathIsinf => "isinf",
        BuiltinId::StrSplit => "split",
        BuiltinId::StrJoin => "join",
        BuiltinId::StrStrip => "strip",
        BuiltinId::StrLstrip => "lstrip",
        BuiltinId::StrRstrip => "rstrip",
        BuiltinId::StrUpper => "upper",
        BuiltinId::StrLower => "lower",
        BuiltinId::StrReplace => "replace",
        BuiltinId::StrFind => "find",
        BuiltinId::StrStartswith => "startswith",
        BuiltinId::StrEndswith => "endswith",
        BuiltinId::StrCount => "count",
        BuiltinId::StrIndex => "index",
        BuiltinId::StrIsdigit => "isdigit",
        BuiltinId::StrTitle => "title",
        BuiltinId::StrSplitlines => "splitlines",
        BuiltinId::StrFormat => "format",
        BuiltinId::ListAppend => "append",
        BuiltinId::ListExtend => "extend",
        BuiltinId::ListPop => "pop",
        BuiltinId::ListSort => "sort",
        BuiltinId::ListInsert => "insert",
        BuiltinId::ListRemove => "remove",
        BuiltinId::ListIndex => "index",
        BuiltinId::ListReverse => "reverse",
        BuiltinId::ListClear => "clear",
        BuiltinId::ListCopy => "copy",
        BuiltinId::DictGet => "get",
        BuiltinId::DictKeys => "keys",
        BuiltinId::DictValues => "values",
        BuiltinId::DictItems => "items",
        BuiltinId::DictUpdate => "update",
        BuiltinId::DictPop => "pop",
        BuiltinId::DictSetdefault => "setdefault",
        BuiltinId::DictClear => "clear",
        BuiltinId::DictCopy => "copy",
        BuiltinId::FileRead => "read",
        BuiltinId::FileReadline => "readline",
        BuiltinId::FileReadlines => "readlines",
        BuiltinId::FileWrite => "write",
        BuiltinId::FileWritelines => "writelines",
        BuiltinId::FileClose => "close",
        BuiltinId::FileFlush => "flush",
        BuiltinId::SetAdd => "add",
        BuiltinId::SetRemove => "remove",
        BuiltinId::SetDiscard => "discard",
        BuiltinId::SetUnion => "union",
        BuiltinId::SetIntersection => "intersection",
        BuiltinId::SetDifference => "difference",
        BuiltinId::SetIssubset => "issubset",
        BuiltinId::SetSuperset => "issuperset",
        BuiltinId::SetCopy => "copy",
        BuiltinId::SetClear => "clear",
        BuiltinId::SetPop => "pop",
        BuiltinId::SetUpdate => "update",
        BuiltinId::SetDifferenceUpdate => "difference_update",
        BuiltinId::SetIntersectionUpdate => "intersection_update",
        BuiltinId::SetSymmetricDifference => "symmetric_difference",
        BuiltinId::SetSymmetricDifferenceUpdate => "symmetric_difference_update",
        BuiltinId::SetIsDisjoint => "isdisjoint",
        BuiltinId::BytesDecode => "decode",
        BuiltinId::BytesHex => "hex",
        BuiltinId::StrEncode => "encode",
        BuiltinId::Map => "map",
        BuiltinId::OsGetpid => "getpid",
        BuiltinId::OsGetppid => "getppid",
        BuiltinId::OsGetcwd => "getcwd",
        BuiltinId::OsListdir => "listdir",
        BuiltinId::OsMkdir => "mkdir",
        BuiltinId::OsMakedirs => "makedirs",
        BuiltinId::OsRemove => "remove",
        BuiltinId::OsRmdir => "rmdir",
        BuiltinId::OsRename => "rename",
        BuiltinId::OsStat => "stat",
        BuiltinId::OsFork => "fork",
        BuiltinId::OsExecv => "execv",
        BuiltinId::OsWaitpid => "waitpid",
        BuiltinId::OsWait => "wait",
        BuiltinId::OsSystem => "system",
        BuiltinId::JsonDumps => "dumps",
        BuiltinId::JsonLoads => "loads",
        BuiltinId::PathJoin => "join",
        BuiltinId::PathBasename => "basename",
        BuiltinId::PathDirname => "dirname",
        BuiltinId::PathExists => "exists",
        BuiltinId::PathIsfile => "isfile",
        BuiltinId::PathIsdir => "isdir",
        BuiltinId::TimeTime => "time",
        BuiltinId::TimeMonotonic => "monotonic",
        BuiltinId::TimeSleep => "sleep",
        BuiltinId::RandomRandom => "random",
        BuiltinId::RandomSeed => "seed",
        BuiltinId::RandomRandint => "randint",
        BuiltinId::RandomChoice => "choice",
        BuiltinId::RandomShuffle => "shuffle",
        BuiltinId::RandomUniform => "uniform",
        BuiltinId::FileEnter => "__enter__",
        BuiltinId::FileExit => "__exit__",
    }
}

// ── the Value enum ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    List(Rc<RefCell<Vec<Value>>>),
    Tuple(Rc<Vec<Value>>),
    Dict(Rc<RefCell<Vec<(Value, Value)>>>),
    DictView(Rc<DictView>),
    Set(Rc<RefCell<Vec<Value>>>),
    Bytes(Rc<Vec<u8>>),
    Function(Rc<Function>),
    Builtin(Rc<BuiltinValue>),
    Class(Rc<Class>),
    Instance(Rc<Instance>),
    BoundMethod(Rc<BoundMethod>),
    Module(Rc<ModuleValue>),
    File(Rc<RefCell<FileValue>>),
    Range(Rc<RangeValue>),
    Iterator(Rc<RefCell<IterState>>),
    Generator(Rc<RefCell<Generator>>),
}

impl Value {
    pub fn str(text: &str) -> Value {
        Value::Str(Rc::from(text))
    }

    pub fn list(items: Vec<Value>) -> Value {
        Value::List(Rc::new(RefCell::new(items)))
    }

    pub fn tuple(items: Vec<Value>) -> Value {
        Value::Tuple(Rc::new(items))
    }

    pub fn dict(entries: Vec<(Value, Value)>) -> Value {
        Value::Dict(Rc::new(RefCell::new(entries)))
    }

    pub fn set(items: Vec<Value>) -> Value {
        Value::Set(Rc::new(RefCell::new(items)))
    }

    pub fn bytes(data: Vec<u8>) -> Value {
        Value::Bytes(Rc::new(data))
    }

    pub fn builtin(id: BuiltinId) -> Value {
        Value::Builtin(Rc::new(BuiltinValue { id, receiver: None }))
    }

    pub fn method(id: BuiltinId, receiver: Value) -> Value {
        Value::Builtin(Rc::new(BuiltinValue {
            id,
            receiver: Some(receiver),
        }))
    }

    #[allow(dead_code)]
    pub fn is_none(&self) -> bool {
        matches!(self, Value::None)
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::None => false,
            Value::Bool(value) => *value,
            Value::Int(value) => *value != 0,
            Value::Float(value) => *value != 0.0,
            Value::Str(text) => !text.is_empty(),
            Value::List(items) => !items.borrow().is_empty(),
            Value::Tuple(items) => !items.is_empty(),
            Value::Dict(entries) => !entries.borrow().is_empty(),
            Value::DictView(view) => !view.entries.borrow().is_empty(),
            Value::Set(items) => !items.borrow().is_empty(),
            Value::Bytes(data) => !data.is_empty(),
            Value::Range(range) => range.len > 0,
            _ => true,
        }
    }

    /// Python's `type(x)` name, used in error messages and `type(x).__name__`.
    pub fn type_name(&self) -> &str {
        match self {
            Value::None => "NoneType",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::List(_) => "list",
            Value::Tuple(_) => "tuple",
            Value::Dict(_) => "dict",
            Value::DictView(view) => view.kind.name(),
            Value::Set(_) => "set",
            Value::Bytes(_) => "bytes",
            Value::Function(_) | Value::BoundMethod(_) => "function",
            Value::Builtin(_) => "builtin_function_or_method",
            Value::Class(_) => "type",
            Value::Instance(instance) => &instance.class.name,
            Value::Module(_) => "module",
            Value::File(_) => "file",
            Value::Range(_) => "range",
            Value::Iterator(_) => "iterator",
            Value::Generator(_) => "generator",
        }
    }

    pub fn type_tag(&self) -> BuiltinType {
        match self {
            Value::None => BuiltinType::NoneType,
            Value::Bool(_) => BuiltinType::Bool,
            Value::Int(_) => BuiltinType::Int,
            Value::Float(_) => BuiltinType::Float,
            Value::Str(_) => BuiltinType::Str,
            Value::List(_) => BuiltinType::List,
            Value::Tuple(_) => BuiltinType::Tuple,
            Value::Dict(_) => BuiltinType::Dict,
            Value::DictView(view) => match view.kind {
                DictViewKind::Keys => BuiltinType::DictKeys,
                DictViewKind::Values => BuiltinType::DictValues,
                DictViewKind::Items => BuiltinType::DictItems,
            },
            Value::Set(_) => BuiltinType::Set,
            Value::Bytes(_) => BuiltinType::Bytes,
            Value::Function(_) | Value::BoundMethod(_) => BuiltinType::Function,
            Value::Builtin(_) => BuiltinType::Function,
            Value::Class(_) => BuiltinType::Type,
            Value::Instance(_) => BuiltinType::Object,
            Value::Module(_) => BuiltinType::Module,
            Value::File(_) => BuiltinType::File,
            Value::Range(_) => BuiltinType::Range,
            Value::Iterator(_) => BuiltinType::Iterator,
            Value::Generator(_) => BuiltinType::Generator,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            Value::Bool(value) => Some(if *value { 1 } else { 0 }),
            _ => None,
        }
    }

    /// Numeric value as f64: ints, bools and floats. Used by the mixed
    /// arithmetic and comparison paths; `as_int` stays integer-only so list
    /// indices and `range()` still reject floats.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Int(value) => Some(*value as f64),
            Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
            Value::Float(value) => Some(*value),
            _ => None,
        }
    }

    pub fn is_hashable(&self) -> bool {
        match self {
            Value::None | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_) => true,
            Value::Tuple(items) => items.iter().all(Value::is_hashable),
            Value::Bytes(_) => true,
            Value::Class(_) | Value::Instance(_) => true,
            Value::DictView(_) => false,
            _ => false,
        }
    }

    pub fn to_str(&self) -> String {
        match self {
            Value::Str(text) => String::from(&**text),
            _ => self.to_repr(),
        }
    }

    pub fn to_repr(&self) -> String {
        let mut out = String::new();
        write_repr(&mut out, self, 0);
        out
    }
}

// ── repr / str ─────────────────────────────────────────────────────────────

fn write_repr(out: &mut String, value: &Value, depth: usize) {
    if depth > 12 {
        out.push_str("...");
        return;
    }
    match value {
        Value::None => out.push_str("None"),
        Value::Bool(true) => out.push_str("True"),
        Value::Bool(false) => out.push_str("False"),
        Value::Int(number) => {
            let _ = write!(out, "{}", number);
        }
        Value::Float(number) => out.push_str(&crate::float::format(*number)),
        Value::Str(text) => write_string_repr(out, text),
        Value::List(items) => {
            out.push('[');
            for (index, item) in items.borrow().iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_repr(out, item, depth + 1);
            }
            out.push(']');
        }
        Value::Tuple(items) => {
            out.push('(');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_repr(out, item, depth + 1);
            }
            if items.len() == 1 {
                out.push(',');
            }
            out.push(')');
        }
        Value::DictView(view) => {
            let entries = view.entries.borrow();
            out.push_str(view.kind.name());
            out.push('(');
            match view.kind {
                DictViewKind::Keys => {
                    out.push('[');
                    for (index, (key, _)) in entries.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        write_repr(out, key, depth + 1);
                    }
                    out.push(']');
                }
                DictViewKind::Values => {
                    out.push('[');
                    for (index, (_, value)) in entries.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        write_repr(out, value, depth + 1);
                    }
                    out.push(']');
                }
                DictViewKind::Items => {
                    out.push('[');
                    for (index, (key, value)) in entries.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        out.push('(');
                        write_repr(out, key, depth + 1);
                        out.push_str(", ");
                        write_repr(out, value, depth + 1);
                        out.push(')');
                    }
                    out.push(']');
                }
            }
            out.push(')');
        }
        Value::Dict(entries) => {
            out.push('{');
            for (index, (key, item)) in entries.borrow().iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_repr(out, key, depth + 1);
                out.push_str(": ");
                write_repr(out, item, depth + 1);
            }
            out.push('}');
        }
        Value::Set(items) => {
            let items = items.borrow();
            if items.is_empty() {
                out.push_str("set()");
            } else {
                out.push('{');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    write_repr(out, item, depth + 1);
                }
                out.push('}');
            }
        }
        Value::Bytes(data) => write_bytes_repr(out, data),
        Value::Function(function) => {
            let _ = write!(out, "<function {}>", function.name);
        }
        Value::Builtin(builtin) => {
            let _ = write!(out, "<built-in function {}>", builtin_name(builtin.id));
        }
        Value::BoundMethod(method) => {
            let _ = write!(
                out,
                "<bound method {} of {}>",
                method.function.name,
                method.receiver.type_name()
            );
        }
        Value::Class(class) => {
            let _ = write!(out, "<class '{}'>", class.name);
        }
        Value::Instance(instance) => {
            if instance.class.is_exception {
                write_exception_str(out, instance, depth);
            } else {
                let _ = write!(out, "<{} object>", instance.class.name);
            }
        }
        Value::Module(module) => {
            let _ = write!(out, "<module '{}'>", module.name);
        }
        Value::File(file) => {
            let borrowed = file.borrow();
            let _ = write!(out, "<file '{}' mode '{}'>", borrowed.name, borrowed.mode);
        }
        Value::Range(range) => {
            if range.step == 1 {
                let _ = write!(out, "range({}, {})", range.start, range.stop);
            } else {
                let _ = write!(
                    out,
                    "range({}, {}, {})",
                    range.start, range.stop, range.step
                );
            }
        }
        Value::Iterator(_) => out.push_str("<iterator>"),
        Value::Generator(generator) => {
            let _ = write!(out, "<generator object {}>", generator.borrow().name);
        }
    }
}

fn write_exception_str(out: &mut String, instance: &Instance, depth: usize) {
    let args = attr_get(&instance.attrs, "args");
    match args {
        Some(Value::Tuple(items)) if items.is_empty() => {}
        Some(Value::Tuple(items)) if items.len() == 1 => {
            let text = items[0].to_str();
            out.push_str(&text);
        }
        Some(Value::Tuple(items)) => {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_repr(out, item, depth + 1);
            }
        }
        _ => {}
    }
}

fn write_string_repr(out: &mut String, text: &str) {
    out.push('\'');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('\'');
}

fn write_bytes_repr(out: &mut String, data: &[u8]) {
    out.push_str("b'");
    for byte in data {
        match *byte {
            b'\\' => out.push_str("\\\\"),
            b'\'' => out.push_str("\\'"),
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b'\r' => out.push_str("\\r"),
            0x20..=0x7e => out.push(*byte as char),
            other => {
                let _ = write!(out, "\\x{:02x}", other);
            }
        }
    }
    out.push('\'');
}

// ── equality and ordering ─────────────────────────────────────────────────

/// The elements a keys/items view compares and tests membership against.
pub fn view_elements(view: &DictView) -> Vec<Value> {
    let entries = view.entries.borrow();
    match view.kind {
        DictViewKind::Keys => entries.iter().map(|(key, _)| key.clone()).collect(),
        DictViewKind::Values => entries.iter().map(|(_, value)| value.clone()).collect(),
        DictViewKind::Items => entries
            .iter()
            .map(|(key, value)| Value::tuple(alloc::vec![key.clone(), value.clone()]))
            .collect(),
    }
}

pub fn values_equal(left: &Value, right: &Value) -> bool {
    equal_depth(left, right, 0)
}

fn equal_depth(left: &Value, right: &Value, depth: usize) -> bool {
    if depth > 20 {
        return false;
    }
    if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
        return a == b;
    }
    // Mixed int/float equality compares the mathematical values, not two
    // independently rounded doubles: a large i64 and an f64 that happens to
    // round to the same double must still compare unequal.
    match (left, right) {
        (Value::Int(integer), Value::Float(float))
        | (Value::Float(float), Value::Int(integer)) => {
            return crate::float::int_equals_float(*integer, *float);
        }
        (Value::Bool(flag), Value::Float(float))
        | (Value::Float(float), Value::Bool(flag)) => {
            return crate::float::int_equals_float(if *flag { 1 } else { 0 }, *float);
        }
        (Value::Float(a), Value::Float(b)) => return a == b,
        _ => {}
    }
    match (left, right) {
        (Value::None, Value::None) => true,
        (Value::Str(a), Value::Str(b)) => a == b,
        (Value::List(a), Value::List(b)) => {
            if Rc::ptr_eq(a, b) {
                return true;
            }
            let a = a.borrow();
            let b = b.borrow();
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| equal_depth(x, y, depth + 1))
        }
        (Value::Tuple(a), Value::Tuple(b)) => {
            if Rc::ptr_eq(a, b) {
                return true;
            }
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| equal_depth(x, y, depth + 1))
        }
        (Value::DictView(a), Value::DictView(b)) => {
            if a.kind != b.kind {
                return false;
            }
            match a.kind {
                // `dict_values` has no meaningful equality in CPython either:
                // only the very same view object is equal to itself.
                DictViewKind::Values => Rc::ptr_eq(a, b),
                _ => {
                    let left = view_elements(a);
                    let right = view_elements(b);
                    left.len() == right.len()
                        && left.iter().all(|item| set_find(&right, item).is_some())
                }
            }
        }
        (Value::Dict(a), Value::Dict(b)) => {
            if Rc::ptr_eq(a, b) {
                return true;
            }
            let a = a.borrow();
            let b = b.borrow();
            if a.len() != b.len() {
                return false;
            }
            for (key, value) in a.iter() {
                match dict_find(&b, key) {
                    Some(index) => {
                        if !equal_depth(value, &b[index].1, depth + 1) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            true
        }
        (Value::Set(a), Value::Set(b)) => {
            if Rc::ptr_eq(a, b) {
                return true;
            }
            let a = a.borrow();
            let b = b.borrow();
            if a.len() != b.len() {
                return false;
            }
            a.iter()
                .all(|item| set_find(&b, item).is_some())
        }
        (Value::Bytes(a), Value::Bytes(b)) => a.as_slice() == b.as_slice(),
        (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
        (Value::Instance(a), Value::Instance(b)) => Rc::ptr_eq(a, b),
        (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
        (Value::Builtin(a), Value::Builtin(b)) => {
            a.id == b.id
                && match (&a.receiver, &b.receiver) {
                    (None, None) => true,
                    (Some(x), Some(y)) => values_equal(x, y),
                    _ => false,
                }
        }
        (Value::BoundMethod(a), Value::BoundMethod(b)) => {
            Rc::ptr_eq(&a.function, &b.function) && values_equal(&a.receiver, &b.receiver)
        }
        (Value::Module(a), Value::Module(b)) => Rc::ptr_eq(a, b),
        (Value::File(a), Value::File(b)) => Rc::ptr_eq(a, b),
        (Value::Range(a), Value::Range(b)) => {
            a.start == b.start && a.stop == b.stop && a.step == b.step
        }
        (Value::Iterator(a), Value::Iterator(b)) => Rc::ptr_eq(a, b),
        (Value::Generator(a), Value::Generator(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}

/// Python's `is`: identity for heap objects, value identity for the interned
/// scalars (`None`, booleans and small ints in CPython; every int here).
pub fn is_same(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::None, Value::None) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
        (Value::Str(a), Value::Str(b)) => Rc::ptr_eq(a, b),
        (Value::List(a), Value::List(b)) => Rc::ptr_eq(a, b),
        (Value::Tuple(a), Value::Tuple(b)) => Rc::ptr_eq(a, b),
        (Value::Dict(a), Value::Dict(b)) => Rc::ptr_eq(a, b),
        (Value::Set(a), Value::Set(b)) => Rc::ptr_eq(a, b),
        (Value::DictView(a), Value::DictView(b)) => Rc::ptr_eq(a, b),
        (Value::Bytes(a), Value::Bytes(b)) => Rc::ptr_eq(a, b),
        (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
        (Value::Builtin(a), Value::Builtin(b)) => Rc::ptr_eq(a, b),
        (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
        (Value::Instance(a), Value::Instance(b)) => Rc::ptr_eq(a, b),
        (Value::BoundMethod(a), Value::BoundMethod(b)) => Rc::ptr_eq(a, b),
        (Value::Module(a), Value::Module(b)) => Rc::ptr_eq(a, b),
        (Value::File(a), Value::File(b)) => Rc::ptr_eq(a, b),
        (Value::Range(a), Value::Range(b)) => Rc::ptr_eq(a, b),
        (Value::Iterator(a), Value::Iterator(b)) => Rc::ptr_eq(a, b),
        (Value::Generator(a), Value::Generator(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}

// ── attributes ─────────────────────────────────────────────────────────────

pub fn attr_get(attrs: &Attrs, name: &str) -> Option<Value> {
    attrs
        .borrow()
        .iter()
        .rev()
        .find(|(key, _)| &**key == name)
        .map(|(_, value)| value.clone())
}

pub fn attr_set(attrs: &Attrs, name: Rc<str>, value: Value) {
    let mut borrowed = attrs.borrow_mut();
    for (key, slot) in borrowed.iter_mut().rev() {
        if *key == name {
            *slot = value;
            return;
        }
    }
    borrowed.push((name, value));
}

/// Look `name` up on a class and then its bases, depth first, left to right.
/// This is the small, single-threaded stand-in for a C3 MRO; for the single
/// inheritance this interpreter documents it is exactly Python's lookup order.
pub fn class_lookup(class: &Rc<Class>, name: &str) -> Option<Value> {
    if let Some(value) = attr_get(&class.attrs, name) {
        return Some(value);
    }
    for base in &class.bases {
        if let Some(value) = class_lookup(base, name) {
            return Some(value);
        }
    }
    None
}

pub fn class_is_subclass(class: &Rc<Class>, target: &Rc<Class>) -> bool {
    if Rc::ptr_eq(class, target) {
        return true;
    }
    class
        .bases
        .iter()
        .any(|base| class_is_subclass(base, target))
}

#[allow(dead_code)]
pub fn class_mro_contains(class: &Rc<Class>, target_name: &str) -> bool {
    if &*class.name == target_name {
        return true;
    }
    class
        .bases
        .iter()
        .any(|base| class_mro_contains(base, target_name))
}

/// `isinstance(value, class)` for a builtin or user class object.
pub fn instance_of_class(value: &Value, target: &Rc<Class>) -> bool {
    if let Some(tag) = target.builtin {
        return match tag {
            BuiltinType::Object => true,
            BuiltinType::Int => matches!(value, Value::Int(_) | Value::Bool(_)),
            BuiltinType::Float => matches!(value, Value::Float(_)),
            BuiltinType::Bool => matches!(value, Value::Bool(_)),
            BuiltinType::Type => matches!(value, Value::Class(_)),
            BuiltinType::Function => matches!(value, Value::Function(_)),
            other => value.type_tag() == other,
        };
    }
    match value {
        Value::Instance(instance) => class_is_subclass(&instance.class, target),
        Value::Class(_) if &*target.name == "object" => true,
        _ => false,
    }
}

// ── dictionaries ───────────────────────────────────────────────────────────

pub fn dict_find(entries: &[(Value, Value)], key: &Value) -> Option<usize> {
    // `is_same` first so a NaN key can be looked up (NaN != NaN, but the same
    // inline f64 has the same bits); CPython gets the same effect through the
    // identity shortcut in its dict lookup.
    entries
        .iter()
        .position(|(candidate, _)| is_same(candidate, key) || values_equal(candidate, key))
}

pub fn dict_get(entries: &[(Value, Value)], key: &Value) -> Option<Value> {
    dict_find(entries, key).map(|index| entries[index].1.clone())
}

pub fn dict_set(entries: &mut Vec<(Value, Value)>, key: Value, value: Value) {
    if let Some(index) = dict_find(entries, &key) {
        entries[index].1 = value;
    } else {
        entries.push((key, value));
    }
}

// ── sets ───────────────────────────────────────────────────────────────────

/// Linear membership test, using Python equality (and the identity shortcut
/// that also makes a NaN element findable).
pub fn set_find(items: &[Value], needle: &Value) -> Option<usize> {
    items
        .iter()
        .position(|candidate| is_same(candidate, needle) || values_equal(candidate, needle))
}

/// Insert without duplicating; returns whether the element was new.
pub fn set_insert(items: &mut Vec<Value>, value: Value) -> bool {
    if set_find(items, &value).is_some() {
        return false;
    }
    items.push(value);
    true
}

// ── iteration ──────────────────────────────────────────────────────────────

impl IterState {
    pub fn next(&mut self) -> Option<Value> {
        match self {
            IterState::Range {
                current,
                stop,
                step,
                remaining,
            } => {
                if *remaining <= 0 {
                    return None;
                }
                let value = *current;
                *remaining -= 1;
                if let Some(next) = current.checked_add(*step) {
                    *current = next;
                } else if *step > 0 {
                    *current = *stop;
                } else {
                    *current = *stop;
                }
                Some(Value::Int(value))
            }
            IterState::Str { text, byte_index } => {
                if *byte_index >= text.len() {
                    return None;
                }
                let ch = text[*byte_index..].chars().next()?;
                *byte_index += ch.len_utf8();
                Some(Value::Str(Rc::from(ch.encode_utf8(&mut [0u8; 4]))))
            }
            IterState::List { items, index } => {
                let borrowed = items.borrow();
                let value = borrowed.get(*index).cloned();
                if value.is_some() {
                    *index += 1;
                }
                value
            }
            IterState::Tuple { items, index } => {
                let value = items.get(*index).cloned();
                if value.is_some() {
                    *index += 1;
                }
                value
            }
            IterState::DictKeys { entries, index } => {
                let borrowed = entries.borrow();
                let entry = borrowed.get(*index).map(|(key, _)| key.clone());
                if entry.is_some() {
                    *index += 1;
                }
                entry
            }
            IterState::DictValues { entries, index } => {
                let borrowed = entries.borrow();
                let value = borrowed.get(*index).map(|(_, value)| value.clone());
                if value.is_some() {
                    *index += 1;
                }
                value
            }
            IterState::DictItems { entries, index } => {
                let borrowed = entries.borrow();
                let value = borrowed
                    .get(*index)
                    .map(|(key, item)| Value::tuple(alloc::vec![key.clone(), item.clone()]));
                if value.is_some() {
                    *index += 1;
                }
                value
            }
            IterState::SetElems { items, index } => {
                let borrowed = items.borrow();
                let value = borrowed.get(*index).cloned();
                if value.is_some() {
                    *index += 1;
                }
                value
            }
            IterState::Bytes { data, index } => {
                let byte = data.get(*index).copied();
                if byte.is_some() {
                    *index += 1;
                }
                byte.map(|value| Value::Int(value as i64))
            }
            IterState::Enumerate { inner, index } => {
                let value = iterator_next(inner)?;
                let counter = *index;
                *index = index.checked_add(1).unwrap_or(*index);
                Some(Value::tuple(alloc::vec![Value::Int(counter), value]))
            }
            IterState::Zip { inners, done } => {
                if *done {
                    return None;
                }
                let mut values = Vec::with_capacity(inners.len());
                for inner in inners.iter() {
                    match iterator_next(inner) {
                        Some(value) => values.push(value),
                        None => {
                            *done = true;
                            return None;
                        }
                    }
                }
                Some(Value::tuple(values))
            }
            IterState::File { file } => {
                let mut borrowed = file.borrow_mut();
                if borrowed.binary {
                    match borrowed.readline_bytes() {
                        Ok(line) if !line.is_empty() => Some(Value::Bytes(Rc::new(line))),
                        _ => None,
                    }
                } else {
                    match borrowed.readline() {
                        Ok(line) if !line.is_empty() => Some(Value::Str(Rc::from(line))),
                        _ => None,
                    }
                }
            }
        }
    }
}

/// Advance an iterator value. Returns `None` at the end.
pub fn iterator_next(value: &Value) -> Option<Value> {
    match value {
        Value::Iterator(state) => state.borrow_mut().next(),
        _ => None,
    }
}

/// Number of elements a `range` will produce, following CPython: an empty
/// range has length 0 regardless of step sign; an out-of-range length is an
/// `OverflowError` at construction time in the interpreter.
pub fn range_length(start: i64, stop: i64, step: i64) -> Option<i64> {
    if step == 0 {
        return None;
    }
    let start = start as i128;
    let stop = stop as i128;
    let step = step as i128;
    let length = if step > 0 {
        if start >= stop {
            0
        } else {
            (stop - start + step - 1) / step
        }
    } else if start <= stop {
        0
    } else {
        (start - stop + (-step) - 1) / (-step)
    };
    i64::try_from(length).ok()
}

// ── one small builtin used by the REPL-free `-c` path ─────────────────────

/// Parse a decimal/`0x`/`0o`/`0b` integer the way `int(text)` does for a
/// single argument. Returns `None` for a malformed literal.
#[allow(dead_code)]
pub fn parse_int_literal(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (sign, rest) = match trimmed.as_bytes().first() {
        Some(b'-') => (-1i64, &trimmed[1..]),
        Some(b'+') => (1i64, &trimmed[1..]),
        _ => (1i64, trimmed),
    };
    let (base, digits) = if let Some(rest) = rest.strip_prefix("0x").or(rest.strip_prefix("0X")) {
        (16, rest)
    } else if let Some(rest) = rest.strip_prefix("0o").or(rest.strip_prefix("0O")) {
        (8, rest)
    } else if let Some(rest) = rest.strip_prefix("0b").or(rest.strip_prefix("0B")) {
        (2, rest)
    } else {
        (10, rest)
    };
    let cleaned: String = digits.chars().filter(|c| *c != '_').collect();
    if cleaned.is_empty() {
        return None;
    }
    let magnitude = i64::from_str_radix(&cleaned, base).ok()?;
    magnitude.checked_mul(sign)
}

/// Format a number for `hex()` (which is only used as a helper, not a builtin
/// requirement); kept here so `builtins.rs` stays focused on dispatch.
pub fn hex_text(value: i64) -> String {
    if value < 0 {
        format!("-0x{:x}", (value as i128).unsigned_abs())
    } else {
        format!("0x{:x}", value)
    }
}
