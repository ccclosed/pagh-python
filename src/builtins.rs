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
    self, dict_find, dict_get, dict_set, hex_text, is_same, range_length, set_find, set_insert,
    values_equal, BuiltinId, BuiltinType, BuiltinValue, FileValue, Generator, IterState,
    RangeValue, Value,
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
        BuiltinId::Set => construct_tag(interp, BuiltinType::Set, args, kwargs, "set"),
        BuiltinId::Bytes => construct_tag(interp, BuiltinType::Bytes, args, kwargs, "bytes"),
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
        BuiltinId::Map => builtin_map(interp, args, kwargs),
        BuiltinId::StaticMethod => builtin_staticmethod(interp, args, kwargs),
        BuiltinId::StaticMethodCall => {
            let function = receiver(interp, builtin, "staticmethod")?.clone();
            interp.call_value(function, args.duplicate(), kwargs.to_vec())
        }
        BuiltinId::OsGetpid => os_getpid(interp, args, kwargs),
        BuiltinId::OsGetppid => os_getppid(interp, args, kwargs),
        BuiltinId::OsGetcwd => os_getcwd(interp, args, kwargs),
        BuiltinId::OsListdir => os_listdir(interp, args, kwargs),
        BuiltinId::OsMkdir => os_mkdir(interp, args, kwargs),
        BuiltinId::OsMakedirs => os_makedirs(interp, args, kwargs),
        BuiltinId::OsRemove => os_remove(interp, args, kwargs),
        BuiltinId::OsRmdir => os_rmdir(interp, args, kwargs),
        BuiltinId::OsRename => os_rename(interp, args, kwargs),
        BuiltinId::OsStat => os_stat(interp, args, kwargs),
        BuiltinId::OsFork => os_fork(interp, args, kwargs),
        BuiltinId::OsExecv => os_execv(interp, args, kwargs),
        BuiltinId::OsWaitpid => os_waitpid(interp, args, kwargs),
        BuiltinId::OsWait => os_wait(interp, args, kwargs),
        BuiltinId::OsSystem => os_system(interp, args, kwargs),
        BuiltinId::JsonDumps => json_dumps(interp, args, kwargs),
        BuiltinId::JsonLoads => json_loads(interp, args, kwargs),
        BuiltinId::PathJoin => path_join(interp, args, kwargs),
        BuiltinId::PathBasename => path_basename(interp, args, kwargs),
        BuiltinId::PathDirname => path_dirname(interp, args, kwargs),
        BuiltinId::PathExists => path_exists(interp, args, kwargs),
        BuiltinId::PathIsfile => path_isfile(interp, args, kwargs),
        BuiltinId::PathIsdir => path_isdir(interp, args, kwargs),
        BuiltinId::TimeTime => time_time(interp, args, kwargs),
        BuiltinId::TimeMonotonic => time_monotonic(interp, args, kwargs),
        BuiltinId::TimeSleep => time_sleep(interp, args, kwargs),
        BuiltinId::RandomRandom => random_random(interp, args, kwargs),
        BuiltinId::RandomSeed => random_seed(interp, args, kwargs),
        BuiltinId::RandomRandint => random_randint(interp, args, kwargs),
        BuiltinId::RandomChoice => random_choice(interp, args, kwargs),
        BuiltinId::RandomShuffle => random_shuffle(interp, args, kwargs),
        BuiltinId::RandomUniform => random_uniform(interp, args, kwargs),
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
    match &result {
        Value::Str(text) => Ok(String::from(&**text)),
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

// ── f-string / format() specs ──────────────────────────────────────────────

struct FormatSpec {
    fill: char,
    align: Option<char>,
    sign: Option<char>,
    alternate: bool,
    zero: bool,
    width: usize,
    grouping: bool,
    precision: Option<usize>,
    kind: Option<char>,
}

fn parse_format_spec(interp: &Interp, spec: &str) -> PyResult<FormatSpec> {
    let chars: Vec<char> = spec.chars().collect();
    let mut parsed = FormatSpec {
        fill: ' ',
        align: None,
        sign: None,
        alternate: false,
        zero: false,
        width: 0,
        grouping: false,
        precision: None,
        kind: None,
    };
    let mut index = 0usize;
    let is_align = |ch: char| matches!(ch, '<' | '>' | '^' | '=');
    if chars.len() >= 2 && is_align(chars[1]) {
        parsed.fill = chars[0];
        parsed.align = Some(chars[1]);
        index = 2;
    } else if chars.first().map(|ch| is_align(*ch)).unwrap_or(false) {
        parsed.align = chars.first().copied();
        index = 1;
    }
    if let Some(ch) = chars.get(index).copied() {
        if matches!(ch, '+' | '-' | ' ') {
            parsed.sign = Some(ch);
            index += 1;
        }
    }
    if chars.get(index) == Some(&'#') {
        parsed.alternate = true;
        index += 1;
    }
    if chars.get(index) == Some(&'0') {
        parsed.zero = true;
        index += 1;
    }
    let mut width = 0usize;
    let mut digits = 0usize;
    while let Some(ch) = chars.get(index).copied() {
        if let Some(digit) = ch.to_digit(10) {
            if width > 100_000 {
                return Err(interp.err(
                    "ValueError",
                    String::from("format specifier width is too large"),
                ));
            }
            width = width * 10 + digit as usize;
            digits += 1;
            index += 1;
        } else {
            break;
        }
    }
    if digits > 0 {
        parsed.width = width.min(10_000);
    }
    if chars.get(index) == Some(&',') {
        parsed.grouping = true;
        index += 1;
    }
    if chars.get(index) == Some(&'.') {
        index += 1;
        let mut precision = 0usize;
        let mut has_digits = false;
        while let Some(ch) = chars.get(index).copied() {
            if let Some(digit) = ch.to_digit(10) {
                precision = precision * 10 + digit as usize;
                has_digits = true;
                index += 1;
                if precision > 10_000 {
                    return Err(interp.err(
                        "ValueError",
                        String::from("format specifier precision is too large"),
                    ));
                }
            } else {
                break;
            }
        }
        if !has_digits {
            return Err(interp.err(
                "ValueError",
                String::from("format specifier is missing precision digits"),
            ));
        }
        parsed.precision = Some(precision);
    }
    if let Some(ch) = chars.get(index).copied() {
        parsed.kind = Some(ch);
        index += 1;
    }
    if index != chars.len() {
        return Err(interp.err("ValueError", format!("invalid format specifier '{}'", spec)));
    }
    Ok(parsed)
}

fn pad_formatted(body: String, spec: &FormatSpec, numeric: bool) -> String {
    let length = body.chars().count();
    if spec.width <= length {
        return body;
    }
    let padding = spec.width - length;
    let default_align = if numeric { '>' } else { '<' };
    let mut align = spec.align.unwrap_or(default_align);
    if spec.zero && numeric && spec.align.is_none() {
        align = '=';
    }
    let fill = if spec.zero && numeric { '0' } else { spec.fill };
    match align {
        '=' => {
            let mut chars = body.chars();
            let first = chars.next();
            let Some(first) = first else {
                return body;
            };
            if matches!(first, '+' | '-' | ' ') {
                let rest: String = chars.collect();
                let mut out = String::with_capacity(spec.width);
                out.push(first);
                for _ in 0..padding {
                    out.push(fill);
                }
                out.push_str(&rest);
                out
            } else {
                let mut out = String::with_capacity(spec.width);
                for _ in 0..padding {
                    out.push(fill);
                }
                out.push_str(&body);
                out
            }
        }
        '<' => {
            let mut out = body;
            for _ in 0..padding {
                out.push(fill);
            }
            out
        }
        '^' => {
            let left = padding / 2;
            let right = padding - left;
            let mut out = String::with_capacity(spec.width);
            for _ in 0..left {
                out.push(fill);
            }
            out.push_str(&body);
            for _ in 0..right {
                out.push(fill);
            }
            out
        }
        _ => {
            let mut out = String::with_capacity(spec.width);
            for _ in 0..padding {
                out.push(fill);
            }
            out.push_str(&body);
            out
        }
    }
}

fn group_digits(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let length = digits.len();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (length - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn sign_prefix(negative: bool, spec: &FormatSpec) -> String {
    if negative {
        String::from("-")
    } else {
        match spec.sign {
            Some('+') => String::from("+"),
            Some(' ') => String::from(" "),
            _ => String::new(),
        }
    }
}

fn format_text_value(interp: &mut Interp, value: &Value, spec: &FormatSpec) -> PyResult<String> {
    let mut text = str_value(interp, value)?;
    if let Some(precision) = spec.precision {
        text = text.chars().take(precision).collect();
    }
    Ok(pad_formatted(text, spec, false))
}

fn format_integer_body(
    _interp: &Interp,
    number: i64,
    spec: &FormatSpec,
    radix: u32,
    upper: bool,
    prefix: &str,
) -> PyResult<String> {
    let negative = number < 0;
    let magnitude = (number as i128).unsigned_abs();
    let mut digits = match radix {
        10 => format!("{}", magnitude),
        2 => format!("{:b}", magnitude),
        8 => format!("{:o}", magnitude),
        16 if upper => format!("{:X}", magnitude),
        _ => format!("{:x}", magnitude),
    };
    if spec.grouping && radix == 10 {
        digits = group_digits(&digits);
    }
    let mut body = String::with_capacity(digits.len() + 2);
    body.push_str(&sign_prefix(negative, spec));
    if spec.alternate {
        body.push_str(prefix);
    }
    body.push_str(&digits);
    Ok(pad_formatted(body, spec, true))
}

fn float_special(value: f64) -> Option<&'static str> {
    if value.is_nan() {
        Some("nan")
    } else if value.is_infinite() {
        Some("inf")
    } else {
        None
    }
}

fn exponent_text(mantissa: String, exponent: i32) -> String {
    let sign = if exponent < 0 { '-' } else { '+' };
    let magnitude = exponent.unsigned_abs();
    format!("{}e{}{:02}", mantissa, sign, magnitude)
}

fn format_float_body(
    _interp: &Interp,
    value: f64,
    spec: &FormatSpec,
    kind: char,
) -> PyResult<String> {
    let negative = value.is_sign_negative() && !value.is_nan();
    let mut digits = if let Some(special) = float_special(value) {
        String::from(special)
    } else {
        match kind {
            'f' => {
                let precision = spec.precision.unwrap_or(6);
                format!("{:.*}", precision, crate::float::abs(value))
            }
            '%' => {
                let precision = spec.precision.unwrap_or(6);
                let mut text = format!("{:.*}", precision, crate::float::abs(value * 100.0));
                text.push('%');
                text
            }
            'e' => {
                let precision = spec.precision.unwrap_or(6);
                let raw = format!("{:.*e}", precision, crate::float::abs(value));
                match raw.split_once('e') {
                    Some((mantissa, exponent)) => {
                        let exponent = exponent.parse::<i32>().unwrap_or(0);
                        exponent_text(String::from(mantissa), exponent)
                    }
                    None => raw,
                }
            }
            _ => crate::float::format(crate::float::abs(value)),
        }
    };
    if spec.grouping && (kind == 'f' || kind == '%') {
        if let Some(dot) = digits.find('.') {
            let (integer, fraction) = digits.split_at(dot);
            digits = format!("{}{}", group_digits(integer), fraction);
        } else if kind == 'f' {
            digits = group_digits(&digits);
        }
    }
    let mut body = String::with_capacity(digits.len() + 1);
    body.push_str(&sign_prefix(negative, spec));
    body.push_str(&digits);
    Ok(pad_formatted(body, spec, true))
}

/// Format one value for an f-string (or, ultimately, a `format()`-like call).
/// `spec` is the text after `:`; `conversion` is the `!r`/`!s`/`!a` letter.
pub(crate) fn format_value(
    interp: &mut Interp,
    value: &Value,
    spec: Option<&str>,
    conversion: Option<char>,
) -> PyResult<String> {
    let Some(spec_text) = spec else {
        if let Some(conversion) = conversion {
            return match conversion {
                'r' | 'a' => repr_value(interp, value),
                _ => str_value(interp, value),
            };
        }
        return str_value(interp, value);
    };
    let parsed = parse_format_spec(interp, spec_text)?;
    if let Some(conversion) = conversion {
        let text = match conversion {
            'r' | 'a' => repr_value(interp, value)?,
            _ => str_value(interp, value)?,
        };
        if !matches!(parsed.kind, None | Some('s')) {
            return Err(interp.err(
                "ValueError",
                String::from("a numeric format spec requires a numeric value, not a conversion"),
            ));
        }
        return format_text_value(interp, &Value::Str(Rc::from(text.as_str())), &parsed);
    }
    match parsed.kind {
        Some('s') => format_text_value(interp, value, &parsed),
        Some('d') => {
            if parsed.precision.is_some() {
                return Err(interp.err(
                    "ValueError",
                    String::from("precision is not allowed in an integer format specifier"),
                ));
            }
            match value.as_int() {
                Some(number) => format_integer_body(interp, number, &parsed, 10, false, ""),
                None => Err(interp.err(
                    "TypeError",
                    format!(
                        "unknown format code 'd' for object of type '{}'",
                        value.type_name()
                    ),
                )),
            }
        }
        Some(kind @ ('b' | 'o' | 'x' | 'X')) => {
            if parsed.precision.is_some() {
                return Err(interp.err(
                    "ValueError",
                    String::from("precision is not allowed in an integer format specifier"),
                ));
            }
            let (radix, upper, prefix) = match kind {
                'b' => (2, false, "0b"),
                'o' => (8, false, "0o"),
                'X' => (16, true, "0X"),
                _ => (16, false, "0x"),
            };
            match value.as_int() {
                Some(number) => format_integer_body(interp, number, &parsed, radix, upper, prefix),
                None => Err(interp.err(
                    "TypeError",
                    format!(
                        "unknown format code '{}' for object of type '{}'",
                        kind,
                        value.type_name()
                    ),
                )),
            }
        }
        Some(kind @ ('f' | 'e' | 'g' | '%')) => match value.as_float() {
            Some(number) => format_float_body(interp, number, &parsed, kind),
            None => Err(interp.err(
                "TypeError",
                format!(
                    "unknown format code '{}' for object of type '{}'",
                    kind,
                    value.type_name()
                ),
            )),
        },
        Some(other) => Err(interp.err("ValueError", format!("unknown format code '{}'", other))),
        None => match value {
            Value::Str(_) => format_text_value(interp, value, &parsed),
            Value::Bool(_) | Value::Int(_) => match value.as_int() {
                Some(number) => format_integer_body(interp, number, &parsed, 10, false, ""),
                None => unreachable!(),
            },
            Value::Float(number) => {
                let negative = number.is_sign_negative() && !number.is_nan();
                let mut digits = if let Some(special) = float_special(*number) {
                    String::from(special)
                } else {
                    crate::float::format(crate::float::abs(*number))
                };
                if parsed.grouping {
                    if let Some(dot) = digits.find('.') {
                        let (integer, fraction) = digits.split_at(dot);
                        digits = format!("{}{}", group_digits(integer), fraction);
                    } else {
                        digits = group_digits(&digits);
                    }
                }
                let mut body = sign_prefix(negative, &parsed);
                body.push_str(&digits);
                Ok(pad_formatted(body, &parsed, true))
            }
            _ => format_text_value(interp, value, &parsed),
        },
    }
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
        Value::DictView(view) => view.entries.borrow().len() as i64,
        Value::Set(items) => items.borrow().len() as i64,
        Value::Bytes(data) => data.len() as i64,
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

    let mut readable = false;
    let mut writable = false;
    let mut append = false;
    let mut truncate = false;
    let mut binary = false;
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
            'b' => binary = true,
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
    if encoding.is_some() {
        if binary {
            return Err(interp.err(
                "ValueError",
                String::from("binary mode doesn't take an encoding argument"),
            ));
        }
        if let Some(value) = encoding {
            match value {
                Value::None => {}
                Value::Str(text) if is_utf8_name(text) => {}
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
    }

    let exists = platform::path_exists(&path);
    let mut data = if truncate {
        Vec::new()
    } else if append && !exists {
        Vec::new()
    } else {
        match platform::read_bytes(&path) {
            Ok(data) => data,
            Err(message) => {
                return Err(interp.err(
                    "FileNotFoundError",
                    format!("cannot open '{}': {}", path, message),
                ))
            }
        }
    };
    if !binary {
        if core::str::from_utf8(&data).is_err() {
            return Err(interp.err(
                "OSError",
                format!(
                    "cannot open '{}': file is not valid UTF-8; use binary mode",
                    path
                ),
            ));
        }
    }
    let pos = if append { data.len() } else { 0 };
    let dirty = truncate || (append && !exists);
    data.shrink_to_fit();
    Ok(Value::File(Rc::new(RefCell::new(FileValue {
        name: Rc::from(path.as_str()),
        mode: Rc::from(mode.as_str()),
        data,
        pos,
        readable,
        writable,
        closed: false,
        dirty,
        binary,
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

fn builtin_divmod(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "divmod")?;
    expect_args(interp, args, "divmod", 2, 2)?;
    let left = argument(interp, args, 0, "divmod")?.clone();
    let right = argument(interp, args, 1, "divmod")?.clone();
    let quotient = interp.binary_op(BinOp::FloorDiv, left.clone(), right.clone())?;
    let remainder = interp.binary_op(BinOp::Mod, left, right)?;
    Ok(Value::tuple(alloc::vec![quotient, remainder]))
}

fn builtin_pow(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
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

fn generator_receiver(
    interp: &Interp,
    builtin: &BuiltinValue,
    function: &str,
) -> PyResult<Rc<RefCell<Generator>>> {
    match receiver(interp, builtin, function)? {
        Value::Generator(generator) => Ok(generator.clone()),
        other => Err(interp.err(
            "TypeError",
            format!(
                "{}() receiver must be a generator, not '{}'",
                function,
                other.type_name()
            ),
        )),
    }
}

// ── generator methods (send/throw/close) ──────────────────────────────────
//
// The machinery (resuming a fiber with a value or an injected exception) lives
// in `interp.rs` next to `generator_resume`; these three only validate
// arguments and build the exception `throw` was asked for.

fn generator_send(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "send")?;
    expect_args(interp, args, "send", 1, 1)?;
    let generator = generator_receiver(interp, builtin, "send")?;
    let value = argument(interp, args, 0, "send")?.clone();
    interp.generator_send(&generator, value)
}

fn generator_throw(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "throw")?;
    let generator = generator_receiver(interp, builtin, "throw")?;
    let error = interp.exception_from_throw_args(args)?;
    interp.generator_throw(&generator, error)
}

fn generator_close(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "close")?;
    expect_args(interp, args, "close", 0, 0)?;
    let generator = generator_receiver(interp, builtin, "close")?;
    interp.generator_close(&generator)?;
    Ok(Value::None)
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
        BuiltinId::GenSend => generator_send(interp, builtin, args, kwargs),
        BuiltinId::GenThrow => generator_throw(interp, builtin, args, kwargs),
        BuiltinId::GenClose => generator_close(interp, builtin, args, kwargs),
        BuiltinId::StrFormat => str_format(interp, builtin, args, kwargs),
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
        BuiltinId::SetAdd => set_add(interp, builtin, args, kwargs),
        BuiltinId::SetRemove => set_remove(interp, builtin, args, kwargs),
        BuiltinId::SetDiscard => set_discard(interp, builtin, args, kwargs),
        BuiltinId::SetUnion => set_union(interp, builtin, args, kwargs),
        BuiltinId::SetIntersection => set_intersection(interp, builtin, args, kwargs),
        BuiltinId::SetDifference => set_difference(interp, builtin, args, kwargs),
        BuiltinId::SetIssubset => set_issubset(interp, builtin, args, kwargs),
        BuiltinId::SetPop => set_pop(interp, builtin, args, kwargs),
        BuiltinId::SetUpdate => set_update(interp, builtin, args, kwargs),
        BuiltinId::SetDifferenceUpdate => {
            set_update_like(interp, builtin, args, kwargs, SetUpdateKind::Difference)
        }
        BuiltinId::SetIntersectionUpdate => {
            set_update_like(interp, builtin, args, kwargs, SetUpdateKind::Intersection)
        }
        BuiltinId::SetSymmetricDifferenceUpdate => set_update_like(
            interp,
            builtin,
            args,
            kwargs,
            SetUpdateKind::SymmetricDifference,
        ),
        BuiltinId::SetSymmetricDifference => {
            set_symmetric_difference(interp, builtin, args, kwargs)
        }
        BuiltinId::SetIsDisjoint => set_isdisjoint(interp, builtin, args, kwargs),
        BuiltinId::SetSuperset => set_issuperset(interp, builtin, args, kwargs),
        BuiltinId::SetCopy => set_copy(interp, builtin, args, kwargs),
        BuiltinId::SetClear => set_clear(interp, builtin, args, kwargs),
        BuiltinId::BytesDecode => bytes_decode(interp, builtin, args, kwargs),
        BuiltinId::BytesHex => bytes_hex(interp, builtin, args, kwargs),
        BuiltinId::StrEncode => str_encode(interp, builtin, args, kwargs),
        BuiltinId::FileRead => file_read(interp, builtin, args, kwargs),
        BuiltinId::FileReadline => file_readline(interp, builtin, args, kwargs),
        BuiltinId::FileReadlines => file_readlines(interp, builtin, args, kwargs),
        BuiltinId::FileWrite => file_write(interp, builtin, args, kwargs),
        BuiltinId::FileWritelines => file_writelines(interp, builtin, args, kwargs),
        BuiltinId::FileClose => file_close(interp, builtin, args, kwargs),
        BuiltinId::FileFlush => file_flush(interp, builtin, args, kwargs),
        BuiltinId::FileEnter => file_enter(interp, builtin, args, kwargs),
        BuiltinId::FileExit => file_exit(interp, builtin, args, kwargs),
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

// ── str.format ─────────────────────────────────────────────────────────────

/// Find `target` outside `[...]` keys and quoted strings. Format fields put
/// attribute/index paths before the conversion and the spec, so splitting on
/// the first top-level `!` or `:` is enough for this subset.
fn find_top_level(chars: &[char], target: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (index, ch) in chars.iter().enumerate() {
        if let Some(open) = quote {
            if *ch == open {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(*ch),
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            ch if *ch == target && depth == 0 => return Some(index),
            _ => {}
        }
    }
    None
}

fn take_field_name(chars: &[char], start: usize) -> usize {
    let mut end = start;
    if end < chars.len() && (chars[end].is_ascii_alphabetic() || chars[end] == '_') {
        end += 1;
        while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
    }
    end
}

fn field_key_value(interp: &Interp, chars: &[char]) -> PyResult<Value> {
    if chars.is_empty() {
        return Err(interp.err(
            "ValueError",
            String::from("empty index in replacement field"),
        ));
    }
    let first = chars[0];
    if first == '\'' || first == '"' {
        if chars.len() < 2 || chars[chars.len() - 1] != first {
            return Err(interp.err(
                "ValueError",
                String::from("unterminated string index in replacement field"),
            ));
        }
        let mut text = String::new();
        let mut index = 1usize;
        while index + 1 < chars.len() {
            if chars[index] == '\\' && index + 2 < chars.len() {
                index += 1;
                match chars[index] {
                    'n' => text.push('\n'),
                    'r' => text.push('\r'),
                    't' => text.push('\t'),
                    other => text.push(other),
                }
            } else {
                text.push(chars[index]);
            }
            index += 1;
        }
        return Ok(Value::Str(Rc::from(text.as_str())));
    }
    let start = if first == '-' { 1 } else { 0 };
    if start < chars.len() && chars[start..].iter().all(|ch| ch.is_ascii_digit()) {
        let text: String = chars.iter().collect();
        return match text.parse::<i64>() {
            Ok(number) => Ok(Value::Int(number)),
            Err(_) => Err(interp.err(
                "ValueError",
                format!("invalid index '{}' in replacement field", text),
            )),
        };
    }
    Ok(Value::Str(Rc::from(
        chars.iter().collect::<String>().as_str(),
    )))
}

fn resolve_format_field(
    interp: &mut Interp,
    field: &str,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    automatic: &mut usize,
    used_automatic: &mut bool,
    used_manual: &mut bool,
) -> PyResult<Value> {
    let chars: Vec<char> = field.chars().collect();
    let mut value;
    let mut rest;
    if chars.is_empty() {
        if *used_manual {
            return Err(interp.err(
                "ValueError",
                String::from(
                    "cannot switch from manual field specification to automatic field numbering",
                ),
            ));
        }
        let index = *automatic;
        *automatic += 1;
        *used_automatic = true;
        value = match args.get(index) {
            Some(value) => value.clone(),
            None => {
                return Err(interp.err(
                    "IndexError",
                    format!(
                        "Replacement index {} out of range for positional args tuple",
                        index
                    ),
                ))
            }
        };
        rest = 0usize;
    } else if chars[0].is_ascii_digit() {
        if *used_automatic {
            return Err(interp.err(
                "ValueError",
                String::from(
                    "cannot switch from automatic field numbering to manual field specification",
                ),
            ));
        }
        *used_manual = true;
        let mut end = 1usize;
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }
        let text: String = chars[..end].iter().collect();
        let index: usize = match text.parse() {
            Ok(index) => index,
            Err(_) => {
                return Err(interp.err(
                    "ValueError",
                    format!("invalid replacement field index '{}'", text),
                ))
            }
        };
        value = match args.get(index) {
            Some(value) => value.clone(),
            None => {
                return Err(interp.err(
                    "IndexError",
                    format!(
                        "Replacement index {} out of range for positional args tuple",
                        index
                    ),
                ))
            }
        };
        rest = end;
    } else {
        let end = take_field_name(&chars, 0);
        if end == 0 {
            return Err(interp.err(
                "ValueError",
                format!("invalid replacement field '{}'", field),
            ));
        }
        let name: String = chars[..end].iter().collect();
        value = match keyword(kwargs, &name) {
            Some(value) => value.clone(),
            None => return Err(interp.err("KeyError", name)),
        };
        rest = end;
    }
    while rest < chars.len() {
        match chars[rest] {
            '.' => {
                let end = take_field_name(&chars, rest + 1);
                if end == rest + 1 {
                    return Err(interp.err(
                        "ValueError",
                        String::from("empty attribute name in replacement field"),
                    ));
                }
                let attribute: String = chars[rest + 1..end].iter().collect();
                value = interp.get_attribute(&value, &attribute)?;
                rest = end;
            }
            '[' => {
                let mut end = rest + 1;
                while end < chars.len() && chars[end] != ']' {
                    end += 1;
                }
                if end >= chars.len() {
                    return Err(interp.err(
                        "ValueError",
                        String::from("missing ']' in replacement field"),
                    ));
                }
                let key = field_key_value(interp, &chars[rest + 1..end])?;
                value = interp.index_value(&value, &key)?;
                rest = end + 1;
            }
            other => {
                return Err(interp.err(
                    "ValueError",
                    format!("unexpected character '{}' in replacement field", other),
                ))
            }
        }
    }
    Ok(value)
}

fn str_format(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    let text = String::from(str_receiver(interp, builtin, "format")?);
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut automatic = 0usize;
    let mut used_automatic = false;
    let mut used_manual = false;
    let mut index = 0usize;
    while index < chars.len() {
        match chars[index] {
            '{' => {
                if chars.get(index + 1) == Some(&'{') {
                    out.push('{');
                    index += 2;
                    continue;
                }
                let start = index + 1;
                let mut end = start;
                let mut depth = 1usize;
                while end < chars.len() {
                    match chars[end] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    end += 1;
                }
                if end >= chars.len() {
                    return Err(interp.err(
                        "ValueError",
                        String::from("expected '}' before end of string"),
                    ));
                }
                let content: String = chars[start..end].iter().collect();
                if content.contains('{') || content.contains('}') {
                    return Err(interp.err(
                        "ValueError",
                        String::from("nested replacement fields are not supported"),
                    ));
                }
                let (field, spec) = match find_top_level(&content.chars().collect::<Vec<_>>(), ':')
                {
                    Some(position) => {
                        let chars: Vec<char> = content.chars().collect();
                        (
                            chars[..position].iter().collect::<String>(),
                            Some(chars[position + 1..].iter().collect::<String>()),
                        )
                    }
                    None => (content, None),
                };
                let field_chars: Vec<char> = field.chars().collect();
                let (name, conversion) = match find_top_level(&field_chars, '!') {
                    Some(position) => {
                        let conversion: Vec<char> = field_chars[position + 1..].to_vec();
                        if conversion.is_empty() {
                            return Err(interp.err(
                                "ValueError",
                                String::from(
                                    "end of string while looking for conversion specifier",
                                ),
                            ));
                        }
                        if conversion.len() != 1 || !matches!(conversion[0], 'r' | 's' | 'a') {
                            return Err(interp.err(
                                "ValueError",
                                format!(
                                    "unknown conversion specifier '{}'",
                                    conversion.iter().collect::<String>()
                                ),
                            ));
                        }
                        (
                            field_chars[..position].iter().collect::<String>(),
                            Some(conversion[0]),
                        )
                    }
                    None => (field, None),
                };
                let value = resolve_format_field(
                    interp,
                    &name,
                    args,
                    kwargs,
                    &mut automatic,
                    &mut used_automatic,
                    &mut used_manual,
                )?;
                let formatted = format_value(interp, &value, spec.as_deref(), conversion)?;
                out.push_str(&formatted);
                index = end + 1;
            }
            '}' => {
                if chars.get(index + 1) == Some(&'}') {
                    out.push('}');
                    index += 2;
                } else {
                    return Err(interp.err(
                        "ValueError",
                        String::from("Single '}' encountered in format string"),
                    ));
                }
            }
            ch => {
                out.push(ch);
                index += 1;
            }
        }
    }
    Ok(Value::Str(Rc::from(out.as_str())))
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
    Ok(Value::DictView(Rc::new(crate::value::DictView {
        entries,
        kind: crate::value::DictViewKind::Keys,
    })))
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
    Ok(Value::DictView(Rc::new(crate::value::DictView {
        entries,
        kind: crate::value::DictViewKind::Values,
    })))
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
    Ok(Value::DictView(Rc::new(crate::value::DictView {
        entries,
        kind: crate::value::DictViewKind::Items,
    })))
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

// ── os module ──────────────────────────────────────────────────────────────

fn os_failure(
    interp: &Interp,
    operation: &str,
    path: &str,
    message: String,
) -> crate::error::PyError {
    let name = if platform::path_exists(path) {
        "OSError"
    } else {
        "FileNotFoundError"
    };
    interp.err(name, format!("{} '{}': {}", operation, path, message))
}

fn os_getpid(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "getpid")?;
    expect_args(interp, args, "getpid", 0, 0)?;
    Ok(Value::Int(platform::getpid()))
}

fn os_getppid(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "getppid")?;
    expect_args(interp, args, "getppid", 0, 0)?;
    Ok(Value::Int(platform::getppid()))
}

fn os_getcwd(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "getcwd")?;
    expect_args(interp, args, "getcwd", 0, 0)?;
    Ok(Value::Str(Rc::from(platform::current_dir().as_str())))
}

fn os_listdir(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "listdir")?;
    expect_args(interp, args, "listdir", 0, 1)?;
    let path = match args.get(0) {
        Some(Value::Str(text)) => String::from(&**text),
        Some(other) => {
            return Err(interp.err(
                "TypeError",
                format!("listdir() path must be str, not '{}'", other.type_name()),
            ))
        }
        None => String::from("."),
    };
    match platform::list_dir(&path) {
        Ok(entries) => Ok(Value::list(
            entries
                .into_iter()
                .map(|(name, _)| Value::Str(Rc::from(name.as_str())))
                .collect(),
        )),
        Err(message) => Err(os_failure(interp, "listdir", &path, message)),
    }
}

fn os_mkdir(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "mkdir")?;
    expect_args(interp, args, "mkdir", 1, 2)?;
    let path = as_str(interp, argument(interp, args, 0, "mkdir")?, "mkdir() path")?;
    if platform::path_exists(path) {
        return Err(interp.err(
            "FileExistsError",
            format!("mkdir '{}': path already exists", path),
        ));
    }
    match platform::create_dir(path) {
        Ok(()) => Ok(Value::None),
        Err(message) => Err(interp.err("OSError", format!("mkdir '{}': {}", path, message))),
    }
}

fn os_makedirs(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    check_kwargs(interp, kwargs, "makedirs", &["mode", "exist_ok"])?;
    expect_args(interp, args, "makedirs", 1, 3)?;
    let path = String::from(as_str(
        interp,
        argument(interp, args, 0, "makedirs")?,
        "makedirs() path",
    )?);
    let exist_ok = match (args.get(2), keyword(kwargs, "exist_ok")) {
        (Some(_), Some(_)) => {
            return Err(interp.err(
                "TypeError",
                String::from("makedirs() got multiple values for argument 'exist_ok'"),
            ))
        }
        (Some(value), None) => value.truthy(),
        (None, Some(value)) => value.truthy(),
        (None, None) => false,
    };
    if platform::path_exists(&path) {
        if exist_ok {
            return Ok(Value::None);
        }
        return Err(interp.err(
            "FileExistsError",
            format!("makedirs '{}': path already exists", path),
        ));
    }
    match platform::create_dir_all(&path) {
        Ok(()) => Ok(Value::None),
        Err(message) => Err(interp.err("OSError", format!("makedirs '{}': {}", path, message))),
    }
}

fn os_remove(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "remove")?;
    expect_args(interp, args, "remove", 1, 1)?;
    let path = as_str(
        interp,
        argument(interp, args, 0, "remove")?,
        "remove() path",
    )?;
    match platform::remove_file(path) {
        Ok(()) => Ok(Value::None),
        Err(message) => Err(os_failure(interp, "remove", path, message)),
    }
}

fn os_rmdir(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "rmdir")?;
    expect_args(interp, args, "rmdir", 1, 1)?;
    let path = as_str(interp, argument(interp, args, 0, "rmdir")?, "rmdir() path")?;
    match platform::remove_dir(path) {
        Ok(()) => Ok(Value::None),
        Err(message) => Err(os_failure(interp, "rmdir", path, message)),
    }
}

fn os_rename(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "rename")?;
    expect_args(interp, args, "rename", 2, 2)?;
    let from = as_str(interp, argument(interp, args, 0, "rename")?, "rename() src")?;
    let to = as_str(interp, argument(interp, args, 1, "rename")?, "rename() dst")?;
    match platform::rename(from, to) {
        Ok(()) => Ok(Value::None),
        Err(message) => Err(os_failure(interp, "rename", from, message)),
    }
}

fn os_stat(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "stat")?;
    expect_args(interp, args, "stat", 1, 1)?;
    let path = as_str(interp, argument(interp, args, 0, "stat")?, "stat() path")?;
    match platform::stat_path(path) {
        Ok((size, mode, kind)) => interp.make_stat_value(size, mode, kind),
        Err(message) => Err(os_failure(interp, "stat", path, message)),
    }
}

