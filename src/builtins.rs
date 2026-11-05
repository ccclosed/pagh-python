//! Builtin functions and methods.
//!
//! Everything here dispatches on [`BuiltinId`]; the interpreter owns the state
//! (`Interp`) and exposes the helpers this module is allowed to reuse:
//! `call_value`, `binary_op`, `cmp_values`, `make_iterator`,
//! `next_from_iterator`, `collect_iterable`, `type_of`, `construct_*`.
//!
//! The implementation is intentionally literal: Python semantics for the
//! supported subset, and a short `TypeError`/`ValueError` for everything else.
//! No builtin mutates interpreter state that is not part of its result.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::cmp::Ordering;

use crate::ast::BinOp;
use crate::error::PyResult;
use crate::interp::{ArgList, Interp};
use crate::platform;
use crate::value::{
    self, dict_find, dict_get, dict_set, hex_text, is_same, range_length, values_equal, BuiltinId,
    BuiltinType, BuiltinValue, FileValue, IterState, RangeValue, Value,
};

// ── dispatch ───────────────────────────────────────────────────────────────

pub fn call(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    match builtin.id {
        BuiltinId::Print => builtin_print(interp, args, kwargs),
        BuiltinId::Len => builtin_len(interp, args, kwargs),
        BuiltinId::Range => builtin_range(interp, args, kwargs),
        BuiltinId::Int => construct_tag(interp, BuiltinType::Int, args, kwargs, "int"),
        BuiltinId::Float => builtin_float(interp, args, kwargs),
        BuiltinId::Str => builtin_str(interp, args, kwargs),
        BuiltinId::Abs => builtin_abs(interp, args, kwargs),
        BuiltinId::Min => builtin_minmax(interp, args, kwargs, false),
        BuiltinId::Max => builtin_minmax(interp, args, kwargs, true),
        BuiltinId::Sum => builtin_sum(interp, args, kwargs),
        BuiltinId::Sorted => builtin_sorted(interp, args, kwargs),
        BuiltinId::Enumerate => builtin_enumerate(interp, args, kwargs),
        BuiltinId::Zip => builtin_zip(interp, args, kwargs),
        BuiltinId::List => construct_tag(interp, BuiltinType::List, args, kwargs, "list"),
        BuiltinId::Dict => construct_tag(interp, BuiltinType::Dict, args, kwargs, "dict"),
        BuiltinId::Tuple => construct_tag(interp, BuiltinType::Tuple, args, kwargs, "tuple"),
        BuiltinId::Type => construct_tag(interp, BuiltinType::Type, args, kwargs, "type"),
        BuiltinId::IsInstance => builtin_isinstance(interp, args, kwargs),
        BuiltinId::Open => builtin_open(interp, args, kwargs),
        BuiltinId::Chr => builtin_chr(interp, args, kwargs),
        BuiltinId::Ord => builtin_ord(interp, args, kwargs),
        BuiltinId::Repr => builtin_repr(interp, args, kwargs),
        BuiltinId::Bool => construct_tag(interp, BuiltinType::Bool, args, kwargs, "bool"),
        BuiltinId::Iter => builtin_iter(interp, args, kwargs),
        BuiltinId::Next => builtin_next(interp, args, kwargs),
        BuiltinId::Any => builtin_any_all(interp, args, kwargs, false),
        BuiltinId::All => builtin_any_all(interp, args, kwargs, true),
        BuiltinId::Divmod => builtin_divmod(interp, args, kwargs),
        BuiltinId::Pow => builtin_pow(interp, args, kwargs),
        BuiltinId::Round => builtin_round(interp, args, kwargs),
        BuiltinId::MathSqrt => math_sqrt(interp, args, kwargs),
        BuiltinId::MathFloor => math_floor(interp, args, kwargs),
        BuiltinId::MathCeil => math_ceil(interp, args, kwargs),
        BuiltinId::MathTrunc => math_trunc(interp, args, kwargs),
        BuiltinId::MathFabs => math_fabs(interp, args, kwargs),
        BuiltinId::MathPow => math_pow(interp, args, kwargs),
        BuiltinId::MathFmod => math_fmod(interp, args, kwargs),
        BuiltinId::MathSin => math_sin(interp, args, kwargs),
        BuiltinId::MathCos => math_cos(interp, args, kwargs),
        BuiltinId::MathTan => math_tan(interp, args, kwargs),
        BuiltinId::MathLog => math_log(interp, args, kwargs),
        BuiltinId::MathLog10 => math_log10(interp, args, kwargs),
        BuiltinId::MathExp => math_exp(interp, args, kwargs),
        BuiltinId::MathIsnan => math_isnan(interp, args, kwargs),
        BuiltinId::MathIsinf => math_isinf(interp, args, kwargs),
        BuiltinId::Hex => builtin_hex(interp, args, kwargs),
        BuiltinId::Id => builtin_id(interp, args, kwargs),
        _ => call_method(interp, builtin, args, kwargs),
    }
}

// ── shared argument helpers ────────────────────────────────────────────────

fn unexpected(interp: &Interp, function: &str, name: &str) -> crate::error::PyError {
    interp.err(
        "TypeError",
        format!(
            "{}() got an unexpected keyword argument '{}'",
            function, name
        ),
    )
}

fn no_kwargs(interp: &Interp, kwargs: &[(Rc<str>, Value)], function: &str) -> PyResult<()> {
    if let Some((name, _)) = kwargs.first() {
        return Err(unexpected(interp, function, name));
    }
    Ok(())
}

fn check_kwargs(
    interp: &Interp,
    kwargs: &[(Rc<str>, Value)],
    function: &str,
    allowed: &[&str],
) -> PyResult<()> {
    for (name, _) in kwargs {
        if !allowed.iter().any(|candidate| *candidate == &**name) {
            return Err(unexpected(interp, function, name));
        }
    }
    Ok(())
}

fn keyword<'a>(kwargs: &'a [(Rc<str>, Value)], name: &str) -> Option<&'a Value> {
    kwargs
        .iter()
        .find(|(key, _)| &**key == name)
        .map(|(_, value)| value)
}

fn argument<'a>(
    interp: &Interp,
    args: &'a ArgList,
    index: usize,
    function: &str,
) -> PyResult<&'a Value> {
    match args.get(index) {
        Some(value) => Ok(value),
        None => Err(interp.err("TypeError", format!("{}() takes more arguments", function))),
    }
}

fn expect_args(
    interp: &Interp,
    args: &ArgList,
    function: &str,
    min: usize,
    max: usize,
) -> PyResult<()> {
    let given = args.len();
    if given < min || given > max {
        let expected = if min == max {
            format!(
                "takes exactly {} argument{} ({} given)",
                min,
                if min == 1 { "" } else { "s" },
                given
            )
        } else if given < min {
            format!(
                "takes at least {} argument{} ({} given)",
                min,
                if min == 1 { "" } else { "s" },
                given
            )
        } else {
            format!(
                "takes at most {} argument{} ({} given)",
                max,
                if max == 1 { "" } else { "s" },
                given
            )
        };
        return Err(interp.err("TypeError", format!("{}() {}", function, expected)));
    }
    Ok(())
}

fn as_int(interp: &Interp, value: &Value, what: &str) -> PyResult<i64> {
    match value.as_int() {
        Some(number) => Ok(number),
        None => Err(interp.err(
            "TypeError",
            format!("{} must be an integer, not '{}'", what, value.type_name()),
        )),
    }
}

fn as_str<'a>(interp: &Interp, value: &'a Value, what: &str) -> PyResult<&'a str> {
    match value {
        Value::Str(text) => Ok(text),
        other => Err(interp.err(
            "TypeError",
            format!("{} must be str, not '{}'", what, other.type_name()),
        )),
    }
}

fn keyword_str(
    interp: &Interp,
    kwargs: &[(Rc<str>, Value)],
    name: &str,
    default: &str,
    function: &str,
) -> PyResult<String> {
    match keyword(kwargs, name) {
        None => Ok(String::from(default)),
        Some(Value::None) => Ok(String::from(default)),
        Some(Value::Str(text)) => Ok(String::from(&**text)),
        Some(other) => Err(interp.err(
            "TypeError",
            format!(
                "{}() argument '{}' must be str or None, not '{}'",
                function,
                name,
                other.type_name()
            ),
        )),
    }
}

// ── __str__ / __repr__ / __len__ hooks ─────────────────────────────────────

fn call_dunder(interp: &mut Interp, value: &Value, name: &str) -> PyResult<Option<Value>> {
    if !matches!(value, Value::Instance(_)) {
        return Ok(None);
    }
    match interp.get_attribute(value, name) {
        Ok(method) => Ok(Some(interp.call_value(
            method,
            ArgList::new(),
            Vec::new(),
        )?)),
        Err(error) if error.type_name() == "AttributeError" => Ok(None),
        Err(error) => Err(error),
    }
}

fn dunder_string(interp: &Interp, result: Value, name: &str) -> PyResult<String> {
    match result {
        Value::Str(text) => Ok(String::from(&*text)),
        other => Err(interp.err(
            "TypeError",
            format!(
                "{}() returned non-string (type '{}')",
                name,
                other.type_name()
            ),
        )),
    }
}

pub(crate) fn str_value(interp: &mut Interp, value: &Value) -> PyResult<String> {
    if let Some(result) = call_dunder(interp, value, "__str__")? {
        return dunder_string(interp, result, "__str__");
    }
    if let Some(result) = call_dunder(interp, value, "__repr__")? {
        return dunder_string(interp, result, "__repr__");
    }
    Ok(value.to_str())
}

