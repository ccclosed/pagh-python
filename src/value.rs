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

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt::Write;

use crate::ast::Stmt;

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

pub struct Function {
    pub name: Rc<str>,
    /// Parameters with defaults already evaluated at `def` time.
    pub params: Vec<(Rc<str>, Option<Value>)>,
    pub body: Rc<Vec<Stmt>>,
    pub closure: EnvRef,
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
    Str,
    List,
    Tuple,
    Dict,
    Function,
    Type,
    Module,
    File,
    Range,
    Iterator,
}

impl BuiltinType {
    pub fn name(self) -> &'static str {
        match self {
            BuiltinType::Object => "object",
            BuiltinType::NoneType => "NoneType",
            BuiltinType::Bool => "bool",
            BuiltinType::Int => "int",
            BuiltinType::Str => "str",
            BuiltinType::List => "list",
            BuiltinType::Tuple => "tuple",
            BuiltinType::Dict => "dict",
            BuiltinType::Function => "function",
            BuiltinType::Type => "type",
            BuiltinType::Module => "module",
            BuiltinType::File => "file",
            BuiltinType::Range => "range",
            BuiltinType::Iterator => "iterator",
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
    pub data: String,
    pub pos: usize,
    pub readable: bool,
    pub writable: bool,
    pub closed: bool,
    pub dirty: bool,
}

impl Drop for FileValue {
    fn drop(&mut self) {
        // Python closes the file when the last reference dies, and a buffered
        // writer flushes. Our `open(..., 'w')` buffers in memory, so this is
        // where an unclosed file's text reaches the filesystem. A write error
        // here cannot be reported anywhere (the value is being dropped), so it
        // is ignored exactly like a close error in CPython's `__del__`.
        if self.writable && self.dirty && !self.closed {
            let _ = crate::platform::write_text(&self.name, &self.data);
        }
    }
}

impl FileValue {
    pub fn close(&mut self) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        if self.writable && self.dirty {
            crate::platform::write_text(&self.name, &self.data)?;
            self.dirty = false;
        }
        self.closed = true;
        Ok(())
    }

    pub fn read(&mut self, count: Option<i64>) -> Result<String, String> {
        if self.closed {
            return Err(String::from("I/O operation on closed file"));
        }
        if !self.readable {
            return Err(String::from("not readable"));
        }
        match count {
            None => {
                let text = String::from(&self.data[self.pos..]);
                self.pos = self.data.len();
                Ok(text)
            }
            Some(count) => {
                if count < 0 {
                    return Err(String::from("read length must be non-negative"));
                }
                let mut end = self.pos;
                let mut taken = 0i64;
                while taken < count && end < self.data.len() {
                    let mut iter = self.data[end..].chars();
                    if let Some(ch) = iter.next() {
                        end += ch.len_utf8();
                        taken += 1;
                    } else {
                        break;
                    }
                }
                let text = String::from(&self.data[self.pos..end]);
                self.pos = end;
                Ok(text)
            }
        }
    }

    pub fn readline(&mut self) -> Result<String, String> {
        if self.closed {
            return Err(String::from("I/O operation on closed file"));
        }
        if !self.readable {
            return Err(String::from("not readable"));
        }
        if self.pos >= self.data.len() {
            return Ok(String::new());
        }
        let rest = &self.data[self.pos..];
        let end = match rest.find('\n') {
            Some(offset) => self.pos + offset + 1,
            None => self.data.len(),
        };
        let text = String::from(&self.data[self.pos..end]);
        self.pos = end;
        Ok(text)
    }

    pub fn write(&mut self, text: &str) -> Result<usize, String> {
        if self.closed {
            return Err(String::from("I/O operation on closed file"));
        }
        if !self.writable {
            return Err(String::from("not writable"));
        }
        if self.pos >= self.data.len() {
            self.data.push_str(text);
            self.pos = self.data.len();
        } else {
            self.data.replace_range(self.pos..self.pos, text);
            self.pos += text.len();
        }
        self.dirty = true;
        Ok(text.chars().count())
    }
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
    Enumerate {
        inner: Value,
        index: i64,
    },
    Zip {
        inners: Vec<Value>,
        done: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BuiltinId {
    Print,
    Len,
    Range,
    Int,
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
    Hex,
    Id,
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
    // file methods
    FileRead,
    FileReadline,
    FileReadlines,
    FileWrite,
    FileClose,
    FileFlush,
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
        BuiltinId::Hex => "hex",
        BuiltinId::Id => "id",
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
        BuiltinId::FileClose => "close",
        BuiltinId::FileFlush => "flush",
    }
}

// ── the Value enum ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Str(Rc<str>),
    List(Rc<RefCell<Vec<Value>>>),
    Tuple(Rc<Vec<Value>>),
    Dict(Rc<RefCell<Vec<(Value, Value)>>>),
    Function(Rc<Function>),
    Builtin(Rc<BuiltinValue>),
    Class(Rc<Class>),
    Instance(Rc<Instance>),
    BoundMethod(Rc<BoundMethod>),
    Module(Rc<ModuleValue>),
    File(Rc<RefCell<FileValue>>),
    Range(Rc<RangeValue>),
    Iterator(Rc<RefCell<IterState>>),
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