fn os_fork(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "fork")?;
    expect_args(interp, args, "fork", 0, 0)?;
    match platform::fork_process() {
        Ok(pid) => Ok(Value::Int(pid)),
        Err(message) => Err(interp.err("OSError", format!("fork: {}", message))),
    }
}

fn os_execv(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "execv")?;
    expect_args(interp, args, "execv", 2, 2)?;
    let path = String::from(as_str(
        interp,
        argument(interp, args, 0, "execv")?,
        "execv() path",
    )?);
    let source = argument(interp, args, 1, "execv")?.clone();
    let values = interp.collect_iterable(source)?;
    let mut arguments: Vec<String> = Vec::with_capacity(values.len());
    for value in values {
        match &value {
            Value::Str(text) => arguments.push(String::from(&**text)),
            other => {
                return Err(interp.err(
                    "TypeError",
                    format!(
                        "execv() arguments must be str, not '{}'",
                        other.type_name()
                    ),
                ))
            }
        }
    }
    match platform::execv_replace(&path, &arguments) {
        Ok(()) => Err(interp.err(
            "OSError",
            format!(
                "execv '{}': execve returned without replacing the process",
                path
            ),
        )),
        Err(message) => Err(interp.err("OSError", format!("execv '{}': {}", path, message))),
    }
}