fn repr_value(interp: &mut Interp, value: &Value) -> PyResult<String> {
    if let Some(result) = call_dunder(interp, value, "__repr__")? {
        return dunder_string(interp, result, "__repr__");
    }
    Ok(value.to_repr())
}

// ── free functions ─────────────────────────────────────────────────────────

fn builtin_print(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    check_kwargs(interp, kwargs, "print", &["sep", "end", "file"])?;
    let sep = keyword_str(interp, kwargs, "sep", " ", "print")?;
    let end = keyword_str(interp, kwargs, "end", "\n", "print")?;
    let file = match keyword(kwargs, "file") {
        None | Some(Value::None) => None,
        Some(Value::File(file)) => Some(file.clone()),
        Some(other) => {
            return Err(interp.err(
                "TypeError",
                format!(
                    "print() file must be a file object or None, not '{}'",
                    other.type_name()
                ),
            ))
        }
    };
    let mut out = String::new();
    for index in 0..args.len() {
        if index > 0 {
            out.push_str(&sep);
        }
        let value = argument(interp, args, index, "print")?;
        out.push_str(&str_value(interp, value)?);
    }
    out.push_str(&end);
    match file {
        Some(file) => {
            let mut borrowed = file.borrow_mut();
            if let Err(message) = borrowed.write(&out) {
                drop(borrowed);
                return Err(file_error(interp, message));
            }
        }
        None => platform::write_stdout(&out),
    }
    Ok(Value::None)
}

fn builtin_len(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "len")?;
    expect_args(interp, args, "len", 1, 1)?;
    let value = argument(interp, args, 0, "len")?;
    let length = match value {
        Value::Str(text) => text.chars().count() as i64,
        Value::List(items) => items.borrow().len() as i64,
        Value::Tuple(items) => items.len() as i64,
        Value::Dict(entries) => entries.borrow().len() as i64,
        Value::Range(range) => range.len,
        Value::Instance(_) => match call_dunder(interp, value, "__len__")? {
            Some(result) => match result.as_int() {
                Some(number) if number >= 0 => number,
                Some(_) => {
                    return Err(
                        interp.err("ValueError", String::from("__len__() should return >= 0"))
                    )
                }
                None => {
                    return Err(interp.err(
                        "TypeError",
                        format!(
                            "'{}' object cannot be interpreted as an integer",
                            result.type_name()
                        ),
                    ))
                }
            },
            None => {
                return Err(interp.err(
                    "TypeError",
                    format!("object of type '{}' has no len()", value.type_name()),
                ))
            }
        },
        other => {
            return Err(interp.err(
                "TypeError",
                format!("object of type '{}' has no len()", other.type_name()),
            ))
        }
    };
    Ok(Value::Int(length))
}

fn builtin_range(interp: &Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "range")?;
    expect_args(interp, args, "range", 1, 3)?;
    let mut numbers = Vec::with_capacity(args.len());
    for index in 0..args.len() {
        let value = argument(interp, args, index, "range")?;
        numbers.push(as_int(interp, value, "range() argument")?);
    }
    let (start, stop, step) = match numbers.len() {
        1 => (0, numbers[0], 1),
        2 => (numbers[0], numbers[1], 1),
        _ => (numbers[0], numbers[1], numbers[2]),
    };
    if step == 0 {
        return Err(interp.err("ValueError", String::from("range() arg 3 must not be zero")));
    }
    match range_length(start, stop, step) {
        Some(len) => Ok(Value::Range(Rc::new(RangeValue {
            start,
            stop,
            step,
            len,
        }))),
        None => Err(interp.err(
            "OverflowError",
            String::from("range() result has too many items"),
        )),
    }
}

fn builtin_float(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    // `float` is exposed as a class, so this path is only a defensive fallback
    // for a BuiltinId::Float value created elsewhere.
    interp.construct_float(args, kwargs)
}

fn builtin_str(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "str")?;
    expect_args(interp, args, "str", 0, 1)?;
    match args.get(0) {
        Some(value) => Ok(Value::Str(Rc::from(str_value(interp, value)?))),
        None => Ok(Value::str("")),
    }
}

fn builtin_abs(interp: &Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "abs")?;
    expect_args(interp, args, "abs", 1, 1)?;
    let value = argument(interp, args, 0, "abs")?;
    if let Value::Float(number) = value {
        return Ok(Value::Float(crate::float::abs(*number)));
    }
    match value.as_int() {
        Some(number) => match number.checked_abs() {
            Some(result) => Ok(Value::Int(result)),
            None => Err(interp.err(
                "OverflowError",
                String::from("abs() result is too large for a 64-bit integer"),
            )),
        },
        None => Err(interp.err(
            "TypeError",
            format!("bad operand type for abs(): '{}'", value.type_name()),
        )),
    }
}

fn minmax_key(interp: &mut Interp, key: &Option<Value>, value: &Value) -> PyResult<Value> {
    match key {
        Some(function) => {
            let mut args = ArgList::new();
            args.push(value.clone());
            interp.call_value(function.clone(), args, Vec::new())
        }
        None => Ok(value.clone()),
    }
}

fn builtin_minmax(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    want_max: bool,
) -> PyResult<Value> {
    let name = if want_max { "max" } else { "min" };
    check_kwargs(interp, kwargs, name, &["key"])?;
    let key = match keyword(kwargs, "key") {
        None | Some(Value::None) => None,
        Some(value) => Some(value.clone()),
    };
    if args.len() == 0 {
        return Err(interp.err(
            "TypeError",
            format!("{}() expected at least 1 argument, got 0", name),
        ));
    }
    let candidates = if args.len() == 1 {
        let iterable = argument(interp, args, 0, name)?.clone();
        interp.collect_iterable(iterable)?
    } else {
        args.to_vec()
    };
    if candidates.is_empty() {
        return Err(interp.err("ValueError", format!("{}() arg is an empty sequence", name)));
    }
    let mut best = candidates[0].clone();
    let mut best_key = minmax_key(interp, &key, &best)?;
    for candidate in candidates.iter().skip(1) {
        let candidate_key = minmax_key(interp, &key, candidate)?;
        let ordering = interp.cmp_values(&candidate_key, &best_key)?;
        let better = if want_max {
            ordering == Ordering::Greater
        } else {
            ordering == Ordering::Less
        };
        if better {
            best = candidate.clone();
            best_key = candidate_key;
        }
    }
    Ok(best)
}

fn builtin_sum(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    check_kwargs(interp, kwargs, "sum", &["start"])?;
    expect_args(interp, args, "sum", 1, 2)?;
    let mut start = match keyword(kwargs, "start") {
        Some(value) => value.clone(),
        None => Value::Int(0),
    };
    if args.len() == 2 {
        if keyword(kwargs, "start").is_some() {
            return Err(interp.err(
                "TypeError",
                String::from("sum() got multiple values for argument 'start'"),
            ));
        }
        start = argument(interp, args, 1, "sum")?.clone();
    }
    let iterable = argument(interp, args, 0, "sum")?.clone();
    let items = interp.collect_iterable(iterable)?;
    let mut total = start;
    for item in items {
        total = interp.binary_op(BinOp::Add, total, item)?;
    }
    Ok(total)
}

fn sorted_values(
    interp: &mut Interp,
    items: Vec<Value>,
    key: Option<Value>,
    reverse: bool,
) -> PyResult<Vec<Value>> {
    let mut decorated: Vec<(Value, Value)> = Vec::with_capacity(items.len());
    for item in items {
        let sort_key = match &key {
            Some(function) => {
                let mut args = ArgList::new();
                args.push(item.clone());
                interp.call_value(function.clone(), args, Vec::new())?
            }
            None => item.clone(),
        };
        decorated.push((sort_key, item));
    }
    // Insertion sort: stable, fallible (a bad comparison becomes a Python
    // TypeError) and perfectly adequate for the small scripts this ABI runs.
    let mut index = 1;
    while index < decorated.len() {
        let mut position = index;
        while position > 0 {
            let ordering = interp.cmp_values(&decorated[position - 1].0, &decorated[position].0)?;
            let move_left = if reverse {
                ordering == Ordering::Less
            } else {
                ordering == Ordering::Greater
            };
            if !move_left {
                break;
            }
            decorated.swap(position - 1, position);
            position -= 1;
        }
        index += 1;
    }
    Ok(decorated.into_iter().map(|(_, item)| item).collect())
}

fn builtin_sorted(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    check_kwargs(interp, kwargs, "sorted", &["key", "reverse"])?;
    expect_args(interp, args, "sorted", 1, 1)?;
    let key = match keyword(kwargs, "key") {
        None | Some(Value::None) => None,
        Some(value) => Some(value.clone()),
    };
    let reverse = keyword(kwargs, "reverse")
        .map(Value::truthy)
        .unwrap_or(false);
    let iterable = argument(interp, args, 0, "sorted")?.clone();
    let items = interp.collect_iterable(iterable)?;
    Ok(Value::list(sorted_values(interp, items, key, reverse)?))
}

