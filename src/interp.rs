//! The tree-walking evaluator: scopes, control flow, calls, classes,
//! exceptions and imports.
//!
//! # Memory and stack discipline
//!
//! pagh gives a process a fixed 1 MiB bump heap (never freed) and a 64 KiB
//! user stack. Two design decisions keep normal scripts comfortable:
//!
//! * function-call environments are recycled through `env_pool` (an environment
//!   is only pooled when no closure captured it, checked with `Rc::strong_count`);
//! * positional arguments use [`ArgList`], a small inline array with a `Vec`
//!   spill only for calls with more than eight arguments.
//!
//! Recursion is capped at [`RECURSION_LIMIT`] Python call frames so a runaway
//! recursive program raises `RecursionError` instead of running off the small
//! U-mode stack.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;
use core::cmp::Ordering;

use crate::ast::{
    BinOp, BoolOp, CallArg, CallKeyword, CmpOp, CompGen, Expr, FStringPart, Handler, Name, Param,
    ParamKind, Stmt, StmtKind, UnOp, WithItem,
};
use crate::fiber;
use crate::builtins;
use crate::error::{ErrorName, PyError, PyResult, TraceFrame};
use crate::parser;
use crate::platform;
use crate::value;
use crate::value::{
    attr_get, attr_set, class_is_subclass, class_lookup, dict_get, dict_set, env_find_nonlocal,
    env_lookup, is_same, set_find, set_insert, values_equal, BuiltinId, BuiltinType, BuiltinValue,
    Class, Env, EnvRef, Function, FunctionBody, Generator, Instance, IterState, ModuleValue,
    RangeValue, Value,
};

/// Maximum number of nested Python calls. See the module docs.
pub const RECURSION_LIMIT: usize = 56;

/// A positional argument list that does not allocate for the common case.
///
/// The first eight values live inline; the ninth moves them to a `Vec` and
/// every later value goes there. All access is by index, so no auxiliary
/// slice or iterator type is needed.
pub struct ArgList {
    inline: [Value; 8],
    len: usize,
    spill: Option<Vec<Value>>,
}

impl ArgList {
    pub fn new() -> ArgList {
        ArgList {
            inline: core::array::from_fn(|_| Value::None),
            len: 0,
            spill: None,
        }
    }

    pub fn push(&mut self, value: Value) {
        if let Some(spill) = &mut self.spill {
            spill.push(value);
            self.len += 1;
            return;
        }
        if self.len < self.inline.len() {
            self.inline[self.len] = value;
            self.len += 1;
            return;
        }
        let mut spill = Vec::with_capacity(self.inline.len() + 1);
        for slot in self.inline.iter_mut() {
            spill.push(core::mem::replace(slot, Value::None));
        }
        spill.push(value);
        self.spill = Some(spill);
        self.len += 1;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn get(&self, index: usize) -> Option<&Value> {
        match &self.spill {
            Some(spill) => spill.get(index),
            None => {
                if index < self.len {
                    self.inline.get(index)
                } else {
                    None
                }
            }
        }
    }

    pub fn to_vec(&self) -> Vec<Value> {
        let mut out = Vec::with_capacity(self.len);
        for index in 0..self.len {
            if let Some(value) = self.get(index) {
                out.push(value.clone());
            }
        }
        out
    }

    /// A copy, for the rare call that must forward its arguments.
    pub fn duplicate(&self) -> ArgList {
        let mut out = ArgList::new();
        for index in 0..self.len {
            if let Some(value) = self.get(index) {
                out.push(value.clone());
            }
        }
        out
    }

    /// A list with `receiver` prepended (method calls).
    pub fn with_receiver(receiver: Value, args: &ArgList) -> ArgList {
        let mut out = ArgList::new();
        out.push(receiver);
        for index in 0..args.len() {
            if let Some(value) = args.get(index) {
                out.push(value.clone());
            }
        }
        out
    }
}

/// The seed `random` starts from: a fixed constant, so every boot that does
/// not call `random.seed()` sees the same sequence.
const RNG_SEED: u64 = 0x5041_4748_5059_5448; // "PAGHPYTH"

/// What a generator's fiber body needs on its first entry. The fiber copies
/// both pointers out immediately: this record lives in the resumer's frame.
struct GeneratorStart {
    interp: *mut Interp,
    generator: *mut Generator,
}

/// Bookkeeping for one running generator body.
#[derive(Clone, Copy)]
struct ActiveFiber {
    fiber: *mut fiber::Fiber,
    generator: *mut Generator,
    /// `call_stack` length just below the generator's own frame: a `yield`
    /// parks the frames above it so the resumer's tracebacks stay its own.
    frame_depth: usize,
}

pub struct Frame {
    pub filename: Rc<str>,
    pub name: Rc<str>,
    pub line: u32,
}

pub enum Flow {
    Normal,
    Break,
    Continue,
    Return(Value),
}

pub struct Interp {
    pub module_env: EnvRef,
    pub builtins: Vec<(Rc<str>, Value)>,
    pub exceptions: Vec<Rc<Class>>,
    pub classes: Vec<(BuiltinType, Rc<Class>)>,
    pub argv: Vec<String>,
    pub search_paths: Vec<String>,
    pub call_stack: Vec<Frame>,
    pub pool: Vec<EnvRef>,
    pub active_errors: Vec<Option<PyError>>,
    pub depth: usize,
    pub class_env: Option<EnvRef>,
    /// Stable `id()` values. Python's contract is only "unique while the
    /// object lives"; a monotonic counter keyed by identity gives the same
    /// guarantee without exposing host addresses (which differ between the
    /// host harness and the kernel).
    pub ids: Vec<(Value, i64)>,
    /// Lazily created `os.stat_result` class, shared by every call.
    stat_class: Option<Rc<Class>>,
    module_cache: Vec<(String, Value)>,
    /// The `random` module's xorshift64* state. Seeded with a fixed constant so
    /// the module is deterministic until a script calls `random.seed()`.
    pub rng_state: u64,
    /// The fibers currently running, innermost last. `yield` needs the
    /// innermost one to switch back to its resumer, and each entry remembers
    /// where the generator's traceback frames start.
    active_fibers: Vec<ActiveFiber>,
}

impl Interp {
    pub fn new(argv: Vec<String>, script_dir: Option<String>) -> Interp {
        let object_class = simple_class("object", BuiltinType::Object, None);
        let type_class = simple_class("type", BuiltinType::Type, Some(object_class.clone()));
        let mut classes: Vec<(BuiltinType, Rc<Class>)> = Vec::new();
        classes.push((BuiltinType::Object, object_class.clone()));
        classes.push((BuiltinType::Type, type_class.clone()));
        for tag in [
            BuiltinType::NoneType,
            BuiltinType::Int,
            BuiltinType::Float,
            BuiltinType::Bool,
            BuiltinType::Str,
            BuiltinType::List,
            BuiltinType::Tuple,
            BuiltinType::Dict,
            BuiltinType::Set,
            BuiltinType::Bytes,
            BuiltinType::Function,
            BuiltinType::Module,
            BuiltinType::File,
            BuiltinType::Range,
            BuiltinType::Iterator,
            BuiltinType::Generator,
        ] {
            let base = if tag == BuiltinType::Bool {
                // `bool` is a subclass of `int`, exactly as in Python.
                match classes
                    .iter()
                    .find(|(candidate, _)| *candidate == BuiltinType::Int)
                {
                    Some((_, int_class)) => int_class.clone(),
                    None => object_class.clone(),
                }
            } else {
                object_class.clone()
            };
            classes.push((tag, simple_class(tag.name(), tag, Some(base))));
        }

        let base_exception =
            exception_class("BaseException", alloc::vec![object_class.clone()], None);
        let exception = exception_class("Exception", alloc::vec![base_exception.clone()], None);
        let arithmetic = exception_class("ArithmeticError", alloc::vec![exception.clone()], None);
        let zero_division =
            exception_class("ZeroDivisionError", alloc::vec![arithmetic.clone()], None);
        let overflow = exception_class("OverflowError", alloc::vec![arithmetic.clone()], None);
        let lookup = exception_class("LookupError", alloc::vec![exception.clone()], None);
        let index = exception_class("IndexError", alloc::vec![lookup.clone()], None);
        let key = exception_class("KeyError", alloc::vec![lookup.clone()], None);
        let runtime = exception_class("RuntimeError", alloc::vec![exception.clone()], None);
        let not_implemented =
            exception_class("NotImplementedError", alloc::vec![runtime.clone()], None);
        let recursion = exception_class("RecursionError", alloc::vec![runtime.clone()], None);
        let value_error = exception_class("ValueError", alloc::vec![exception.clone()], None);
        let unicode_error = exception_class("UnicodeError", alloc::vec![value_error.clone()], None);
        let unicode_decode =
            exception_class("UnicodeDecodeError", alloc::vec![unicode_error.clone()], None);
        let unicode_encode =
            exception_class("UnicodeEncodeError", alloc::vec![unicode_error.clone()], None);
        let json_decode =
            exception_class("JSONDecodeError", alloc::vec![value_error.clone()], None);
        let type_error = exception_class("TypeError", alloc::vec![exception.clone()], None);
        let name_error = exception_class("NameError", alloc::vec![exception.clone()], None);
        let attribute = exception_class("AttributeError", alloc::vec![exception.clone()], None);
        let os_error = exception_class("OSError", alloc::vec![exception.clone()], None);
        let file_error = exception_class("FileNotFoundError", alloc::vec![os_error.clone()], None);
        let file_exists =
            exception_class("FileExistsError", alloc::vec![os_error.clone()], None);
        let import_error = exception_class("ImportError", alloc::vec![exception.clone()], None);
        let module_error = exception_class(
            "ModuleNotFoundError",
            alloc::vec![import_error.clone()],
            None,
        );
        let stop_iteration = exception_class("StopIteration", alloc::vec![exception.clone()], None);
        let syntax = exception_class("SyntaxError", alloc::vec![exception.clone()], None);
        let indentation = exception_class("IndentationError", alloc::vec![syntax.clone()], None);
        let assertion = exception_class("AssertionError", alloc::vec![exception.clone()], None);
        let memory = exception_class("MemoryError", alloc::vec![exception.clone()], None);
        let system_exit = exception_class("SystemExit", alloc::vec![base_exception.clone()], None);

        let exceptions = alloc::vec![
            base_exception,
            exception,
            arithmetic,
            zero_division,
            overflow,
            lookup,
            index,
            key,
            runtime,
            not_implemented,
            recursion,
            value_error,
            unicode_error,
            unicode_decode,
            unicode_encode,
            json_decode,
            type_error,
            name_error,
            attribute,
            os_error,
            file_error,
            file_exists,
            import_error,
            module_error,
            stop_iteration,
            syntax,
            indentation,
            assertion,
            memory,
            system_exit,
        ];

        let module_env = Env::new(None);
        let mut interp = Interp {
            module_env,
            builtins: Vec::new(),
            exceptions,
            classes,
            argv,
            search_paths: Vec::new(),
            call_stack: Vec::new(),
            pool: Vec::new(),
            active_errors: Vec::new(),
            depth: 0,
            class_env: None,
            ids: Vec::new(),
            stat_class: None,
            module_cache: Vec::new(),
            rng_state: RNG_SEED,
            active_fibers: Vec::new(),
        };
        interp.init_search_paths(script_dir);
        interp.init_builtins(object_class, type_class);
        interp
    }

    fn init_search_paths(&mut self, script_dir: Option<String>) {
        let mut add = |path: &str| {
            if path.is_empty() {
                return;
            }
            if !self.search_paths.iter().any(|existing| existing == path) {
                self.search_paths.push(String::from(path));
            }
        };
        if let Some(dir) = script_dir {
            add(&dir);
        }
        add(&platform::current_dir());
        add("/mnt");
        add("/");
    }

    fn init_builtins(&mut self, object_class: Rc<Class>, type_class: Rc<Class>) {
        let mut entries: Vec<(Rc<str>, Value)> = Vec::new();
        entries.push((Rc::from("object"), Value::Class(object_class)));
        entries.push((Rc::from("type"), Value::Class(type_class)));
        for tag in [
            BuiltinType::Int,
            BuiltinType::Float,
            BuiltinType::Bool,
            BuiltinType::Str,
            BuiltinType::List,
            BuiltinType::Tuple,
            BuiltinType::Dict,
            BuiltinType::Set,
            BuiltinType::Bytes,
        ] {
            if let Some(class) = self.class_for(tag) {
                entries.push((Rc::from(tag.name()), Value::Class(class)));
            }
        }
        for (name, id) in [
            ("print", BuiltinId::Print),
            ("len", BuiltinId::Len),
            ("range", BuiltinId::Range),
            ("abs", BuiltinId::Abs),
            ("min", BuiltinId::Min),
            ("max", BuiltinId::Max),
            ("sum", BuiltinId::Sum),
            ("sorted", BuiltinId::Sorted),
            ("enumerate", BuiltinId::Enumerate),
            ("zip", BuiltinId::Zip),
            ("isinstance", BuiltinId::IsInstance),
            ("open", BuiltinId::Open),
            ("chr", BuiltinId::Chr),
            ("ord", BuiltinId::Ord),
            ("repr", BuiltinId::Repr),
            ("iter", BuiltinId::Iter),
            ("staticmethod", BuiltinId::StaticMethod),
            ("next", BuiltinId::Next),
            ("any", BuiltinId::Any),
            ("all", BuiltinId::All),
            ("divmod", BuiltinId::Divmod),
            ("pow", BuiltinId::Pow),
            ("round", BuiltinId::Round),
            ("hex", BuiltinId::Hex),
            ("id", BuiltinId::Id),
            ("map", BuiltinId::Map),
        ] {
            entries.push((Rc::from(name), Value::builtin(id)));
        }
        for class in &self.exceptions {
            entries.push((class.name.clone(), Value::Class(class.clone())));
        }
        self.builtins = entries;
    }

    pub fn class_for(&self, tag: BuiltinType) -> Option<Rc<Class>> {
        self.classes
            .iter()
            .find(|(candidate, _)| *candidate == tag)
            .map(|(_, class)| class.clone())
    }

    pub fn exception_class(&self, name: &str) -> Option<Rc<Class>> {
        self.exceptions
            .iter()
            .find(|class| &*class.name == name)
            .cloned()
    }

    // ── running a module ───────────────────────────────────────────────────

    pub fn run_module(&mut self, body: &[Stmt], filename: &str) -> PyResult<()> {
        let filename: Rc<str> = Rc::from(filename);
        {
            let mut env = self.module_env.borrow_mut();
            env.set_local(Rc::from("__name__"), value::Value::str("__main__"));
            env.set_local(Rc::from("__file__"), Value::Str(filename.clone()));
        }
        self.call_stack.push(Frame {
            filename,
            name: Rc::from("<module>"),
            line: 1,
        });
        let outcome = {
            let env = self.module_env.clone();
            self.exec_block(body, &env)
        };
        let result = match outcome {
            Ok(Flow::Normal) => Ok(()),
            Ok(_) => Err(self.err(
                "SyntaxError",
                String::from("'break' or 'return' outside a function"),
            )),
            Err(error) => Err(error),
        };
        self.call_stack.pop();
        result
    }