fn os_waitpid(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "waitpid")?;
    expect_args(interp, args, "waitpid", 1, 2)?;
    let pid = as_int(
        interp,
        argument(interp, args, 0, "waitpid")?,
        "waitpid() pid",
    )?;
    let options = match args.get(1) {
        Some(value) => as_int(interp, value, "waitpid() options")?,
        None => 0,
    };
    match platform::wait_child(pid, options & 1 != 0) {
        Ok((child, status)) => Ok(Value::tuple(alloc::vec![
            Value::Int(child),
            Value::Int(status as i64),
        ])),
        Err(message) => Err(interp.err("OSError", format!("waitpid: {}", message))),
    }
}

fn os_wait(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "wait")?;
    expect_args(interp, args, "wait", 0, 0)?;
    match platform::wait_child(-1, false) {
        Ok((child, status)) => Ok(Value::tuple(alloc::vec![
            Value::Int(child),
            Value::Int(status as i64),
        ])),
        Err(message) => Err(interp.err("OSError", format!("wait: {}", message))),
    }
}

fn os_system(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "system")?;
    expect_args(interp, args, "system", 1, 1)?;
    Err(interp.err(
        "NotImplementedError",
        String::from(
            "os.system() needs a shell executable, and pagh has none; \
             use os.fork() + os.execv() instead",
        ),
    ))
}