fn builtin_enumerate(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    check_kwargs(interp, kwargs, "enumerate", &["start"])?;
    expect_args(interp, args, "enumerate", 1, 2)?;
    let mut start = match keyword(kwargs, "start") {
        Some(value) => as_int(interp, value, "enumerate() start")?,
        None => 0,
    };
    if args.len() == 2 {
        if keyword(kwargs, "start").is_some() {
            return Err(interp.err(
                "TypeError",
                String::from("enumerate() got multiple values for argument 'start'"),
            ));
        }
        start = as_int(
            interp,
            argument(interp, args, 1, "enumerate")?,
            "enumerate() start",
        )?;
    }
    let iterable = argument(interp, args, 0, "enumerate")?.clone();
    let inner = interp.make_iterator(iterable)?;
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IterState::Enumerate {
            inner,
            index: start,
        },
    ))))
}

fn builtin_zip(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "zip")?;
    let mut inners = Vec::with_capacity(args.len());
    for index in 0..args.len() {
        inners.push(interp.make_iterator(argument(interp, args, index, "zip")?.clone())?);
    }
    let done = inners.is_empty();
    Ok(Value::Iterator(Rc::new(RefCell::new(IterState::Zip {
        inners,
        done,
    }))))
}

fn builtin_isinstance(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "isinstance")?;
    expect_args(interp, args, "isinstance", 2, 2)?;
    let value = argument(interp, args, 0, "isinstance")?;
    let class_value = argument(interp, args, 1, "isinstance")?.clone();
    Ok(Value::Bool(isinstance_of(interp, value, &class_value)?))
}

fn isinstance_of(interp: &Interp, value: &Value, class_value: &Value) -> PyResult<bool> {
    match class_value {
        Value::Class(class) => Ok(value::instance_of_class(value, class)),
        Value::Tuple(items) => {
            for item in items.iter() {
                if isinstance_of(interp, value, item)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        other => Err(interp.err(
            "TypeError",
            format!(
                "isinstance() arg 2 must be a type or tuple of types, not '{}'",
                other.type_name()
            ),
        )),
    }
}

fn builtin_open(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    check_kwargs(interp, kwargs, "open", &["mode", "buffering", "encoding"])?;
    expect_args(interp, args, "open", 1, 4)?;
    let path = match argument(interp, args, 0, "open")? {
        Value::Str(text) => String::from(&**text),
        other => {
            return Err(interp.err(
                "TypeError",
                format!(
                    "open() file must be str, not '{}' (file descriptors are not supported)",
                    other.type_name()
                ),
            ))
        }
    };
    let mut mode = match keyword(kwargs, "mode") {
        Some(Value::Str(text)) => String::from(&**text),
        Some(Value::None) => String::from("r"),
        Some(other) => {
            return Err(interp.err(
                "TypeError",
                format!("open() mode must be str, not '{}'", other.type_name()),
            ))
        }
        None => String::from("r"),
    };
    if args.len() >= 2 {
        if keyword(kwargs, "mode").is_some() {
            return Err(interp.err(
                "TypeError",
                String::from("open() got multiple values for argument 'mode'"),
            ));
        }
        mode = String::from(as_str(
            interp,
            argument(interp, args, 1, "open")?,
            "open() mode",
        )?);
    }
    for (index, name) in [(2usize, "buffering"), (3, "encoding")] {
        if args.len() > index && keyword(kwargs, name).is_some() {
            return Err(interp.err(
                "TypeError",
                format!("open() got multiple values for argument '{}'", name),
            ));
        }
    }
    if let Some(value) = kwargs
        .iter()
        .find(|(name, _)| &**name == "buffering")
        .map(|(_, value)| value)
        .or_else(|| args.get(2))
    {
        as_int(interp, value, "open() buffering")?;
    }
    let encoding = match args.get(3) {
        Some(value) => Some(value),
        None => keyword(kwargs, "encoding"),
    };
    if let Some(value) = encoding {
        match value {
            Value::None => {}
            Value::Str(text)
                if &**text == "utf-8"
                    || &**text == "utf8"
                    || &**text == "UTF-8"
                    || &**text == "UTF8" => {}
            Value::Str(text) => {
                return Err(interp.err(
                    "NotImplementedError",
                    format!("open() encoding '{}' is not supported; only UTF-8", text),
                ))
            }
            other => {
                return Err(interp.err(
                    "TypeError",
                    format!("open() encoding must be str, not '{}'", other.type_name()),
                ))
            }
        }
    }

    let mut readable = false;
    let mut writable = false;
    let mut append = false;
    let mut truncate = false;
    let mut bases = 0;
    for ch in mode.chars() {
        match ch {
            'r' => {
                readable = true;
                bases += 1;
            }
            'w' => {
                writable = true;
                truncate = true;
                bases += 1;
            }
            'a' => {
                writable = true;
                append = true;
                bases += 1;
            }
            '+' => {
                readable = true;
                writable = true;
            }
            't' => {}
            'b' => {
                return Err(interp.err(
                    "ValueError",
                    String::from(
                        "binary mode is not supported: the pagh ABI has text-only file I/O",
                    ),
                ))
            }
            'x' => {
                return Err(interp.err(
                    "NotImplementedError",
                    String::from("exclusive creation mode 'x' is not supported"),
                ))
            }
            other => return Err(interp.err("ValueError", format!("invalid mode: '{}'", other))),
        }
    }
    if bases != 1 {
        return Err(interp.err("ValueError", format!("invalid mode: '{}'", mode)));
    }

    let existing = if truncate {
        String::new()
    } else if append {
        if platform::path_exists(&path) {
            match platform::read_text(&path) {
                Ok(text) => text,
                Err(message) => {
                    return Err(
                        interp.err("OSError", format!("cannot read '{}': {}", path, message))
                    )
                }
            }
        } else {
            String::new()
        }
    } else {
        match platform::read_text(&path) {
            Ok(text) => text,
            Err(message) => {
                return Err(interp.err(
                    "FileNotFoundError",
                    format!("cannot open '{}': {}", path, message),
                ))
            }
        }
    };
    let pos = if append { existing.len() } else { 0 };
    let dirty = truncate || (append && !platform::path_exists(&path));
    Ok(Value::File(Rc::new(RefCell::new(FileValue {
        name: Rc::from(path.as_str()),
        mode: Rc::from(mode.as_str()),
        data: existing,
        pos,
        readable,
        writable,
        closed: false,
        dirty,
    }))))
}

fn builtin_chr(interp: &Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "chr")?;
    expect_args(interp, args, "chr", 1, 1)?;
    let number = as_int(interp, argument(interp, args, 0, "chr")?, "chr() argument")?;
    if !(0..=0x10ffff).contains(&number) {
        return Err(interp.err(
            "ValueError",
            String::from("chr() arg not in range(0x110000)"),
        ));
    }
    match char::from_u32(number as u32) {
        Some(ch) => {
            let mut buffer = [0u8; 4];
            Ok(Value::Str(Rc::from(&*ch.encode_utf8(&mut buffer))))
        }
        None => Err(interp.err(
            "ValueError",
            String::from("chr() arg is a surrogate code point"),
        )),
    }
}

fn builtin_ord(interp: &Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "ord")?;
    expect_args(interp, args, "ord", 1, 1)?;
    match argument(interp, args, 0, "ord")? {
        Value::Str(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => Ok(Value::Int(ch as i64)),
                _ => Err(interp.err(
                    "TypeError",
                    format!(
                        "ord() expected a character, but string of length {} found",
                        text.chars().count()
                    ),
                )),
            }
        }
        other => Err(interp.err(
            "TypeError",
            format!(
                "ord() expected string of length 1, but '{}' found",
                other.type_name()
            ),
        )),
    }
}

fn builtin_repr(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "repr")?;
    expect_args(interp, args, "repr", 1, 1)?;
    let value = argument(interp, args, 0, "repr")?;
    Ok(Value::Str(Rc::from(repr_value(interp, value)?)))
}

fn builtin_iter(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "iter")?;
    expect_args(interp, args, "iter", 1, 2)?;
    if args.len() == 2 {
        return Err(interp.err(
            "NotImplementedError",
            String::from(
                "iter(callable, sentinel) is not supported; use a while loop with a \
                 break on the sentinel",
            ),
        ));
    }
    let value = argument(interp, args, 0, "iter")?.clone();
    interp.make_iterator(value)
}

fn builtin_next(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "next")?;
    expect_args(interp, args, "next", 1, 2)?;
    let iterator = argument(interp, args, 0, "next")?;
    match interp.next_from_iterator(iterator)? {
        Some(value) => Ok(value),
        None => match args.get(1) {
            Some(default) => Ok(default.clone()),
            None => Err(interp.err("StopIteration", String::new())),
        },
    }
}

fn builtin_any_all(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    want_all: bool,
) -> PyResult<Value> {
    let name = if want_all { "all" } else { "any" };
    no_kwargs(interp, kwargs, name)?;
    expect_args(interp, args, name, 1, 1)?;
    let iterable = argument(interp, args, 0, name)?.clone();
    let iterator = interp.make_iterator(iterable)?;
    while let Some(value) = interp.next_from_iterator(&iterator)? {
        if want_all {
            if !value.truthy() {
                return Ok(Value::Bool(false));
            }
        } else if value.truthy() {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(want_all))
}

fn builtin_divmod(interp: &Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "divmod")?;
    expect_args(interp, args, "divmod", 2, 2)?;
    let left = argument(interp, args, 0, "divmod")?.clone();
    let right = argument(interp, args, 1, "divmod")?.clone();
    let quotient = interp.binary_op(BinOp::FloorDiv, left.clone(), right.clone())?;
    let remainder = interp.binary_op(BinOp::Mod, left, right)?;
    Ok(Value::tuple(alloc::vec![quotient, remainder]))
}