    fn exec_block(&mut self, statements: &[Stmt], env: &EnvRef) -> PyResult<Flow> {
        for statement in statements {
            self.set_line(statement.line);
            match self.exec_stmt(statement, env)? {
                Flow::Normal => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_stmt(&mut self, statement: &Stmt, env: &EnvRef) -> PyResult<Flow> {
        self.set_line(statement.line);
        match &statement.kind {
            StmtKind::Expr(expression) => {
                self.eval_expr(expression, env)?;
                Ok(Flow::Normal)
            }
            StmtKind::Assign { targets, value } => {
                let value = self.eval_expr(value, env)?;
                for target in targets {
                    self.assign_target(target, value.clone(), env)?;
                }
                Ok(Flow::Normal)
            }
            StmtKind::AugAssign { target, op, value } => {
                self.exec_aug_assign(target, *op, value, env)?;
                Ok(Flow::Normal)
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for (condition, body) in branches {
                    if self.eval_expr(condition, env)?.truthy() {
                        return self.exec_block(body, env);
                    }
                }
                self.exec_block(else_body, env)
            }
            StmtKind::While { condition, body, else_body } => {
                let mut broke = false;
                loop {
                    self.set_line(statement.line);
                    if !self.eval_expr(condition, env)?.truthy() {
                        break;
                    }
                    match self.exec_block(body, env)? {
                        Flow::Normal => {}
                        Flow::Continue => continue,
                        Flow::Break => { broke = true; break; }
                        other => return Ok(other),
                    }
                }
                if !broke { self.exec_block(else_body, env) } else { Ok(Flow::Normal) }
            }
            StmtKind::For {
                target,
                iterable,
                body,
                else_body,
            } => {
                let iterable_value = self.eval_expr(iterable, env)?;
                let iterator = self.make_iterator(iterable_value)?;
                let mut broke = false;
                loop {
                    self.set_line(statement.line);
                    match self.next_from_iterator(&iterator)? {
                        None => break,
                        Some(item) => {
                            self.assign_target(target, item, env)?;
                            match self.exec_block(body, env)? {
                                Flow::Normal => {}
                                Flow::Continue => continue,
                                Flow::Break => { broke = true; break; }
                                other => return Ok(other),
                            }
                        }
                    }
                }
                if !broke { self.exec_block(else_body, env) } else { Ok(Flow::Normal) }
            }
            StmtKind::Del(targets) => {
                for target in targets {
                    self.delete_target(target, env)?;
                }
                Ok(Flow::Normal)
            }
            StmtKind::Break => Ok(Flow::Break),
            StmtKind::Continue => Ok(Flow::Continue),
            StmtKind::Pass => Ok(Flow::Normal),
            StmtKind::Def {
                name,
                params,
                body,
                decorators,
                is_generator,
            } => {
                let function =
                    self.make_function(name, params, body, *is_generator, statement.line, env)?;
                let value =
                    self.apply_decorators(Value::Function(Rc::new(function)), decorators, env)?;
                self.set_name(env, name.clone(), value)?;
                Ok(Flow::Normal)
            }
            StmtKind::Return(expression) => match expression {
                Some(expression) => Ok(Flow::Return(self.eval_expr(expression, env)?)),
                None => Ok(Flow::Return(Value::None)),
            },
            StmtKind::Class {
                name,
                bases,
                body,
                decorators,
            } => {
                let value = self.exec_class(name, bases, body, env)?;
                let value = self.apply_decorators(value, decorators, env)?;
                self.set_name(env, name.clone(), value)?;
                Ok(Flow::Normal)
            }
            StmtKind::Try {
                body,
                handlers,
                else_body,
                finally_body,
            } => self.exec_try(body, handlers, else_body, finally_body, env),
            StmtKind::With { items, body } => self.exec_with(items, body, env),
            StmtKind::Raise(expression) => match expression {
                Some(expression) => {
                    let value = self.eval_expr(expression, env)?;
                    Err(self.error_from_raised_value(value))
                }
                None => match self.active_errors.last_mut() {
                    Some(slot) => match slot.take() {
                        Some(error) => Err(error),
                        None => Err(self.err(
                            "RuntimeError",
                            String::from("No active exception to re-raise"),
                        )),
                    },
                    None => Err(self.err(
                        "RuntimeError",
                        String::from("No active exception to re-raise"),
                    )),
                },
            },
            StmtKind::Import(names) => {
                for alias in names {
                    // `import os.path` loads the submodule but binds the top
                    // package, exactly as Python does; the only way to bind
                    // `path` itself is `import os.path as path`.
                    let module = self.import_module(&alias.name)?;
                    let (bind_name, module) = match &alias.alias {
                        Some(alias) => (alias.clone(), module),
                        None => match alias.name.split('.').next() {
                            Some(first) if first.len() < alias.name.len() => {
                                (Rc::from(first), self.import_module(first)?)
                            }
                            _ => (alias.name.clone(), module),
                        },
                    };
                    self.set_name(env, bind_name, module)?;
                }
                Ok(Flow::Normal)
            }
            StmtKind::FromImport { module, names } => {
                let module_value = self.import_module(module)?;
                for alias in names {
                    if &*alias.name == "*" {
                        if let Value::Module(module_object) = &module_value {
                            let attrs: Vec<(Rc<str>, Value)> = module_object.attrs.borrow().clone();
                            for (name, value) in attrs {
                                if !name.starts_with('_') {
                                    self.set_name(env, name, value)?;
                                }
                            }
                        }
                        continue;
                    }
                    let value = match &module_value {
                        Value::Module(module_object) => attr_get(&module_object.attrs, &alias.name),
                        _ => None,
                    };
                    match value {
                        Some(value) => {
                            let bind_name = match &alias.alias {
                                Some(alias) => alias.clone(),
                                None => alias.name.clone(),
                            };
                            self.set_name(env, bind_name, value)?;
                        }
                        None => {
                            return Err(self.err(
                                "ImportError",
                                format!("cannot import name '{}' from '{}'", alias.name, module),
                            ))
                        }
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Global(names) => {
                let mut borrowed = env.borrow_mut();
                for name in names {
                    if !borrowed.globals.iter().any(|existing| existing == name) {
                        borrowed.globals.push(name.clone());
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Nonlocal(names) => {
                let mut borrowed = env.borrow_mut();
                for name in names {
                    if !borrowed.nonlocals.iter().any(|existing| existing == name) {
                        borrowed.nonlocals.push(name.clone());
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Assert { test, message } => {
                if self.eval_expr(test, env)?.truthy() {
                    Ok(Flow::Normal)
                } else {
                    let text = match message {
                        Some(expression) => self.eval_expr(expression, env)?.to_str(),
                        None => String::from("assertion failed"),
                    };
                    Err(self.err("AssertionError", text))
                }
            }
        }
    }

    fn make_function(
        &mut self,
        name: &Name,
        params: &[Param],
        body: &Rc<Vec<Stmt>>,
        is_generator: bool,
        def_line: u32,
        env: &EnvRef,
    ) -> PyResult<Function> {
        let mut bound = Vec::with_capacity(params.len());
        let mut varargs = None;
        let mut kwargs = None;
        for parameter in params {
            match parameter.kind {
                ParamKind::Normal => {
                    let default = match &parameter.default {
                        Some(expression) => Some(self.eval_expr(expression, env)?),
                        None => None,
                    };
                    bound.push((parameter.name.clone(), default));
                }
                ParamKind::VarArgs => varargs = Some(parameter.name.clone()),
                ParamKind::KwArgs => kwargs = Some(parameter.name.clone()),
            }
        }
        let in_class_body = match &self.class_env {
            Some(class_env) => Rc::ptr_eq(class_env, env),
            None => false,
        };
        let closure = if in_class_body {
            match env.borrow().parent.clone() {
                Some(parent) => parent,
                None => env.clone(),
            }
        } else {
            env.clone()
        };
        Ok(Function {
            name: name.clone(),
            params: bound,
            varargs,
            kwargs,
            body: FunctionBody::Block(body.clone()),
            closure,
            is_generator,
            filename: self.current_filename(),
            def_line,
        })
    }

    /// Enter every manager left to right, run the body, then leave right to
    /// left. An `__exit__` returning a truthy value suppresses the exception.
    fn exec_with(&mut self, items: &[WithItem], body: &[Stmt], env: &EnvRef) -> PyResult<Flow> {
        self.exec_with_from(items, 0, body, env)
    }

    fn exec_with_from(
        &mut self,
        items: &[WithItem],
        index: usize,
        body: &[Stmt],
        env: &EnvRef,
    ) -> PyResult<Flow> {
        let Some(item) = items.get(index) else {
            return self.exec_block(body, env);
        };
        let manager = self.eval_expr(&item.context, env)?;
        let enter = self.get_attribute(&manager, "__enter__")?;
        let entered = self.call_value(enter, ArgList::new(), Vec::new())?;
        if let Some(target) = &item.target {
            self.assign_target(target, entered, env)?;
        }
        let outcome = self.exec_with_from(items, index + 1, body, env);
        let exit = self.get_attribute(&manager, "__exit__")?;
        match outcome {
            Ok(flow) => {
                let mut args = ArgList::new();
                args.push(Value::None);
                args.push(Value::None);
                args.push(Value::None);
                self.call_value(exit, args, Vec::new())?;
                Ok(flow)
            }
            Err(error) => {
                let (kind, value) = self.exception_type_and_value(&error);
                let mut args = ArgList::new();
                args.push(kind);
                args.push(value);
                args.push(Value::None);
                let suppress = self.call_value(exit, args, Vec::new())?.truthy();
                if suppress {
                    Ok(Flow::Normal)
                } else {
                    Err(error)
                }
            }
        }
    }

    fn exception_type_and_value(&self, error: &PyError) -> (Value, Value) {
        let class = match self.error_class(error) {
            Some(class) => class,
            None => match self.exception_class("Exception") {
                Some(class) => class,
                None => return (Value::None, Value::None),
            },
        };
        let value = self.exception_value(error);
        (Value::Class(class), value)
    }

    fn make_lambda_function(
        &mut self,
        params: &[Param],
        body: &Rc<Expr>,
        env: &EnvRef,
    ) -> PyResult<Function> {
        let mut bound = Vec::with_capacity(params.len());
        let mut varargs = None;
        let mut kwargs = None;
        for parameter in params {
            match parameter.kind {
                ParamKind::Normal => {
                    let default = match &parameter.default {
                        Some(expression) => Some(self.eval_expr(expression, env)?),
                        None => None,
                    };
                    bound.push((parameter.name.clone(), default));
                }
                ParamKind::VarArgs => varargs = Some(parameter.name.clone()),
                ParamKind::KwArgs => kwargs = Some(parameter.name.clone()),
            }
        }
        let in_class_body = match &self.class_env {
            Some(class_env) => Rc::ptr_eq(class_env, env),
            None => false,
        };
        let closure = if in_class_body {
            match env.borrow().parent.clone() {
                Some(parent) => parent,
                None => env.clone(),
            }
        } else {
            env.clone()
        };
        let def_line = match self.call_stack.last() {
            Some(frame) => frame.line,
            None => 1,
        };
        Ok(Function {
            name: Rc::from("<lambda>"),
            params: bound,
            varargs,
            kwargs,
            body: FunctionBody::Expression(body.clone()),
            closure,
            is_generator: false,
            filename: self.current_filename(),
            def_line,
        })
    }

    /// Execute a class body and build the class object. The caller binds the
    /// name, because a decorated class is bound to the decorator's result.
    fn exec_class(
        &mut self,
        name: &Name,
        base_expressions: &[Expr],
        body: &[Stmt],
        env: &EnvRef,
    ) -> PyResult<Value> {
        let mut bases = Vec::new();
        let object_class = match self.class_for(BuiltinType::Object) {
            Some(class) => class,
            None => {
                return Err(self.err(
                    "RuntimeError",
                    String::from("interpreter is missing the object class"),
                ))
            }
        };
        for expression in base_expressions {
            let value = self.eval_expr(expression, env)?;
            match value {
                Value::Class(class) => bases.push(class),
                other => {
                    return Err(self.err(
                        "TypeError",
                        format!("class base must be a class, not '{}'", other.type_name()),
                    ))
                }
            }
        }
        if bases.is_empty() {
            bases.push(object_class);
        }
        let is_exception = bases.iter().any(|base| base.is_exception);
        let class = Rc::new(Class {
            name: name.clone(),
            bases,
            attrs: RefCell::new(Vec::new()),
            builtin: None,
            is_exception,
        });
        let class_env = Env::new(Some(env.clone()));
        let previous = self.class_env.replace(class_env.clone());
        let outcome = self.exec_block(body, &class_env);
        self.class_env = previous;
        match outcome {
            Ok(Flow::Normal) => {}
            Ok(_) => {
                return Err(self.err(
                    "SyntaxError",
                    String::from("'break' or 'return' outside a function"),
                ))
            }
            Err(error) => return Err(error),
        }
        let vars = match Rc::try_unwrap(class_env) {
            Ok(cell) => cell.into_inner().vars,
            Err(still_shared) => still_shared.borrow().vars.clone(),
        };
        for (attr_name, value) in vars {
            attr_set(&class.attrs, attr_name, value);
        }
        Ok(Value::Class(class))
    }

    fn exec_try(
        &mut self,
        body: &[Stmt],
        handlers: &[Handler],
        else_body: &[Stmt],
        finally_body: &[Stmt],
        env: &EnvRef,
    ) -> PyResult<Flow> {
        let mut outcome = self.exec_block(body, env);
        if matches!(&outcome, Ok(Flow::Normal)) && !else_body.is_empty() {
            outcome = self.exec_block(else_body, env);
        }
        outcome = match outcome {
            Err(error) => self.run_handlers(error, handlers, env),
            ok => ok,
        };
        if !finally_body.is_empty() {
            match self.exec_block(finally_body, env) {
                Ok(Flow::Normal) => {}
                Ok(flow) => return Ok(flow),
                Err(error) => return Err(error),
            }
        }
        outcome
    }

    fn run_handlers(
        &mut self,
        error: PyError,
        handlers: &[Handler],
        env: &EnvRef,
    ) -> PyResult<Flow> {
        for handler in handlers {
            if !self.handler_matches(&error, handler, env)? {
                continue;
            }
            let exception_value = self.exception_value(&error);
            if let Some(name) = &handler.name {
                self.set_name(env, name.clone(), exception_value)?;
            }
            self.active_errors.push(Some(error));
            let outcome = self.exec_block(&handler.body, env);
            self.active_errors.pop();
            return outcome;
        }
        Err(error)
    }

    fn handler_matches(
        &mut self,
        error: &PyError,
        handler: &Handler,
        env: &EnvRef,
    ) -> PyResult<bool> {
        let Some(kind_expression) = &handler.kind else {
            return Ok(true);
        };
        let kind = self.eval_expr(kind_expression, env)?;
        let classes = self.except_classes(&kind)?;
        let error_class = self.error_class(error);
        for target in classes {
            if let Some(error_class) = &error_class {
                if class_is_subclass(error_class, &target) {
                    return Ok(true);
                }
            } else if &*target.name == error.type_name() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn except_classes(&self, value: &Value) -> PyResult<Vec<Rc<Class>>> {
        match value {
            Value::Class(class) if class.is_exception => Ok(alloc::vec![class.clone()]),
            Value::Tuple(items) => {
                let mut classes = Vec::with_capacity(items.len());
                for item in items.iter() {
                    match item {
                        Value::Class(class) if class.is_exception => classes.push(class.clone()),
                        _ => {
                            return Err(self.err(
                                "TypeError",
                                String::from(
                                    "catching classes that do not inherit from BaseException is not allowed",
                                ),
                            ))
                        }
                    }
                }
                Ok(classes)
            }
            _ => Err(self.err(
                "TypeError",
                String::from(
                    "except clause requires an exception class or a tuple of exception classes",
                ),
            )),
        }
    }

    fn error_class(&self, error: &PyError) -> Option<Rc<Class>> {
        match &error.value {
            Some(Value::Instance(instance)) => Some(instance.class.clone()),
            _ => self.exception_class(error.type_name()),
        }
    }

    fn exception_value(&self, error: &PyError) -> Value {
        if let Some(value) = &error.value {
            return value.clone();
        }
        let class = match self.exception_class(error.type_name()) {
            Some(class) => class,
            None => match self.exception_class("Exception") {
                Some(class) => class,
                None => return Value::str(&error.message),
            },
        };
        self.make_exception_instance(class, alloc::vec![Value::str(&error.message)])
    }

    fn make_exception_instance(&self, class: Rc<Class>, args: Vec<Value>) -> Value {
        let instance = Instance {
            class,
            attrs: RefCell::new(Vec::new()),
        };
        attr_set(
            &instance.attrs,
            Rc::from("args"),
            Value::Tuple(Rc::new(args)),
        );
        Value::Instance(Rc::new(instance))
    }

    fn error_from_raised_value(&self, value: Value) -> PyError {
        match value {
            Value::Class(class) => {
                if !class.is_exception {
                    return self.err(
                        "TypeError",
                        String::from("exceptions must derive from BaseException"),
                    );
                }
                let instance = self.make_exception_instance(class.clone(), Vec::new());
                self.error_from_exception(&class, instance)
            }
            Value::Instance(instance) => {
                if !instance.class.is_exception {
                    return self.err(
                        "TypeError",
                        String::from("exceptions must derive from BaseException"),
                    );
                }
                let class = instance.class.clone();
                self.error_from_exception(&class, Value::Instance(instance))
            }
            other => self.err(
                "TypeError",
                format!(
                    "exceptions must derive from BaseException, not '{}'",
                    other.type_name()
                ),
            ),
        }
    }

    fn error_from_exception(&self, class: &Rc<Class>, instance: Value) -> PyError {
        let message = instance.to_str();
        let mut error = PyError::new(ErrorName::Dynamic(class.name.clone()), message);
        error.frames = self.snapshot();
        error.value = Some(instance);
        error
    }

    // ── assignment ─────────────────────────────────────────────────────────

    fn exec_aug_assign(
        &mut self,
        target: &Expr,
        op: BinOp,
        value_expression: &Expr,
        env: &EnvRef,
    ) -> PyResult<()> {
        match target {
            Expr::Name(name) => {
                let current = self.lookup_name(env, name)?;
                let right = self.eval_expr(value_expression, env)?;
                let result = self.binary_op(op, current, right)?;
                self.set_name(env, name.clone(), result)
            }
            Expr::Index { target, index } => {
                let object = self.eval_expr(target, env)?;
                let index = self.eval_expr(index, env)?;
                let current = self.index_value(&object, &index)?;
                let right = self.eval_expr(value_expression, env)?;
                let result = self.binary_op(op, current, right)?;
                self.set_index(object, index, result)
            }
            Expr::Attribute { target, name } => {
                let object = self.eval_expr(target, env)?;
                let current = self.get_attribute(&object, name)?;
                let right = self.eval_expr(value_expression, env)?;
                let result = self.binary_op(op, current, right)?;
                self.set_attribute(&object, name.clone(), result)
            }
            _ => Err(self.err(
                "SyntaxError",
                String::from("illegal target for an augmented assignment"),
            )),
        }
    }

    fn assign_target(&mut self, target: &Expr, value: Value, env: &EnvRef) -> PyResult<()> {
        match target {
            Expr::Name(name) => self.set_name(env, name.clone(), value),
            Expr::Tuple(items) | Expr::List(items) => {
                let values = self.collect_iterable(value)?;
                if values.len() != items.len() {
                    return Err(self.err(
                        "ValueError",
                        format!(
                            "expected {} values to unpack, got {}",
                            items.len(),
                            values.len()
                        ),
                    ));
                }
                for (item, item_value) in items.iter().zip(values.into_iter()) {
                    self.assign_target(item, item_value, env)?;
                }
                Ok(())
            }
            Expr::Attribute { target, name } => {
                let object = self.eval_expr(target, env)?;
                self.set_attribute(&object, name.clone(), value)
            }
            Expr::Index { target, index } => {
                let object = self.eval_expr(target, env)?;
                let index_value = self.eval_expr(index, env)?;
                self.set_index(object, index_value, value)
            }
            Expr::Slice { target, start, stop, step } => {
                let object = self.eval_expr(target, env)?;
                let start = match start { Some(e) => Some(self.eval_slice_bound(e, env)?), None => None };
                let stop = match stop { Some(e) => Some(self.eval_slice_bound(e, env)?), None => None };
                let step = match step { Some(e) => Some(self.eval_slice_bound(e, env)?), None => None };
                self.set_slice(object, start, stop, step, value)
            }
            _ => Err(self.err(
                "SyntaxError",
                String::from("cannot assign to this expression"),
            )),
        }
    }

    fn set_name(&mut self, env: &EnvRef, name: Name, value: Value) -> PyResult<()> {
        let (is_global, is_nonlocal) = {
            let borrowed = env.borrow();
            (
                borrowed.globals.iter().any(|candidate| *candidate == name),
                borrowed
                    .nonlocals
                    .iter()
                    .any(|candidate| *candidate == name),
            )
        };
        if is_global {
            let module = self.module_env.clone();
            let mut borrowed = module.borrow_mut();
            borrowed.set_local(name, value);
            return Ok(());
        }
        if is_nonlocal {
            match env_find_nonlocal(env, &name) {
                Some(frame) => {
                    frame.borrow_mut().set_local(name, value);
                    Ok(())
                }
                None => Err(self.err(
                    "NameError",
                    format!("no binding for nonlocal '{}' found", name),
                )),
            }
        } else {
            env.borrow_mut().set_local(name, value);
            Ok(())
        }
    }

    fn lookup_name(&self, env: &EnvRef, name: &str) -> PyResult<Value> {
        if let Some(value) = env_lookup(env, name) {
            return Ok(value);
        }
        if let Some((_, value)) = self.builtins.iter().find(|(key, _)| &**key == name) {
            return Ok(value.clone());
        }
        Err(self.err("NameError", format!("name '{}' is not defined", name)))
    }

    // ── expressions ────────────────────────────────────────────────────────

    fn eval_expr(&mut self, expression: &Expr, env: &EnvRef) -> PyResult<Value> {
        match expression {
            Expr::None => Ok(Value::None),
            Expr::True => Ok(Value::Bool(true)),
            Expr::False => Ok(Value::Bool(false)),
            Expr::Int(value) => Ok(Value::Int(*value)),
            Expr::Float(value) => Ok(Value::Float(*value)),
            Expr::Str(text) => Ok(Value::Str(text.clone())),
            Expr::Bytes(data) => Ok(Value::Bytes(data.clone())),
            Expr::Name(name) => self.lookup_name(env, name),
            Expr::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.eval_expr(item, env)?);
                }
                Ok(Value::list(values))
            }
            Expr::Tuple(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.eval_expr(item, env)?);
                }
                Ok(Value::tuple(values))
            }
            Expr::Dict(entries) => {
                let mut values: Vec<(Value, Value)> = Vec::with_capacity(entries.len());
                for (key_expression, value_expression) in entries {
                    let key = self.eval_expr(key_expression, env)?;
                    if !key.is_hashable() {
                        return Err(self.err(
                            "TypeError",
                            format!("unhashable type: '{}'", key.type_name()),
                        ));
                    }
                    let value = self.eval_expr(value_expression, env)?;
                    dict_set(&mut values, key, value);
                }
                Ok(Value::dict(values))
            }
            Expr::Set(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    let value = self.eval_expr(item, env)?;
                    if !value.is_hashable() {
                        return Err(self.err(
                            "TypeError",
                            format!("unhashable type: '{}'", value.type_name()),
                        ));
                    }
                    set_insert(&mut values, value);
                }
                Ok(Value::set(values))
            }
            Expr::FString(parts) => {
                let mut out = String::new();
                for part in parts {
                    match part {
                        FStringPart::Text(text) => out.push_str(text),
                        FStringPart::Value {
                            expression,
                            spec,
                            conversion,
                        } => {
                            let value = self.eval_expr(expression, env)?;
                            out.push_str(&builtins::format_value(
                                self,
                                &value,
                                spec.as_deref(),
                                *conversion,
                            )?);
                        }
                    }
                }
                Ok(Value::Str(Rc::from(out.as_str())))
            }
            Expr::Lambda { params, body } => {
                let function = self.make_lambda_function(params, body, env)?;
                Ok(Value::Function(Rc::new(function)))
            }
            Expr::Yield(value) => {
                let produced = match value {
                    Some(expression) => self.eval_expr(expression, env)?,
                    None => Value::None,
                };
                self.fiber_yield(produced)
            }
            Expr::YieldFrom(iterable) => {
                // `yield from` is the iteration protocol in this interpreter:
                // the operand is evaluated once, then every value it produces
                // is yielded on to the resumer. A delegated generator is
                // resumed through the same nesting path as a `for` loop (the
                // outer fiber resumes the inner one), so laziness and the
                // 8-suspended-fiber limit are unchanged. The expression
                // evaluates to `None`: there is no `return` value to capture.
                let iterable = self.eval_expr(iterable, env)?;
                let iterator = self.make_iterator(iterable)?;
                while let Some(item) = self.next_from_iterator(&iterator)? {
                    self.fiber_yield(item)?;
                }
                Ok(Value::None)
            }
            Expr::GeneratorExp { element, generators } => {
                self.make_generator_expression(element, generators, env)
            }
            Expr::Unary { op, value } => {
                let value = self.eval_expr(value, env)?;
                match op {
                    UnOp::Not => Ok(Value::Bool(!value.truthy())),
                    UnOp::Minus => match value {
                        Value::Float(number) => Ok(Value::Float(-number)),
                        other => match other.as_int() {
                            Some(number) => match number.checked_neg() {
                                Some(result) => Ok(Value::Int(result)),
                                None => Err(self.err(
                                    "OverflowError",
                                    String::from("integer negation overflow"),
                                )),
                            },
                            None => Err(self.err(
                                "TypeError",
                                format!("bad operand type for unary -: '{}'", other.type_name()),
                            )),
                        },
                    },
                    UnOp::Plus => match value {
                        Value::Float(number) => Ok(Value::Float(number)),
                        other => match other.as_int() {
                            Some(number) => Ok(Value::Int(number)),
                            None => Err(self.err(
                                "TypeError",
                                format!("bad operand type for unary +: '{}'", other.type_name()),
                            )),
                        },
                    },
                }
            }
            Expr::Binary { op, left, right } => {
                let left = self.eval_expr(left, env)?;
                let right = self.eval_expr(right, env)?;
                self.binary_op(*op, left, right)
            }
            Expr::BoolOp { op, values } => {
                let mut last = Value::None;
                for value in values {
                    last = self.eval_expr(value, env)?;
                    match op {
                        BoolOp::And => {
                            if !last.truthy() {
                                return Ok(last);
                            }
                        }
                        BoolOp::Or => {
                            if last.truthy() {
                                return Ok(last);
                            }
                        }
                    }
                }
                Ok(last)
            }
            Expr::Compare {
                left,
                ops,
                comparators,
            } => {
                let mut current = self.eval_expr(left, env)?;
                for (op, comparator) in ops.iter().zip(comparators.iter()) {
                    let right = self.eval_expr(comparator, env)?;
                    if !self.compare_op(*op, &current, &right)? {
                        return Ok(Value::Bool(false));
                    }
                    current = right;
                }
                Ok(Value::Bool(true))
            }
            Expr::Call {
                function,
                args,
                kwargs,
            } => {
                let function = self.eval_expr(function, env)?;
                let mut arguments = ArgList::new();
                for argument in args {
                    match argument {
                        CallArg::Value(expression) => {
                            let value = self.eval_expr(expression, env)?;
                            arguments.push(value);
                        }
                        CallArg::Unpack(expression) => {
                            let iterable = self.eval_expr(expression, env)?;
                            for value in self.collect_iterable(iterable)? {
                                arguments.push(value);
                            }
                        }
                    }
                }
                let mut keywords = Vec::with_capacity(kwargs.len());
                for keyword in kwargs {
                    match keyword {
                        CallKeyword::Named(name, expression) => {
                            let value = self.eval_expr(expression, env)?;
                            if keywords.iter().any(|(key, _)| key == name) {
                                return Err(self.err(
                                    "TypeError",
                                    format!(
                                        "got multiple values for keyword argument '{}'",
                                        name
                                    ),
                                ));
                            }
                            keywords.push((name.clone(), value));
                        }
                        CallKeyword::Unpack(expression) => {
                            let mapping = self.eval_expr(expression, env)?;
                            let entries = match &mapping {
                                Value::Dict(entries) => entries.clone(),
                                other => {
                                    return Err(self.err(
                                        "TypeError",
                                        format!(
                                            "argument after ** must be a mapping, not '{}'",
                                            other.type_name()
                                        ),
                                    ))
                                }
                            };
                            for (key, value) in entries.borrow().iter() {
                                let name = match key {
                                    Value::Str(text) => text.clone(),
                                    other => {
                                        return Err(self.err(
                                            "TypeError",
                                            format!(
                                                "keywords must be strings, not '{}'",
                                                other.type_name()
                                            ),
                                        ))
                                    }
                                };
                                if keywords.iter().any(|(existing, _)| existing == &name) {
                                    return Err(self.err(
                                        "TypeError",
                                        format!(
                                            "got multiple values for keyword argument '{}'",
                                            name
                                        ),
                                    ));
                                }
                                keywords.push((name, value.clone()));
                            }
                        }
                    }
                }
                self.call_value(function, arguments, keywords)
            }
            Expr::Index { target, index } => {
                let target = self.eval_expr(target, env)?;
                let index = self.eval_expr(index, env)?;
                self.index_value(&target, &index)
            }
            Expr::Slice {
                target,
                start,
                stop,
                step,
            } => {
                let target = self.eval_expr(target, env)?;
                let start = match start {
                    Some(expression) => Some(self.eval_slice_bound(expression, env)?),
                    None => None,
                };
                let stop = match stop {
                    Some(expression) => Some(self.eval_slice_bound(expression, env)?),
                    None => None,
                };
                let step = match step {
                    Some(expression) => Some(self.eval_slice_bound(expression, env)?),
                    None => None,
                };
                self.slice_value(&target, start, stop, step)
            }
            Expr::Attribute { target, name } => {
                let target = self.eval_expr(target, env)?;
                self.get_attribute(&target, name)
            }
            Expr::ListComp {
                element,
                generators,
            } => self.eval_list_comprehension(element, generators, env),
            Expr::SetComp {
                element,
                generators,
            } => self.eval_set_comprehension(element, generators, env),
            Expr::DictComp {
                key,
                value,
                generators,
            } => self.eval_dict_comprehension(key, value, generators, env),
            Expr::IfExpr {
                body,
                condition,
                otherwise,
            } => {
                if self.eval_expr(condition, env)?.truthy() {
                    self.eval_expr(body, env)
                } else {
                    self.eval_expr(otherwise, env)
                }
            }
        }
    }

    fn eval_slice_bound(&mut self, expression: &Expr, env: &EnvRef) -> PyResult<i64> {
        let value = self.eval_expr(expression, env)?;
        match value.as_int() {
            Some(number) => Ok(number),
            None => Err(self.err(
                "TypeError",
                format!(
                    "slice indices must be integers or None, not '{}'",
                    value.type_name()
                ),
            )),
        }
    }

    fn eval_list_comprehension(
        &mut self,
        element: &Expr,
        generators: &[CompGen],
        env: &EnvRef,
    ) -> PyResult<Value> {
        let comp_env = Env::new(Some(env.clone()));
        let mut items = Vec::new();
        let outcome = self.run_generators(generators, 0, &comp_env, element, &mut items);
        self.release_env(comp_env);
        outcome?;
        Ok(Value::list(items))
    }

    fn eval_set_comprehension(
        &mut self,
        element: &Expr,
        generators: &[CompGen],
        env: &EnvRef,
    ) -> PyResult<Value> {
        let comp_env = Env::new(Some(env.clone()));
        let mut items = Vec::new();
        let outcome = self.run_set_generators(generators, 0, &comp_env, element, &mut items);
        self.release_env(comp_env);
        outcome?;
        Ok(Value::set(items))
    }

    fn run_set_generators(
        &mut self,
        generators: &[CompGen],
        index: usize,
        env: &EnvRef,
        element: &Expr,
        out: &mut Vec<Value>,
    ) -> PyResult<()> {
        if index >= generators.len() {
            let value = self.eval_expr(element, env)?;
            if !value.is_hashable() {
                return Err(self.err(
                    "TypeError",
                    format!("unhashable type: '{}'", value.type_name()),
                ));
            }
            set_insert(out, value);
            return Ok(());
        }
        let generator = &generators[index];
        let iterable = self.eval_expr(&generator.iterable, env)?;
        let iterator = self.make_iterator(iterable)?;
        while let Some(item) = self.next_from_iterator(&iterator)? {
            self.assign_target(&generator.target, item, env)?;
            let mut keep = true;
            for condition in &generator.conditions {
                if !self.eval_expr(condition, env)?.truthy() {
                    keep = false;
                    break;
                }
            }
            if keep {
                self.run_set_generators(generators, index + 1, env, element, out)?;
            }
        }
        Ok(())
    }

    fn eval_dict_comprehension(
        &mut self,
        key: &Expr,
        value: &Expr,
        generators: &[CompGen],
        env: &EnvRef,
    ) -> PyResult<Value> {
        let comp_env = Env::new(Some(env.clone()));
        let mut entries: Vec<(Value, Value)> = Vec::new();
        let outcome =
            self.run_dict_generators(generators, 0, &comp_env, key, value, &mut entries);
        self.release_env(comp_env);
        outcome?;
        Ok(Value::dict(entries))
    }

    fn run_dict_generators(
        &mut self,
        generators: &[CompGen],
        index: usize,
        env: &EnvRef,
        key_expression: &Expr,
        value_expression: &Expr,
        out: &mut Vec<(Value, Value)>,
    ) -> PyResult<()> {
        if index >= generators.len() {
            let key = self.eval_expr(key_expression, env)?;
            if !key.is_hashable() {
                return Err(self.err(
                    "TypeError",
                    format!("unhashable type: '{}'", key.type_name()),
                ));
            }
            let value = self.eval_expr(value_expression, env)?;
            dict_set(out, key, value);
            return Ok(());
        }
        let generator = &generators[index];
        let iterable = self.eval_expr(&generator.iterable, env)?;
        let iterator = self.make_iterator(iterable)?;
        while let Some(item) = self.next_from_iterator(&iterator)? {
            self.assign_target(&generator.target, item, env)?;
            let mut keep = true;
            for condition in &generator.conditions {
                if !self.eval_expr(condition, env)?.truthy() {
                    keep = false;
                    break;
                }
            }
            if keep {
                self.run_dict_generators(
                    generators,
                    index + 1,
                    env,
                    key_expression,
                    value_expression,
                    out,
                )?;
            }
        }
        Ok(())
    }

    fn run_generators(
        &mut self,
        generators: &[CompGen],
        index: usize,
        env: &EnvRef,
        element: &Expr,
        out: &mut Vec<Value>,
    ) -> PyResult<()> {
        if index >= generators.len() {
            out.push(self.eval_expr(element, env)?);
            return Ok(());
        }
        let generator = &generators[index];
        let iterable = self.eval_expr(&generator.iterable, env)?;
        let iterator = self.make_iterator(iterable)?;
        while let Some(item) = self.next_from_iterator(&iterator)? {
            self.assign_target(&generator.target, item, env)?;
            let mut keep = true;
            for condition in &generator.conditions {
                if !self.eval_expr(condition, env)?.truthy() {
                    keep = false;
                    break;
                }
            }
            if keep {
                self.run_generators(generators, index + 1, env, element, out)?;
            }
        }
        Ok(())
    }

    /// Apply `@decorator` expressions to a freshly created function or class.
    ///
    /// Python applies them bottom-up: the decorator written last is called
    /// first, and each one receives the previous result. They are evaluated
    /// here, in the enclosing scope, after the definition was built.
    fn apply_decorators(
        &mut self,
        value: Value,
        decorators: &[Expr],
        env: &EnvRef,
    ) -> PyResult<Value> {
        let mut value = value;
        for expression in decorators.iter().rev() {
            let decorator = self.eval_expr(expression, env)?;
            let mut args = ArgList::new();
            args.push(value);
            value = self.call_value(decorator, args, Vec::new())?;
        }
        Ok(value)
    }

    // ── generators ─────────────────────────────────────────────────────────

    /// The entry the fiber runs: the generator body, on the fiber's own stack.
    ///
    /// It copies the two pointers out of the start record first, because that
    /// record lives in the resumer's frame and dies when the body first yields.
    unsafe extern "C" fn generator_fiber_entry(start: *mut u8) -> ! {
        let start = &*(start as *const GeneratorStart);
        let interp = &mut *start.interp;
        let generator = start.generator;
        let function = (*generator).function.clone();
        let env = (*generator).env.clone();
        let fiber = (*generator)
            .fiber
            .as_mut()
            .map(|boxed| &mut **boxed as *mut fiber::Fiber);
        let outcome = match interp.run_generator_body(&function, &env) {
            Ok(flow) => match flow {
                Flow::Return(value) => Ok(value),
                Flow::Normal => Ok(Value::None),
                _ => Err(interp.err(
                    "SyntaxError",
                    String::from("'break' or 'return' outside a loop/function"),
                )),
            },
            Err(error) => Err(error),
        };
        if let Some(fiber) = fiber {
            (*generator).finished = true;
            (*generator).running = false;
            match outcome {
                Ok(value) => (*generator).yielded = Some(value),
                Err(error) => (*generator).failed = Some(error),
            }
            // Hand control back for the last time, then park: a finished
            // generator is never resumed, so this is a safety net rather than
            // a real path.
            loop {
                (*fiber).yield_to_caller();
            }
        }
        loop {
            core::hint::spin_loop();
        }
    }

    /// Run one generator body: a statement block, a lambda expression, or a
    /// generator expression. Called on the fiber's stack.
    fn run_generator_body(&mut self, function: &Rc<Function>, env: &EnvRef) -> PyResult<Flow> {
        self.call_stack.push(Frame {
            filename: function.filename.clone(),
            name: function.name.clone(),
            line: function.def_line,
        });
        self.depth += 1;
        let outcome = match &function.body {
            FunctionBody::Block(body) => self.exec_block(body, env),
            FunctionBody::Expression(expression) => {
                self.eval_expr(expression, env).map(Flow::Return)
            }
            FunctionBody::GeneratorExpr { element, generators } => self
                .run_generator_expression(element, generators, env)
                .map(|_| Flow::Normal),
        };
        self.depth -= 1;
        self.call_stack.pop();
        outcome
    }

    /// Walk a comprehension, yielding each produced element.
    fn run_generator_expression(
        &mut self,
        element: &Expr,
        generators: &Rc<Vec<CompGen>>,
        env: &EnvRef,
    ) -> PyResult<()> {
        self.run_generator_clauses(generators, 0, env, element)
    }

    fn run_generator_clauses(
        &mut self,
        generators: &[CompGen],
        index: usize,
        env: &EnvRef,
        element: &Expr,
    ) -> PyResult<()> {
        if index >= generators.len() {
            let value = self.eval_expr(element, env)?;
            self.fiber_yield(value)?;
            return Ok(());
        }
        let generator = &generators[index];
        let iterable = self.eval_expr(&generator.iterable, env)?;
        let iterator = self.make_iterator(iterable)?;
        while let Some(item) = self.next_from_iterator(&iterator)? {
            self.assign_target(&generator.target, item, env)?;
            let mut keep = true;
            for condition in &generator.conditions {
                if !self.eval_expr(condition, env)?.truthy() {
                    keep = false;
                    break;
                }
            }
            if keep {
                self.run_generator_clauses(generators, index + 1, env, element)?;
            }
        }
        Ok(())
    }

    /// Build a generator from a generator function and its bound arguments.
    ///
    /// The parameters bind here, at call time, exactly as they do for a
    /// normal call: `f(1)` with a bad signature still raises `TypeError`
    /// immediately, and the body does not run until the first `next()`.
    fn make_generator(
        &mut self,
        function: &Rc<Function>,
        args: &ArgList,
        kwargs: &[(Name, Value)],
    ) -> PyResult<Value> {
        let env = Env::new(None);
        env.borrow_mut().parent = Some(function.closure.clone());
        self.bind_parameters(&env, function, args, kwargs)?;
        // No fiber yet: the 64 KiB stack is allocated by the first `next()`,
        // so a generator that is created and never iterated costs nothing.
        let generator = Value::Generator(Rc::new(RefCell::new(Generator {
            function: function.clone(),
            env,
            name: function.name.clone(),
            fiber: None,
            running: false,
            finished: false,
            failed: None,
            yielded: None,
            sent: Value::None,
        })));
        Ok(generator)
    }

    /// Build the generator a `(element for ...)` expression evaluates to.
    fn make_generator_expression(
        &mut self,
        element: &Expr,
        generators: &[CompGen],
        env: &EnvRef,
    ) -> PyResult<Value> {
        let function = Rc::new(Function {
            name: Rc::from("<genexpr>"),
            params: Vec::new(),
            varargs: None,
            kwargs: None,
            body: FunctionBody::GeneratorExpr {
                element: Rc::new(element.clone()),
                generators: Rc::new(generators.to_vec()),
            },
            closure: env.clone(),
            is_generator: true,
            filename: self.current_filename(),
            def_line: self.current_line(),
        });
        self.make_generator(&function, &ArgList::new(), &[])
    }

    /// `next()` on a generator: resume the fiber and bring back what it did.
    pub fn generator_resume(
        &mut self,
        generator: &Rc<RefCell<Generator>>,
    ) -> PyResult<Option<Value>> {
        {
            let borrowed = generator.borrow();
            if borrowed.running {
                return Err(self.err(
                    "ValueError",
                    String::from("generator already executing"),
                ));
            }
            if borrowed.finished {
                return match &borrowed.failed {
                    Some(error) => Err(error.clone()),
                    None => Ok(None),
                };
            }
        }
        // The raw pointer stays valid: `keep` holds the generator for the
        // whole resume, and the fiber only runs inside this function.
        let keep = generator.clone();
        let cell: &RefCell<Generator> = &generator;
        let generator_ptr = cell.as_ptr();
        // SAFETY: `keep` owns the generator for the whole resume, so the raw
        // pointer stays valid and `start`/`resume` only touch disjoint fields.
        let fiber_ptr = unsafe {
            match (*generator_ptr).fiber.as_deref_mut() {
                Some(fiber) => fiber as *mut fiber::Fiber,
                None => {
                    let fiber = match fiber::Fiber::new() {
                        Some(fiber) => fiber,
                        None => {
                            return Err(self.err(
                                "MemoryError",
                                format!(
                                    "cannot start a generator: {} are already suspended ({} KiB each)",
                                    fiber::MAX_LIVE,
                                    fiber::STACK_SIZE / 1024
                                ),
                            ))
                        }
                    };
                    let raw = &*fiber as *const fiber::Fiber as *mut fiber::Fiber;
                    (*generator_ptr).fiber = Some(fiber);
                    raw
                }
            }
        };
        let mut start = GeneratorStart {
            interp: self as *mut Interp,
            generator: generator_ptr,
        };
        // SAFETY: as above; `start` is read by the first entry, which copies
        // its two pointers before it can yield.
        unsafe {
            (*generator_ptr).running = true;
            (*generator_ptr).yielded = None;
        }
        self.active_fibers.push(ActiveFiber {
            fiber: fiber_ptr,
            generator: generator_ptr,
            frame_depth: self.call_stack.len(),
        });
        // SAFETY: the fiber is suspended (checked above) and this is the only
        // running context; the resumer's frame outlives the switch.
        unsafe {
            if !(*fiber_ptr).started() {
                (*fiber_ptr).start(
                    Self::generator_fiber_entry,
                    &mut start as *mut GeneratorStart as *mut u8,
                );
            } else {
                (*fiber_ptr).resume();
            }
        }
        self.active_fibers.pop();
        // A finished generator never runs again, so its stack goes back to the
        // pool immediately: without this, a script that creates generators in
        // a loop would run out of fiber slots, because these generators are
        // often still reachable from the namespace that their own closure
        // chain points at (a reference cycle the interpreter has no GC for).
        if unsafe { (*generator_ptr).finished } {
            unsafe {
                (*generator_ptr).fiber = None;
                // Drop the captured frame too: it is one half of that cycle,
                // and a finished body cannot need it again.
                (*generator_ptr).env = Env::new(None);
            }
        }
        let mut borrowed = keep.borrow_mut();
        borrowed.running = false;
        if let Some(error) = borrowed.failed.take() {
            return Err(error);
        }
        if borrowed.finished {
            return Ok(None);
        }
        Ok(borrowed.yielded.take())
    }

    /// Suspend the running generator at a `yield`, handing `value` to its
    /// resumer. Returns the value the resumer sends back (always `None`).
    fn fiber_yield(&mut self, value: Value) -> PyResult<Value> {
        let active = match self.active_fibers.last().copied() {
            Some(active) => active,
            None => {
                return Err(self.err(
                    "SyntaxError",
                    String::from("'yield' outside a function"),
                ))
            }
        };
        // The resumer must not see the generator's frames in a traceback while
        // the generator is suspended; they come back on resume.
        let parked = self.call_stack.split_off(active.frame_depth);
        unsafe {
            (*active.generator).yielded = Some(value);
            (*active.generator).running = false;
            (*active.fiber).yield_to_caller();
            (*active.generator).running = true;
        }
        self.call_stack.extend(parked);
        Ok(core::mem::replace(
            unsafe { &mut (*active.generator).sent },
            Value::None,
        ))
    }

    // ── calls ──────────────────────────────────────────────────────────────

    pub fn call_value(
        &mut self,
        function: Value,
        args: ArgList,
        kwargs: Vec<(Name, Value)>,
    ) -> PyResult<Value> {
        match function {
            Value::Function(function) => self.call_user_function(&function, &args, &kwargs),
            Value::BoundMethod(method) => {
                let call_args = ArgList::with_receiver(method.receiver.clone(), &args);
                self.call_user_function(&method.function, &call_args, &kwargs)
            }
            Value::Builtin(builtin) => self.call_builtin(&builtin, &args, &kwargs),
            Value::Class(class) => self.instantiate(class, &args, &kwargs),
            other => Err(self.err(
                "TypeError",
                format!("'{}' object is not callable", other.type_name()),
            )),
        }
    }

    fn call_builtin(
        &mut self,
        builtin: &Rc<BuiltinValue>,
        args: &ArgList,
        kwargs: &[(Name, Value)],
    ) -> PyResult<Value> {
        let result = builtins::call(self, builtin, args, kwargs);
        match result {
            Err(mut error) => {
                if error.frames.is_empty() {
                    error.frames = self.snapshot();
                }
                Err(error)
            }
            ok => ok,
        }
    }

    fn call_user_function(
        &mut self,
        function: &Rc<Function>,
        args: &ArgList,
        kwargs: &[(Name, Value)],
    ) -> PyResult<Value> {
        if self.depth >= RECURSION_LIMIT {
            return Err(self.err(
                "RecursionError",
                format!(
                    "maximum recursion depth exceeded ({} Python frames)",
                    RECURSION_LIMIT
                ),
            ));
        }
        if function.is_generator {
            // No body runs here; the generator binds its parameters and waits
            // for the first `next()`.
            return self.make_generator(function, args, kwargs);
        }
        self.check_fiber_headroom()?;
        let env = match self.pool.pop() {
            Some(env) => env,
            None => Env::new(None),
        };
        {
            let mut borrowed = env.borrow_mut();
            borrowed.parent = Some(function.closure.clone());
        }
        if let Err(error) = self.bind_parameters(&env, function, args, kwargs) {
            self.release_env(env);
            return Err(error);
        }
        self.depth += 1;
        self.call_stack.push(Frame {
            filename: function.filename.clone(),
            name: function.name.clone(),
            line: function.def_line,
        });
        let outcome = match &function.body {
            FunctionBody::Block(body) => self.exec_block(body, &env),
            FunctionBody::Expression(expression) => self
                .eval_expr(expression, &env)
                .map(Flow::Return),
            // A generator expression's synthetic function is always a
            // generator, so `call_user_function` diverts it before this point.
            FunctionBody::GeneratorExpr { .. } => Err(self.err(
                "RuntimeError",
                String::from("a generator expression body ran as a call"),
            )),
        };
        self.call_stack.pop();
        self.depth -= 1;
        self.release_env(env);
        match outcome {
            Ok(Flow::Return(value)) => Ok(value),
            Ok(Flow::Normal) => Ok(Value::None),
            Ok(_) => Err(self.err(
                "SyntaxError",
                String::from("'break' or 'return' outside a loop/function"),
            )),
            Err(error) => Err(error),
        }
    }

    fn bind_parameters(
        &mut self,
        env: &EnvRef,
        function: &Rc<Function>,
        args: &ArgList,
        kwargs: &[(Name, Value)],
    ) -> PyResult<()> {
        let mut positional = 0usize;
        for (name, default) in &function.params {
            let value = if positional < args.len() {
                if kwargs.iter().any(|(key, _)| key == name) {
                    return Err(self.err(
                        "TypeError",
                        format!(
                            "{}() got multiple values for argument '{}'",
                            function.name, name
                        ),
                    ));
                }
                let value = match args.get(positional) {
                    Some(value) => value.clone(),
                    None => Value::None,
                };
                positional += 1;
                value
            } else if let Some((_, value)) = kwargs.iter().find(|(key, _)| key == name) {
                value.clone()
            } else if let Some(default) = default {
                default.clone()
            } else {
                return Err(self.err(
                    "TypeError",
                    format!(
                        "{}() missing required positional argument: '{}'",
                        function.name, name
                    ),
                ));
            };
            env.borrow_mut().set_local(name.clone(), value);
        }
        if let Some(name) = &function.varargs {
            let mut extras = Vec::new();
            while positional < args.len() {
                if let Some(value) = args.get(positional) {
                    extras.push(value.clone());
                }
                positional += 1;
            }
            env.borrow_mut()
                .set_local(name.clone(), Value::tuple(extras));
        } else if positional < args.len() {
            return Err(self.err(
                "TypeError",
                format!(
                    "{}() takes {} positional arguments but {} were given",
                    function.name,
                    function.params.len(),
                    args.len()
                ),
            ));
        }
        if let Some(name) = &function.kwargs {
            let mut entries = Vec::new();
            for (key, value) in kwargs {
                if function.params.iter().any(|(param, _)| param == key) {
                    continue;
                }
                entries.push((Value::Str(key.clone()), value.clone()));
            }
            env.borrow_mut()
                .set_local(name.clone(), Value::dict(entries));
        } else {
            for (key, _) in kwargs {
                if !function.params.iter().any(|(name, _)| name == key) {
                    return Err(self.err(
                        "TypeError",
                        format!(
                            "{}() got an unexpected keyword argument '{}'",
                            function.name, key
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    fn release_env(&mut self, env: EnvRef) {
        if Rc::strong_count(&env) == 1 {
            env.borrow_mut().clear_for_pool();
            self.pool.push(env);
        }
    }

    fn instantiate(
        &mut self,
        class: Rc<Class>,
        args: &ArgList,
        kwargs: &[(Name, Value)],
    ) -> PyResult<Value> {
        if class.is_exception {
            return self.instantiate_exception(class, args, kwargs);
        }
        if let Some(tag) = class.builtin {
            return self.construct_builtin(tag, class, args, kwargs);
        }
        let instance = Rc::new(Instance {
            class: class.clone(),
            attrs: RefCell::new(Vec::new()),
        });
        match class_lookup(&class, "__init__") {
            Some(Value::Function(function)) => {
                // `call_user_function` takes positional arguments with the
                // receiver already in slot 0; going through `call_value` with
                // a BoundMethod would prepend it a second time.
                let call_args = ArgList::with_receiver(Value::Instance(instance.clone()), args);
                self.call_user_function(&function, &call_args, kwargs)?;
            }
            Some(_) => {
                return Err(self.err(
                    "TypeError",
                    format!("{}() __init__ is not a function", class.name),
                ))
            }
            None => {
                if args.len() != 0 || !kwargs.is_empty() {
                    return Err(
                        self.err("TypeError", format!("{}() takes no arguments", class.name))
                    );
                }
            }
        }
        Ok(Value::Instance(instance))
    }

    fn instantiate_exception(
        &mut self,
        class: Rc<Class>,
        args: &ArgList,
        kwargs: &[(Name, Value)],
    ) -> PyResult<Value> {
        if !kwargs.is_empty() {
            return Err(self.err(
                "TypeError",
                format!("{}() does not accept keyword arguments", class.name),
            ));
        }
        let values = args.to_vec();
        let instance = self.make_exception_instance(class.clone(), values.clone());
        if let Some(Value::Function(function)) = class_lookup(&class, "__init__") {
            let mut call_args = ArgList::new();
            call_args.push(instance.clone());
            for value in values {
                call_args.push(value);
            }
            self.call_user_function(&function, &call_args, &[])?;
        }
        Ok(instance)
    }

    pub(crate) fn construct_builtin(
        &mut self,
        tag: BuiltinType,
        class: Rc<Class>,
        args: &ArgList,
        kwargs: &[(Name, Value)],
    ) -> PyResult<Value> {
        match tag {
            BuiltinType::Object => {
                if args.len() != 0 || !kwargs.is_empty() {
                    return Err(self.err("TypeError", String::from("object() takes no arguments")));
                }
                Ok(Value::Instance(Rc::new(Instance {
                    class,
                    attrs: RefCell::new(Vec::new()),
                })))
            }
            BuiltinType::Bool => {
                if !kwargs.is_empty() || args.len() > 1 {
                    return Err(self.err(
                        "TypeError",
                        String::from("bool() takes at most one argument"),
                    ));
                }
                match args.get(0) {
                    Some(value) => Ok(Value::Bool(value.truthy())),
                    None => Ok(Value::Bool(false)),
                }
            }
            BuiltinType::Int => self.construct_int(args, kwargs),
            BuiltinType::Float => self.construct_float(args, kwargs),
            BuiltinType::Str => {
                if !kwargs.is_empty() || args.len() > 1 {
                    return Err(self.err(
                        "TypeError",
                        String::from("str() takes at most one argument"),
                    ));
                }
                match args.get(0) {
                    // A user class may define `__str__`; consult it exactly
                    // like `print` does instead of falling back to a repr.
                    Some(value) => Ok(Value::Str(Rc::from(
                        crate::builtins::str_value(self, value)?.as_str(),
                    ))),
                    None => Ok(Value::str("")),
                }
            }
            BuiltinType::List => {
                if !kwargs.is_empty() || args.len() > 1 {
                    return Err(self.err(
                        "TypeError",
                        String::from("list() takes at most one argument"),
                    ));
                }
                match args.get(0) {
                    Some(value) => Ok(Value::list(self.collect_iterable(value.clone())?)),
                    None => Ok(Value::list(Vec::new())),
                }
            }
            BuiltinType::Tuple => {
                if !kwargs.is_empty() || args.len() > 1 {
                    return Err(self.err(
                        "TypeError",
                        String::from("tuple() takes at most one argument"),
                    ));
                }
                match args.get(0) {
                    Some(value) => Ok(Value::tuple(self.collect_iterable(value.clone())?)),
                    None => Ok(Value::tuple(Vec::new())),
                }
            }
            BuiltinType::Dict => self.construct_dict(args, kwargs),
            BuiltinType::Set => {
                if !kwargs.is_empty() || args.len() > 1 {
                    return Err(self.err(
                        "TypeError",
                        String::from("set() takes at most one argument"),
                    ));
                }
                match args.get(0) {
                    Some(value) => {
                        let items = self.collect_iterable(value.clone())?;
                        let mut unique = Vec::with_capacity(items.len());
                        for item in items {
                            if !item.is_hashable() {
                                return Err(self.err(
                                    "TypeError",
                                    format!("unhashable type: '{}'", item.type_name()),
                                ));
                            }
                            set_insert(&mut unique, item);
                        }
                        Ok(Value::set(unique))
                    }
                    None => Ok(Value::set(Vec::new())),
                }
            }
            BuiltinType::Bytes => self.construct_bytes(args, kwargs),
            BuiltinType::Type => {
                if !kwargs.is_empty() || args.len() != 1 {
                    return Err(self.err(
                        "TypeError",
                        String::from(
                            "type() takes exactly one argument (three-argument type() is not supported)",
                        ),
                    ));
                }
                match args.get(0) {
                    Some(value) => self.type_of(value),
                    None => Err(self.err(
                        "TypeError",
                        String::from("type() takes exactly one argument"),
                    )),
                }
            }
            _ => Err(self.err(
                "TypeError",
                format!("cannot create '{}' instances", tag.name()),
            )),
        }
    }

    /// `float()` / `float(x)` with no keyword arguments, mirroring CPython's
    /// accepted inputs: int, bool, float and decimal/exponent strings
    /// (including `inf` and `nan`). No locale is involved.
    pub(crate) fn construct_float(
        &mut self,
        args: &ArgList,
        kwargs: &[(Name, Value)],
    ) -> PyResult<Value> {
        if let Some((name, _)) = kwargs.first() {
            return Err(self.err(
                "TypeError",
                format!("float() got an unexpected keyword argument '{}'", name),
            ));
        }
        if args.len() > 1 {
            return Err(self.err(
                "TypeError",
                format!(
                    "float() takes at most 1 argument ({} given)",
                    args.len()
                ),
            ));
        }
        let Some(value) = args.get(0) else {
            return Ok(Value::Float(0.0));
        };
        match value {
            Value::Float(number) => Ok(Value::Float(*number)),
            Value::Int(number) => Ok(Value::Float(*number as f64)),
            Value::Bool(flag) => Ok(Value::Float(if *flag { 1.0 } else { 0.0 })),
            Value::Str(text) => match crate::float::parse(text) {
                Some(number) => Ok(Value::Float(number)),
                None => Err(self.err(
                    "ValueError",
                    format!("could not convert string to float: '{}'", text),
                )),
            },
            other => Err(self.err(
                "TypeError",
                format!(
                    "float() argument must be a string or a real number, not '{}'",
                    other.type_name()
                ),
            )),
        }
    }

    pub(crate) fn construct_int(
        &mut self,
        args: &ArgList,
        kwargs: &[(Name, Value)],
    ) -> PyResult<Value> {
        let mut base: i64 = 10;
        for (name, value) in kwargs {
            if &**name == "base" {
                match value.as_int() {
                    Some(number) => base = number,
                    None => {
                        return Err(
                            self.err("TypeError", String::from("int() base must be an integer"))
                        )
                    }
                }
            } else {
                return Err(self.err(
                    "TypeError",
                    format!("int() got an unexpected keyword argument '{}'", name),
                ));
            }
        }
        if base != 0 && !(2..=36).contains(&base) {
            return Err(self.err(
                "ValueError",
                String::from("int() base must be >= 2 and <= 36, or 0"),
            ));
        }
        if args.len() == 0 {
            if kwargs.is_empty() {
                return Ok(Value::Int(0));
            }
            return Err(self.err("TypeError", String::from("int() missing string argument")));
        }
        if args.len() > 2 {
            return Err(self.err(
                "TypeError",
                String::from("int() takes at most two arguments"),
            ));
        }
        let value = match args.get(0) {
            Some(value) => value,
            None => return Ok(Value::Int(0)),
        };
        match value {
            Value::Int(number) => Ok(Value::Int(*number)),
            Value::Bool(flag) => Ok(Value::Int(if *flag { 1 } else { 0 })),
            Value::Float(number) => {
                if number.is_nan() {
                    return Err(self.err(
                        "ValueError",
                        String::from("cannot convert float NaN to integer"),
                    ));
                }
                match crate::float::to_i64(*number) {
                    Some(value) => Ok(Value::Int(value)),
                    None if number.is_infinite() => Err(self.err(
                        "OverflowError",
                        String::from("cannot convert float infinity to integer"),
                    )),
                    None => Err(self.err(
                        "OverflowError",
                        String::from("int too large to convert to a 64-bit integer"),
                    )),
                }
            }
            Value::Str(text) => {
                let explicit_base = if args.len() == 2 {
                    match args.get(1).and_then(|value| value.as_int()) {
                        Some(number) => Some(number),
                        None => {
                            return Err(self
                                .err("TypeError", String::from("int() base must be an integer")))
                        }
                    }
                } else {
                    None
                };
                let use_base = explicit_base.unwrap_or(base);
                match parse_int_with_base(text, use_base) {
                    Some(number) => Ok(Value::Int(number)),
                    None => Err(self.err(
                        "ValueError",
                        format!(
                            "invalid literal for int() with base {}: '{}'",
                            use_base, text
                        ),
                    )),
                }
            }
            other => Err(self.err(
                "TypeError",
                format!(
                    "int() argument must be a string, a number, or a bool, not '{}'",
                    other.type_name()
                ),
            )),
        }
    }

    pub(crate) fn construct_dict(
        &mut self,
        args: &ArgList,
        kwargs: &[(Name, Value)],
    ) -> PyResult<Value> {
        if args.len() > 1 {
            return Err(self.err(
                "TypeError",
                String::from("dict() takes at most one positional argument"),
            ));
        }
        let mut entries: Vec<(Value, Value)> = Vec::new();
        if let Some(value) = args.get(0) {
            match value {
                Value::Dict(source) => {
                    for (key, item) in source.borrow().iter() {
                        dict_set(&mut entries, key.clone(), item.clone());
                    }
                }
                _ => {
                    let pairs = self.collect_iterable(value.clone())?;
                    for pair in pairs {
                        let parts = self.collect_iterable(pair)?;
                        if parts.len() != 2 {
                            return Err(self.err(
                                "ValueError",
                                String::from(
                                    "dictionary update sequence element does not have length 2",
                                ),
                            ));
                        }
                        let key = parts[0].clone();
                        if !key.is_hashable() {
                            return Err(self.err(
                                "TypeError",
                                format!("unhashable type: '{}'", key.type_name()),
                            ));
                        }
                        dict_set(&mut entries, key, parts[1].clone());
                    }
                }
            }
        }
        for (name, value) in kwargs {
            dict_set(&mut entries, Value::Str(name.clone()), value.clone());
        }
        Ok(Value::dict(entries))
    }

    /// `bytes()`, `bytes(iterable_of_int)`, `bytes(bytes)`, `bytes(int)`
    /// (zero bytes) and `bytes(str, "utf-8")`.
    pub(crate) fn construct_bytes(
        &mut self,
        args: &ArgList,
        kwargs: &[(Name, Value)],
    ) -> PyResult<Value> {
        if let Some((name, _)) = kwargs.first() {
            return Err(self.err(
                "TypeError",
                format!("bytes() got an unexpected keyword argument '{}'", name),
            ));
        }
        if args.len() > 2 {
            return Err(self.err(
                "TypeError",
                format!("bytes() takes at most 2 arguments ({} given)", args.len()),
            ));
        }
        let Some(value) = args.get(0) else {
            return Ok(Value::bytes(Vec::new()));
        };
        match value {
            Value::Bytes(data) => Ok(Value::Bytes(data.clone())),
            Value::Str(text) => {
                let encoding = match args.get(1) {
                    Some(Value::Str(encoding)) => String::from(&**encoding),
                    Some(other) => {
                        return Err(self.err(
                            "TypeError",
                            format!(
                                "bytes() encoding must be str, not '{}'",
                                other.type_name()
                            ),
                        ))
                    }
                    None => {
                        return Err(self.err(
                            "TypeError",
                            String::from("string argument without an encoding"),
                        ))
                    }
                };
                if !is_utf8_name(&encoding) {
                    return Err(self.err(
                        "LookupError",
                        format!("unknown encoding: {}", encoding),
                    ));
                }
                Ok(Value::bytes(text.as_bytes().to_vec()))
            }
            other => {
                if let Some(count) = other.as_int() {
                    if count < 0 {
                        return Err(self.err(
                            "ValueError",
                            String::from("negative count for bytes()"),
                        ));
                    }
                    if count > 1_000_000 {
                        return Err(self.err(
                            "MemoryError",
                            String::from("bytes() result is too large for the 1 MiB heap"),
                        ));
                    }
                    return Ok(Value::bytes(alloc::vec![0u8; count as usize]));
                }
                let items = self.collect_iterable(other.clone())?;
                let mut data = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    match item.as_int() {
                        Some(number) if (0..=255).contains(&number) => data.push(number as u8),
                        Some(_) => {
                            return Err(self.err(
                                "ValueError",
                                format!(
                                    "bytes must be in range(0, 256) (element {} is {})",
                                    index,
                                    item.to_repr()
                                ),
                            ))
                        }
                        None => {
                            return Err(self.err(
                                "TypeError",
                                format!(
                                    "bytes() argument must be an integer or iterable of                                      integers, not '{}'",
                                    item.type_name()
                                ),
                            ))
                        }
                    }
                }
                Ok(Value::bytes(data))
            }
        }
    }

    pub fn type_of(&self, value: &Value) -> PyResult<Value> {
        if let Value::Instance(instance) = value {
            return Ok(Value::Class(instance.class.clone()));
        }
        if let Value::Class(_) = value {
            return match self.class_for(BuiltinType::Type) {
                Some(class) => Ok(Value::Class(class)),
                None => Err(self.err("RuntimeError", String::from("missing type class"))),
            };
        }
        let tag = value.type_tag();
        match self.class_for(tag) {
            Some(class) => Ok(Value::Class(class)),
            None => Err(self.err(
                "RuntimeError",
                format!("interpreter is missing the '{}' class", tag.name()),
            )),
        }
    }

    // ── operators ──────────────────────────────────────────────────────────

    pub fn binary_op(&mut self, op: BinOp, left: Value, right: Value) -> PyResult<Value> {
        match op {
            BinOp::Add => self.add_values(left, right),
            BinOp::Sub => self.numeric_op(op, left, right),
            BinOp::Mul => self.multiply_values(left, right),
            BinOp::Div => self.divide_values(left, right),
            BinOp::FloorDiv => self.floor_divide_values(left, right),
            BinOp::Mod => self.modulo_values(left, right),
            BinOp::Pow => self.power_values(left, right),
        }
    }

    fn add_values(&self, left: Value, right: Value) -> PyResult<Value> {
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            return match a.checked_add(b) {
                Some(result) => Ok(Value::Int(result)),
                None => Err(self.err("OverflowError", String::from("integer addition overflow"))),
            };
        }
        if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
            return Ok(Value::Float(a + b));
        }
        match (&left, &right) {
            (Value::Str(a), Value::Str(b)) => {
                let mut joined = String::with_capacity(a.len() + b.len());
                joined.push_str(a);
                joined.push_str(b);
                Ok(Value::Str(Rc::from(joined)))
            }
            (Value::List(a), Value::List(b)) => {
                let mut items = a.borrow().clone();
                items.extend(b.borrow().iter().cloned());
                Ok(Value::list(items))
            }
            (Value::Tuple(a), Value::Tuple(b)) => {
                let mut items = Vec::with_capacity(a.len() + b.len());
                items.extend(a.iter().cloned());
                items.extend(b.iter().cloned());
                Ok(Value::tuple(items))
            }
            (Value::Bytes(a), Value::Bytes(b)) => {
                if a.len().saturating_add(b.len()) > 1_000_000 {
                    return Err(self.err(
                        "MemoryError",
                        String::from("bytes concatenation is too large for the 1 MiB heap"),
                    ));
                }
                let mut data = Vec::with_capacity(a.len() + b.len());
                data.extend(a.iter().copied());
                data.extend(b.iter().copied());
                Ok(Value::bytes(data))
            }
            _ => Err(self.err(
                "TypeError",
                format!(
                    "unsupported operand type(s) for +: '{}' and '{}'",
                    left.type_name(),
                    right.type_name()
                ),
            )),
        }
    }

    fn numeric_op(&self, op: BinOp, left: Value, right: Value) -> PyResult<Value> {
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            let result = match op {
                BinOp::Sub => a.checked_sub(b),
                _ => None,
            };
            return match result {
                Some(value) => Ok(Value::Int(value)),
                None => {
                    Err(self.err("OverflowError", format!("integer {} overflow", op_text(op))))
                }
            };
        }
        if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
            return match op {
                BinOp::Sub => Ok(Value::Float(a - b)),
                _ => Err(self.err(
                    "TypeError",
                    format!(
                        "unsupported operand type(s) for {}: '{}' and '{}'",
                        op_text(op),
                        left.type_name(),
                        right.type_name()
                    ),
                )),
            };
        }
        Err(self.err(
            "TypeError",
            format!(
                "unsupported operand type(s) for {}: '{}' and '{}'",
                op_text(op),
                left.type_name(),
                right.type_name()
            ),
        ))
    }

    fn multiply_values(&self, left: Value, right: Value) -> PyResult<Value> {
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            return match a.checked_mul(b) {
                Some(result) => Ok(Value::Int(result)),
                None => Err(self.err(
                    "OverflowError",
                    String::from("integer multiplication overflow"),
                )),
            };
        }
        if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
            return Ok(Value::Float(a * b));
        }
        let (text, count) = match (&left, &right) {
            (Value::Str(text), _) => match right.as_int() {
                Some(count) => (Some(text.clone()), count),
                None => (None, 0),
            },
            (_, Value::Str(text)) => match left.as_int() {
                Some(count) => (Some(text.clone()), count),
                None => (None, 0),
            },
            _ => (None, 0),
        };
        if let Some(text) = text {
            let count = if count < 0 { 0 } else { count as usize };
            if text.len().saturating_mul(count) > 1_000_000 {
                return Err(self.err(
                    "MemoryError",
                    String::from("string repetition is too large for the 1 MiB heap"),
                ));
            }
            return Ok(Value::Str(Rc::from(text.repeat(count))));
        }
        match (&left, &right) {
            (Value::Bytes(data), _) => {
                if let Some(count) = right.as_int() {
                    return self.repeat_bytes(data, count);
                }
            }
            (_, Value::Bytes(data)) => {
                if let Some(count) = left.as_int() {
                    return self.repeat_bytes(data, count);
                }
            }
            (Value::List(items), _) => {
                if let Some(count) = right.as_int() {
                    return self.repeat_list(items, count);
                }
            }
            (_, Value::List(items)) => {
                if let Some(count) = left.as_int() {
                    return self.repeat_list(items, count);
                }
            }
            (Value::Tuple(items), _) => {
                if let Some(count) = right.as_int() {
                    return self.repeat_tuple(items, count);
                }
            }
            (_, Value::Tuple(items)) => {
                if let Some(count) = left.as_int() {
                    return self.repeat_tuple(items, count);
                }
            }
            _ => {}
        }
        Err(self.err(
            "TypeError",
            format!(
                "unsupported operand type(s) for *: '{}' and '{}'",
                left.type_name(),
                right.type_name()
            ),
        ))
    }

    fn repeat_list(&self, items: &Rc<RefCell<Vec<Value>>>, count: i64) -> PyResult<Value> {
        let count = if count < 0 { 0 } else { count as usize };
        let source = items.borrow();
        if source.len().saturating_mul(count) > 100_000 {
            return Err(self.err(
                "MemoryError",
                String::from("list repetition is too large for the 1 MiB heap"),
            ));
        }
        let mut result = Vec::with_capacity(source.len() * count);
        for _ in 0..count {
            result.extend(source.iter().cloned());
        }
        Ok(Value::list(result))
    }

    fn repeat_tuple(&self, items: &Rc<Vec<Value>>, count: i64) -> PyResult<Value> {
        let count = if count < 0 { 0 } else { count as usize };
        if items.len().saturating_mul(count) > 100_000 {
            return Err(self.err(
                "MemoryError",
                String::from("tuple repetition is too large for the 1 MiB heap"),
            ));
        }
        let mut result = Vec::with_capacity(items.len() * count);
        for _ in 0..count {
            result.extend(items.iter().cloned());
        }
        Ok(Value::tuple(result))
    }

    fn repeat_bytes(&self, data: &Rc<Vec<u8>>, count: i64) -> PyResult<Value> {
        let count = if count < 0 { 0 } else { count as usize };
        if data.len().saturating_mul(count) > 1_000_000 {
            return Err(self.err(
                "MemoryError",
                String::from("bytes repetition is too large for the 1 MiB heap"),
            ));
        }
        let mut out = Vec::with_capacity(data.len() * count);
        for _ in 0..count {
            out.extend(data.iter().copied());
        }
        Ok(Value::bytes(out))
    }

    /// `/` always produces a float, including for two integers (`1 / 2` is
    /// `0.5`), exactly like Python 3.
    fn divide_values(&self, left: Value, right: Value) -> PyResult<Value> {
        let (Some(a), Some(b)) = (left.as_float(), right.as_float()) else {
            return Err(self.err(
                "TypeError",
                format!(
                    "unsupported operand type(s) for /: '{}' and '{}'",
                    left.type_name(),
                    right.type_name()
                ),
            ));
        };
        if b == 0.0 {
            return Err(self.err("ZeroDivisionError", String::from("division by zero")));
        }
        Ok(Value::Float(a / b))
    }

    fn floor_divide_values(&self, left: Value, right: Value) -> PyResult<Value> {
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            return match floor_div_wide(a, b) {
                Some(result) => Ok(Value::Int(result)),
                None => Err(self.err(
                    "ZeroDivisionError",
                    String::from("integer division or modulo by zero"),
                )),
            };
        }
        let (Some(a), Some(b)) = (left.as_float(), right.as_float()) else {
            return Err(self.err(
                "TypeError",
                format!(
                    "unsupported operand type(s) for //: '{}' and '{}'",
                    left.type_name(),
                    right.type_name()
                ),
            ));
        };
        if b == 0.0 {
            return Err(self.err(
                "ZeroDivisionError",
                String::from("float floor division by zero"),
            ));
        }
        Ok(Value::Float(crate::float::divmod(a, b).0))
    }

    fn modulo_values(&mut self, left: Value, right: Value) -> PyResult<Value> {
        if let Value::Str(format) = &left {
            // `str % args` is printf-style formatting, not arithmetic.
            return self.percent_format(format, right);
        }
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            if b == 0 {
                return Err(self.err(
                    "ZeroDivisionError",
                    String::from("integer division or modulo by zero"),
                ));
            }
            let quotient = floor_div_i128(a, b);
            let remainder = a as i128 - quotient * (b as i128);
            return match i64::try_from(remainder) {
                Ok(result) => Ok(Value::Int(result)),
                Err(_) => Err(self.err("OverflowError", String::from("integer modulo overflow"))),
            };
        }
        let (Some(a), Some(b)) = (left.as_float(), right.as_float()) else {
            return Err(self.err(
                "TypeError",
                format!(
                    "unsupported operand type(s) for %: '{}' and '{}'",
                    left.type_name(),
                    right.type_name()
                ),
            ));
        };
        if b == 0.0 {
            return Err(self.err(
                "ZeroDivisionError",
                String::from("float modulo by zero"),
            ));
        }
        Ok(Value::Float(crate::float::divmod(a, b).1))
    }

    /// `fmt % args`: printf-style string formatting.
    ///
    /// The conversion is a thin translation of the `%` syntax onto the
    /// f-string formatter ([`builtins::format_value`]), so `%5.2f`, `%+d`,
    /// `%#x` and friends pad, sign and round exactly like the equivalent
    /// replacement field. Supported conversions: `s r a d i u f F e E g G x X
    /// o b c %`. The `%(name)s` mapping form is not implemented and reports
    /// the unsupported character.
    fn percent_format(&mut self, format: &Rc<str>, right: Value) -> PyResult<Value> {
        let values: Vec<Value> = match right {
            // A tuple supplies one argument per conversion; every other value
            // (including a list) is a single argument.
            Value::Tuple(items) => items.as_ref().clone(),
            other => alloc::vec![other],
        };
        let chars: Vec<char> = format.chars().collect();
        let mut out = String::new();
        let mut index = 0usize;
        let mut argument = 0usize;
        while index < chars.len() {
            if chars[index] != '%' {
                out.push(chars[index]);
                index += 1;
                continue;
            }
            index += 1;
            if index >= chars.len() {
                return Err(self.err("ValueError", String::from("incomplete format")));
            }
            if chars[index] == '%' {
                out.push('%');
                index += 1;
                continue;
            }
            let mut left_align = false;
            let mut zero_pad = false;
            let mut plus = false;
            let mut space = false;
            let mut alternate = false;
            loop {
                match chars.get(index) {
                    Some('-') => left_align = true,
                    Some('0') => zero_pad = true,
                    Some('+') => plus = true,
                    Some(' ') => space = true,
                    Some('#') => alternate = true,
                    _ => break,
                }
                index += 1;
            }
            let mut width = String::new();
            while let Some(ch) = chars.get(index) {
                if ch.is_ascii_digit() {
                    width.push(*ch);
                    index += 1;
                } else {
                    break;
                }
            }
            let mut precision: Option<String> = None;
            if chars.get(index) == Some(&'.') {
                index += 1;
                let mut digits = String::new();
                while let Some(ch) = chars.get(index) {
                    if ch.is_ascii_digit() {
                        digits.push(*ch);
                        index += 1;
                    } else {
                        break;
                    }
                }
                precision = Some(if digits.is_empty() {
                    String::from("0")
                } else {
                    digits
                });
            }
            let kind = match chars.get(index) {
                Some(kind) => *kind,
                None => return Err(self.err("ValueError", String::from("incomplete format"))),
            };
            let position = index;
            index += 1;
            if argument >= values.len() {
                return Err(self.err(
                    "TypeError",
                    String::from("not enough arguments for format string"),
                ));
            }
            let value = values[argument].clone();
            argument += 1;
            if kind == 'c' {
                let text = self.percent_character(value)?;
                out.push_str(&self.percent_pad(text, &width, left_align));
                continue;
            }
            let mut spec = String::new();
            if left_align {
                spec.push('<');
            }
            if plus {
                spec.push('+');
            } else if space {
                spec.push(' ');
            }
            if alternate {
                spec.push('#');
            }
            if zero_pad && !left_align {
                spec.push('0');
            }
            spec.push_str(&width);
            // `%r`/`%a` reuse the formatter's `!r` conversion; everything else
            // is a plain spec. `%a` is `repr` here: there is no `ascii()`.
            let conversion = match kind {
                's' | 'r' | 'a' => {
                    if let Some(precision) = &precision {
                        spec.push('.');
                        spec.push_str(precision);
                    }
                    spec.push('s');
                    if kind == 's' {
                        None
                    } else {
                        Some('r')
                    }
                }
                'd' | 'i' | 'u' => {
                    spec.push('d');
                    None
                }
                'f' | 'F' => {
                    spec.push('.');
                    spec.push_str(precision.as_deref().unwrap_or("6"));
                    spec.push('f');
                    None
                }
                'e' | 'E' => {
                    spec.push('.');
                    spec.push_str(precision.as_deref().unwrap_or("6"));
                    spec.push('e');
                    None
                }
                'g' | 'G' => {
                    spec.push('.');
                    spec.push_str(precision.as_deref().unwrap_or("6"));
                    spec.push('g');
                    None
                }
                'x' | 'X' | 'o' | 'b' => {
                    spec.push(kind);
                    None
                }
                other => {
                    return Err(self.err(
                        "ValueError",
                        format!(
                            "unsupported format character '{}' (0x{:x}) at index {}",
                            other, other as u32, position
                        ),
                    ))
                }
            };
            let text = builtins::format_value(self, &value, Some(&spec), conversion)?;
            if matches!(kind, 'E' | 'G') {
                out.push_str(&text.to_uppercase());
            } else {
                out.push_str(&text);
            }
        }
        if argument < values.len() {
            return Err(self.err(
                "TypeError",
                String::from("not all arguments converted during string formatting"),
            ));
        }
        Ok(Value::Str(Rc::from(out.as_str())))
    }

    /// The `%c` argument: a code point or a one-character string.
    fn percent_character(&self, value: Value) -> PyResult<String> {
        if let Some(number) = value.as_int() {
            let code = match u32::try_from(number) {
                Ok(code) => code,
                Err(_) => {
                    return Err(self.err(
                        "OverflowError",
                        String::from("%c arg not in range(0x110000)"),
                    ))
                }
            };
            return match char::from_u32(code) {
                Some(ch) => Ok(String::from(ch)),
                None => Err(self.err(
                    "OverflowError",
                    String::from("%c arg not in range(0x110000)"),
                )),
            };
        }
        match &value {
            Value::Str(text) if text.chars().count() == 1 => Ok(String::from(&**text)),
            _ => Err(self.err("TypeError", String::from("%c requires int or char"))),
        }
    }

    /// `%5c`-style padding for the one conversion the formatter cannot take.
    fn percent_pad(&self, text: String, width: &str, left_align: bool) -> String {
        let width: usize = width.parse().unwrap_or(0);
        let len = text.chars().count();
        if len >= width {
            return text;
        }
        let padding = " ".repeat(width - len);
        if left_align {
            let mut out = text;
            out.push_str(&padding);
            out
        } else {
            let mut padding = padding;
            padding.push_str(&text);
            padding
        }
    }

    fn power_values(&self, left: Value, right: Value) -> PyResult<Value> {
        let integers = match (left.as_int(), right.as_int()) {
            (Some(base), Some(exponent)) => Some((base, exponent)),
            _ => None,
        };
        if let Some((base, exponent)) = integers {
            if exponent >= 0 {
                return self.integer_power(base, exponent);
            }
            return self.float_power(base as f64, exponent as f64);
        }
        let (Some(base), Some(exponent)) = (left.as_float(), right.as_float()) else {
            return Err(self.err(
                "TypeError",
                format!(
                    "unsupported operand type(s) for **: '{}' and '{}'",
                    left.type_name(),
                    right.type_name()
                ),
            ));
        };
        self.float_power(base, exponent)
    }

    fn integer_power(&self, base: i64, exponent: i64) -> PyResult<Value> {
        if exponent > u32::MAX as i64 {
            if base == 0 {
                return Ok(Value::Int(0));
            }
            if base == 1 {
                return Ok(Value::Int(1));
            }
            if base == -1 {
                return Ok(Value::Int(if exponent % 2 == 0 { 1 } else { -1 }));
            }
            return Err(self.err(
                "OverflowError",
                String::from("integer exponentiation overflow"),
            ));
        }
        match base.checked_pow(exponent as u32) {
            Some(result) => Ok(Value::Int(result)),
            None => Err(self.err(
                "OverflowError",
                String::from("integer exponentiation overflow"),
            )),
        }
    }

    fn float_power(&self, base: f64, exponent: f64) -> PyResult<Value> {
        if exponent == 0.0 {
            return Ok(Value::Float(1.0));
        }
        if base == 0.0 && exponent < 0.0 {
            return Err(self.err(
                "ZeroDivisionError",
                String::from("zero to a negative power"),
            ));
        }
        let result = crate::float::pow(base, exponent);
        if result.is_nan() && base < 0.0 && !exponent.is_nan() {
            return Err(self.err(
                "ValueError",
                String::from(
                    "a negative base with a non-integer exponent produces a complex \
                     number, which is not supported",
                ),
            ));
        }
        Ok(Value::Float(result))
    }

    fn compare_op(&self, op: CmpOp, left: &Value, right: &Value) -> PyResult<bool> {
        match op {
            CmpOp::Eq => Ok(values_equal(left, right)),
            CmpOp::NotEq => Ok(!values_equal(left, right)),
            CmpOp::Is => Ok(is_same(left, right)),
            CmpOp::IsNot => Ok(!is_same(left, right)),
            CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge => {
                if let (Value::Set(a), Value::Set(b)) = (left, right) {
                    let a = a.borrow();
                    let b = b.borrow();
                    let subset = set_is_subset(&a, &b);
                    return Ok(match op {
                        CmpOp::Lt => subset && a.len() < b.len(),
                        CmpOp::Le => subset,
                        CmpOp::Gt => set_is_subset(&b, &a) && b.len() < a.len(),
                        _ => set_is_subset(&b, &a),
                    });
                }
                let ordering = match self.numeric_ordering(left, right) {
                    Some(ordering) => ordering,
                    None => Some(self.cmp_values(left, right)?),
                };
                // A NaN pair has no ordering: Python makes all four
                // relational comparisons false.
                let Some(ordering) = ordering else {
                    return Ok(false);
                };
                Ok(match op {
                    CmpOp::Lt => ordering == Ordering::Less,
                    CmpOp::Le => ordering != Ordering::Greater,
                    CmpOp::Gt => ordering == Ordering::Greater,
                    _ => ordering != Ordering::Less,
                })
            }
            CmpOp::In => self.contains_value(right, left),
            CmpOp::NotIn => Ok(!self.contains_value(right, left)?),
        }
    }

    /// Exact numeric ordering for an int/bool/float pair.
    ///
    /// The outer `Option` is `None` when the pair is not numeric (strings,
    /// lists, ...); the inner one is `None` for NaN, where Python's relational
    /// operators are all false. Int-vs-float comparisons go through
    /// `float::cmp_int_float`, so an i64 above 2^53 is not rounded into a
    /// different ordering.
    fn numeric_ordering(&self, left: &Value, right: &Value) -> Option<Option<Ordering>> {
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            return Some(Some(a.cmp(&b)));
        }
        let (a, b) = match (left.as_float(), right.as_float()) {
            (Some(a), Some(b)) => (a, b),
            _ => return None,
        };
        if a.is_nan() || b.is_nan() {
            return Some(None);
        }
        let ordering = match (left, right) {
            (Value::Int(integer), Value::Float(float)) => {
                crate::float::cmp_int_float(*integer, *float)
            }
            (Value::Float(float), Value::Int(integer)) => {
                crate::float::cmp_float_int(*float, *integer)
            }
            (Value::Bool(flag), Value::Float(float)) => {
                crate::float::cmp_int_float(if *flag { 1 } else { 0 }, *float)
            }
            (Value::Float(float), Value::Bool(flag)) => {
                crate::float::cmp_float_int(*float, if *flag { 1 } else { 0 })
            }
            _ => a.partial_cmp(&b)?,
        };
        Some(Some(ordering))
    }

    /// Ordering used by comparisons, `min`, `max`, `sorted` and `list.sort`.
    /// Only int/bool/float/str and list/tuple of those are orderable;
    /// everything else is a Python `TypeError`, not an internal panic. A NaN
    /// sorts as equal to its neighbour (Python's `min`/`sorted` ordering with
    /// NaN is unspecified; relational operators use `numeric_ordering` and
    /// return false instead).
    pub fn cmp_values(&self, left: &Value, right: &Value) -> PyResult<Ordering> {
        if let Some(ordering) = self.numeric_ordering(left, right) {
            return Ok(ordering.unwrap_or(Ordering::Equal));
        }
        match (left, right) {
            (Value::Str(a), Value::Str(b)) => Ok(a.cmp(b)),
            (Value::Bytes(a), Value::Bytes(b)) => Ok(a.as_slice().cmp(b.as_slice())),
            (Value::List(a), Value::List(b)) => {
                let a = a.borrow();
                let b = b.borrow();
                compare_sequences(self, &a, &b)
            }
            (Value::Tuple(a), Value::Tuple(b)) => compare_sequences(self, a, b),
            _ => Err(self.err(
                "TypeError",
                format!(
                    "'<' not supported between instances of '{}' and '{}'",
                    left.type_name(),
                    right.type_name()
                ),
            )),
        }
    }

    fn contains_value(&self, container: &Value, item: &Value) -> PyResult<bool> {
        match container {
            Value::List(items) => Ok(items
                .borrow()
                .iter()
                .any(|candidate| values_equal(candidate, item))),
            Value::Tuple(items) => Ok(items.iter().any(|candidate| values_equal(candidate, item))),
            Value::Dict(entries) => {
                if !item.is_hashable() {
                    return Err(self.err(
                        "TypeError",
                        format!("unhashable type: '{}'", item.type_name()),
                    ));
                }
                Ok(dict_get(&entries.borrow(), item).is_some())
            }
            Value::Set(items) => {
                if !item.is_hashable() {
                    return Err(self.err(
                        "TypeError",
                        format!("unhashable type: '{}'", item.type_name()),
                    ));
                }
                Ok(set_find(&items.borrow(), item).is_some())
            }
            Value::Bytes(data) => match item {
                Value::Int(number) => {
                    if !(0..=255).contains(number) {
                        return Err(self.err(
                            "ValueError",
                            String::from("byte must be in range(0, 256)"),
                        ));
                    }
                    Ok(data.iter().any(|byte| *byte as i64 == *number))
                }
                Value::Bytes(needle) => Ok(find_bytes(data, needle)),
                other => Err(self.err(
                    "TypeError",
                    format!(
                        "a bytes-like object is required, not '{}'",
                        other.type_name()
                    ),
                )),
            },
            Value::Str(text) => match item {
                Value::Str(needle) => Ok(text.contains(&**needle)),
                _ => Err(self.err(
                    "TypeError",
                    String::from("'in <string>' requires string as left operand"),
                )),
            },
            Value::Range(range) => match item.as_int() {
                Some(number) => Ok(range_contains(range, number)),
                None => Ok(false),
            },
            _ => Err(self.err(
                "TypeError",
                format!(
                    "argument of type '{}' is not iterable",
                    container.type_name()
                ),
            )),
        }
    }

    // ── indexing, slicing, attributes ─────────────────────────────────────

    pub(crate) fn index_value(&mut self, target: &Value, index: &Value) -> PyResult<Value> {
        match target {
            Value::List(items) => {
                let number = self.require_int(index, "list indices must be integers")?;
                let items = items.borrow();
                let position = normalize_index(number, items.len());
                match position.and_then(|position| items.get(position)) {
                    Some(value) => Ok(value.clone()),
                    None => Err(self.err("IndexError", String::from("list index out of range"))),
                }
            }
            Value::Tuple(items) => {
                let number = self.require_int(index, "tuple indices must be integers")?;
                let position = normalize_index(number, items.len());
                match position.and_then(|position| items.get(position)) {
                    Some(value) => Ok(value.clone()),
                    None => Err(self.err("IndexError", String::from("tuple index out of range"))),
                }
            }
            Value::Str(text) => {
                let number = self.require_int(index, "string indices must be integers")?;
                let chars: Vec<char> = text.chars().collect();
                let position = normalize_index(number, chars.len());
                match position.and_then(|position| chars.get(position)) {
                    Some(ch) => Ok(Value::Str(Rc::from(ch.to_string()))),
                    None => Err(self.err("IndexError", String::from("string index out of range"))),
                }
            }
            Value::Bytes(data) => {
                let number = self.require_int(index, "bytes indices must be integers")?;
                let position = normalize_index(number, data.len());
                match position.and_then(|position| data.get(position)) {
                    Some(byte) => Ok(Value::Int(*byte as i64)),
                    None => Err(self.err(
                        "IndexError",
                        String::from("bytes index out of range"),
                    )),
                }
            }
            Value::Dict(entries) => {
                if !index.is_hashable() {
                    return Err(self.err(
                        "TypeError",
                        format!("unhashable type: '{}'", index.type_name()),
                    ));
                }
                match dict_get(&entries.borrow(), index) {
                    Some(value) => Ok(value),
                    None => Err(self.err("KeyError", index.to_repr())),
                }
            }
            Value::Instance(instance) => {
                if let Some(method) = class_lookup(&instance.class, "__getitem__") {
                    let bound = bind_method(method, target.clone())?;
                    let mut args = ArgList::new();
                    args.push(index.clone());
                    self.call_value(bound, args, Vec::new())
                } else {
                    Err(self.err(
                        "TypeError",
                        format!("'{}' object is not subscriptable", target.type_name()),
                    ))
                }
            }
            _ => Err(self.err(
                "TypeError",
                format!("'{}' object is not subscriptable", target.type_name()),
            )),
        }
    }

    fn delete_target(&mut self, target: &Expr, env: &EnvRef) -> PyResult<()> {
        match target {
            Expr::Name(name) => {
                let mut frame = env.clone();
                loop {
                    let found = {
                        let mut borrowed = frame.borrow_mut();
                        match borrowed.vars.iter().position(|(key, _)| key == name) {
                            Some(index) => { borrowed.vars.remove(index); true }
                            None => false,
                        }
                    };
                    if found { return Ok(()); }
                    let parent = frame.borrow().parent.clone();
                    match parent {
                        Some(parent) => frame = parent,
                        None => break,
                    }
                }
                Err(self.err("NameError", format!("name '{}' is not defined", name)))
            }
            Expr::Index { target, index } => {
                let object = self.eval_expr(target, env)?;
                let index_value = self.eval_expr(index, env)?;
                match &object {
                    Value::List(items) => {
                        let number = self.require_int(&index_value, "list indices must be integers")?;
                        let mut items = items.borrow_mut();
                        let position = normalize_index(number, items.len()).ok_or_else(|| {
                            self.err("IndexError", String::from("list assignment index out of range"))
                        })?;
                        items.remove(position);
                        Ok(())
                    }
                    Value::Dict(entries) => {
                        if !index_value.is_hashable() {
                            return Err(self.err("TypeError", format!("unhashable type: '{}'", index_value.type_name())));
                        }
                        {
                            let mut entries = entries.borrow_mut();
                            if let Some(index) = crate::value::dict_find(&entries, &index_value) {
                                entries.remove(index);
                            }
                        }
                        Ok(())
                    }
                    _ => Err(self.err("TypeError", format!("'{}' object doesn't support item deletion", object.type_name()))),
                }
            }
            Expr::Attribute { target, name } => {
                let object = self.eval_expr(target, env)?;
                match &object {
                    Value::Instance(instance) => {
                        let mut attributes = instance.attrs.borrow_mut();
                        if let Some(index) = attributes.iter().position(|(key, _)| key == name) {
                            attributes.remove(index);
                        }
                        Ok(())
                    }
                    _ => Err(self.err("AttributeError", format!("'{}' object has no attribute '{}'", object.type_name(), name))),
                }
            }
            Expr::Slice { target, start, stop, step } => {
                let object = self.eval_expr(target, env)?;
                let start = match start { Some(e) => Some(self.eval_slice_bound(e, env)?), None => None };
                let stop = match stop { Some(e) => Some(self.eval_slice_bound(e, env)?), None => None };
                let step = match step { Some(e) => Some(self.eval_slice_bound(e, env)?), None => None };
                match &object {
                    Value::List(items) => {
                        let (start, stop, step) = self.slice_bounds(items.borrow().len() as i64, start, stop, step)?;
                        if step != 1 {
                            return Err(self.err("ValueError", String::from("extended slice deletion is not supported")));
                        }
                        items.borrow_mut().drain(start as usize..stop as usize);
                        Ok(())
                    }
                    _ => Err(self.err("TypeError", format!("'{}' object doesn't support slice deletion", object.type_name()))),
                }
            }
            _ => Err(self.err("SyntaxError", String::from("cannot delete this expression"))),
        }
    }

    fn set_slice(&mut self, target: Value, start: Option<i64>, stop: Option<i64>, step: Option<i64>, value: Value) -> PyResult<()> {
        match &target {
            Value::List(items) => {
                let (start, stop, step) = self.slice_bounds(items.borrow().len() as i64, start, stop, step)?;
                if step != 1 {
                    return Err(self.err("ValueError", String::from("extended slice assignment is not supported")));
                }
                let values = self.collect_iterable(value)?;
                let mut items = items.borrow_mut();
                items.splice(start as usize..stop as usize, values);
                Ok(())
            }
            _ => Err(self.err("TypeError", format!("'{}' object doesn't support slice assignment", target.type_name()))),
        }
    }

    fn set_index(&mut self, target: Value, index: Value, value: Value) -> PyResult<()> {
        match &target {
            Value::List(items) => {
                let number = self.require_int(&index, "list indices must be integers")?;
                let mut items = items.borrow_mut();
                let position = normalize_index(number, items.len());
                match position {
                    Some(position) => {
                        if let Some(slot) = items.get_mut(position) {
                            *slot = value;
                            Ok(())
                        } else {
                            Err(self.err(
                                "IndexError",
                                String::from("list assignment index out of range"),
                            ))
                        }
                    }
                    None => Err(self.err(
                        "IndexError",
                        String::from("list assignment index out of range"),
                    )),
                }
            }
            Value::Dict(entries) => {
                if !index.is_hashable() {
                    return Err(self.err(
                        "TypeError",
                        format!("unhashable type: '{}'", index.type_name()),
                    ));
                }
                dict_set(&mut entries.borrow_mut(), index, value);
                Ok(())
            }
            Value::Instance(instance) => {
                if let Some(method) = class_lookup(&instance.class, "__setitem__") {
                    let bound = bind_method(method, target.clone())?;
                    let mut args = ArgList::new();
                    args.push(index);
                    args.push(value);
                    self.call_value(bound, args, Vec::new())?;
                    Ok(())
                } else {
                    Err(self.err(
                        "TypeError",
                        format!(
                            "'{}' object does not support item assignment",
                            target.type_name()
                        ),
                    ))
                }
            }
            _ => Err(self.err(
                "TypeError",
                format!(
                    "'{}' object does not support item assignment",
                    target.type_name()
                ),
            )),
        }
    }

    fn require_int(&self, value: &Value, message: &str) -> PyResult<i64> {
        match value.as_int() {
            Some(number) => Ok(number),
            None => Err(self.err(
                "TypeError",
                format!("{} (got '{}')", message, value.type_name()),
            )),
        }
    }

    fn slice_value(
        &self,
        target: &Value,
        start: Option<i64>,
        stop: Option<i64>,
        step: Option<i64>,
    ) -> PyResult<Value> {
        let (start, stop, step) = self.slice_bounds(target_len(target), start, stop, step)?;
        match target {
            Value::Str(text) => {
                let chars: Vec<char> = text.chars().collect();
                let mut out = String::new();
                let mut position = start;
                if step > 0 {
                    while position < stop {
                        if let Some(ch) = chars.get(position as usize) {
                            out.push(*ch);
                        }
                        position += step;
                    }
                } else {
                    while position > stop {
                        if let Some(ch) = chars.get(position as usize) {
                            out.push(*ch);
                        }
                        position += step;
                    }
                }
                Ok(Value::Str(Rc::from(out)))
            }
            Value::List(items) => {
                let source = items.borrow();
                let mut out = Vec::new();
                let mut position = start;
                if step > 0 {
                    while position < stop {
                        if let Some(value) = source.get(position as usize) {
                            out.push(value.clone());
                        }
                        position += step;
                    }
                } else {
                    while position > stop {
                        if let Some(value) = source.get(position as usize) {
                            out.push(value.clone());
                        }
                        position += step;
                    }
                }
                Ok(Value::list(out))
            }
            Value::Tuple(items) => {
                let mut out = Vec::new();
                let mut position = start;
                if step > 0 {
                    while position < stop {
                        if let Some(value) = items.get(position as usize) {
                            out.push(value.clone());
                        }
                        position += step;
                    }
                } else {
                    while position > stop {
                        if let Some(value) = items.get(position as usize) {
                            out.push(value.clone());
                        }
                        position += step;
                    }
                }
                Ok(Value::tuple(out))
            }
            Value::Bytes(data) => {
                let mut out = Vec::new();
                let mut position = start;
                if step > 0 {
                    while position < stop {
                        if let Some(byte) = data.get(position as usize) {
                            out.push(*byte);
                        }
                        position += step;
                    }
                } else {
                    while position > stop {
                        if let Some(byte) = data.get(position as usize) {
                            out.push(*byte);
                        }
                        position += step;
                    }
                }
                Ok(Value::bytes(out))
            }
            _ => Err(self.err(
                "TypeError",
                format!("'{}' object is not sliceable", target.type_name()),
            )),
        }
    }

    fn slice_bounds(
        &self,
        len: i64,
        start: Option<i64>,
        stop: Option<i64>,
        step: Option<i64>,
    ) -> PyResult<(i64, i64, i64)> {
        let step = step.unwrap_or(1);
        if step == 0 {
            return Err(self.err("ValueError", String::from("slice step cannot be zero")));
        }
        let positive = step > 0;
        let lower: i64 = if positive { 0 } else { -1 };
        let upper: i64 = if positive { len } else { len - 1 };
        let normalize = |value: i64| -> i64 {
            if value < 0 {
                let shifted = value as i128 + len as i128;
                if shifted < lower as i128 {
                    lower
                } else if shifted > upper as i128 {
                    upper
                } else {
                    shifted as i64
                }
            } else if value > upper {
                upper
            } else {
                value
            }
        };
        let start = match start {
            Some(value) => normalize(value),
            None => {
                if positive {
                    0
                } else {
                    upper
                }
            }
        };
        let stop = match stop {
            Some(value) => normalize(value),
            None => {
                if positive {
                    len
                } else {
                    -1
                }
            }
        };
        Ok((start, stop, step))
    }

    pub fn get_attribute(&mut self, target: &Value, name: &str) -> PyResult<Value> {
        match target {
            Value::Str(_) => {
                if name == "encode" {
                    return Ok(Value::method(BuiltinId::StrEncode, target.clone()));
                }
                if let Some(id) = string_method(name) {
                    return Ok(Value::method(id, target.clone()));
                }
            }
            Value::Bytes(_) => {
                if let Some(id) = bytes_method(name) {
                    return Ok(Value::method(id, target.clone()));
                }
            }
            Value::Set(_) => {
                if let Some(id) = set_method(name) {
                    return Ok(Value::method(id, target.clone()));
                }
            }
            Value::List(_) => {
                if let Some(id) = list_method(name) {
                    return Ok(Value::method(id, target.clone()));
                }
            }
            Value::Dict(_) => {
                if let Some(id) = dict_method(name) {
                    return Ok(Value::method(id, target.clone()));
                }
            }
            Value::File(_) => {
                if name == "__enter__" {
                    return Ok(Value::method(BuiltinId::FileEnter, target.clone()));
                }
                if name == "__exit__" {
                    return Ok(Value::method(BuiltinId::FileExit, target.clone()));
                }
                if let Some(value) = self.file_attribute(target, name) {
                    return value;
                }
                if let Some(id) = file_method(name) {
                    return Ok(Value::method(id, target.clone()));
                }
            }
            Value::Range(range) => match name {
                "start" => return Ok(Value::Int(range.start)),
                "stop" => return Ok(Value::Int(range.stop)),
                "step" => return Ok(Value::Int(range.step)),
                _ => {}
            },
            Value::Module(module) => {
                if let Some(value) = attr_get(&module.attrs, name) {
                    return Ok(value);
                }
                return Err(self.err(
                    "AttributeError",
                    format!("module '{}' has no attribute '{}'", module.name, name),
                ));
            }
            Value::Class(class) => {
                match name {
                    "__name__" => return Ok(Value::Str(class.name.clone())),
                    "__bases__" => {
                        let bases = class
                            .bases
                            .iter()
                            .map(|base| Value::Class(base.clone()))
                            .collect();
                        return Ok(Value::tuple(bases));
                    }
                    _ => {}
                }
                if let Some(value) = class_lookup(class, name) {
                    return Ok(value);
                }
                return Err(self.err(
                    "AttributeError",
                    format!("type object '{}' has no attribute '{}'", class.name, name),
                ));
            }
            Value::Instance(instance) => {
                if let Some(value) = attr_get(&instance.attrs, name) {
                    return Ok(value);
                }
                if let Some(value) = class_lookup(&instance.class, name) {
                    if let Value::Function(function) = value {
                        return Ok(Value::BoundMethod(Rc::new(crate::value::BoundMethod {
                            function,
                            receiver: target.clone(),
                        })));
                    }
                    return Ok(value);
                }
                if instance.class.is_exception && name == "args" {
                    return Ok(Value::tuple(Vec::new()));
                }
                return Err(self.err(
                    "AttributeError",
                    format!(
                        "'{}' object has no attribute '{}'",
                        instance.class.name, name
                    ),
                ));
            }
            _ => {}
        }
        Err(self.err(
            "AttributeError",
            format!(
                "'{}' object has no attribute '{}'",
                target.type_name(),
                name
            ),
        ))
    }

    fn file_attribute(&self, target: &Value, name: &str) -> Option<PyResult<Value>> {
        let Value::File(file) = target else {
            return None;
        };
        let borrowed = file.borrow();
        match name {
            "name" => Some(Ok(Value::Str(borrowed.name.clone()))),
            "mode" => Some(Ok(Value::Str(borrowed.mode.clone()))),
            "closed" => Some(Ok(Value::Bool(borrowed.closed))),
            _ => None,
        }
    }

    fn set_attribute(&mut self, target: &Value, name: Name, value: Value) -> PyResult<()> {
        match target {
            Value::Instance(instance) => {
                attr_set(&instance.attrs, name, value);
                Ok(())
            }
            Value::Class(class) => {
                attr_set(&class.attrs, name, value);
                Ok(())
            }
            Value::Module(module) => {
                attr_set(&module.attrs, name, value);
                Ok(())
            }
            _ => Err(self.err(
                "AttributeError",
                format!(
                    "'{}' object attribute '{}' is read-only or does not exist",
                    target.type_name(),
                    name
                ),
            )),
        }
    }

    // ── iteration ──────────────────────────────────────────────────────────

    pub fn make_iterator(&mut self, value: Value) -> PyResult<Value> {
        match &value {
            Value::Iterator(_) | Value::Generator(_) => Ok(value),
            Value::Range(range) => Ok(Value::Iterator(Rc::new(RefCell::new(IterState::Range {
                current: range.start,
                stop: range.stop,
                step: range.step,
                remaining: range.len,
            })))),
            Value::Str(text) => Ok(Value::Iterator(Rc::new(RefCell::new(IterState::Str {
                text: text.clone(),
                byte_index: 0,
            })))),
            Value::List(items) => Ok(Value::Iterator(Rc::new(RefCell::new(IterState::List {
                items: items.clone(),
                index: 0,
            })))),
            Value::Tuple(items) => Ok(Value::Iterator(Rc::new(RefCell::new(IterState::Tuple {
                items: items.clone(),
                index: 0,
            })))),
            Value::Dict(entries) => Ok(Value::Iterator(Rc::new(RefCell::new(
                IterState::DictKeys {
                    entries: entries.clone(),
                    index: 0,
                },
            )))),
            Value::Set(items) => Ok(Value::Iterator(Rc::new(RefCell::new(
                IterState::SetElems {
                    items: items.clone(),
                    index: 0,
                },
            )))),
            Value::Bytes(data) => Ok(Value::Iterator(Rc::new(RefCell::new(IterState::Bytes {
                data: data.clone(),
                index: 0,
            })))),
            Value::File(file) => Ok(Value::Iterator(Rc::new(RefCell::new(IterState::File {
                file: file.clone(),
            })))),
            Value::Instance(instance) => {
                if let Some(method) = class_lookup(&instance.class, "__iter__") {
                    let bound = bind_method(method, value.clone())?;
                    let produced = self.call_value(bound, ArgList::new(), Vec::new())?;
                    if matches!(produced, Value::Iterator(_)) {
                        return Ok(produced);
                    }
                    return Err(self.err(
                        "TypeError",
                        String::from("__iter__ returned a non-iterator"),
                    ));
                }
                Err(self.err(
                    "TypeError",
                    format!("'{}' object is not iterable", value.type_name()),
                ))
            }
            _ => Err(self.err(
                "TypeError",
                format!("'{}' object is not iterable", value.type_name()),
            )),
        }
    }

    pub fn next_from_iterator(&mut self, iterator: &Value) -> PyResult<Option<Value>> {
        match iterator {
            Value::Iterator(state) => Ok(state.borrow_mut().next()),
            Value::Generator(generator) => self.generator_resume(generator),
            _ => Err(self.err(
                "TypeError",
                format!("'{}' object is not an iterator", iterator.type_name()),
            )),
        }
    }

    pub fn collect_iterable(&mut self, value: Value) -> PyResult<Vec<Value>> {
        let iterator = self.make_iterator(value)?;
        let mut items = Vec::new();
        while let Some(item) = self.next_from_iterator(&iterator)? {
            items.push(item);
        }
        Ok(items)
    }

    // ── imports ────────────────────────────────────────────────────────────

    fn import_module(&mut self, module: &str) -> PyResult<Value> {
        // `import os.path` / `from os.path import join`: import the head and
        // walk the attributes, which is what makes `os.path` a module object
        // inside `os` rather than a name the loader has to know about.
        if let Some((head, rest)) = module.split_once('.') {
            let mut value = self.import_module(head)?;
            for part in rest.split('.') {
                value = self.get_attribute(&value, part)?;
            }
            return Ok(value);
        }
        if module == "sys" {
            return self.sys_module();
        }
        if module == "math" {
            return self.math_module();
        }
        if module == "os" {
            return self.os_module();
        }
        if module == "json" {
            return self.json_module();
        }
        if module == "time" {
            return self.time_module();
        }
        if module == "random" {
            return self.random_module();
        }
        if let Some((_, value)) = self.module_cache.iter().find(|(name, _)| name == module) {
            return Ok(value.clone());
        }
        let path = match self.find_module_file(module) {
            Some(path) => path,
            None => {
                return Err(self.err(
                    "ModuleNotFoundError",
                    format!("No module named '{}'", module),
                ))
            }
        };
        let source = match platform::read_text(&path) {
            Ok(source) => source,
            Err(error) => {
                return Err(self.err(
                    "OSError",
                    format!("cannot read module '{}': {}", path, error),
                ))
            }
        };
        let parsed = match parser::parse(&source) {
            Ok(module) => module,
            Err(error) => {
                return Err(self.err(
                    "SyntaxError",
                    format!("{}:{}: {}", path, error.line, error.message),
                ))
            }
        };
        let env = Env::new(None);
        env.borrow_mut()
            .set_local(Rc::from("__name__"), Value::str(module));
        let module_value = Value::Module(Rc::new(ModuleValue {
            name: Rc::from(module),
            attrs: RefCell::new(Vec::new()),
        }));
        self.module_cache
            .push((String::from(module), module_value.clone()));
        self.call_stack.push(Frame {
            filename: Rc::from(path.as_str()),
            name: Rc::from("<module>"),
            line: 1,
        });
        let outcome = self.exec_block(&parsed.body, &env);
        self.call_stack.pop();
        outcome?;
        if let Value::Module(module_object) = &module_value {
            let mut attrs = module_object.attrs.borrow_mut();
            for (name, value) in env.borrow().vars.iter() {
                attrs.push((name.clone(), value.clone()));
            }
        }
        Ok(module_value)
    }

    fn find_module_file(&self, module: &str) -> Option<String> {
        let relative = module.replace('.', "/");
        for directory in &self.search_paths {
            let base = if directory.ends_with('/') {
                directory.clone()
            } else {
                format!("{}/", directory)
            };
            let file = format!("{}{}.py", base, relative);
            if platform::path_exists(&file) {
                return Some(file);
            }
            let package = format!("{}{}/__init__.py", base, relative);
            if platform::path_exists(&package) {
                return Some(package);
            }
        }
        None
    }

    /// The built-in `math` module. Constants and functions are installed as
    /// ordinary module attributes, so `import math`, `from math import *` and
    /// `math.sqrt` all work without a `.py` file.
    fn math_module(&mut self) -> PyResult<Value> {
        if let Some((_, value)) = self.module_cache.iter().find(|(name, _)| name == "math") {
            return Ok(value.clone());
        }
        let mut attrs: Vec<(Rc<str>, Value)> = Vec::new();
        attrs.push((Rc::from("pi"), Value::Float(crate::float::PI)));
        attrs.push((Rc::from("e"), Value::Float(crate::float::E)));
        attrs.push((Rc::from("inf"), Value::Float(f64::INFINITY)));
        attrs.push((Rc::from("nan"), Value::Float(f64::NAN)));
        for (name, id) in [
            ("sqrt", BuiltinId::MathSqrt),
            ("floor", BuiltinId::MathFloor),
            ("ceil", BuiltinId::MathCeil),
            ("trunc", BuiltinId::MathTrunc),
            ("fabs", BuiltinId::MathFabs),
            ("pow", BuiltinId::MathPow),
            ("fmod", BuiltinId::MathFmod),
            ("sin", BuiltinId::MathSin),
            ("cos", BuiltinId::MathCos),
            ("tan", BuiltinId::MathTan),
            ("log", BuiltinId::MathLog),
            ("log10", BuiltinId::MathLog10),
            ("exp", BuiltinId::MathExp),
            ("isnan", BuiltinId::MathIsnan),
            ("isinf", BuiltinId::MathIsinf),
        ] {
            attrs.push((Rc::from(name), Value::builtin(id)));
        }
        let module = Value::Module(Rc::new(ModuleValue {
            name: Rc::from("math"),
            attrs: RefCell::new(attrs),
        }));
        self.module_cache
            .push((String::from("math"), module.clone()));
        Ok(module)
    }

    /// The built-in `os` module. `environ` is a snapshot of the process
    /// environment; there is no `putenv` in the ABI, so writing to the dict
    /// changes only the Python copy.
    fn os_module(&mut self) -> PyResult<Value> {
        if let Some((_, value)) = self.module_cache.iter().find(|(name, _)| name == "os") {
            return Ok(value.clone());
        }
        let mut attrs: Vec<(Rc<str>, Value)> = Vec::new();
        for (name, id) in [
            ("getpid", BuiltinId::OsGetpid),
            ("getppid", BuiltinId::OsGetppid),
            ("getcwd", BuiltinId::OsGetcwd),
            ("listdir", BuiltinId::OsListdir),
            ("mkdir", BuiltinId::OsMkdir),
            ("makedirs", BuiltinId::OsMakedirs),
            ("remove", BuiltinId::OsRemove),
            ("rmdir", BuiltinId::OsRmdir),
            ("rename", BuiltinId::OsRename),
            ("stat", BuiltinId::OsStat),
            ("fork", BuiltinId::OsFork),
            ("execv", BuiltinId::OsExecv),
            ("waitpid", BuiltinId::OsWaitpid),
            ("wait", BuiltinId::OsWait),
            ("system", BuiltinId::OsSystem),
        ] {
            attrs.push((Rc::from(name), Value::builtin(id)));
        }
        let mut environment: Vec<(Value, Value)> = Vec::new();
        for (key, value) in platform::vars() {
            dict_set(
                &mut environment,
                Value::Str(Rc::from(key.as_str())),
                Value::Str(Rc::from(value.as_str())),
            );
        }
        if !environment
            .iter()
            .any(|(key, _)| matches!(key, Value::Str(text) if &**text == "PWD"))
        {
            dict_set(
                &mut environment,
                Value::str("PWD"),
                Value::Str(Rc::from(platform::current_dir().as_str())),
            );
        }
        let path_module = self.path_module()?;
        attrs.push((Rc::from("path"), path_module));
        attrs.push((Rc::from("environ"), Value::dict(environment)));
        attrs.push((Rc::from("sep"), Value::str("/")));
        attrs.push((Rc::from("linesep"), Value::str("\n")));
        let stat_class = self.ensure_stat_class()?;
        attrs.push((Rc::from("stat_result"), Value::Class(stat_class)));
        let module = Value::Module(Rc::new(ModuleValue {
            name: Rc::from("os"),
            attrs: RefCell::new(attrs),
        }));
        self.module_cache
            .push((String::from("os"), module.clone()));
        Ok(module)
    }

    /// `os.path`: the pure path helpers plus the existence probes. It is a
    /// module object (`os.path.join`, `from os.path import basename`) and is
    /// cached under its dotted name.
    fn path_module(&mut self) -> PyResult<Value> {
        if let Some((_, value)) = self.module_cache.iter().find(|(name, _)| name == "os.path") {
            return Ok(value.clone());
        }
        let mut attrs: Vec<(Rc<str>, Value)> = Vec::new();
        for (name, id) in [
            ("join", BuiltinId::PathJoin),
            ("basename", BuiltinId::PathBasename),
            ("dirname", BuiltinId::PathDirname),
            ("exists", BuiltinId::PathExists),
            ("isfile", BuiltinId::PathIsfile),
            ("isdir", BuiltinId::PathIsdir),
        ] {
            attrs.push((Rc::from(name), Value::builtin(id)));
        }
        attrs.push((Rc::from("sep"), Value::str("/")));
        let module = Value::Module(Rc::new(ModuleValue {
            name: Rc::from("os.path"),
            attrs: RefCell::new(attrs),
        }));
        self.module_cache
            .push((String::from("os.path"), module.clone()));
        Ok(module)
    }

    /// `time`: the two ABI clocks and `sleep`.
    fn time_module(&mut self) -> PyResult<Value> {
        if let Some((_, value)) = self.module_cache.iter().find(|(name, _)| name == "time") {
            return Ok(value.clone());
        }
        let mut attrs: Vec<(Rc<str>, Value)> = Vec::new();
        for (name, id) in [
            ("time", BuiltinId::TimeTime),
            ("monotonic", BuiltinId::TimeMonotonic),
            ("sleep", BuiltinId::TimeSleep),
        ] {
            attrs.push((Rc::from(name), Value::builtin(id)));
        }
        let module = Value::Module(Rc::new(ModuleValue {
            name: Rc::from("time"),
            attrs: RefCell::new(attrs),
        }));
        self.module_cache
            .push((String::from("time"), module.clone()));
        Ok(module)
    }

    /// `random`: a deterministic xorshift64* generator (see `RNG_SEED`).
    fn random_module(&mut self) -> PyResult<Value> {
        if let Some((_, value)) = self.module_cache.iter().find(|(name, _)| name == "random") {
            return Ok(value.clone());
        }
        let mut attrs: Vec<(Rc<str>, Value)> = Vec::new();
        for (name, id) in [
            ("random", BuiltinId::RandomRandom),
            ("seed", BuiltinId::RandomSeed),
            ("randint", BuiltinId::RandomRandint),
            ("choice", BuiltinId::RandomChoice),
            ("shuffle", BuiltinId::RandomShuffle),
            ("uniform", BuiltinId::RandomUniform),
        ] {
            attrs.push((Rc::from(name), Value::builtin(id)));
        }
        let module = Value::Module(Rc::new(ModuleValue {
            name: Rc::from("random"),
            attrs: RefCell::new(attrs),
        }));
        self.module_cache
            .push((String::from("random"), module.clone()));
        Ok(module)
    }

    fn json_module(&mut self) -> PyResult<Value> {
        if let Some((_, value)) = self.module_cache.iter().find(|(name, _)| name == "json") {
            return Ok(value.clone());
        }
        let mut attrs: Vec<(Rc<str>, Value)> = Vec::new();
        attrs.push((Rc::from("dumps"), Value::builtin(BuiltinId::JsonDumps)));
        attrs.push((Rc::from("loads"), Value::builtin(BuiltinId::JsonLoads)));
        let module = Value::Module(Rc::new(ModuleValue {
            name: Rc::from("json"),
            attrs: RefCell::new(attrs),
        }));
        self.module_cache
            .push((String::from("json"), module.clone()));
        Ok(module)
    }

    fn ensure_stat_class(&mut self) -> PyResult<Rc<Class>> {
        if let Some(class) = &self.stat_class {
            return Ok(class.clone());
        }
        let object = match self.class_for(BuiltinType::Object) {
            Some(class) => class,
            None => {
                return Err(self.err(
                    "RuntimeError",
                    String::from("interpreter is missing the object class"),
                ))
            }
        };
        let class = Rc::new(Class {
            name: Rc::from("os.stat_result"),
            bases: alloc::vec![object],
            attrs: RefCell::new(Vec::new()),
            builtin: None,
            is_exception: false,
        });
        self.stat_class = Some(class.clone());
        Ok(class)
    }

    /// Build the small `os.stat_result` object: size, mode, kind and two
    /// convenience booleans.
    pub(crate) fn make_stat_value(&mut self, size: u64, mode: u32, kind: u32) -> PyResult<Value> {
        let class = self.ensure_stat_class()?;
        let instance = Instance {
            class,
            attrs: RefCell::new(Vec::new()),
        };
        let size = i64::try_from(size).unwrap_or(i64::MAX);
        let kind_name = match kind {
            0 => "file",
            1 => "dir",
            2 => "symlink",
            _ => "other",
        };
        attr_set(&instance.attrs, Rc::from("st_size"), Value::Int(size));
        attr_set(&instance.attrs, Rc::from("st_mode"), Value::Int(mode as i64));
        attr_set(&instance.attrs, Rc::from("st_kind"), Value::str(kind_name));
        attr_set(&instance.attrs, Rc::from("st_isdir"), Value::Bool(kind == 1));
        attr_set(&instance.attrs, Rc::from("st_isfile"), Value::Bool(kind == 0));
        Ok(Value::Instance(Rc::new(instance)))
    }

    fn sys_module(&mut self) -> PyResult<Value> {
        if let Some((_, value)) = self.module_cache.iter().find(|(name, _)| name == "sys") {
            return Ok(value.clone());
        }
        let mut attrs: Vec<(Rc<str>, Value)> = Vec::new();
        let argv = self
            .argv
            .iter()
            .map(|argument| Value::Str(Rc::from(argument.as_str())))
            .collect();
        attrs.push((Rc::from("argv"), Value::list(argv)));
        let path = self
            .search_paths
            .iter()
            .map(|directory| Value::Str(Rc::from(directory.as_str())))
            .collect();
        attrs.push((Rc::from("path"), Value::list(path)));
        attrs.push((
            Rc::from("version"),
            Value::str("3.4.0 (pagh-python, float)"),
        ));
        attrs.push((Rc::from("platform"), Value::str("pagh-riscv")));
        attrs.push((Rc::from("maxsize"), Value::Int(i64::MAX)));
        let mut info: Vec<(Rc<str>, Value)> = Vec::new();
        for (name, value) in [
            ("max", Value::Float(f64::MAX)),
            ("min", Value::Float(f64::MIN_POSITIVE)),
            ("epsilon", Value::Float(f64::EPSILON)),
            ("dig", Value::Int(15)),
            ("mant_dig", Value::Int(53)),
            ("max_exp", Value::Int(1024)),
            ("min_exp", Value::Int(-1021)),
            ("max_10_exp", Value::Int(308)),
            ("min_10_exp", Value::Int(-307)),
            ("radix", Value::Int(2)),
            ("rounds", Value::Int(1)),
        ] {
            info.push((Rc::from(name), value));
        }
        attrs.push((
            Rc::from("float_info"),
            Value::Module(Rc::new(ModuleValue {
                name: Rc::from("sys.float_info"),
                attrs: RefCell::new(info),
            })),
        ));
        let module = Value::Module(Rc::new(ModuleValue {
            name: Rc::from("sys"),
            attrs: RefCell::new(attrs),
        }));
        self.module_cache
            .push((String::from("sys"), module.clone()));
        Ok(module)
    }

    // ── tracebacks ─────────────────────────────────────────────────────────

    /// Refuse to start another Python call when the running generator's fiber
    /// stack is nearly exhausted.
    ///
    /// A fiber has no guard page under it and overrunning it would corrupt the
    /// interpreter's heap, so a generator body that recurses too deeply gets
    /// the `RecursionError` Python expects. (The *process* stack, which plain
    /// calls use, is not checked: see [`fiber::PROCESS_STACK`].)
    fn check_fiber_headroom(&self) -> PyResult<()> {
        let active = match self.active_fibers.last() {
            Some(active) => active,
            None => return Ok(()),
        };
        // SAFETY: the pointer refers to the fiber that is running right now
        // (it was pushed before the switch and is popped after it).
        let headroom = unsafe { (*active.fiber).headroom() };
        if headroom < fiber::STACK_MARGIN {
            return Err(self.err(
                "RecursionError",
                format!(
                    "generator stack exhausted ({} bytes left of {} KiB)",
                    headroom,
                    fiber::STACK_SIZE / 1024
                ),
            ));
        }
        Ok(())
    }

    pub fn set_line(&mut self, line: u32) {
        if let Some(frame) = self.call_stack.last_mut() {
            frame.line = line;
        }
    }

    fn current_line(&self) -> u32 {
        match self.call_stack.last() {
            Some(frame) => frame.line,
            None => 1,
        }
    }

    fn current_filename(&self) -> Rc<str> {
        match self.call_stack.last() {
            Some(frame) => frame.filename.clone(),
            None => Rc::from("<unknown>"),
        }
    }

    pub fn snapshot(&self) -> Vec<TraceFrame> {
        self.call_stack
            .iter()
            .map(|frame| TraceFrame {
                filename: frame.filename.clone(),
                name: frame.name.clone(),
                line: frame.line,
            })
            .collect()
    }

    pub fn err(&self, name: &'static str, message: String) -> PyError {
        let mut error = PyError::builtin(name, message);
        error.frames = self.snapshot();
        error
    }
}

// ── helpers outside the impl (no `&self` needed) ──────────────────────────

fn simple_class(name: &str, tag: BuiltinType, base: Option<Rc<Class>>) -> Rc<Class> {
    Rc::new(Class {
        name: Rc::from(name),
        bases: match base {
            Some(base) => alloc::vec![base],
            None => Vec::new(),
        },
        attrs: RefCell::new(Vec::new()),
        builtin: Some(tag),
        is_exception: false,
    })
}

fn exception_class(name: &str, bases: Vec<Rc<Class>>, _unused: Option<()>) -> Rc<Class> {
    Rc::new(Class {
        name: Rc::from(name),
        bases,
        attrs: RefCell::new(Vec::new()),
        builtin: None,
        is_exception: true,
    })
}

fn bind_method(value: Value, receiver: Value) -> PyResult<Value> {
    match value {
        Value::Function(function) => Ok(Value::BoundMethod(Rc::new(crate::value::BoundMethod {
            function,
            receiver,
        }))),
        other => Ok(other),
    }
}

fn compare_sequences(interp: &Interp, left: &[Value], right: &[Value]) -> PyResult<Ordering> {
    let mut index = 0;
    while index < left.len() && index < right.len() {
        let ordering = interp.cmp_values(&left[index], &right[index])?;
        if ordering != Ordering::Equal {
            return Ok(ordering);
        }
        index += 1;
    }
    Ok(left.len().cmp(&right.len()))
}

fn target_len(target: &Value) -> i64 {
    match target {
        Value::Str(text) => text.chars().count() as i64,
        Value::List(items) => items.borrow().len() as i64,
        Value::Tuple(items) => items.len() as i64,
        Value::Bytes(data) => data.len() as i64,
        _ => 0,
    }
}

fn normalize_index(index: i64, len: usize) -> Option<usize> {
    let len = len as i128;
    let index = index as i128;
    let resolved = if index < 0 { index + len } else { index };
    if resolved < 0 || resolved >= len {
        None
    } else {
        Some(resolved as usize)
    }
}

fn floor_div_i128(a: i64, b: i64) -> i128 {
    let a = a as i128;
    let b = b as i128;
    let mut quotient = a / b;
    let remainder = a % b;
    if remainder != 0 && ((remainder < 0) != (b < 0)) {
        quotient -= 1;
    }
    quotient
}

fn floor_div_wide(a: i64, b: i64) -> Option<i64> {
    if b == 0 {
        return None;
    }
    i64::try_from(floor_div_i128(a, b)).ok()
}

fn range_contains(range: &RangeValue, value: i64) -> bool {
    if range.len == 0 {
        return false;
    }
    if range.step > 0 {
        if value < range.start || value >= range.stop {
            return false;
        }
    } else if value > range.start || value <= range.stop {
        return false;
    }
    let difference = value as i128 - range.start as i128;
    let step = range.step as i128;
    difference % step == 0
}

fn op_text(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::FloorDiv => "//",
        BinOp::Mod => "%",
        BinOp::Pow => "**",
    }
}

fn set_method(name: &str) -> Option<BuiltinId> {
    Some(match name {
        "add" => BuiltinId::SetAdd,
        "remove" => BuiltinId::SetRemove,
        "discard" => BuiltinId::SetDiscard,
        "union" => BuiltinId::SetUnion,
        "intersection" => BuiltinId::SetIntersection,
        "difference" => BuiltinId::SetDifference,
        "issubset" => BuiltinId::SetIssubset,
        "issuperset" => BuiltinId::SetSuperset,
        "copy" => BuiltinId::SetCopy,
        "clear" => BuiltinId::SetClear,
        _ => return None,
    })
}

fn bytes_method(name: &str) -> Option<BuiltinId> {
    Some(match name {
        "decode" => BuiltinId::BytesDecode,
        "hex" => BuiltinId::BytesHex,
        _ => return None,
    })
}

fn set_is_subset(left: &[Value], right: &[Value]) -> bool {
    left.iter()
        .all(|item| value::set_find(right, item).is_some())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn is_utf8_name(name: &str) -> bool {
    matches!(name, "utf-8" | "utf8" | "UTF-8" | "UTF8" | "utf_8")
}

fn string_method(name: &str) -> Option<BuiltinId> {
    Some(match name {
        "split" => BuiltinId::StrSplit,
        "join" => BuiltinId::StrJoin,
        "strip" => BuiltinId::StrStrip,
        "lstrip" => BuiltinId::StrLstrip,
        "rstrip" => BuiltinId::StrRstrip,
        "upper" => BuiltinId::StrUpper,
        "lower" => BuiltinId::StrLower,
        "replace" => BuiltinId::StrReplace,
        "find" => BuiltinId::StrFind,
        "startswith" => BuiltinId::StrStartswith,
        "endswith" => BuiltinId::StrEndswith,
        "count" => BuiltinId::StrCount,
        "index" => BuiltinId::StrIndex,
        "isdigit" => BuiltinId::StrIsdigit,
        "title" => BuiltinId::StrTitle,
        "splitlines" => BuiltinId::StrSplitlines,
        "format" => BuiltinId::StrFormat,
        _ => return None,
    })
}

fn list_method(name: &str) -> Option<BuiltinId> {
    Some(match name {
        "append" => BuiltinId::ListAppend,
        "extend" => BuiltinId::ListExtend,
        "pop" => BuiltinId::ListPop,
        "sort" => BuiltinId::ListSort,
        "insert" => BuiltinId::ListInsert,
        "remove" => BuiltinId::ListRemove,
        "index" => BuiltinId::ListIndex,
        "reverse" => BuiltinId::ListReverse,
        "clear" => BuiltinId::ListClear,
        "copy" => BuiltinId::ListCopy,
        _ => return None,
    })
}

fn dict_method(name: &str) -> Option<BuiltinId> {
    Some(match name {
        "get" => BuiltinId::DictGet,
        "keys" => BuiltinId::DictKeys,
        "values" => BuiltinId::DictValues,
        "items" => BuiltinId::DictItems,
        "update" => BuiltinId::DictUpdate,
        "pop" => BuiltinId::DictPop,
        "setdefault" => BuiltinId::DictSetdefault,
        "clear" => BuiltinId::DictClear,
        "copy" => BuiltinId::DictCopy,
        _ => return None,
    })
}

fn file_method(name: &str) -> Option<BuiltinId> {
    Some(match name {
        "read" => BuiltinId::FileRead,
        "readline" => BuiltinId::FileReadline,
        "readlines" => BuiltinId::FileReadlines,
        "write" => BuiltinId::FileWrite,
        "writelines" => BuiltinId::FileWritelines,
        "close" => BuiltinId::FileClose,
        "flush" => BuiltinId::FileFlush,
        _ => return None,
    })
}

/// Parse an integer with an explicit base (used by `int(text, base)`).
pub(crate) fn parse_int_with_base(text: &str, base: i64) -> Option<i64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (negative, digits) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    let (effective_base, digits) = if base == 0 {
        if let Some(rest) = digits.strip_prefix("0x").or(digits.strip_prefix("0X")) {
            (16, rest)
        } else if let Some(rest) = digits.strip_prefix("0o").or(digits.strip_prefix("0O")) {
            (8, rest)
        } else if let Some(rest) = digits.strip_prefix("0b").or(digits.strip_prefix("0B")) {
            (2, rest)
        } else {
            (10, digits)
        }
    } else if base == 16 {
        (
            16,
            digits
                .strip_prefix("0x")
                .or(digits.strip_prefix("0X"))
                .unwrap_or(digits),
        )
    } else if base == 8 {
        (
            8,
            digits
                .strip_prefix("0o")
                .or(digits.strip_prefix("0O"))
                .unwrap_or(digits),
        )
    } else if base == 2 {
        (
            2,
            digits
                .strip_prefix("0b")
                .or(digits.strip_prefix("0B"))
                .unwrap_or(digits),
        )
    } else {
        (base, digits)
    };
    if effective_base < 2 || effective_base > 36 {
        return None;
    }
    let cleaned: String = digits.chars().filter(|c| *c != '_').collect();
    if cleaned.is_empty() {
        return None;
    }
    let magnitude = i128::from_str_radix(&cleaned, effective_base as u32).ok()?;
    let signed = if negative { -magnitude } else { magnitude };
    i64::try_from(signed).ok()
}
