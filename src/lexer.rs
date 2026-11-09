//! The lexer: source text to a token stream with Python's INDENT/DEDENT and
//! implicit line joining.
//!
//! Design choices that matter for this kernel:
//!
//! * Source is collected into `Vec<char>` once, so columns are character
//!   positions (not byte offsets) and the rest of the scanner is index-based
//!   without panics. Scripts are small; this is the simplest correct choice.
//! * Tabs expand to the next multiple of 8 for indentation, exactly as
//!   CPython's lexer does when it compares indentation columns. Mixing tabs
//!   and spaces is accepted when the resulting widths are consistent; an
//!   inconsistent dedent is an `IndentationError`.
//! * Newlines inside `()`, `[]`, `{}` are ignored (implicit continuation), and
//!   a backslash-newline continues a logical line outside brackets.
//! * Blank lines and comment-only lines never produce NEWLINE or indentation
//!   tokens, which is what makes blank lines inside a block harmless.
//! * Integer literals are parsed to `i64` and decimal/exponent literals to
//!   `f64` here, so the parser and evaluator never carry numeric text. Rust's
//!   `FromStr` for f64 supplies the correctly rounded conversion; a literal
//!   too large for f64 becomes `inf`, exactly like CPython.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::SyntaxError;

#[derive(Clone, PartialEq, Debug)]
pub enum Tok {
    Name(Rc<str>),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    Bytes(Rc<Vec<u8>>),
    FStr(Rc<Vec<FStringLexPart>>),
    Newline,
    Indent,
    Dedent,
    Eof,
    Op(Op),
    Kw(Kw),
}

/// The lexer's half of an f-string: literal chunks are unescaped here, while
/// expression chunks keep their source text until the parser parses them.
#[derive(Clone, PartialEq, Debug)]
pub enum FStringLexPart {
    Text(Rc<str>),
    Value {
        text: Rc<str>,
        spec: Option<Rc<str>>,
        conversion: Option<char>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Plus,
    Minus,
    Star,
    Slash,
    SlashSlash,
    Percent,
    StarStar,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Assign,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    SlashSlashEq,
    PercentEq,
    StarStarEq,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
    Semi,
    /// `@`, only used by decorators today.
    At,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kw {
    And,
    As,
    Assert,
    Break,
    Class,
    Continue,
    Def,
    Del,
    Elif,
    Else,
    Except,
    False,
    Finally,
    For,
    From,
    Global,
    If,
    Import,
    In,
    Is,
    Lambda,
    None,
    Nonlocal,
    Not,
    Or,
    Pass,
    Raise,
    Return,
    True,
    Try,
    While,
    With,
    Yield,
}

pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub column: u32,
}

pub fn tokenize(source: &str) -> Result<Vec<Token>, SyntaxError> {
    Lexer::new(source).run()
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    line_start: usize,
    indents: Vec<usize>,
    tokens: Vec<Token>,
    depth: usize,
    at_line_start: bool,
    continued: bool,
    last_was_newline: bool,
    emitted_any: bool,
}

impl Lexer {
    fn new(source: &str) -> Lexer {
        Lexer {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            line_start: 0,
            indents: alloc::vec![0],
            tokens: Vec::new(),
            depth: 0,
            at_line_start: true,
            continued: false,
            last_was_newline: true,
            emitted_any: false,
        }
    }

    fn run(mut self) -> Result<Vec<Token>, SyntaxError> {
        loop {
            if self.at_line_start {
                if self.depth > 0 || self.continued {
                    self.skip_inline_space();
                    self.at_line_start = false;
                    self.continued = false;
                } else if self.handle_line_start()? {
                    if self.peek().is_none() {
                        break;
                    }
                    continue;
                }
            }
            self.skip_inline_space();

            let Some(ch) = self.peek() else { break };
            match ch {
                '\n' => {
                    let line = self.line;
                    let column = self.column();
                    self.bump();
                    self.at_line_start = true;
                    if self.depth == 0 && !self.last_was_newline {
                        self.push(Tok::Newline, line, column);
                        self.last_was_newline = true;
                    }
                }
                '#' => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                '\\' if self.peek_at(1) == Some('\n') => {
                    self.bump();
                    self.bump();
                    self.at_line_start = true;
                    self.continued = true;
                }
                '\'' | '"' => {
                    let line = self.line;
                    let column = self.column();
                    let text = self.lex_string_body(false, line, column)?;
                    self.push(Tok::Str(text), line, column);
                }
                '.' if self.peek_at(1).map_or(false, |c| c.is_ascii_digit()) => {
                    let token = self.lex_number()?;
                    self.push(token.0, token.1, token.2);
                }
                c if c.is_ascii_digit() => {
                    let token = self.lex_number()?;
                    self.push(token.0, token.1, token.2);
                }
                c if is_ident_start(c) => self.lex_name_or_prefixed_string()?,
                _ => {
                    let token = self.lex_operator()?;
                    self.push(token.0, token.1, token.2);
                }
            }
        }

        if self.continued {
            return Err(self.error("unexpected end of file after a line continuation"));
        }
        if self.emitted_any && !self.last_was_newline {
            let line = self.line;
            let column = self.column();
            self.push(Tok::Newline, line, column);
        }
        while self.indents.len() > 1 {
            self.indents.pop();
            let line = self.line;
            self.push(Tok::Dedent, line, 1);
        }
        let line = self.line;
        let column = self.column();
        self.push(Tok::Eof, line, column);
        Ok(self.tokens)
    }