fn builtin_pow(interp: &Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "pow")?;
    expect_args(interp, args, "pow", 2, 3)?;
    let base_value = argument(interp, args, 0, "pow")?.clone();
    let exponent_value = argument(interp, args, 1, "pow")?.clone();
    if args.len() == 2 {
        return interp.binary_op(BinOp::Pow, base_value, exponent_value);
    }
    let base = as_int(interp, &base_value, "pow() argument")?;
    let exponent = as_int(interp, &exponent_value, "pow() argument")?;
    let modulus = as_int(interp, argument(interp, args, 2, "pow")?, "pow() argument")?;
    if modulus == 0 {
        return Err(interp.err("ValueError", String::from("pow() 3rd argument cannot be 0")));
    }
    if exponent < 0 {
        return Err(interp.err(
            "NotImplementedError",
            String::from(
                "pow(base, exponent, modulus) with a negative exponent needs a modular \
                 inverse, which is not implemented",
            ),
        ));
    }
    Ok(Value::Int(pow_mod(base, exponent, modulus)))
}

fn pow_mod(base: i64, mut exponent: i64, modulus: i64) -> i64 {
    let magnitude = (modulus as i128).abs();
    if magnitude == 1 {
        return 0;
    }
    let mut result: i128 = 1;
    let mut factor = (base as i128).rem_euclid(magnitude);
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = (result * factor) % magnitude;
        }
        factor = (factor * factor) % magnitude;
        exponent >>= 1;
    }
    if modulus < 0 && result != 0 {
        (result - magnitude) as i64
    } else {
        result as i64
    }
}

// ── float builtins ─────────────────────────────────────────────────────────

fn as_float(interp: &Interp, value: &Value, what: &str) -> PyResult<f64> {
    match value.as_float() {
        Some(number) => Ok(number),
        None => Err(interp.err(
            "TypeError",
            format!("{} must be a real number, not '{}'", what, value.type_name()),
        )),
    }
}

fn builtin_round(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    check_kwargs(interp, kwargs, "round", &["ndigits"])?;
    expect_args(interp, args, "round", 1, 2)?;
    let value = argument(interp, args, 0, "round")?;
    let digits = match (args.get(1), keyword(kwargs, "ndigits")) {
        (Some(_), Some(_)) => {
            return Err(interp.err(
                "TypeError",
                String::from("round() got multiple values for argument 'ndigits'"),
            ))
        }
        (Some(value), None) => Some(as_int(interp, value, "round() ndigits")?),
        (None, Some(value)) => Some(as_int(interp, value, "round() ndigits")?),
        (None, None) => None,
    };
    match value {
        Value::Float(number) => {
            let number = *number;
            match digits {
                None => {
                    if number.is_nan() {
                        return Err(interp.err(
                            "ValueError",
                            String::from("cannot convert float NaN to integer"),
                        ));
                    }
                    if number.is_infinite() {
                        return Err(interp.err(
                            "OverflowError",
                            String::from("cannot convert float infinity to integer"),
                        ));
                    }
                    match crate::float::round_to_i64(number) {
                        Some(result) => Ok(Value::Int(result)),
                        None => Err(interp.err(
                            "OverflowError",
                            String::from("rounded value is too large for a 64-bit integer"),
                        )),
                    }
                }
                Some(digits) => Ok(Value::Float(round_float_digits(number, digits))),
            }
        }
        other => match other.as_int() {
            Some(number) => match digits {
                None => Ok(Value::Int(number)),
                Some(digits) if digits >= 0 => Ok(Value::Int(number)),
                Some(digits) => round_int_digits(interp, number, digits),
            },
            None => Err(interp.err(
                "TypeError",
                format!(
                    "type '{}' doesn't define __round__ method",
                    other.type_name()
                ),
            )),
        },
    }
}

/// `round(integer, negative_ndigits)`: half-even on the decimal scale, using
/// i128 so `round(15, -1)` is 20 and `round(25, -1)` is 20, not a panic.
fn round_int_digits(interp: &Interp, value: i64, digits: i64) -> PyResult<Value> {
    let places = digits.unsigned_abs();
    if places > 38 {
        return Ok(Value::Int(0));
    }
    let factor = match 10i128.checked_pow(places as u32) {
        Some(factor) => factor,
        None => return Ok(Value::Int(0)),
    };
    let value = value as i128;
    let quotient = value / factor;
    let remainder = value % factor;
    let mut rounded = quotient;
    let doubled = remainder.abs() * 2;
    if doubled > factor || (doubled == factor && quotient & 1 != 0) {
        if remainder > 0 {
            rounded += 1;
        } else if remainder < 0 {
            rounded -= 1;
        }
    }
    match i64::try_from(rounded * factor) {
        Ok(result) => Ok(Value::Int(result)),
        Err(_) => Err(interp.err(
            "OverflowError",
            String::from("rounded value is too large for a 64-bit integer"),
        )),
    }
}

fn pow10(exponent: u32) -> f64 {
    let mut factor = 1.0;
    for _ in 0..exponent {
        factor *= 10.0;
    }
    factor
}

/// `round(float, ndigits)`, which returns a float.
///
/// `core::fmt` already performs correctly rounded decimal conversion and
/// ties-to-even, so positive `ndigits` formats the value with that many
/// fractional digits and parses it back. That reproduces CPython's visible
/// results (`round(2.675, 2) == 2.67`, `round(0.125, 2) == 0.12`) where a
/// naive `value * 10^n` scale would round 2.675 up. Negative `ndigits` scales
/// by the power of ten first and then formats with zero fractional digits.
fn round_float_digits(value: f64, digits: i64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    if digits >= 309 {
        return value;
    }
    if digits <= -309 {
        return crate::float::copysign(0.0, value);
    }
    if digits >= 0 {
        let text = format!("{:.*}", digits as usize, value);
        return text.parse::<f64>().unwrap_or(value);
    }
    let factor = pow10(digits.unsigned_abs() as u32);
    let scaled = value / factor;
    if !scaled.is_finite() {
        return crate::float::copysign(0.0, value);
    }
    format!("{:.0}", scaled).parse::<f64>().unwrap_or(0.0) * factor
}

// ── math module ────────────────────────────────────────────────────────────

fn math_float_argument(
    interp: &Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    name: &str,
) -> PyResult<f64> {
    no_kwargs(interp, kwargs, name)?;
    expect_args(interp, args, name, 1, 1)?;
    as_float(interp, argument(interp, args, 0, name)?, "math argument")
}

fn math_sqrt(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    let value = math_float_argument(interp, args, kwargs, "sqrt")?;
    if value < 0.0 {
        return Err(interp.err("ValueError", String::from("math domain error")));
    }
    Ok(Value::Float(crate::float::sqrt(value)))
}

fn math_floor(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    math_integer_result(interp, args, kwargs, "floor", crate::float::floor_to_i64)
}

fn math_ceil(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    math_integer_result(interp, args, kwargs, "ceil", crate::float::ceil_to_i64)
}

fn math_trunc(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    math_integer_result(interp, args, kwargs, "trunc", crate::float::to_i64)
}

/// `math.floor`/`ceil`/`trunc` return Python ints. An i64 argument is
/// returned unchanged, which also avoids rounding it through f64.
fn math_integer_result(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    name: &str,
    convert: fn(f64) -> Option<i64>,
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, name)?;
    expect_args(interp, args, name, 1, 1)?;
    let value = argument(interp, args, 0, name)?;
    if let Some(integer) = value.as_int() {
        return Ok(Value::Int(integer));
    }
    let number = as_float(interp, value, "math argument")?;
    if number.is_nan() {
        return Err(interp.err(
            "ValueError",
            String::from("cannot convert float NaN to integer"),
        ));
    }
    match convert(number) {
        Some(result) => Ok(Value::Int(result)),
        None if number.is_infinite() => Err(interp.err(
            "OverflowError",
            String::from("cannot convert float infinity to integer"),
        )),
        None => Err(interp.err(
            "OverflowError",
            String::from("int too large to convert to a 64-bit integer"),
        )),
    }
}

fn math_fabs(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    let value = math_float_argument(interp, args, kwargs, "fabs")?;
    Ok(Value::Float(crate::float::abs(value)))
}

fn math_pow(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "pow")?;
    expect_args(interp, args, "pow", 2, 2)?;
    let base = as_float(interp, argument(interp, args, 0, "pow")?, "math argument")?;
    let exponent = as_float(interp, argument(interp, args, 1, "pow")?, "math argument")?;
    if base == 0.0 && exponent < 0.0 {
        return Err(interp.err("ValueError", String::from("math domain error")));
    }
    let result = crate::float::pow(base, exponent);
    if result.is_nan() && base < 0.0 && !exponent.is_nan() {
        return Err(interp.err("ValueError", String::from("math domain error")));
    }
    Ok(Value::Float(result))
}

fn math_fmod(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "fmod")?;
    expect_args(interp, args, "fmod", 2, 2)?;
    let left = as_float(interp, argument(interp, args, 0, "fmod")?, "math argument")?;
    let right = as_float(interp, argument(interp, args, 1, "fmod")?, "math argument")?;
    if right == 0.0 || left.is_infinite() {
        return Err(interp.err("ValueError", String::from("math domain error")));
    }
    Ok(Value::Float(crate::float::fmod(left, right)))
}

