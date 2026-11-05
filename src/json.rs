//! The `json` module: `dumps` and `loads` for the Python value subset.
//!
//! The encoder always produces ASCII (`ensure_ascii=True`, CPython's default),
//! uses `", "` / `": "` separators and keeps dict insertion order. The decoder
//! accepts the standard JSON grammar including `\uXXXX` escapes and surrogate
//! pairs, and rejects trailing data, leading zeros, raw control characters and
//! nesting deeper than [`MAX_DEPTH`]. Errors are messages for `JSONDecodeError`.
//!
//! This is deliberately not a general codec: there are no `default=`,
//! `sort_keys` or `indent` options, and Python objects other than
//! None/bool/int/float/str/list/tuple/dict are `TypeError`s.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::value::Value;

/// Recursion guard for both directions; deeper input is rejected rather than
/// running off the 64 KiB U-mode stack.
pub const MAX_DEPTH: usize = 64;

// ── encoding ───────────────────────────────────────────────────────────────

pub fn dumps(value: &Value) -> Result<String, String> {
    let mut out = String::new();
    write_value(&mut out, value, 0)?;
    Ok(out)
}

fn write_value(out: &mut String, value: &Value, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(String::from("JSON nesting is too deep"));
    }
    match value {
        Value::None => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Int(number) => out.push_str(&format!("{}", number)),
        Value::Float(number) => out.push_str(&format_float(*number)),
        Value::Str(text) => write_string(out, text),
        Value::List(items) => {
            out.push('[');
            for (index, item) in items.borrow().iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_value(out, item, depth + 1)?;
            }
            out.push(']');
        }
        Value::Tuple(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_value(out, item, depth + 1)?;
            }
            out.push(']');
        }
        Value::Dict(entries) => {
            out.push('{');
            for (index, (key, item)) in entries.borrow().iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                match json_key(key) {
                    Some(text) => write_string(out, &text),
                    None => {
                        return Err(format!(
                            "keys must be str, int, float, bool or None, not '{}'",
                            key.type_name()
                        ))
                    }
                }
                out.push_str(": ");
                write_value(out, item, depth + 1)?;
            }
            out.push('}');
        }
        other => {
            return Err(format!(
                "Object of type {} is not JSON serializable",
                other.type_name()
            ))
        }
    }
    Ok(())
}

fn format_float(value: f64) -> String {
    if value.is_nan() {
        return String::from("NaN");
    }
    if value.is_infinite() {
        return String::from(if value < 0.0 { "-Infinity" } else { "Infinity" });
    }
    crate::float::format(value)
}

fn json_key(key: &Value) -> Option<String> {
    match key {
        Value::Str(text) => Some(String::from(&**text)),
        Value::Bool(true) => Some(String::from("true")),
        Value::Bool(false) => Some(String::from("false")),
        Value::None => Some(String::from("null")),
        Value::Int(number) => Some(format!("{}", number)),
        Value::Float(number) => Some(format_float(*number)),
        _ => None,
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) <= 0x7e => out.push(c),
            c => {
                let mut code = c as u32;
                if code > 0xffff {
                    code -= 0x1_0000;
                    let high = 0xd800 + ((code >> 10) & 0x3ff);
                    let low = 0xdc00 + (code & 0x3ff);
                    out.push_str(&format!("\\u{:04x}\\u{:04x}", high, low));
                } else {
                    out.push_str(&format!("\\u{:04x}", code));
                }
            }
        }
    }
    out.push('"');
}

// ── decoding ───────────────────────────────────────────────────────────────