// ── json module ────────────────────────────────────────────────────────────

fn json_dumps(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "dumps")?;
    expect_args(interp, args, "dumps", 1, 1)?;
    let value = argument(interp, args, 0, "dumps")?;
    match crate::json::dumps(value) {
        Ok(text) => Ok(Value::Str(Rc::from(text.as_str()))),
        Err(message) => {
            let name = if message.contains("nesting") {
                "ValueError"
            } else {
                "TypeError"
            };
            Err(interp.err(name, message))
        }
    }
}

fn json_loads(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "loads")?;
    expect_args(interp, args, "loads", 1, 1)?;
    let value = argument(interp, args, 0, "loads")?;
    let text = match value {
        Value::Str(text) => String::from(&**text),
        Value::Bytes(data) => match core::str::from_utf8(data) {
            Ok(text) => String::from(text),
            Err(error) => {
                return Err(interp.err(
                    "UnicodeDecodeError",
                    format!("invalid UTF-8 in json.loads at offset {}", error.valid_up_to()),
                ))
            }
        },
        other => {
            return Err(interp.err(
                "TypeError",
                format!(
                    "the JSON object must be str or bytes, not '{}'",
                    other.type_name()
                ),
            ))
        }
    };
    match crate::json::loads(&text) {
        Ok(value) => Ok(value),
        Err(message) => Err(interp.err("JSONDecodeError", message)),
    }
}