/// `math.sin`/`cos`/`tan`: NaN propagates, an infinite argument is a domain
/// error (CPython raises `ValueError` for the latter).
fn math_trig(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    name: &str,
    function: fn(f64) -> f64,
) -> PyResult<Value> {
    let value = math_float_argument(interp, args, kwargs, name)?;
    if value.is_nan() {
        return Ok(Value::Float(f64::NAN));
    }
    if !value.is_finite() {
        return Err(interp.err("ValueError", String::from("math domain error")));
    }
    Ok(Value::Float(function(value)))
}

fn math_sin(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    math_trig(interp, args, kwargs, "sin", crate::float::sin)
}

fn math_cos(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    math_trig(interp, args, kwargs, "cos", crate::float::cos)
}

fn math_tan(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    math_trig(interp, args, kwargs, "tan", crate::float::tan)
}

fn math_log(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "log")?;
    expect_args(interp, args, "log", 1, 2)?;
    let value = as_float(interp, argument(interp, args, 0, "log")?, "math argument")?;
    if value < 0.0 || value == 0.0 {
        return Err(interp.err("ValueError", String::from("math domain error")));
    }
    let numerator = crate::float::ln(value);
    let Some(base_value) = args.get(1) else {
        return Ok(Value::Float(numerator));
    };
    let base = as_float(interp, base_value, "math argument")?;
    if base <= 0.0 {
        return Err(interp.err("ValueError", String::from("math domain error")));
    }
    let denominator = crate::float::ln(base);
    if denominator == 0.0 {
        return Err(interp.err("ZeroDivisionError", String::from("division by zero")));
    }
    Ok(Value::Float(numerator / denominator))
}

fn math_log10(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    let value = math_float_argument(interp, args, kwargs, "log10")?;
    if value < 0.0 || value == 0.0 {
        return Err(interp.err("ValueError", String::from("math domain error")));
    }
    Ok(Value::Float(crate::float::log10(value)))
}

fn math_exp(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    let value = math_float_argument(interp, args, kwargs, "exp")?;
    let result = crate::float::exp(value);
    if result.is_infinite() && value.is_finite() {
        return Err(interp.err("OverflowError", String::from("math range error")));
    }
    Ok(Value::Float(result))
}

fn math_isnan(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    let value = math_float_argument(interp, args, kwargs, "isnan")?;
    Ok(Value::Bool(value.is_nan()))
}

fn math_isinf(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    let value = math_float_argument(interp, args, kwargs, "isinf")?;
    Ok(Value::Bool(value.is_infinite()))
}

fn builtin_hex(interp: &Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "hex")?;
    expect_args(interp, args, "hex", 1, 1)?;
    let number = as_int(interp, argument(interp, args, 0, "hex")?, "hex() argument")?;
    Ok(Value::Str(Rc::from(hex_text(number).as_str())))
}

fn builtin_id(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "id")?;
    expect_args(interp, args, "id", 1, 1)?;
    let value = argument(interp, args, 0, "id")?.clone();
    for (candidate, id) in &interp.ids {
        if is_same(candidate, &value) {
            return Ok(Value::Int(*id));
        }
    }
    let next = 0x1000 + (interp.ids.len() as i64) * 16;
    interp.ids.push((value, next));
    Ok(Value::Int(next))
}

// ── method dispatch ────────────────────────────────────────────────────────

fn receiver<'a>(interp: &Interp, builtin: &'a BuiltinValue, function: &str) -> PyResult<&'a Value> {
    match &builtin.receiver {
        Some(value) => Ok(value),
        None => Err(interp.err(
            "TypeError",
            format!("{}() is a method and needs a receiver", function),
        )),
    }
}

fn str_receiver<'a>(
    interp: &Interp,
    builtin: &'a BuiltinValue,
    function: &str,
) -> PyResult<&'a str> {
    match receiver(interp, builtin, function)? {
        Value::Str(text) => Ok(text),
        other => Err(interp.err(
            "TypeError",
            format!(
                "{}() receiver must be str, not '{}'",
                function,
                other.type_name()
            ),
        )),
    }
}

fn list_receiver(
    interp: &Interp,
    builtin: &BuiltinValue,
    function: &str,
) -> PyResult<Rc<RefCell<Vec<Value>>>> {
    match receiver(interp, builtin, function)? {
        Value::List(items) => Ok(items.clone()),
        other => Err(interp.err(
            "TypeError",
            format!(
                "{}() receiver must be list, not '{}'",
                function,
                other.type_name()
            ),
        )),
    }
}

fn dict_receiver(
    interp: &Interp,
    builtin: &BuiltinValue,
    function: &str,
) -> PyResult<Rc<RefCell<Vec<(Value, Value)>>>> {
    match receiver(interp, builtin, function)? {
        Value::Dict(entries) => Ok(entries.clone()),
        other => Err(interp.err(
            "TypeError",
            format!(
                "{}() receiver must be dict, not '{}'",
                function,
                other.type_name()
            ),
        )),
    }
}

fn file_receiver(
    interp: &Interp,
    builtin: &BuiltinValue,
    function: &str,
) -> PyResult<Rc<RefCell<FileValue>>> {
    match receiver(interp, builtin, function)? {
        Value::File(file) => Ok(file.clone()),
        other => Err(interp.err(
            "TypeError",
            format!(
                "{}() receiver must be a file, not '{}'",
                function,
                other.type_name()
            ),
        )),
    }
}

fn file_error(interp: &Interp, message: String) -> crate::error::PyError {
    let name = if message.starts_with("I/O operation on closed file") {
        "ValueError"
    } else {
        "OSError"
    };
    interp.err(name, message)
}

fn call_method(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    match builtin.id {
        BuiltinId::StrSplit => str_split(interp, builtin, args, kwargs),
        BuiltinId::StrJoin => str_join(interp, builtin, args, kwargs),
        BuiltinId::StrStrip => str_strip(interp, builtin, args, kwargs, true, true),
        BuiltinId::StrLstrip => str_strip(interp, builtin, args, kwargs, true, false),
        BuiltinId::StrRstrip => str_strip(interp, builtin, args, kwargs, false, true),
        BuiltinId::StrUpper => str_case(interp, builtin, args, kwargs, true),
        BuiltinId::StrLower => str_case(interp, builtin, args, kwargs, false),
        BuiltinId::StrReplace => str_replace(interp, builtin, args, kwargs),
        BuiltinId::StrFind => str_find(interp, builtin, args, kwargs, false),
        BuiltinId::StrIndex => str_find(interp, builtin, args, kwargs, true),
        BuiltinId::StrStartswith => str_affix(interp, builtin, args, kwargs, true),
        BuiltinId::StrEndswith => str_affix(interp, builtin, args, kwargs, false),
        BuiltinId::StrCount => str_count(interp, builtin, args, kwargs),
        BuiltinId::StrIsdigit => str_isdigit(interp, builtin, args, kwargs),
        BuiltinId::StrTitle => str_title(interp, builtin, args, kwargs),
        BuiltinId::StrSplitlines => str_splitlines(interp, builtin, args, kwargs),
        BuiltinId::ListAppend => list_append(interp, builtin, args, kwargs),
        BuiltinId::ListExtend => list_extend(interp, builtin, args, kwargs),
        BuiltinId::ListPop => list_pop(interp, builtin, args, kwargs),
        BuiltinId::ListSort => list_sort(interp, builtin, args, kwargs),
        BuiltinId::ListInsert => list_insert(interp, builtin, args, kwargs),
        BuiltinId::ListRemove => list_remove(interp, builtin, args, kwargs),
        BuiltinId::ListIndex => list_index(interp, builtin, args, kwargs),
        BuiltinId::ListReverse => list_reverse(interp, builtin, args, kwargs),
        BuiltinId::ListClear => list_clear(interp, builtin, args, kwargs),
        BuiltinId::ListCopy => list_copy(interp, builtin, args, kwargs),
        BuiltinId::DictGet => dict_method_get(interp, builtin, args, kwargs),
        BuiltinId::DictKeys => dict_method_keys(interp, builtin, args, kwargs),
        BuiltinId::DictValues => dict_method_values(interp, builtin, args, kwargs),
        BuiltinId::DictItems => dict_method_items(interp, builtin, args, kwargs),
        BuiltinId::DictUpdate => dict_method_update(interp, builtin, args, kwargs),
        BuiltinId::DictPop => dict_method_pop(interp, builtin, args, kwargs),
        BuiltinId::DictSetdefault => dict_method_setdefault(interp, builtin, args, kwargs),
        BuiltinId::DictClear => dict_method_clear(interp, builtin, args, kwargs),
        BuiltinId::DictCopy => dict_method_copy(interp, builtin, args, kwargs),
        BuiltinId::FileRead => file_read(interp, builtin, args, kwargs),
        BuiltinId::FileReadline => file_readline(interp, builtin, args, kwargs),
        BuiltinId::FileReadlines => file_readlines(interp, builtin, args, kwargs),
        BuiltinId::FileWrite => file_write(interp, builtin, args, kwargs),
        BuiltinId::FileWritelines => file_writelines(interp, builtin, args, kwargs),
        BuiltinId::FileClose => file_close(interp, builtin, args, kwargs),
        BuiltinId::FileFlush => file_flush(interp, builtin, args, kwargs),
        other => Err(interp.err(
            "RuntimeError",
            format!("builtin '{}' is not a method", value::builtin_name(other)),
        )),
    }
}