pub fn loads(text: &str) -> Result<Value, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut parser = Parser { chars, pos: 0 };
    parser.skip_whitespace();
    let value = parser.parse_value(0)?;
    parser.skip_whitespace();
    if parser.pos != parser.chars.len() {
        return Err(format!("Extra data at character {}", parser.pos + 1));
    }
    Ok(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        Some(ch)
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn error<T>(&self, message: &str) -> Result<T, String> {
        Err(format!("{} at character {}", message, self.pos + 1))
    }

    fn parse_value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(String::from("JSON nesting is too deep"));
        }
        self.skip_whitespace();
        match self.peek() {
            Some('{') => self.parse_object(depth),
            Some('[') => self.parse_array(depth),
            Some('"') => Ok(Value::Str(alloc::rc::Rc::from(self.parse_string()?.as_str()))),
            Some('t') => {
                self.expect_word("true")?;
                Ok(Value::Bool(true))
            }
            Some('f') => {
                self.expect_word("false")?;
                Ok(Value::Bool(false))
            }
            Some('n') => {
                self.expect_word("null")?;
                Ok(Value::None)
            }
            Some(ch) if ch == '-' || ch.is_ascii_digit() => self.parse_number(),
            Some(_) => self.error("Expecting value"),
            None => self.error("Expecting value"),
        }
    }

    fn expect_word(&mut self, word: &str) -> Result<(), String> {
        for expected in word.chars() {
            match self.bump() {
                Some(actual) if actual == expected => {}
                _ => return self.error("Expecting value"),
            }
        }
        Ok(())
    }

    fn parse_object(&mut self, depth: usize) -> Result<Value, String> {
        self.bump(); // {
        self.skip_whitespace();
        let mut entries: Vec<(Value, Value)> = Vec::new();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Value::dict(entries));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some('"') {
                return self.error("Expecting property name enclosed in double quotes");
            }
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.bump() != Some(':') {
                return self.error("Expecting ':' delimiter");
            }
            let value = self.parse_value(depth + 1)?;
            crate::value::dict_set(&mut entries, Value::Str(alloc::rc::Rc::from(key.as_str())), value);
            self.skip_whitespace();
            match self.bump() {
                Some(',') => continue,
                Some('}') => break,
                _ => return self.error("Expecting ',' delimiter or '}'"),
            }
        }
        Ok(Value::dict(entries))
    }

    fn parse_array(&mut self, depth: usize) -> Result<Value, String> {
        self.bump(); // [
        self.skip_whitespace();
        let mut items = Vec::new();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(Value::list(items));
        }
        loop {
            items.push(self.parse_value(depth + 1)?);
            self.skip_whitespace();
            match self.bump() {
                Some(',') => continue,
                Some(']') => break,
                _ => return self.error("Expecting ',' delimiter or ']'"),
            }
        }
        Ok(Value::list(items))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.bump() != Some('"') {
            return self.error("Expecting string");
        }
        let mut out = String::new();
        loop {
            let Some(ch) = self.bump() else {
                return self.error("Unterminated string");
            };
            match ch {
                '"' => return Ok(out),
                '\\' => {
                    let Some(escape) = self.bump() else {
                        return self.error("Unterminated escape");
                    };
                    match escape {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{08}'),
                        'f' => out.push('\u{0c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let value = self.read_hex4()?;
                            if (0xd800..=0xdbff).contains(&value) {
                                if self.bump() != Some('\\') || self.bump() != Some('u') {
                                    return self.error("Unpaired surrogate in a string");
                                }
                                let low = self.read_hex4()?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return self.error("Unpaired surrogate in a string");
                                }
                                let code =
                                    0x1_0000 + ((value - 0xd800) << 10) + (low - 0xdc00);
                                match char::from_u32(code) {
                                    Some(decoded) => out.push(decoded),
                                    None => return self.error("Invalid Unicode escape"),
                                }
                            } else if (0xdc00..=0xdfff).contains(&value) {
                                return self.error("Unpaired surrogate in a string");
                            } else {
                                match char::from_u32(value) {
                                    Some(decoded) => out.push(decoded),
                                    None => return self.error("Invalid Unicode escape"),
                                }
                            }
                        }
                        _ => return self.error("Invalid escape"),
                    }
                }
                c if (c as u32) < 0x20 => {
                    return self.error("Invalid control character in a string")
                }
                c => out.push(c),
            }
        }
    }

    fn read_hex4(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(ch) = self.bump() else {
                return self.error("Truncated \\u escape");
            };
            let Some(digit) = ch.to_digit(16) else {
                return self.error("Invalid \\u escape");
            };
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        let mut is_float = false;
        if self.peek() == Some('-') {
            self.bump();
        }
        match self.peek() {
            Some('0') => {
                self.bump();
                if matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                    return self.error("Leading zeros are not permitted");
                }
            }
            Some(ch) if ch.is_ascii_digit() => {
                while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                    self.bump();
                }
            }
            _ => return self.error("Expecting value"),
        }
        if self.peek() == Some('.') {
            is_float = true;
            self.bump();
            if !matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                return self.error("Expecting a digit after the decimal point");
            }
            while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                self.bump();
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            is_float = true;
            self.bump();
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.bump();
            }
            if !matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                return self.error("Expecting an exponent");
            }
            while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                self.bump();
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        if is_float {
            match text.parse::<f64>() {
                Ok(number) => Ok(Value::Float(number)),
                Err(_) => self.error("Invalid number"),
            }
        } else {
            match text.parse::<i64>() {
                Ok(number) => Ok(Value::Int(number)),
                Err(_) => self.error("Integer is too large for a 64-bit int"),
            }
        }
    }
}