// ── set methods ────────────────────────────────────────────────────────────

fn set_receiver(
    interp: &Interp,
    builtin: &BuiltinValue,
    function: &str,
) -> PyResult<Rc<RefCell<Vec<Value>>>> {
    match receiver(interp, builtin, function)? {
        Value::Set(items) => Ok(items.clone()),
        other => Err(interp.err(
            "TypeError",
            format!(
                "{}() receiver must be set, not '{}'",
                function,
                other.type_name()
            ),
        )),
    }
}

fn require_set_hashable(interp: &Interp, value: &Value) -> PyResult<()> {
    if value.is_hashable() {
        Ok(())
    } else {
        Err(interp.err(
            "TypeError",
            format!("unhashable type: '{}'", value.type_name()),
        ))
    }
}

fn set_add(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "add")?;
    expect_args(interp, args, "add", 1, 1)?;
    let items = set_receiver(interp, builtin, "add")?;
    let value = argument(interp, args, 0, "add")?.clone();
    require_set_hashable(interp, &value)?;
    set_insert(&mut items.borrow_mut(), value);
    Ok(Value::None)
}

fn set_remove(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "remove")?;
    expect_args(interp, args, "remove", 1, 1)?;
    let items = set_receiver(interp, builtin, "remove")?;
    let value = argument(interp, args, 0, "remove")?.clone();
    let mut borrowed = items.borrow_mut();
    match set_find(&borrowed, &value) {
        Some(index) => {
            borrowed.remove(index);
            Ok(Value::None)
        }
        None => Err(interp.err("KeyError", value.to_repr())),
    }
}