fn construct_tag(
    interp: &mut Interp,
    tag: BuiltinType,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    function: &str,
) -> PyResult<Value> {
    let class = match interp.class_for(tag) {
        Some(class) => class,
        None => {
            return Err(interp.err(
                "RuntimeError",
                format!("interpreter is missing the '{}' class", function),
            ))
        }
    };
    interp.construct_builtin(tag, class, args, kwargs)
}

// ── str methods ────────────────────────────────────────────────────────────

fn chars_of(text: &str) -> Vec<char> {
    text.chars().collect()
}

/// Python's `start`/`end` slice-like clamping for the string search methods.
/// Returns `None` when `start` is past the end, which is what makes
/// `"abc".find("", 4) == -1` rather than `3`.
fn clamp_range(start: i64, end: i64, len: usize) -> Option<(usize, usize)> {
    let len = len as i128;
    if start > len as i64 {
        return None;
    }
    let normalize = |value: i64| -> i128 {
        let value = value as i128;
        if value < 0 {
            let shifted = value + len;
            if shifted < 0 {
                0
            } else {
                shifted
            }
        } else if value > len {
            len
        } else {
            value
        }
    };
    let start = normalize(start);
    let end = normalize(end);
    Some((start as usize, end as usize))
}

fn find_chars(hay: &[char], needle: &[char], start: usize, end: usize) -> Option<usize> {
    if start > end {
        return None;
    }
    if needle.is_empty() {
        return Some(start);
    }
    let mut position = start;
    while position + needle.len() <= end {
        if hay[position..position + needle.len()] == *needle {
            return Some(position);
        }
        position += 1;
    }
    None
}

fn str_split(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    check_kwargs(interp, kwargs, "split", &["sep", "maxsplit"])?;
    let text = str_receiver(interp, builtin, "split")?;
    let sep = match args.get(0) {
        Some(value) => Some(value),
        None => keyword(kwargs, "sep"),
    };
    let mut maxsplit = match args.get(1) {
        Some(value) => as_int(interp, value, "split() maxsplit")?,
        None => match keyword(kwargs, "maxsplit") {
            Some(value) => as_int(interp, value, "split() maxsplit")?,
            None => -1,
        },
    };
    if args.len() > 2 {
        return Err(interp.err(
            "TypeError",
            String::from("split() takes at most 2 arguments"),
        ));
    }
    if args.len() == 2 && keyword(kwargs, "maxsplit").is_some() {
        return Err(interp.err(
            "TypeError",
            String::from("split() got multiple values for argument 'maxsplit'"),
        ));
    }
    if maxsplit < 0 {
        maxsplit = -1;
    }
    let chars = chars_of(text);
    let mut out = Vec::new();
    match sep {
        None | Some(Value::None) => {
            let mut index = 0;
            let mut splits = 0i64;
            loop {
                while index < chars.len() && chars[index].is_whitespace() {
                    index += 1;
                }
                if index >= chars.len() {
                    break;
                }
                if maxsplit >= 0 && splits >= maxsplit {
                    out.push(Value::Str(Rc::from(
                        chars[index..].iter().collect::<String>().as_str(),
                    )));
                    break;
                }
                let start = index;
                while index < chars.len() && !chars[index].is_whitespace() {
                    index += 1;
                }
                out.push(Value::Str(Rc::from(
                    chars[start..index].iter().collect::<String>().as_str(),
                )));
                splits += 1;
            }
        }
        Some(Value::Str(separator)) => {
            let needle = chars_of(separator);
            if needle.is_empty() {
                return Err(interp.err("ValueError", String::from("empty separator")));
            }
            let mut start = 0;
            let mut index = 0;
            let mut splits = 0i64;
            while index + needle.len() <= chars.len() {
                if maxsplit >= 0 && splits >= maxsplit {
                    break;
                }
                if chars[index..index + needle.len()] == needle[..] {
                    out.push(Value::Str(Rc::from(
                        chars[start..index].iter().collect::<String>().as_str(),
                    )));
                    index += needle.len();
                    start = index;
                    splits += 1;
                } else {
                    index += 1;
                }
            }
            out.push(Value::Str(Rc::from(
                chars[start..].iter().collect::<String>().as_str(),
            )));
        }
        Some(other) => {
            return Err(interp.err(
                "TypeError",
                format!(
                    "split() separator must be str or None, not '{}'",
                    other.type_name()
                ),
            ))
        }
    }
    Ok(Value::list(out))
}

fn str_join(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "join")?;
    expect_args(interp, args, "join", 1, 1)?;
    let separator = str_receiver(interp, builtin, "join")?;
    let iterable = argument(interp, args, 0, "join")?.clone();
    let items = interp.collect_iterable(iterable)?;
    let mut out = String::new();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(separator);
        }
        match item {
            Value::Str(text) => out.push_str(text),
            other => {
                return Err(interp.err(
                    "TypeError",
                    format!(
                        "sequence item {}: expected str instance, {} found",
                        index,
                        other.type_name()
                    ),
                ))
            }
        }
    }
    Ok(Value::Str(Rc::from(out.as_str())))
}

fn str_strip(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    left: bool,
    right: bool,
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "strip")?;
    expect_args(interp, args, "strip", 0, 1)?;
    let text = str_receiver(interp, builtin, "strip")?;
    let chars = match args.get(0) {
        None | Some(Value::None) => None,
        Some(Value::Str(set)) => Some(String::from(&**set)),
        Some(other) => {
            return Err(interp.err(
                "TypeError",
                format!(
                    "strip() arg must be None or str, not '{}'",
                    other.type_name()
                ),
            ))
        }
    };
    let matches = |ch: char| match &chars {
        Some(set) => set.chars().any(|candidate| candidate == ch),
        None => ch.is_whitespace(),
    };
    let mut start = 0usize;
    let mut end = text.len();
    if left {
        while start < end {
            let ch = match text[start..].chars().next() {
                Some(ch) => ch,
                None => break,
            };
            if matches(ch) {
                start += ch.len_utf8();
            } else {
                break;
            }
        }
    }
    if right {
        while end > start {
            let ch = match text[..end].chars().next_back() {
                Some(ch) => ch,
                None => break,
            };
            if matches(ch) {
                end -= ch.len_utf8();
            } else {
                break;
            }
        }
    }
    Ok(Value::Str(Rc::from(&text[start..end])))
}

fn str_case(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    upper: bool,
) -> PyResult<Value> {
    let name = if upper { "upper" } else { "lower" };
    no_kwargs(interp, kwargs, name)?;
    expect_args(interp, args, name, 0, 0)?;
    let text = str_receiver(interp, builtin, name)?;
    let out: String = if upper {
        text.chars().flat_map(char::to_uppercase).collect()
    } else {
        text.chars().flat_map(char::to_lowercase).collect()
    };
    Ok(Value::Str(Rc::from(out.as_str())))
}

fn str_replace(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "replace")?;
    expect_args(interp, args, "replace", 2, 3)?;
    let text = str_receiver(interp, builtin, "replace")?;
    let old = chars_of(as_str(
        interp,
        argument(interp, args, 0, "replace")?,
        "replace() old",
    )?);
    let new = as_str(
        interp,
        argument(interp, args, 1, "replace")?,
        "replace() new",
    )?;
    let mut count = match args.get(2) {
        Some(value) => as_int(interp, value, "replace() count")?,
        None => -1,
    };
    if count < 0 {
        count = i64::MAX;
    }
    let chars = chars_of(text);
    let mut out = String::new();
    if old.is_empty() {
        let mut remaining = count;
        for ch in &chars {
            if remaining > 0 {
                out.push_str(new);
                remaining -= 1;
            }
            out.push(*ch);
        }
        if remaining > 0 {
            out.push_str(new);
        }
    } else {
        let mut index = 0;
        let mut replaced = 0i64;
        while index < chars.len() {
            if replaced < count
                && index + old.len() <= chars.len()
                && chars[index..index + old.len()] == old[..]
            {
                out.push_str(new);
                index += old.len();
                replaced += 1;
            } else {
                out.push(chars[index]);
                index += 1;
            }
        }
    }
    Ok(Value::Str(Rc::from(out.as_str())))
}

fn str_find(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    raise: bool,
) -> PyResult<Value> {
    let name = if raise { "index" } else { "find" };
    no_kwargs(interp, kwargs, name)?;
    expect_args(interp, args, name, 1, 3)?;
    let text = str_receiver(interp, builtin, name)?;
    let needle = chars_of(as_str(
        interp,
        argument(interp, args, 0, name)?,
        "substring",
    )?);
    let char_len = text.chars().count();
    let start = match args.get(1) {
        Some(value) => as_int(interp, value, "start")?,
        None => 0,
    };
    let end = match args.get(2) {
        Some(value) => as_int(interp, value, "end")?,
        None => char_len as i64,
    };
    let chars = chars_of(text);
    let found = match clamp_range(start, end, char_len) {
        Some((start, end)) => find_chars(&chars, &needle, start, end),
        None => None,
    };
    match found {
        Some(position) => Ok(Value::Int(position as i64)),
        None if raise => Err(interp.err("ValueError", String::from("substring not found"))),
        None => Ok(Value::Int(-1)),
    }
}