    // ── token plumbing ─────────────────────────────────────────────────────

    fn push(&mut self, tok: Tok, line: u32, column: u32) {
        if !matches!(tok, Tok::Newline | Tok::Indent | Tok::Dedent) {
            self.emitted_any = true;
            self.last_was_newline = false;
        }
        self.tokens.push(Token { tok, line, column });
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.line_start = self.pos;
        }
        Some(ch)
    }

    fn column(&self) -> u32 {
        (self.pos - self.line_start + 1) as u32
    }

    fn error(&self, message: &str) -> SyntaxError {
        SyntaxError::new(String::from(message), self.line, self.column())
    }

    fn error_at(&self, message: String, line: u32, column: u32) -> SyntaxError {
        SyntaxError::new(message, line, column)
    }

    fn skip_inline_space(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' || c == '\u{000c}' {
                self.bump();
            } else {
                break;
            }
        }
    }

    // ── indentation ────────────────────────────────────────────────────────

    /// Handle the beginning of a logical line. Returns `Ok(true)` when the line
    /// was blank/comment-only (and was consumed), `Ok(false)` when the caller
    /// should scan the first token.
    fn handle_line_start(&mut self) -> Result<bool, SyntaxError> {
        let line = self.line;
        let mut width = 0usize;
        loop {
            match self.peek() {
                Some(' ') => {
                    width += 1;
                    self.bump();
                }
                Some('\t') => {
                    width = (width / 8 + 1) * 8;
                    self.bump();
                }
                Some('\u{000c}') => {
                    width = 0;
                    self.bump();
                }
                _ => break,
            }
        }

        match self.peek() {
            None => {
                self.at_line_start = true;
                Ok(true)
            }
            Some('\n') => {
                self.bump();
                self.at_line_start = true;
                Ok(true)
            }
            Some('#') => {
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.bump();
                }
                if self.peek() == Some('\n') {
                    self.bump();
                }
                self.at_line_start = true;
                Ok(true)
            }
            Some(_) => {
                let column = self.column();
                let top = *self.indents.last().unwrap_or(&0);
                if width > top {
                    self.indents.push(width);
                    self.push(Tok::Indent, line, column);
                } else if width < top {
                    while *self.indents.last().unwrap_or(&0) > width {
                        self.indents.pop();
                        self.push(Tok::Dedent, line, column);
                    }
                    if *self.indents.last().unwrap_or(&0) != width {
                        return Err(SyntaxError::indentation(
                            String::from(
                                "unindent does not match any outer indentation level",
                            ),
                            line,
                            column,
                        ));
                    }
                }
                self.at_line_start = false;
                Ok(false)
            }
        }
    }

    // ── names, keywords, prefixed strings ──────────────────────────────────

    fn lex_name_or_prefixed_string(&mut self) -> Result<(), SyntaxError> {
        let line = self.line;
        let column = self.column();
        let start = self.pos;
        self.bump();
        while let Some(c) = self.peek() {
            if is_ident_continue(c) {
                self.bump();
            } else {
                break;
            }
        }
        let word: String = self.chars[start..self.pos].iter().collect();

        if let Some(quote) = self.peek() {
            if (quote == '\'' || quote == '"') && is_string_prefix(&word) {
                let lower = word.to_ascii_lowercase();
                let raw = lower.contains('r');
                if lower.contains('f') {
                    let token = self.lex_fstring(raw, line, column)?;
                    self.push(token, line, column);
                    return Ok(());
                }
                if lower.contains('b') {
                    let bytes = self.lex_bytes_literal(raw, line, column)?;
                    self.push(Tok::Bytes(bytes), line, column);
                    return Ok(());
                }
                let text = self.lex_string_body(raw, line, column)?;
                self.push(Tok::Str(text), line, column);
                return Ok(());
            }
        }

        let tok = match keyword(&word) {
            Some(kw) => Tok::Kw(kw),
            None => Tok::Name(Rc::from(word)),
        };
        self.push(tok, line, column);
        Ok(())
    }

    // ── numbers ────────────────────────────────────────────────────────────

    fn lex_number(&mut self) -> Result<(Tok, u32, u32), SyntaxError> {
        let line = self.line;
        let column = self.column();
        let start = self.pos;
        let first = self.peek().unwrap_or('0');

        if first == '.' {
            // `.5` and `.5e3`
            self.bump();
            self.consume_digits();
            if self.exponent_ahead() {
                self.consume_exponent();
            }
            let raw: String = self.chars[start..self.pos].iter().collect();
            return self.float_token(&raw, line, column);
        }

        if first == '0' {
            if let Some(prefix) = self.peek_at(1) {
                let base = match prefix {
                    'x' | 'X' => Some(16),
                    'o' | 'O' => Some(8),
                    'b' | 'B' => Some(2),
                    _ => None,
                };
                if let Some(base) = base {
                    self.bump();
                    self.bump();
                    let digits_start = self.pos;
                    while let Some(c) = self.peek() {
                        if c.is_ascii_alphanumeric() || c == '_' {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    let raw: String = self.chars[digits_start..self.pos].iter().collect();
                    let cleaned: String =
                        raw.chars().filter(|c| *c != '_').collect();
                    let label = match base {
                        16 => "hexadecimal",
                        8 => "octal",
                        _ => "binary",
                    };
                    if cleaned.is_empty() {
                        return Err(self.error_at(
                            format!("missing digits after the {} prefix", label),
                            line,
                            column,
                        ));
                    }
                    return match i64::from_str_radix(&cleaned, base) {
                        Ok(value) => Ok((Tok::Int(value), line, column)),
                        Err(_) => Err(self.error_at(
                            format!(
                                "invalid or too-large {} integer literal '{}{}' \
                                 (this interpreter has 64-bit ints)",
                                label, prefix, raw
                            ),
                            line,
                            column,
                        )),
                    };
                }
            }
        }

        self.consume_digits();
        let mut is_float = false;
        if self.peek() == Some('.') && self.peek_at(1) != Some('.') {
            // `1.` and `1.5` are floats; `1..2` is not valid Python either, so
            // stopping at the second dot is fine.
            is_float = true;
            self.bump();
            self.consume_digits();
        }
        if self.exponent_ahead() {
            is_float = true;
            self.consume_exponent();
        }
        let raw: String = self.chars[start..self.pos].iter().collect();
        if is_float {
            return self.float_token(&raw, line, column);
        }
        let cleaned: String = raw.chars().filter(|c| *c != '_').collect();
        match cleaned.parse::<i64>() {
            Ok(value) => Ok((Tok::Int(value), line, column)),
            Err(_) => Err(self.error_at(
                format!(
                    "integer literal '{}' does not fit in a signed 64-bit int",
                    raw
                ),
                line,
                column,
            )),
        }
    }

    /// Convert a float-shaped literal to f64. `float::parse` strips legal
    /// digit-separating underscores and accepts `.5`/`1.`/`1e-2`; the lexer
    /// only calls it after seeing a decimal point or an exponent.
    fn float_token(
        &self,
        raw: &str,
        line: u32,
        column: u32,
    ) -> Result<(Tok, u32, u32), SyntaxError> {
        match crate::float::parse(raw) {
            Some(value) => Ok((Tok::Float(value), line, column)),
            None => Err(self.error_at(
                format!("invalid floating-point literal '{}'", raw),
                line,
                column,
            )),
        }
    }

    fn consume_digits(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn exponent_ahead(&self) -> bool {
        if !matches!(self.peek(), Some('e') | Some('E')) {
            return false;
        }
        match self.peek_at(1) {
            Some(c) if c.is_ascii_digit() => true,
            Some('+') | Some('-') => {
                matches!(self.peek_at(2), Some(c) if c.is_ascii_digit())
            }
            _ => false,
        }
    }

    fn consume_exponent(&mut self) {
        self.bump(); // e/E
        if matches!(self.peek(), Some('+') | Some('-')) {
            self.bump();
        }
        self.consume_digits();
    }

    // ── strings ────────────────────────────────────────────────────────────

    fn lex_string_body(
        &mut self,
        raw: bool,
        start_line: u32,
        start_column: u32,
    ) -> Result<Rc<str>, SyntaxError> {
        let Some(quote) = self.peek() else {
            return Err(self.error_at(
                String::from("unterminated string literal"),
                start_line,
                start_column,
            ));
        };
        let triple = self.peek_at(1) == Some(quote) && self.peek_at(2) == Some(quote);
        self.bump();
        if triple {
            self.bump();
            self.bump();
        }
        let mut out = String::new();
        loop {
            let Some(ch) = self.peek() else {
                return Err(self.error_at(
                    String::from("unterminated string literal"),
                    start_line,
                    start_column,
                ));
            };
            if ch == quote {
                if triple {
                    if self.peek_at(1) == Some(quote) && self.peek_at(2) == Some(quote) {
                        self.bump();
                        self.bump();
                        self.bump();
                        break;
                    }
                    out.push(ch);
                    self.bump();
                    continue;
                }
                self.bump();
                break;
            }
            if ch == '\n' && !triple {
                return Err(self.error_at(
                    String::from("unterminated string literal (newline in a single-quoted string)"),
                    start_line,
                    start_column,
                ));
            }
            if ch == '\\' {
                if raw {
                    out.push('\\');
                    self.bump();
                    if let Some(next) = self.peek() {
                        out.push(next);
                        self.bump();
                    }
                    continue;
                }
                self.bump();
                let Some(escape) = self.peek() else {
                    return Err(self.error_at(
                        String::from("unterminated escape sequence at end of file"),
                        start_line,
                        start_column,
                    ));
                };
                match escape {
                    'n' => {
                        out.push('\n');
                        self.bump();
                    }
                    't' => {
                        out.push('\t');
                        self.bump();
                    }
                    'r' => {
                        out.push('\r');
                        self.bump();
                    }
                    'a' => {
                        out.push('\u{07}');
                        self.bump();
                    }
                    'b' => {
                        out.push('\u{08}');
                        self.bump();
                    }
                    'f' => {
                        out.push('\u{0c}');
                        self.bump();
                    }
                    'v' => {
                        out.push('\u{0b}');
                        self.bump();
                    }
                    '0' => {
                        out.push('\0');
                        self.bump();
                    }
                    '\\' => {
                        out.push('\\');
                        self.bump();
                    }
                    '\'' => {
                        out.push('\'');
                        self.bump();
                    }
                    '"' => {
                        out.push('"');
                        self.bump();
                    }
                    '\n' => {
                        // Backslash-newline inside a string is a continuation
                        // and contributes no character.
                        self.bump();
                    }
                    'x' => {
                        self.bump();
                        let value = self.read_hex_escape(2, start_line, start_column)?;
                        match char::from_u32(value) {
                            Some(c) => out.push(c),
                            None => {
                                return Err(self.error_at(
                                    String::from("invalid \\x escape value"),
                                    start_line,
                                    start_column,
                                ))
                            }
                        }
                    }
                    'u' => {
                        self.bump();
                        let value = self.read_hex_escape(4, start_line, start_column)?;
                        match char::from_u32(value) {
                            Some(c) => out.push(c),
                            None => {
                                return Err(self.error_at(
                                    String::from("invalid \\u escape value"),
                                    start_line,
                                    start_column,
                                ))
                            }
                        }
                    }
                    'U' => {
                        self.bump();
                        let value = self.read_hex_escape(8, start_line, start_column)?;
                        match char::from_u32(value) {
                            Some(c) => out.push(c),
                            None => {
                                return Err(self.error_at(
                                    String::from("invalid \\U escape value"),
                                    start_line,
                                    start_column,
                                ))
                            }
                        }
                    }
                    other => {
                        // Python leaves unknown escapes as a backslash plus the
                        // character (and warns); keep that behaviour.
                        out.push('\\');
                        out.push(other);
                        self.bump();
                    }
                }
                continue;
            }
            out.push(ch);
            self.bump();
        }
        Ok(Rc::from(out))
    }

    /// Scan a `b"..."` literal. Bytes are ASCII-only on input (as in CPython);
    /// `\xNN` produces one arbitrary byte, and `\u`/`\U` are rejected.
    fn lex_bytes_literal(
        &mut self,
        raw: bool,
        start_line: u32,
        start_column: u32,
    ) -> Result<Rc<Vec<u8>>, SyntaxError> {
        let Some(quote) = self.peek() else {
            return Err(self.error_at(
                String::from("unterminated bytes literal"),
                start_line,
                start_column,
            ));
        };
        let triple = self.peek_at(1) == Some(quote) && self.peek_at(2) == Some(quote);
        self.bump();
        if triple {
            self.bump();
            self.bump();
        }
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(ch) = self.peek() else {
                return Err(self.error_at(
                    String::from("unterminated bytes literal"),
                    start_line,
                    start_column,
                ));
            };
            if ch == quote {
                if triple {
                    if self.peek_at(1) == Some(quote) && self.peek_at(2) == Some(quote) {
                        self.bump();
                        self.bump();
                        self.bump();
                        break;
                    }
                    out.push(ch as u8);
                    self.bump();
                    continue;
                }
                self.bump();
                break;
            }
            if ch == '\n' && !triple {
                return Err(self.error_at(
                    String::from(
                        "unterminated bytes literal (newline in a single-quoted literal)",
                    ),
                    start_line,
                    start_column,
                ));
            }
            if ch == '\\' {
                if raw {
                    out.push(b'\\');
                    self.bump();
                    if let Some(next) = self.peek() {
                        if !next.is_ascii() {
                            return Err(self.error_at(
                                String::from(
                                    "bytes can only contain ASCII literal characters",
                                ),
                                start_line,
                                start_column,
                            ));
                        }
                        out.push(next as u8);
                        self.bump();
                    }
                    continue;
                }
                self.bump();
                let Some(escape) = self.peek() else {
                    return Err(self.error_at(
                        String::from("unterminated escape sequence at end of file"),
                        start_line,
                        start_column,
                    ));
                };
                match escape {
                    'n' => {
                        out.push(b'\n');
                        self.bump();
                    }
                    't' => {
                        out.push(b'\t');
                        self.bump();
                    }
                    'r' => {
                        out.push(b'\r');
                        self.bump();
                    }
                    'a' => {
                        out.push(0x07);
                        self.bump();
                    }
                    'b' => {
                        out.push(0x08);
                        self.bump();
                    }
                    'f' => {
                        out.push(0x0c);
                        self.bump();
                    }
                    'v' => {
                        out.push(0x0b);
                        self.bump();
                    }
                    '0' => {
                        out.push(0);
                        self.bump();
                    }
                    '\\' => {
                        out.push(b'\\');
                        self.bump();
                    }
                    '\'' => {
                        out.push(b'\'');
                        self.bump();
                    }
                    '"' => {
                        out.push(b'"');
                        self.bump();
                    }
                    '\n' => {
                        self.bump();
                    }
                    'x' => {
                        self.bump();
                        let value = self.read_hex_escape(2, start_line, start_column)?;
                        out.push(value as u8);
                    }
                    'u' | 'U' => {
                        return Err(self.error_at(
                            String::from(
                                "\\u and \\U escapes are not allowed in bytes literals; use \\xNN",
                            ),
                            start_line,
                            start_column,
                        ))
                    }
                    other => {
                        if !other.is_ascii() {
                            return Err(self.error_at(
                                String::from(
                                    "bytes can only contain ASCII literal characters",
                                ),
                                start_line,
                                start_column,
                            ));
                        }
                        out.push(b'\\');
                        out.push(other as u8);
                        self.bump();
                    }
                }
                continue;
            }
            if !ch.is_ascii() {
                return Err(self.error_at(
                    String::from("bytes can only contain ASCII literal characters"),
                    start_line,
                    start_column,
                ));
            }
            out.push(ch as u8);
            self.bump();
        }
        Ok(Rc::new(out))
    }

    /// Scan an f-string. Expressions are captured as source text and parsed by
    /// the parser; nested quotes and backslashes inside `{...}` are rejected,
    /// which keeps the scanner independent of the tokenizer recursion.
    fn lex_fstring(
        &mut self,
        raw: bool,
        start_line: u32,
        start_column: u32,
    ) -> Result<Tok, SyntaxError> {
        let Some(quote) = self.peek() else {
            return Err(self.error_at(
                String::from("unterminated f-string literal"),
                start_line,
                start_column,
            ));
        };
        let triple = self.peek_at(1) == Some(quote) && self.peek_at(2) == Some(quote);
        self.bump();
        if triple {
            self.bump();
            self.bump();
        }
        let mut parts: Vec<FStringLexPart> = Vec::new();
        let mut text = String::new();
        loop {
            let Some(ch) = self.peek() else {
                return Err(self.error_at(
                    String::from("unterminated f-string literal"),
                    start_line,
                    start_column,
                ));
            };
            if ch == quote {
                if triple {
                    if self.peek_at(1) == Some(quote) && self.peek_at(2) == Some(quote) {
                        self.bump();
                        self.bump();
                        self.bump();
                        break;
                    }
                    text.push(ch);
                    self.bump();
                    continue;
                }
                self.bump();
                break;
            }
            if ch == '\n' && !triple {
                return Err(self.error_at(
                    String::from(
                        "unterminated f-string literal (newline in a single-quoted f-string)",
                    ),
                    start_line,
                    start_column,
                ));
            }
            if ch == '\\' {
                if raw {
                    text.push('\\');
                    self.bump();
                    if let Some(next) = self.peek() {
                        text.push(next);
                        self.bump();
                    }
                } else {
                    self.read_escape_string(&mut text, start_line, start_column)?;
                }
                continue;
            }
            if ch == '{' {
                if self.peek_at(1) == Some('{') {
                    text.push('{');
                    self.bump();
                    self.bump();
                    continue;
                }
                if !text.is_empty() {
                    parts.push(FStringLexPart::Text(Rc::from(text.as_str())));
                    text.clear();
                }
                self.bump();
                let mut expression = String::new();
                let mut spec: Option<String> = None;
                let mut conversion: Option<char> = None;
                let mut depth = 0usize;
                loop {
                    let Some(current) = self.peek() else {
                        return Err(self.error_at(
                            String::from("unterminated f-string expression"),
                            start_line,
                            start_column,
                        ));
                    };
                    match current {
                        '\n' => {
                            return Err(self.error_at(
                                String::from("f-string expressions cannot span a line"),
                                start_line,
                                start_column,
                            ))
                        }
                        '\'' | '"' => {
                            return Err(self.error_at(
                                String::from(
                                    "nested quotes in f-string expressions are not supported",
                                ),
                                start_line,
                                start_column,
                            ))
                        }
                        '\\' => {
                            return Err(self.error_at(
                                String::from(
                                    "backslashes in f-string expressions are not supported",
                                ),
                                start_line,
                                start_column,
                            ))
                        }
                        '(' | '[' | '{' => {
                            depth += 1;
                            expression.push(current);
                            self.bump();
                        }
                        ')' | ']' => {
                            if depth == 0 {
                                return Err(self.error_at(
                                    String::from(
                                        "unmatched closing bracket in an f-string expression",
                                    ),
                                    start_line,
                                    start_column,
                                ));
                            }
                            depth -= 1;
                            expression.push(current);
                            self.bump();
                        }
                        '}' => {
                            if depth == 0 {
                                self.bump();
                                break;
                            }
                            depth -= 1;
                            expression.push(current);
                            self.bump();
                        }
                        '!' if depth == 0 && self.peek_at(1) != Some('=') => {
                            self.bump();
                            match self.peek() {
                                Some(value @ ('r' | 's' | 'a')) => {
                                    conversion = Some(value);
                                    self.bump();
                                }
                                _ => {
                                    return Err(self.error_at(
                                        String::from(
                                            "expected 'r', 's' or 'a' after '!' in an f-string",
                                        ),
                                        start_line,
                                        start_column,
                                    ))
                                }
                            }
                            if self.peek() == Some(':') {
                                self.bump();
                                spec = Some(self.read_format_spec(start_line, start_column)?);
                            }
                            if self.peek() != Some('}') {
                                return Err(self.error_at(
                                    String::from("expected '}' after an f-string conversion"),
                                    start_line,
                                    start_column,
                                ));
                            }
                            self.bump();
                            break;
                        }
                        ':' if depth == 0 => {
                            self.bump();
                            spec = Some(self.read_format_spec(start_line, start_column)?);
                            if self.peek() != Some('}') {
                                return Err(self.error_at(
                                    String::from("expected '}' after an f-string format spec"),
                                    start_line,
                                    start_column,
                                ));
                            }
                            self.bump();
                            break;
                        }
                        other => {
                            expression.push(other);
                            self.bump();
                        }
                    }
                }
                let trimmed = expression.trim();
                if trimmed.is_empty() {
                    return Err(self.error_at(
                        String::from("empty expression in an f-string"),
                        start_line,
                        start_column,
                    ));
                }
                parts.push(FStringLexPart::Value {
                    text: Rc::from(trimmed),
                    spec: spec.map(|value| Rc::from(value.as_str())),
                    conversion,
                });
                continue;
            }
            if ch == '}' {
                if self.peek_at(1) == Some('}') {
                    text.push('}');
                    self.bump();
                    self.bump();
                    continue;
                }
                return Err(self.error_at(
                    String::from(
                        "single '}' is not allowed in an f-string; use '}}' for a literal brace",
                    ),
                    start_line,
                    start_column,
                ));
            }
            text.push(ch);
            self.bump();
        }
        if !text.is_empty() {
            parts.push(FStringLexPart::Text(Rc::from(text.as_str())));
        }
        Ok(Tok::FStr(Rc::new(parts)))
    }

    /// The format spec between `:` and the closing `}` of an f-string
    /// expression. Nested replacement fields are not supported.
    fn read_format_spec(
        &mut self,
        start_line: u32,
        start_column: u32,
    ) -> Result<String, SyntaxError> {
        let mut out = String::new();
        loop {
            let Some(ch) = self.peek() else {
                return Err(self.error_at(
                    String::from("unterminated f-string format spec"),
                    start_line,
                    start_column,
                ));
            };
            if ch == '}' {
                return Ok(out);
            }
            if ch == '\n' {
                return Err(self.error_at(
                    String::from("f-string format specs cannot span a line"),
                    start_line,
                    start_column,
                ));
            }
            if ch == '{' || ch == '\'' || ch == '"' {
                return Err(self.error_at(
                    String::from(
                        "nested replacement fields and quotes in f-string format specs \
                         are not supported",
                    ),
                    start_line,
                    start_column,
                ));
            }
            out.push(ch);
            self.bump();
        }
    }

    /// Decode one backslash escape (the current character is the backslash)
    /// into `out`. Shared by f-string literal chunks.
    fn read_escape_string(
        &mut self,
        out: &mut String,
        start_line: u32,
        start_column: u32,
    ) -> Result<(), SyntaxError> {
        self.bump();
        let Some(escape) = self.peek() else {
            return Err(self.error_at(
                String::from("unterminated escape sequence at end of file"),
                start_line,
                start_column,
            ));
        };
        match escape {
            'n' => {
                out.push('\n');
                self.bump();
            }
            't' => {
                out.push('\t');
                self.bump();
            }
            'r' => {
                out.push('\r');
                self.bump();
            }
            'a' => {
                out.push('\u{07}');
                self.bump();
            }
            'b' => {
                out.push('\u{08}');
                self.bump();
            }
            'f' => {
                out.push('\u{0c}');
                self.bump();
            }
            'v' => {
                out.push('\u{0b}');
                self.bump();
            }
            '0' => {
                out.push('\0');
                self.bump();
            }
            '\\' => {
                out.push('\\');
                self.bump();
            }
            '\'' => {
                out.push('\'');
                self.bump();
            }
            '"' => {
                out.push('"');
                self.bump();
            }
            '\n' => {
                self.bump();
            }
            'x' => {
                self.bump();
                let value = self.read_hex_escape(2, start_line, start_column)?;
                match char::from_u32(value) {
                    Some(ch) => out.push(ch),
                    None => {
                        return Err(self.error_at(
                            String::from("invalid \\x escape value"),
                            start_line,
                            start_column,
                        ))
                    }
                }
            }
            'u' => {
                self.bump();
                let value = self.read_hex_escape(4, start_line, start_column)?;
                match char::from_u32(value) {
                    Some(ch) => out.push(ch),
                    None => {
                        return Err(self.error_at(
                            String::from("invalid \\u escape value"),
                            start_line,
                            start_column,
                        ))
                    }
                }
            }
            'U' => {
                self.bump();
                let value = self.read_hex_escape(8, start_line, start_column)?;
                match char::from_u32(value) {
                    Some(ch) => out.push(ch),
                    None => {
                        return Err(self.error_at(
                            String::from("invalid \\U escape value"),
                            start_line,
                            start_column,
                        ))
                    }
                }
            }
            other => {
                out.push('\\');
                out.push(other);
                self.bump();
            }
        }
        Ok(())
    }

    fn read_hex_escape(
        &mut self,
        count: usize,
        start_line: u32,
        start_column: u32,
    ) -> Result<u32, SyntaxError> {
        let mut value = 0u32;
        for _ in 0..count {
            let Some(c) = self.peek() else {
                return Err(self.error_at(
                    String::from("truncated hexadecimal escape"),
                    start_line,
                    start_column,
                ));
            };
            let Some(digit) = c.to_digit(16) else {
                return Err(self.error_at(
                    format!("invalid hexadecimal digit '{}' in escape", c),
                    start_line,
                    start_column,
                ));
            };
            value = value * 16 + digit;
            self.bump();
        }
        Ok(value)
    }

    // ── operators ──────────────────────────────────────────────────────────

    fn lex_operator(&mut self) -> Result<(Tok, u32, u32), SyntaxError> {
        let line = self.line;
        let column = self.column();
        let ch = self.peek().unwrap_or('\0');
        let two = |lexer: &Lexer| lexer.peek_at(1);
        let three = |lexer: &Lexer| lexer.peek_at(2);

        let (op, width) = match ch {
            '+' => match two(self) {
                Some('=') => (Op::PlusEq, 2),
                _ => (Op::Plus, 1),
            },
            '-' => match two(self) {
                Some('=') => (Op::MinusEq, 2),
                _ => (Op::Minus, 1),
            },
            '*' => match (two(self), three(self)) {
                (Some('*'), Some('=')) => (Op::StarStarEq, 3),
                (Some('*'), _) => (Op::StarStar, 2),
                (Some('='), _) => (Op::StarEq, 2),
                _ => (Op::Star, 1),
            },
            '/' => match (two(self), three(self)) {
                (Some('/'), Some('=')) => (Op::SlashSlashEq, 3),
                (Some('/'), _) => (Op::SlashSlash, 2),
                (Some('='), _) => (Op::SlashEq, 2),
                _ => (Op::Slash, 1),
            },
            '%' => match two(self) {
                Some('=') => (Op::PercentEq, 2),
                _ => (Op::Percent, 1),
            },
            '=' => match two(self) {
                Some('=') => (Op::EqEq, 2),
                _ => (Op::Assign, 1),
            },
            '!' => match two(self) {
                Some('=') => (Op::NotEq, 2),
                _ => {
                    return Err(self.error(
                        "unexpected character '!' (did you mean '!=' or 'not'?)",
                    ))
                }
            },
            '<' => match two(self) {
                Some('=') => (Op::Le, 2),
                _ => (Op::Lt, 1),
            },
            '>' => match two(self) {
                Some('=') => (Op::Ge, 2),
                _ => (Op::Gt, 1),
            },
            '(' => (Op::LParen, 1),
            ')' => (Op::RParen, 1),
            '[' => (Op::LBracket, 1),
            ']' => (Op::RBracket, 1),
            '{' => (Op::LBrace, 1),
            '}' => (Op::RBrace, 1),
            ',' => (Op::Comma, 1),
            ':' => (Op::Colon, 1),
            '.' => (Op::Dot, 1),
            ';' => (Op::Semi, 1),
            '@' => (Op::At, 1),
            _ => {
                return Err(self.error(&format!(
                    "unexpected character {:?} in source",
                    ch
                )))
            }
        };

        for _ in 0..width {
            self.bump();
        }
        if matches!(op, Op::LParen | Op::LBracket | Op::LBrace) {
            self.depth += 1;
        } else if matches!(op, Op::RParen | Op::RBracket | Op::RBrace) {
            if self.depth == 0 {
                return Err(self.error_at(
                    String::from("unmatched closing bracket"),
                    line,
                    column,
                ));
            }
            self.depth -= 1;
        }
        Ok((Tok::Op(op), line, column))
    }
}

// ── character classes and tables ───────────────────────────────────────────

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

fn is_string_prefix(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "r" | "u" | "f" | "b" | "rb" | "br" | "fr" | "rf"
    )
}

fn keyword(word: &str) -> Option<Kw> {
    Some(match word {
        "and" => Kw::And,
        "as" => Kw::As,
        "assert" => Kw::Assert,
        "break" => Kw::Break,
        "class" => Kw::Class,
        "continue" => Kw::Continue,
        "def" => Kw::Def,
        "del" => Kw::Del,
        "elif" => Kw::Elif,
        "else" => Kw::Else,
        "except" => Kw::Except,
        "False" => Kw::False,
        "finally" => Kw::Finally,
        "for" => Kw::For,
        "from" => Kw::From,
        "global" => Kw::Global,
        "if" => Kw::If,
        "import" => Kw::Import,
        "in" => Kw::In,
        "is" => Kw::Is,
        "lambda" => Kw::Lambda,
        "None" => Kw::None,
        "nonlocal" => Kw::Nonlocal,
        "not" => Kw::Not,
        "or" => Kw::Or,
        "pass" => Kw::Pass,
        "raise" => Kw::Raise,
        "return" => Kw::Return,
        "True" => Kw::True,
        "try" => Kw::Try,
        "while" => Kw::While,
        "with" => Kw::With,
        "yield" => Kw::Yield,
        _ => return None,
    })
}