fn set_discard(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "discard")?;
    expect_args(interp, args, "discard", 1, 1)?;
    let items = set_receiver(interp, builtin, "discard")?;
    let value = argument(interp, args, 0, "discard")?.clone();
    let mut borrowed = items.borrow_mut();
    if let Some(index) = set_find(&borrowed, &value) {
        borrowed.remove(index);
    }
    Ok(Value::None)
}

fn set_union(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "union")?;
    let items = set_receiver(interp, builtin, "union")?;
    let mut result = items.borrow().clone();
    for index in 0..args.len() {
        let source = argument(interp, args, index, "union")?.clone();
        for value in interp.collect_iterable(source)? {
            require_set_hashable(interp, &value)?;
            set_insert(&mut result, value);
        }
    }
    Ok(Value::set(result))
}

fn set_intersection(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "intersection")?;
    let items = set_receiver(interp, builtin, "intersection")?;
    let mut others: Vec<Vec<Value>> = Vec::with_capacity(args.len());
    for index in 0..args.len() {
        let source = argument(interp, args, index, "intersection")?.clone();
        others.push(interp.collect_iterable(source)?);
    }
    let result: Vec<Value> = items
        .borrow()
        .iter()
        .filter(|value| others.iter().all(|other| set_find(other, value).is_some()))
        .cloned()
        .collect();
    Ok(Value::set(result))
}

fn set_difference(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "difference")?;
    let items = set_receiver(interp, builtin, "difference")?;
    let mut others: Vec<Vec<Value>> = Vec::with_capacity(args.len());
    for index in 0..args.len() {
        let source = argument(interp, args, index, "difference")?.clone();
        others.push(interp.collect_iterable(source)?);
    }
    let result: Vec<Value> = items
        .borrow()
        .iter()
        .filter(|value| others.iter().all(|other| set_find(other, value).is_none()))
        .cloned()
        .collect();
    Ok(Value::set(result))
}

fn set_issubset(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "issubset")?;
    expect_args(interp, args, "issubset", 1, 1)?;
    let items = set_receiver(interp, builtin, "issubset")?;
    let source = argument(interp, args, 0, "issubset")?.clone();
    let other = interp.collect_iterable(source)?;
    let subset = items
        .borrow()
        .iter()
        .all(|value| set_find(&other, value).is_some());
    Ok(Value::Bool(subset))
}

fn set_issuperset(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "issuperset")?;
    expect_args(interp, args, "issuperset", 1, 1)?;
    let items = set_receiver(interp, builtin, "issuperset")?;
    let source = argument(interp, args, 0, "issuperset")?.clone();
    let other = interp.collect_iterable(source)?;
    let borrowed = items.borrow();
    let superset = other
        .iter()
        .all(|value| set_find(&borrowed, value).is_some());
    Ok(Value::Bool(superset))
}

fn set_pop(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "pop")?;
    expect_args(interp, args, "pop", 0, 0)?;
    let items = set_receiver(interp, builtin, "pop")?;
    let mut borrowed = items.borrow_mut();
    if borrowed.is_empty() {
        return Err(interp.err("KeyError", String::from("pop from an empty set")));
    }
    // This implementation keeps insertion order, so `pop()` removes the
    // first element, like CPython's arbitrary-but-deterministic set order
    // would pick *some* element; scripts must not depend on which one.
    Ok(borrowed.remove(0))
}

fn set_update(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "update")?;
    let items = set_receiver(interp, builtin, "update")?;
    for index in 0..args.len() {
        let source = argument(interp, args, index, "update")?.clone();
        for value in interp.collect_iterable(source)? {
            require_set_hashable(interp, &value)?;
            set_insert(&mut items.borrow_mut(), value);
        }
    }
    Ok(Value::None)
}

enum SetUpdateKind {
    Difference,
    Intersection,
    SymmetricDifference,
}

/// The `*_update` family: mutate the receiver (returning None) the way
/// CPython's in-place set methods do, so aliases see the change.
fn set_update_like(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
    kind: SetUpdateKind,
) -> PyResult<Value> {
    let name = match kind {
        SetUpdateKind::Difference => "difference_update",
        SetUpdateKind::Intersection => "intersection_update",
        SetUpdateKind::SymmetricDifference => "symmetric_difference_update",
    };
    no_kwargs(interp, kwargs, name)?;
    let items = set_receiver(interp, builtin, name)?;
    if args.len() == 0 {
        return Ok(Value::None);
    }
    let mut others: Vec<Vec<Value>> = Vec::with_capacity(args.len());
    for index in 0..args.len() {
        let source = argument(interp, args, index, name)?.clone();
        others.push(interp.collect_iterable(source)?);
    }
    let mut borrowed = items.borrow_mut();
    match kind {
        SetUpdateKind::Difference => {
            borrowed.retain(|value| others.iter().all(|other| set_find(other, value).is_none()));
        }
        SetUpdateKind::Intersection => {
            borrowed.retain(|value| others.iter().all(|other| set_find(other, value).is_some()));
        }
        SetUpdateKind::SymmetricDifference => {
            let removed: Vec<Value> = borrowed
                .iter()
                .filter(|value| set_find(&others[0], value).is_some())
                .cloned()
                .collect();
            borrowed.retain(|value| !removed.iter().any(|gone| values_equal(gone, value)));
            for value in others[0].iter() {
                if !removed.iter().any(|gone| values_equal(gone, value)) {
                    set_insert(&mut borrowed, value.clone());
                }
            }
        }
    }
    Ok(Value::None)
}

fn set_symmetric_difference(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "symmetric_difference")?;
    expect_args(interp, args, "symmetric_difference", 1, 1)?;
    let items = set_receiver(interp, builtin, "symmetric_difference")?;
    let source = argument(interp, args, 0, "symmetric_difference")?.clone();
    let other = interp.collect_iterable(source)?;
    let borrowed = items.borrow();
    let mut result = Vec::new();
    for value in borrowed.iter() {
        if set_find(&other, value).is_none() {
            set_insert(&mut result, value.clone());
        }
    }
    for value in other.iter() {
        if set_find(&borrowed, value).is_none() {
            set_insert(&mut result, value.clone());
        }
    }
    Ok(Value::set(result))
}

fn set_isdisjoint(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "isdisjoint")?;
    expect_args(interp, args, "isdisjoint", 1, 1)?;
    let items = set_receiver(interp, builtin, "isdisjoint")?;
    let source = argument(interp, args, 0, "isdisjoint")?.clone();
    let other = interp.collect_iterable(source)?;
    let borrowed = items.borrow();
    let disjoint = borrowed
        .iter()
        .all(|value| set_find(&other, value).is_none());
    Ok(Value::Bool(disjoint))
}

fn set_copy(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "copy")?;
    expect_args(interp, args, "copy", 0, 0)?;
    let items = set_receiver(interp, builtin, "copy")?;
    let snapshot = items.borrow().clone();
    Ok(Value::set(snapshot))
}

fn set_clear(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "clear")?;
    expect_args(interp, args, "clear", 0, 0)?;
    let items = set_receiver(interp, builtin, "clear")?;
    items.borrow_mut().clear();
    Ok(Value::None)
}

// ── bytes methods ──────────────────────────────────────────────────────────

fn bytes_receiver<'a>(
    interp: &Interp,
    builtin: &'a BuiltinValue,
    function: &str,
) -> PyResult<&'a Rc<Vec<u8>>> {
    match receiver(interp, builtin, function)? {
        Value::Bytes(data) => Ok(data),
        other => Err(interp.err(
            "TypeError",
            format!(
                "{}() receiver must be bytes, not '{}'",
                function,
                other.type_name()
            ),
        )),
    }
}

fn decode_arguments(interp: &Interp, args: &ArgList, function: &str) -> PyResult<()> {
    if args.len() > 2 {
        return Err(interp.err(
            "TypeError",
            format!("{}() takes at most 2 arguments", function),
        ));
    }
    if let Some(value) = args.get(0) {
        match value {
            Value::Str(name) if is_utf8_name(name) => {}
            Value::Str(name) => {
                return Err(interp.err("LookupError", format!("unknown encoding: {}", name)))
            }
            other => {
                return Err(interp.err(
                    "TypeError",
                    format!(
                        "{}() encoding must be str, not '{}'",
                        function,
                        other.type_name()
                    ),
                ))
            }
        }
    }
    if let Some(value) = args.get(1) {
        match value {
            Value::Str(name) if &**name == "strict" => {}
            Value::Str(name) => {
                return Err(interp.err(
                    "NotImplementedError",
                    format!(
                        "{}() error handler '{}' is not supported; only 'strict'",
                        function, name
                    ),
                ))
            }
            other => {
                return Err(interp.err(
                    "TypeError",
                    format!(
                        "{}() errors must be str, not '{}'",
                        function,
                        other.type_name()
                    ),
                ))
            }
        }
    }
    Ok(())
}