fn str_affix(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    prefix: bool,
) -> PyResult<Value> {
    let name = if prefix { "startswith" } else { "endswith" };
    no_kwargs(interp, kwargs, name)?;
    expect_args(interp, args, name, 1, 3)?;
    let text = str_receiver(interp, builtin, name)?;
    let char_len = text.chars().count();
    let start = match args.get(1) {
        Some(value) => as_int(interp, value, "start")?,
        None => 0,
    };
    let end = match args.get(2) {
        Some(value) => as_int(interp, value, "end")?,
        None => char_len as i64,
    };
    let chars = chars_of(text);
    let (start, end) = match clamp_range(start, end, char_len) {
        Some(range) => range,
        None => return Ok(Value::Bool(false)),
    };
    if end < start {
        return Ok(Value::Bool(false));
    }
    let hay = &chars[start..end];
    let check = |needle: &Value| -> PyResult<bool> {
        let needle = chars_of(as_str(interp, needle, name)?);
        if needle.len() > hay.len() {
            return Ok(false);
        }
        let slice = if prefix {
            &hay[..needle.len()]
        } else {
            &hay[hay.len() - needle.len()..]
        };
        Ok(slice == &needle[..])
    };
    match argument(interp, args, 0, name)? {
        Value::Str(text) => {
            let needle = chars_of(text);
            if needle.len() > hay.len() {
                Ok(Value::Bool(false))
            } else {
                let slice = if prefix {
                    &hay[..needle.len()]
                } else {
                    &hay[hay.len() - needle.len()..]
                };
                Ok(Value::Bool(slice == &needle[..]))
            }
        }
        Value::Tuple(items) => {
            for item in items.iter() {
                if check(item)? {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        other => Err(interp.err(
            "TypeError",
            format!(
                "{}() first arg must be str or a tuple of str, not '{}'",
                name,
                other.type_name()
            ),
        )),
    }
}

fn str_count(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "count")?;
    expect_args(interp, args, "count", 1, 3)?;
    let text = str_receiver(interp, builtin, "count")?;
    let needle = chars_of(as_str(
        interp,
        argument(interp, args, 0, "count")?,
        "substring",
    )?);
    let char_len = text.chars().count();
    let start = match args.get(1) {
        Some(value) => as_int(interp, value, "start")?,
        None => 0,
    };
    let end = match args.get(2) {
        Some(value) => as_int(interp, value, "end")?,
        None => char_len as i64,
    };
    let chars = chars_of(text);
    let (start, end) = match clamp_range(start, end, char_len) {
        Some(range) => range,
        None => return Ok(Value::Int(0)),
    };
    if end < start {
        return Ok(Value::Int(0));
    }
    if needle.is_empty() {
        return Ok(Value::Int((end - start + 1) as i64));
    }
    let mut count = 0i64;
    let mut index = start;
    while index + needle.len() <= end {
        if chars[index..index + needle.len()] == needle[..] {
            count += 1;
            index += needle.len();
        } else {
            index += 1;
        }
    }
    Ok(Value::Int(count))
}

fn str_isdigit(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "isdigit")?;
    expect_args(interp, args, "isdigit", 0, 0)?;
    let text = str_receiver(interp, builtin, "isdigit")?;
    Ok(Value::Bool(
        !text.is_empty() && text.chars().all(|ch| ch.is_ascii_digit()),
    ))
}

fn str_title(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "title")?;
    expect_args(interp, args, "title", 0, 0)?;
    let text = str_receiver(interp, builtin, "title")?;
    let mut out = String::new();
    let mut previous_cased = false;
    for ch in text.chars() {
        if previous_cased {
            out.extend(ch.to_lowercase());
        } else {
            out.extend(ch.to_uppercase());
        }
        previous_cased = ch.is_alphabetic();
    }
    Ok(Value::Str(Rc::from(out.as_str())))
}

fn str_splitlines(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "splitlines")?;
    expect_args(interp, args, "splitlines", 0, 1)?;
    let text = str_receiver(interp, builtin, "splitlines")?;
    let keepends = match args.get(0) {
        Some(value) => value.truthy(),
        None => false,
    };
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        let is_break = matches!(
            ch,
            '\n' | '\r'
                | '\u{000b}'
                | '\u{000c}'
                | '\u{001c}'
                | '\u{001d}'
                | '\u{001e}'
                | '\u{0085}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if is_break {
            if keepends {
                current.push(ch);
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    current.push(chars.next().unwrap_or('\n'));
                }
            } else if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push(Value::Str(Rc::from(current.as_str())));
            current.clear();
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        out.push(Value::Str(Rc::from(current.as_str())));
    }
    Ok(Value::list(out))
}

// ── list methods ───────────────────────────────────────────────────────────

fn list_append(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "append")?;
    expect_args(interp, args, "append", 1, 1)?;
    let items = list_receiver(interp, builtin, "append")?;
    items
        .borrow_mut()
        .push(argument(interp, args, 0, "append")?.clone());
    Ok(Value::None)
}

fn list_extend(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "extend")?;
    expect_args(interp, args, "extend", 1, 1)?;
    let items = list_receiver(interp, builtin, "extend")?;
    let source = argument(interp, args, 0, "extend")?.clone();
    let values = interp.collect_iterable(source)?;
    items.borrow_mut().extend(values);
    Ok(Value::None)
}

fn list_pop(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "pop")?;
    expect_args(interp, args, "pop", 0, 1)?;
    let items = list_receiver(interp, builtin, "pop")?;
    let index = match args.get(0) {
        Some(value) => as_int(interp, value, "pop() index")?,
        None => -1,
    };
    let mut borrowed = items.borrow_mut();
    if borrowed.is_empty() {
        return Err(interp.err("IndexError", String::from("pop from empty list")));
    }
    let position = normalize_index(index, borrowed.len());
    match position {
        Some(position) => Ok(borrowed.remove(position)),
        None => Err(interp.err("IndexError", String::from("pop index out of range"))),
    }
}

fn list_sort(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    check_kwargs(interp, kwargs, "sort", &["key", "reverse"])?;
    if args.len() != 0 {
        return Err(interp.err(
            "TypeError",
            String::from("sort() takes no positional arguments"),
        ));
    }
    let items = list_receiver(interp, builtin, "sort")?;
    let key = match keyword(kwargs, "key") {
        None | Some(Value::None) => None,
        Some(value) => Some(value.clone()),
    };
    let reverse = keyword(kwargs, "reverse")
        .map(Value::truthy)
        .unwrap_or(false);
    let snapshot = items.borrow().clone();
    let sorted = sorted_values(interp, snapshot, key, reverse)?;
    *items.borrow_mut() = sorted;
    Ok(Value::None)
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

fn list_insert(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "insert")?;
    expect_args(interp, args, "insert", 2, 2)?;
    let items = list_receiver(interp, builtin, "insert")?;
    let index = as_int(
        interp,
        argument(interp, args, 0, "insert")?,
        "insert() index",
    )?;
    let value = argument(interp, args, 1, "insert")?.clone();
    let mut borrowed = items.borrow_mut();
    let len = borrowed.len() as i128;
    let mut position = index as i128;
    if position < 0 {
        position += len;
        if position < 0 {
            position = 0;
        }
    }
    if position > len {
        position = len;
    }
    borrowed.insert(position as usize, value);
    Ok(Value::None)
}

fn list_remove(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "remove")?;
    expect_args(interp, args, "remove", 1, 1)?;
    let items = list_receiver(interp, builtin, "remove")?;
    let needle = argument(interp, args, 0, "remove")?;
    let mut borrowed = items.borrow_mut();
    match borrowed
        .iter()
        .position(|candidate| values_equal(candidate, needle))
    {
        Some(position) => {
            borrowed.remove(position);
            Ok(Value::None)
        }
        None => Err(interp.err("ValueError", String::from("list.remove(x): x not in list"))),
    }
}

fn list_index(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "index")?;
    expect_args(interp, args, "index", 1, 3)?;
    let items = list_receiver(interp, builtin, "index")?;
    let needle = argument(interp, args, 0, "index")?;
    let borrowed = items.borrow();
    let len = borrowed.len() as i64;
    let start = match args.get(1) {
        Some(value) => as_int(interp, value, "index() start")?,
        None => 0,
    };
    let stop = match args.get(2) {
        Some(value) => as_int(interp, value, "index() stop")?,
        None => len,
    };
    let normalize = |value: i64| -> usize {
        let mut value = value as i128;
        if value < 0 {
            value += len as i128;
            if value < 0 {
                value = 0;
            }
        }
        if value > len as i128 {
            value = len as i128;
        }
        value as usize
    };
    let start = normalize(start);
    let stop = normalize(stop);
    if start < stop {
        for index in start..stop {
            if values_equal(&borrowed[index], needle) {
                return Ok(Value::Int(index as i64));
            }
        }
    }
    Err(interp.err("ValueError", String::from("x is not in list")))
}

fn list_reverse(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "reverse")?;
    expect_args(interp, args, "reverse", 0, 0)?;
    let items = list_receiver(interp, builtin, "reverse")?;
    items.borrow_mut().reverse();
    Ok(Value::None)
}

fn list_clear(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "clear")?;
    expect_args(interp, args, "clear", 0, 0)?;
    let items = list_receiver(interp, builtin, "clear")?;
    items.borrow_mut().clear();
    Ok(Value::None)
}

fn list_copy(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "copy")?;
    expect_args(interp, args, "copy", 0, 0)?;
    let items = list_receiver(interp, builtin, "copy")?;
    let snapshot = items.borrow().clone();
    Ok(Value::list(snapshot))
}

// ── dict methods ───────────────────────────────────────────────────────────