    pub fn builtin(id: BuiltinId) -> Value {
        Value::Builtin(Rc::new(BuiltinValue {
            id,
            receiver: None,
        }))
    }

    pub fn method(id: BuiltinId, receiver: Value) -> Value {
        Value::Builtin(Rc::new(BuiltinValue {
            id,
            receiver: Some(receiver),
        }))
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Value::None)
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::None => false,
            Value::Bool(value) => *value,
            Value::Int(value) => *value != 0,
            Value::Str(text) => !text.is_empty(),
            Value::List(items) => !items.borrow().is_empty(),
            Value::Tuple(items) => !items.is_empty(),
            Value::Dict(entries) => !entries.borrow().is_empty(),
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
            Value::Str(_) => "str",
            Value::List(_) => "list",
            Value::Tuple(_) => "tuple",
            Value::Dict(_) => "dict",
            Value::Function(_) | Value::BoundMethod(_) => "function",
            Value::Builtin(_) => "builtin_function_or_method",
            Value::Class(_) => "type",
            Value::Instance(instance) => &instance.class.name,
            Value::Module(_) => "module",
            Value::File(_) => "file",
            Value::Range(_) => "range",
            Value::Iterator(_) => "iterator",
        }
    }

    pub fn type_tag(&self) -> BuiltinType {
        match self {
            Value::None => BuiltinType::NoneType,
            Value::Bool(_) => BuiltinType::Bool,
            Value::Int(_) => BuiltinType::Int,
            Value::Str(_) => BuiltinType::Str,
            Value::List(_) => BuiltinType::List,
            Value::Tuple(_) => BuiltinType::Tuple,
            Value::Dict(_) => BuiltinType::Dict,
            Value::Function(_) | Value::BoundMethod(_) => BuiltinType::Function,
            Value::Builtin(_) => BuiltinType::Function,
            Value::Class(_) => BuiltinType::Type,
            Value::Instance(_) => BuiltinType::Object,
            Value::Module(_) => BuiltinType::Module,
            Value::File(_) => BuiltinType::File,
            Value::Range(_) => BuiltinType::Range,
            Value::Iterator(_) => BuiltinType::Iterator,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            Value::Bool(value) => Some(if *value { 1 } else { 0 }),
            _ => None,
        }
    }

    pub fn is_hashable(&self) -> bool {
        match self {
            Value::None | Value::Bool(_) | Value::Int(_) | Value::Str(_) => true,
            Value::Tuple(items) => items.iter().all(Value::is_hashable),
            Value::Class(_) | Value::Instance(_) => true,
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

// ── equality and ordering ─────────────────────────────────────────────────

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
        (Value::Str(a), Value::Str(b)) => Rc::ptr_eq(a, b),
        (Value::List(a), Value::List(b)) => Rc::ptr_eq(a, b),
        (Value::Tuple(a), Value::Tuple(b)) => Rc::ptr_eq(a, b),
        (Value::Dict(a), Value::Dict(b)) => Rc::ptr_eq(a, b),
        (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
        (Value::Builtin(a), Value::Builtin(b)) => Rc::ptr_eq(a, b),
        (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
        (Value::Instance(a), Value::Instance(b)) => Rc::ptr_eq(a, b),
        (Value::BoundMethod(a), Value::BoundMethod(b)) => Rc::ptr_eq(a, b),
        (Value::Module(a), Value::Module(b)) => Rc::ptr_eq(a, b),
        (Value::File(a), Value::File(b)) => Rc::ptr_eq(a, b),
        (Value::Range(a), Value::Range(b)) => Rc::ptr_eq(a, b),
        (Value::Iterator(a), Value::Iterator(b)) => Rc::ptr_eq(a, b),
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
    entries
        .iter()
        .position(|(candidate, _)| values_equal(candidate, key))
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