fn bytes_decode(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "decode")?;
    decode_arguments(interp, args, "decode")?;
    let data = bytes_receiver(interp, builtin, "decode")?;
    match core::str::from_utf8(data) {
        Ok(text) => Ok(Value::str(text)),
        Err(error) => Err(interp.err(
            "UnicodeDecodeError",
            format!(
                "'utf-8' codec can't decode byte at offset {}: invalid UTF-8",
                error.valid_up_to()
            ),
        )),
    }
}

fn bytes_hex(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "hex")?;
    expect_args(interp, args, "hex", 0, 0)?;
    let data = bytes_receiver(interp, builtin, "hex")?;
    let mut out = String::with_capacity(data.len() * 2);
    for byte in data.iter() {
        let _ = core::fmt::Write::write_fmt(&mut out, format_args!("{:02x}", byte));
    }
    Ok(Value::Str(Rc::from(out.as_str())))
}

fn str_encode(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "encode")?;
    decode_arguments(interp, args, "encode")?;
    let text = str_receiver(interp, builtin, "encode")?;
    Ok(Value::bytes(text.as_bytes().to_vec()))
}

fn is_utf8_name(name: &str) -> bool {
    matches!(name, "utf-8" | "utf8" | "UTF-8" | "UTF8" | "utf_8")
}

// ── map ────────────────────────────────────────────────────────────────────

/// `map(function, iterable, ...)`.
///
/// This implementation is eager: it collects into a list instead of returning
/// a lazy iterator. The values and the call semantics are Python's; only the
/// evaluation time differs, which the README documents.
/// `staticmethod(function)`: the decorator itself.
///
/// The interpreter binds a plain `Value::Function` found on a class as a
/// method, so this returns a small wrapper the attribute path passes through
/// untouched. `@staticmethod` therefore behaves like the real thing for
/// methods that do not mention `self`, without a descriptor protocol.
fn builtin_staticmethod(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "staticmethod")?;
    expect_args(interp, args, "staticmethod", 1, 1)?;
    let function = argument(interp, args, 0, "staticmethod")?.clone();
    Ok(Value::Builtin(Rc::new(BuiltinValue {
        id: BuiltinId::StaticMethodCall,
        receiver: Some(function),
    })))
}

fn builtin_map(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "map")?;
    if args.len() < 2 {
        return Err(interp.err(
            "TypeError",
            format!("map() expected at least 2 arguments, got {}", args.len()),
        ));
    }
    let function = argument(interp, args, 0, "map")?.clone();
    let mut sources: Vec<Vec<Value>> = Vec::with_capacity(args.len() - 1);
    for index in 1..args.len() {
        let iterable = argument(interp, args, index, "map")?.clone();
        sources.push(interp.collect_iterable(iterable)?);
    }
    let count = sources.iter().map(|values| values.len()).min().unwrap_or(0);
    let mut out = Vec::with_capacity(count);
    for position in 0..count {
        let mut call_args = ArgList::new();
        for source in &sources {
            if let Some(value) = source.get(position) {
                call_args.push(value.clone());
            }
        }
        out.push(interp.call_value(function.clone(), call_args, Vec::new())?);
    }
    Ok(Value::list(out))
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
    if borrowed.binary {
        match borrowed.read_bytes(count) {
            Ok(data) => Ok(Value::bytes(data)),
            Err(message) => {
                drop(borrowed);
                Err(file_error(interp, message))
            }
        }
    } else {
        match borrowed.read(count) {
            Ok(text) => Ok(Value::Str(Rc::from(text.as_str()))),
            Err(message) => {
                drop(borrowed);
                Err(file_error(interp, message))
            }
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
    if borrowed.binary {
        let result = match args.get(0) {
            None => borrowed.readline_bytes(),
            Some(value) => {
                let size = as_int(interp, value, "readline() size")?;
                let mut out = Vec::new();
                let mut taken = 0i64;
                let mut error = None;
                while size < 0 || taken < size {
                    match borrowed.read_bytes(Some(1)) {
                        Ok(data) => {
                            if data.is_empty() {
                                break;
                            }
                            let newline = data[0] == b'\n';
                            out.extend_from_slice(&data);
                            taken += 1;
                            if newline {
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
        return match result {
            Ok(data) => Ok(Value::bytes(data)),
            Err(message) => {
                drop(borrowed);
                Err(file_error(interp, message))
            }
        };
    }
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
                        let newline = text == "\n";
                        out.push_str(&text);
                        taken += 1;
                        if newline {
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
    let binary = file.borrow().binary;
    let mut out = Vec::new();
    let mut total = 0i64;
    loop {
        let (line, more) = {
            let mut borrowed = file.borrow_mut();
            if binary {
                match borrowed.readline_bytes() {
                    Ok(line) => {
                        let more = !line.is_empty();
                        total += line.len() as i64;
                        (Value::bytes(line), more)
                    }
                    Err(message) => return Err(file_error(interp, message)),
                }
            } else {
                match borrowed.readline() {
                    Ok(line) => {
                        let more = !line.is_empty();
                        total += line.chars().count() as i64;
                        (Value::Str(Rc::from(line.as_str())), more)
                    }
                    Err(message) => return Err(file_error(interp, message)),
                }
            }
        };
        if !more {
            break;
        }
        out.push(line);
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
    let value = argument(interp, args, 0, "write")?;
    let mut borrowed = file.borrow_mut();
    let result = if borrowed.binary {
        match value {
            Value::Bytes(data) => borrowed.write_bytes(data),
            other => {
                return Err(interp.err(
                    "TypeError",
                    format!(
                        "write() argument must be bytes, not '{}' (binary file)",
                        other.type_name()
                    ),
                ))
            }
        }
    } else {
        match value {
            Value::Str(text) => borrowed.write(text),
            other => {
                return Err(interp.err(
                    "TypeError",
                    format!(
                        "write() argument must be str, not '{}' (text file)",
                        other.type_name()
                    ),
                ))
            }
        }
    };
    match result {
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
    let binary = file.borrow().binary;
    for line in lines {
        let mut borrowed = file.borrow_mut();
        let result = if binary {
            match &line {
                Value::Bytes(data) => borrowed.write_bytes(data).map(|_| ()),
                other => {
                    return Err(interp.err(
                        "TypeError",
                        format!(
                            "writelines() argument must be bytes-like, not '{}'",
                            other.type_name()
                        ),
                    ))
                }
            }
        } else {
            match &line {
                Value::Str(text) => borrowed.write(text).map(|_| ()),
                other => {
                    return Err(interp.err(
                        "TypeError",
                        format!(
                            "writelines() argument must be str, not '{}'",
                            other.type_name()
                        ),
                    ))
                }
            }
        };
        if let Err(message) = result {
            drop(borrowed);
            return Err(file_error(interp, message));
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
        if let Err(message) = platform::write_bytes(&borrowed.name, &borrowed.data) {
            drop(borrowed);
            return Err(file_error(interp, message));
        }
        borrowed.dirty = false;
    }
    Ok(Value::None)
}

fn file_enter(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "__enter__")?;
    expect_args(interp, args, "__enter__", 0, 0)?;
    Ok(receiver(interp, builtin, "__enter__")?.clone())
}

fn file_exit(
    interp: &mut Interp,
    builtin: &BuiltinValue,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "__exit__")?;
    expect_args(interp, args, "__exit__", 3, 3)?;
    let file = file_receiver(interp, builtin, "__exit__")?;
    let mut borrowed = file.borrow_mut();
    match borrowed.close() {
        // CPython's file.__exit__ returns None, which is false: an exception
        // inside `with open(...)` is never suppressed.
        Ok(()) => Ok(Value::None),
        Err(message) => {
            drop(borrowed);
            Err(file_error(interp, message))
        }
    }
}

// ── os.path ────────────────────────────────────────────────────────────────
//
// `os.path` is a module object inside `os`, built by `Interp::path_module`.
// The functions are pure string/stat helpers over the one path separator this
// ABI has, so they live beside the platform calls they wrap.

/// `os.path.join(*parts)`: join with `/`, restarting at an absolute part.
fn path_join(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "os.path.join")?;
    expect_args(interp, args, "os.path.join", 1, usize::MAX)?;
    let mut joined = String::new();
    for index in 0..args.len() {
        let part = as_str(
            interp,
            argument(interp, args, index, "os.path.join")?,
            "path",
        )?;
        if part.starts_with('/') || joined.is_empty() {
            joined = String::from(part);
        } else if part.is_empty() {
            continue;
        } else if joined.ends_with('/') {
            joined.push_str(part);
        } else {
            joined.push('/');
            joined.push_str(part);
        }
    }
    Ok(Value::Str(Rc::from(joined.as_str())))
}

fn path_basename(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "os.path.basename")?;
    expect_args(interp, args, "os.path.basename", 1, 1)?;
    let path = as_str(
        interp,
        argument(interp, args, 0, "os.path.basename")?,
        "path",
    )?;
    Ok(Value::str(split_path(path).1))
}

fn path_dirname(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "os.path.dirname")?;
    expect_args(interp, args, "os.path.dirname", 1, 1)?;
    let path = as_str(
        interp,
        argument(interp, args, 0, "os.path.dirname")?,
        "path",
    )?;
    Ok(Value::str(split_path(path).0))
}

/// Split `path` into (dirname, basename) following POSIX rules: trailing
/// slashes belong to neither, and the root's dirname is itself.
fn split_path(path: &str) -> (&str, &str) {
    if path.is_empty() {
        return ("", "");
    }
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        // "/" or "///": the whole path is the root.
        return ("/", "");
    }
    match trimmed.rfind('/') {
        None => ("", trimmed),
        Some(0) => ("/", &trimmed[1..]),
        Some(index) => (&trimmed[..index], &trimmed[index + 1..]),
    }
}

fn path_exists(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "os.path.exists")?;
    expect_args(interp, args, "os.path.exists", 1, 1)?;
    let path = as_str(interp, argument(interp, args, 0, "os.path.exists")?, "path")?;
    Ok(Value::Bool(platform::path_exists(path)))
}

fn path_isfile(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "os.path.isfile")?;
    expect_args(interp, args, "os.path.isfile", 1, 1)?;
    let path = as_str(interp, argument(interp, args, 0, "os.path.isfile")?, "path")?;
    Ok(Value::Bool(path_kind(path) == Some(0)))
}

fn path_isdir(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "os.path.isdir")?;
    expect_args(interp, args, "os.path.isdir", 1, 1)?;
    let path = as_str(interp, argument(interp, args, 0, "os.path.isdir")?, "path")?;
    Ok(Value::Bool(path_kind(path) == Some(1)))
}

/// The ABI kind of `path` (0 file, 1 directory, 2 symlink), or `None` when it
/// does not exist.
fn path_kind(path: &str) -> Option<u32> {
    match platform::stat_path(path) {
        Ok((_, _, kind)) => Some(kind),
        Err(_) => None,
    }
}

// ── time ───────────────────────────────────────────────────────────────────

fn time_time(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "time.time")?;
    expect_args(interp, args, "time.time", 0, 0)?;
    match platform::realtime_seconds() {
        Ok(seconds) => Ok(Value::Float(seconds)),
        Err(message) => Err(interp.err("OSError", message)),
    }
}

fn time_monotonic(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "time.monotonic")?;
    expect_args(interp, args, "time.monotonic", 0, 0)?;
    match platform::monotonic_seconds() {
        Ok(seconds) => Ok(Value::Float(seconds)),
        Err(message) => Err(interp.err("OSError", message)),
    }
}