fn require_hashable(interp: &Interp, key: &Value, _function: &str) -> PyResult<()> {
    if key.is_hashable() {
        Ok(())
    } else {
        Err(interp.err(
            "TypeError",
            format!("unhashable type: '{}'", key.type_name()),
        ))
    }
}

fn dict_method_get(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "get")?;
    expect_args(interp, args, "get", 1, 2)?;
    let entries = dict_receiver(interp, builtin, "get")?;
    let key = argument(interp, args, 0, "get")?;
    require_hashable(interp, key, "get")?;
    let found = dict_get(&entries.borrow(), key);
    match found {
        Some(value) => Ok(value),
        None => Ok(args.get(1).cloned().unwrap_or(Value::None)),
    }
}

fn dict_method_keys(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "keys")?;
    expect_args(interp, args, "keys", 0, 0)?;
    let entries = dict_receiver(interp, builtin, "keys")?;
    let keys = entries
        .borrow()
        .iter()
        .map(|(key, _)| key.clone())
        .collect();
    Ok(Value::list(keys))
}

fn dict_method_values(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "values")?;
    expect_args(interp, args, "values", 0, 0)?;
    let entries = dict_receiver(interp, builtin, "values")?;
    let values = entries
        .borrow()
        .iter()
        .map(|(_, value)| value.clone())
        .collect();
    Ok(Value::list(values))
}

fn dict_method_items(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "items")?;
    expect_args(interp, args, "items", 0, 0)?;
    let entries = dict_receiver(interp, builtin, "items")?;
    let items = entries
        .borrow()
        .iter()
        .map(|(key, value)| Value::tuple(alloc::vec![key.clone(), value.clone()]))
        .collect();
    Ok(Value::list(items))
}

fn dict_method_update(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    expect_args(interp, args, "update", 0, 1)?;
    let entries = dict_receiver(interp, builtin, "update")?;
    if let Some(source) = args.get(0) {
        match source {
            Value::Dict(other) => {
                let pairs = other.borrow().clone();
                let mut target = entries.borrow_mut();
                for (key, value) in pairs {
                    dict_set(&mut target, key, value);
                }
            }
            _ => {
                let pairs = interp.collect_iterable(source.clone())?;
                let mut target = entries.borrow_mut();
                for (index, pair) in pairs.into_iter().enumerate() {
                    let parts = interp.collect_iterable(pair)?;
                    if parts.len() != 2 {
                        return Err(interp.err(
                            "ValueError",
                            format!(
                                "dictionary update sequence element #{} has length {}; 2 is required",
                                index,
                                parts.len()
                            ),
                        ));
                    }
                    require_hashable(interp, &parts[0], "update")?;
                    dict_set(&mut target, parts[0].clone(), parts[1].clone());
                }
            }
        }
    }
    if !kwargs.is_empty() {
        let mut target = entries.borrow_mut();
        for (name, value) in kwargs {
            dict_set(&mut target, Value::Str(name.clone()), value.clone());
        }
    }
    Ok(Value::None)
}

fn dict_method_pop(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "pop")?;
    expect_args(interp, args, "pop", 1, 2)?;
    let entries = dict_receiver(interp, builtin, "pop")?;
    let key = argument(interp, args, 0, "pop")?;
    require_hashable(interp, key, "pop")?;
    let mut borrowed = entries.borrow_mut();
    match dict_find(&borrowed, key) {
        Some(index) => Ok(borrowed.remove(index).1),
        None => match args.get(1) {
            Some(default) => Ok(default.clone()),
            None => Err(interp.err("KeyError", key.to_repr())),
        },
    }
}

fn dict_method_setdefault(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "setdefault")?;
    expect_args(interp, args, "setdefault", 1, 2)?;
    let entries = dict_receiver(interp, builtin, "setdefault")?;
    let key = argument(interp, args, 0, "setdefault")?;
    require_hashable(interp, key, "setdefault")?;
    let default = args.get(1).cloned().unwrap_or(Value::None);
    let mut borrowed = entries.borrow_mut();
    match dict_find(&borrowed, key) {
        Some(index) => Ok(borrowed[index].1.clone()),
        None => {
            borrowed.push((key.clone(), default.clone()));
            Ok(default)
        }
    }
}

fn dict_method_clear(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "clear")?;
    expect_args(interp, args, "clear", 0, 0)?;
    let entries = dict_receiver(interp, builtin, "clear")?;
    entries.borrow_mut().clear();
    Ok(Value::None)
}

fn dict_method_copy(
    interp: &Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "copy")?;
    expect_args(interp, args, "copy", 0, 0)?;
    let entries = dict_receiver(interp, builtin, "copy")?;
    let snapshot = entries.borrow().clone();
    Ok(Value::dict(snapshot))
}

// ── file methods ───────────────────────────────────────────────────────────

fn file_read(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "read")?;
    expect_args(interp, args, "read", 0, 1)?;
    let file = file_receiver(interp, builtin, "read")?;
    let count = match args.get(0) {
        Some(value) => Some(as_int(interp, value, "read() size")?),
        None => None,
    };
    let mut borrowed = file.borrow_mut();
    match borrowed.read(count) {
        Ok(text) => Ok(Value::Str(Rc::from(text.as_str()))),
        Err(message) => {
            drop(borrowed);
            Err(file_error(interp, message))
        }
    }
}

fn file_readline(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "readline")?;
    expect_args(interp, args, "readline", 0, 1)?;
    let file = file_receiver(interp, builtin, "readline")?;
    let mut borrowed = file.borrow_mut();
    let result = match args.get(0) {
        None => borrowed.readline(),
        Some(value) => {
            let size = as_int(interp, value, "readline() size")?;
            let mut out = String::new();
            let mut taken = 0i64;
            let mut error = None;
            while size < 0 || taken < size {
                match borrowed.read(Some(1)) {
                    Ok(text) => {
                        if text.is_empty() {
                            break;
                        }
                        out.push_str(&text);
                        taken += 1;
                        if text == "\n" {
                            break;
                        }
                    }
                    Err(message) => {
                        error = Some(message);
                        break;
                    }
                }
            }
            match error {
                Some(message) => Err(message),
                None => Ok(out),
            }
        }
    };
    match result {
        Ok(text) => Ok(Value::Str(Rc::from(text.as_str()))),
        Err(message) => {
            drop(borrowed);
            Err(file_error(interp, message))
        }
    }
}

fn file_readlines(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "readlines")?;
    expect_args(interp, args, "readlines", 0, 1)?;
    let file = file_receiver(interp, builtin, "readlines")?;
    let hint = match args.get(0) {
        Some(value) => as_int(interp, value, "readlines() hint")?,
        None => -1,
    };
    let mut out = Vec::new();
    let mut total = 0i64;
    loop {
        let line = {
            let mut borrowed = file.borrow_mut();
            match borrowed.readline() {
                Ok(line) => line,
                Err(message) => return Err(file_error(interp, message)),
            }
        };
        if line.is_empty() {
            break;
        }
        total += line.chars().count() as i64;
        out.push(Value::Str(Rc::from(line.as_str())));
        if hint >= 0 && total >= hint {
            break;
        }
    }
    Ok(Value::list(out))
}

fn file_write(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "write")?;
    expect_args(interp, args, "write", 1, 1)?;
    let file = file_receiver(interp, builtin, "write")?;
    let text = as_str(
        interp,
        argument(interp, args, 0, "write")?,
        "write() argument",
    )?;
    let mut borrowed = file.borrow_mut();
    match borrowed.write(text) {
        Ok(count) => Ok(Value::Int(count as i64)),
        Err(message) => {
            drop(borrowed);
            Err(file_error(interp, message))
        }
    }
}

fn file_writelines(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "writelines")?;
    expect_args(interp, args, "writelines", 1, 1)?;
    let file = file_receiver(interp, builtin, "writelines")?;
    let source = argument(interp, args, 0, "writelines")?.clone();
    let lines = interp.collect_iterable(source)?;
    for line in lines {
        match &line {
            Value::Str(text) => {
                let mut borrowed = file.borrow_mut();
                if let Err(message) = borrowed.write(text) {
                    drop(borrowed);
                    return Err(file_error(interp, message));
                }
            }
            other => {
                return Err(interp.err(
                    "TypeError",
                    format!(
                        "writelines() argument must be an iterable of strings, not '{}'",
                        other.type_name()
                    ),
                ))
            }
        }
    }
    Ok(Value::None)
}

fn file_close(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "close")?;
    expect_args(interp, args, "close", 0, 0)?;
    let file = file_receiver(interp, builtin, "close")?;
    let mut borrowed = file.borrow_mut();
    match borrowed.close() {
        Ok(()) => Ok(Value::None),
        Err(message) => {
            drop(borrowed);
            Err(file_error(interp, message))
        }
    }
}

fn file_flush(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "flush")?;
    expect_args(interp, args, "flush", 0, 0)?;
    let file = file_receiver(interp, builtin, "flush")?;
    let mut borrowed = file.borrow_mut();
    if borrowed.closed {
        drop(borrowed);
        return Err(file_error(
            interp,
            String::from("I/O operation on closed file"),
        ));
    }
    if borrowed.writable && borrowed.dirty {
        if let Err(message) = platform::write_text(&borrowed.name, &borrowed.data) {
            drop(borrowed);
            return Err(file_error(interp, message));
        }
        borrowed.dirty = false;
    }
    Ok(Value::None)
}
