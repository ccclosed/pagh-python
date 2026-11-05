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

use crate::ast::{BinOp, BoolOp, CmpOp, CompGen, Expr, Handler, Name, Param, Stmt, StmtKind, UnOp};
use crate::builtins;
use crate::error::{ErrorName, PyError, PyResult, TraceFrame};
use crate::parser;
use crate::platform;
use crate::value;
use crate::value::{
    attr_get, attr_set, class_is_subclass, class_lookup, dict_get, dict_set, env_find_nonlocal,
    env_lookup, is_same, values_equal, BuiltinId, BuiltinType, BuiltinValue, Class, Env, EnvRef,
    Function, Instance, IterState, ModuleValue, RangeValue, Value,
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
    module_cache: Vec<(String, Value)>,
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
            BuiltinType::Bool,
            BuiltinType::Str,
            BuiltinType::List,
            BuiltinType::Tuple,
            BuiltinType::Dict,
            BuiltinType::Function,
            BuiltinType::Module,
            BuiltinType::File,
            BuiltinType::Range,
            BuiltinType::Iterator,
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
        let type_error = exception_class("TypeError", alloc::vec![exception.clone()], None);
        let name_error = exception_class("NameError", alloc::vec![exception.clone()], None);
        let attribute = exception_class("AttributeError", alloc::vec![exception.clone()], None);
        let os_error = exception_class("OSError", alloc::vec![exception.clone()], None);
        let file_error = exception_class("FileNotFoundError", alloc::vec![os_error.clone()], None);
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
            type_error,
            name_error,
            attribute,
            os_error,
            file_error,
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
            module_cache: Vec::new(),
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
            BuiltinType::Bool,
            BuiltinType::Str,
            BuiltinType::List,
            BuiltinType::Tuple,
            BuiltinType::Dict,
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
            ("next", BuiltinId::Next),
            ("any", BuiltinId::Any),
            ("all", BuiltinId::All),
            ("divmod", BuiltinId::Divmod),
            ("pow", BuiltinId::Pow),
            ("hex", BuiltinId::Hex),
            ("id", BuiltinId::Id),
        ] {
            entries.push((Rc::from(name), Value::builtin(id)));
        }
        // `float` exists only to give the documented integer-only error.
        entries.push((Rc::from("float"), Value::builtin(BuiltinId::Float)));
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
            StmtKind::While { condition, body } => {
                loop {
                    self.set_line(statement.line);
                    if !self.eval_expr(condition, env)?.truthy() {
                        break;
                    }
                    match self.exec_block(body, env)? {
                        Flow::Normal => {}
                        Flow::Continue => continue,
                        Flow::Break => break,
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::For {
                target,
                iterable,
                body,
            } => {
                let iterable_value = self.eval_expr(iterable, env)?;
                let iterator = self.make_iterator(iterable_value)?;
                loop {
                    self.set_line(statement.line);
                    match self.next_from_iterator(&iterator)? {
                        None => break,
                        Some(item) => {
                            self.assign_target(target, item, env)?;
                            match self.exec_block(body, env)? {
                                Flow::Normal => {}
                                Flow::Continue => continue,
                                Flow::Break => break,
                                other => return Ok(other),
                            }
                        }
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Break => Ok(Flow::Break),
            StmtKind::Continue => Ok(Flow::Continue),
            StmtKind::Pass => Ok(Flow::Normal),
            StmtKind::Def { name, params, body } => {
                let function = self.make_function(name, params, body, statement.line, env)?;
                self.set_name(env, name.clone(), Value::Function(Rc::new(function)))?;
                Ok(Flow::Normal)
            }
            StmtKind::Return(expression) => match expression {
                Some(expression) => Ok(Flow::Return(self.eval_expr(expression, env)?)),
                None => Ok(Flow::Return(Value::None)),
            },
            StmtKind::Class { name, bases, body } => {
                self.exec_class(name, bases, body, env)?;
                Ok(Flow::Normal)
            }
            StmtKind::Try {
                body,
                handlers,
                else_body,
                finally_body,
            } => self.exec_try(body, handlers, else_body, finally_body, env),
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
                    let module = self.import_module(&alias.name)?;
                    let bind_name = match &alias.alias {
                        Some(alias) => alias.clone(),
                        None => match alias.name.split('.').next() {
                            Some(first) => Rc::from(first),
                            None => alias.name.clone(),
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
        def_line: u32,
        env: &EnvRef,
    ) -> PyResult<Function> {
        let mut bound = Vec::with_capacity(params.len());
        for parameter in params {
            let default = match &parameter.default {
                Some(expression) => Some(self.eval_expr(expression, env)?),
                None => None,
            };
            bound.push((parameter.name.clone(), default));
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
            body: body.clone(),
            closure,
            filename: self.current_filename(),
            def_line,
        })
    }

    fn exec_class(
        &mut self,
        name: &Name,
        base_expressions: &[Expr],
        body: &[Stmt],
        env: &EnvRef,
    ) -> PyResult<()> {
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
        self.set_name(env, name.clone(), Value::Class(class))?;
        Ok(())
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
            Expr::Float(raw) => Err(self.err(
                "NotImplementedError",
                format!(
                    "floating-point literal '{}' is not supported: the pagh kernel \
                     runs U-mode with the FPU disabled, so this interpreter is \
                     integer-only",
                    raw
                ),
            )),
            Expr::Str(text) => Ok(Value::Str(text.clone())),
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
            Expr::Unary { op, value } => {
                let value = self.eval_expr(value, env)?;
                match op {
                    UnOp::Not => Ok(Value::Bool(!value.truthy())),
                    UnOp::Minus => match value.as_int() {
                        Some(number) => match number.checked_neg() {
                            Some(result) => Ok(Value::Int(result)),
                            None => Err(self
                                .err("OverflowError", String::from("integer negation overflow"))),
                        },
                        None => Err(self.err(
                            "TypeError",
                            format!("bad operand type for unary -: '{}'", value.type_name()),
                        )),
                    },
                    UnOp::Plus => match value.as_int() {
                        Some(number) => Ok(Value::Int(number)),
                        None => Err(self.err(
                            "TypeError",
                            format!("bad operand type for unary +: '{}'", value.type_name()),
                        )),
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
                    let value = self.eval_expr(argument, env)?;
                    arguments.push(value);
                }
                let mut keywords = Vec::with_capacity(kwargs.len());
                for (name, argument) in kwargs {
                    let value = self.eval_expr(argument, env)?;
                    keywords.push((name.clone(), value));
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
        let outcome = self.exec_block(&function.body, &env);
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
        if positional < args.len() {
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

    pub fn binary_op(&self, op: BinOp, left: Value, right: Value) -> PyResult<Value> {
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
        let (Some(a), Some(b)) = (left.as_int(), right.as_int()) else {
            return Err(self.err(
                "TypeError",
                format!(
                    "unsupported operand type(s) for {}: '{}' and '{}'",
                    op_text(op),
                    left.type_name(),
                    right.type_name()
                ),
            ));
        };
        let result = match op {
            BinOp::Sub => a.checked_sub(b),
            _ => None,
        };
        match result {
            Some(value) => Ok(Value::Int(value)),
            None => Err(self.err("OverflowError", format!("integer {} overflow", op_text(op)))),
        }
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
        let (text, count) = match (&left, &right) {
            (Value::Str(text), _) => match right.as_int() {
                Some(count) => (Some(text.clone()), count),
                None => (Some(text.clone()), 0),
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

    fn divide_values(&self, left: Value, right: Value) -> PyResult<Value> {
        let (Some(a), Some(b)) = (left.as_int(), right.as_int()) else {
            return Err(self.err(
                "TypeError",
                format!(
                    "unsupported operand type(s) for /: '{}' and '{}'",
                    left.type_name(),
                    right.type_name()
                ),
            ));
        };
        if b == 0 {
            return Err(self.err("ZeroDivisionError", String::from("division by zero")));
        }
        let remainder = a.checked_rem(b).unwrap_or(0);
        if remainder != 0 {
            return Err(self.err(
                "NotImplementedError",
                format!(
                    "true division {} / {} is not an integer; floating point is \
                     unavailable because the pagh kernel disables the U-mode FPU. \
                     Use // for floor division",
                    a, b
                ),
            ));
        }
        match a.checked_div(b) {
            Some(result) => Ok(Value::Int(result)),
            None => Err(self.err("OverflowError", String::from("integer division overflow"))),
        }
    }

    fn floor_divide_values(&self, left: Value, right: Value) -> PyResult<Value> {
        let (Some(a), Some(b)) = (left.as_int(), right.as_int()) else {
            return Err(self.err(
                "TypeError",
                format!(
                    "unsupported operand type(s) for //: '{}' and '{}'",
                    left.type_name(),
                    right.type_name()
                ),
            ));
        };
        match floor_div_wide(a, b) {
            Some(result) => Ok(Value::Int(result)),
            None => Err(self.err(
                "ZeroDivisionError",
                String::from("integer division or modulo by zero"),
            )),
        }
    }

    fn modulo_values(&self, left: Value, right: Value) -> PyResult<Value> {
        let (Some(a), Some(b)) = (left.as_int(), right.as_int()) else {
            return Err(self.err(
                "TypeError",
                format!(
                    "unsupported operand type(s) for %: '{}' and '{}'",
                    left.type_name(),
                    right.type_name()
                ),
            ));
        };
        if b == 0 {
            return Err(self.err(
                "ZeroDivisionError",
                String::from("integer division or modulo by zero"),
            ));
        }
        let quotient = floor_div_i128(a, b);
        let remainder = a as i128 - quotient * (b as i128);
        match i64::try_from(remainder) {
            Ok(result) => Ok(Value::Int(result)),
            Err(_) => Err(self.err("OverflowError", String::from("integer modulo overflow"))),
        }
    }

    fn power_values(&self, left: Value, right: Value) -> PyResult<Value> {
        let (Some(base), Some(exponent)) = (left.as_int(), right.as_int()) else {
            return Err(self.err(
                "TypeError",
                format!(
                    "unsupported operand type(s) for **: '{}' and '{}'",
                    left.type_name(),
                    right.type_name()
                ),
            ));
        };
        if exponent < 0 {
            return Err(self.err(
                "NotImplementedError",
                String::from(
                    "a negative exponent produces a float; floating point is \
                     unavailable because the pagh kernel disables the U-mode FPU",
                ),
            ));
        }
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

    fn compare_op(&self, op: CmpOp, left: &Value, right: &Value) -> PyResult<bool> {
        match op {
            CmpOp::Eq => Ok(values_equal(left, right)),
            CmpOp::NotEq => Ok(!values_equal(left, right)),
            CmpOp::Is => Ok(is_same(left, right)),
            CmpOp::IsNot => Ok(!is_same(left, right)),
            CmpOp::Lt => Ok(self.cmp_values(left, right)? == Ordering::Less),
            CmpOp::Le => Ok(self.cmp_values(left, right)? != Ordering::Greater),
            CmpOp::Gt => Ok(self.cmp_values(left, right)? == Ordering::Greater),
            CmpOp::Ge => Ok(self.cmp_values(left, right)? != Ordering::Less),
            CmpOp::In => self.contains_value(right, left),
            CmpOp::NotIn => Ok(!self.contains_value(right, left)?),
        }
    }

    /// Ordering used by comparisons, `min`, `max`, `sorted` and `list.sort`.
    /// Only int/bool/str and list/tuple of those are orderable; everything else
    /// is a Python `TypeError`, not an internal panic.
    pub fn cmp_values(&self, left: &Value, right: &Value) -> PyResult<Ordering> {
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            return Ok(a.cmp(&b));
        }
        match (left, right) {
            (Value::Str(a), Value::Str(b)) => Ok(a.cmp(b)),
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

    fn index_value(&mut self, target: &Value, index: &Value) -> PyResult<Value> {
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
                if let Some(id) = string_method(name) {
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
            Value::Iterator(_) => Ok(value),
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
        if module == "sys" {
            return self.sys_module();
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
            Value::str("3.4.0 (pagh-python, integer-only)"),
        ));
        attrs.push((Rc::from("platform"), Value::str("pagh-riscv")));
        attrs.push((Rc::from("maxsize"), Value::Int(i64::MAX)));
        let module = Value::Module(Rc::new(ModuleValue {
            name: Rc::from("sys"),
            attrs: RefCell::new(attrs),
        }));
        self.module_cache
            .push((String::from("sys"), module.clone()));
        Ok(module)
    }

    // ── tracebacks ─────────────────────────────────────────────────────────

    pub fn set_line(&mut self, line: u32) {
        if let Some(frame) = self.call_stack.last_mut() {
            frame.line = line;
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