fn time_sleep(interp: &mut Interp, args: &ArgList, kwargs: &[(Rc<str>, Value)]) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "time.sleep")?;
    expect_args(interp, args, "time.sleep", 1, 1)?;
    let seconds = as_float(interp, argument(interp, args, 0, "time.sleep")?, "seconds")?;
    if !(seconds >= 0.0) {
        return Err(interp.err(
            "ValueError",
            String::from("sleep length must be non-negative"),
        ));
    }
    match platform::sleep_seconds(seconds) {
        Ok(()) => Ok(Value::None),
        Err(message) => Err(interp.err("OSError", message)),
    }
}

// ── random ─────────────────────────────────────────────────────────────────
//
// A deterministic xorshift64\* generator seeded with a fixed constant, so a
// script that never calls `seed()` produces the same sequence on every boot.
// This is a test-friendly choice, not a cryptographic one, and the module says
// so in the README.

fn random_step(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

fn random_integer(interp: &mut Interp, high: i64) -> PyResult<i64> {
    if high <= 0 {
        return Err(interp.err("ValueError", String::from("empty range for randrange()")));
    }
    // Modulo bias is irrelevant for the scripts this interpreter runs and is
    // documented; the alternative is rejection sampling for a negligible gain.
    let draw = random_step(&mut interp.rng_state) >> 11;
    Ok((draw as i64) % high)
}

fn random_random(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "random.random")?;
    expect_args(interp, args, "random.random", 0, 0)?;
    let draw = random_step(&mut interp.rng_state) >> 11;
    Ok(Value::Float(draw as f64 / (1u64 << 53) as f64))
}

fn random_seed(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "random.seed")?;
    expect_args(interp, args, "random.seed", 1, 1)?;
    let value = as_int(interp, argument(interp, args, 0, "random.seed")?, "seed")?;
    interp.rng_state = if value == 0 {
        0x9E37_79B9_7F4A_7C15
    } else {
        value as u64
    };
    Ok(Value::None)
}

fn random_randint(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "random.randint")?;
    expect_args(interp, args, "random.randint", 2, 2)?;
    let low = as_int(interp, argument(interp, args, 0, "random.randint")?, "a")?;
    let high = as_int(interp, argument(interp, args, 1, "random.randint")?, "b")?;
    if low > high {
        return Err(interp.err("ValueError", String::from("empty range for randint()")));
    }
    let span = (high as i128) - (low as i128) + 1;
    if span > i64::MAX as i128 {
        return Err(interp.err(
            "OverflowError",
            String::from("randint() range is too large"),
        ));
    }
    let offset = random_integer(interp, span as i64)?;
    Ok(Value::Int(low + offset))
}

fn random_uniform(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "random.uniform")?;
    expect_args(interp, args, "random.uniform", 2, 2)?;
    let low = as_float(interp, argument(interp, args, 0, "random.uniform")?, "a")?;
    let high = as_float(interp, argument(interp, args, 1, "random.uniform")?, "b")?;
    let draw = random_step(&mut interp.rng_state) >> 11;
    let unit = draw as f64 / (1u64 << 53) as f64;
    Ok(Value::Float(low + (high - low) * unit))
}

fn random_choice(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "random.choice")?;
    expect_args(interp, args, "random.choice", 1, 1)?;
    let sequence = argument(interp, args, 0, "random.choice")?.clone();
    let items = interp.collect_iterable(sequence)?;
    if items.is_empty() {
        return Err(interp.err("IndexError", String::from("list index out of range")));
    }
    let index = random_integer(interp, items.len() as i64)? as usize;
    Ok(items[index].clone())
}

fn random_shuffle(
    interp: &mut Interp,
    args: &ArgList,
    kwargs: &[(Rc<str>, Value)],
) -> PyResult<Value> {
    no_kwargs(interp, kwargs, "random.shuffle")?;
    expect_args(interp, args, "random.shuffle", 1, 1)?;
    let receiver = argument(interp, args, 0, "random.shuffle")?;
    let Value::List(items) = receiver else {
        return Err(interp.err(
            "TypeError",
            format!(
                "random.shuffle() needs a list, not '{}'",
                receiver.type_name()
            ),
        ));
    };
    let items = items.clone();
    let mut borrowed = items.borrow_mut();
    // Fisher-Yates, in place, exactly like CPython's shuffle.
    for index in (1..borrowed.len()).rev() {
        let swap = random_integer(interp, index as i64 + 1)? as usize;
        borrowed.swap(index, swap);
    }
    Ok(Value::None)
}
