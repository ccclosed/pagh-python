//! Recursive-descent parser for the Python subset this interpreter executes.
//!
//! The grammar mirrors CPython's `Grammar/Grammar` for the constructs in scope,
//! including chained comparisons, right-associative `**`, unary binding, tuple
//! display/assignment, implicit string concatenation, and the indentation-based
//! block structure produced by the lexer.
//!
//! Unsupported syntax is rejected with a specific message ("set literals are
//! not supported", "f-strings are not supported", ...) rather than a generic
//! "invalid syntax": a clear gap is more useful than a vague failure.

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::{
    BinOp, BoolOp, CmpOp, CompGen, Expr, FStringPart, Handler, ImportAlias, Module, Name, Param,
    Stmt, StmtKind, UnOp, WithItem,
};
use crate::error::SyntaxError;
use crate::lexer::{tokenize, FStringLexPart, Kw, Op, Tok, Token};

pub fn parse(source: &str) -> Result<Module, SyntaxError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser { tokens, pos: 0 };
    parser.parse_module()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    // ── token helpers ──────────────────────────────────────────────────────

    fn peek(&self) -> &Tok {
        match self.tokens.get(self.pos) {
            Some(token) => &token.tok,
            None => &Tok::Eof,
        }
    }

    fn peek_at(&self, offset: usize) -> &Tok {
        match self.tokens.get(self.pos + offset) {
            Some(token) => &token.tok,
            None => &Tok::Eof,
        }
    }

    fn line(&self) -> u32 {
        match self.tokens.get(self.pos) {
            Some(token) => token.line,
            None => match self.tokens.last() {
                Some(token) => token.line,
                None => 1,
            },
        }
    }

    fn column(&self) -> u32 {
        match self.tokens.get(self.pos) {
            Some(token) => token.column,
            None => match self.tokens.last() {
                Some(token) => token.column,
                None => 1,
            },
        }
    }

    fn advance(&mut self) {
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
    }

    fn error<T>(&self, message: String) -> Result<T, SyntaxError> {
        Err(SyntaxError::new(message, self.line(), self.column()))
    }

    fn describe(tok: &Tok) -> String {
        match tok {
            Tok::Name(name) => format!("'{}'", name),
            Tok::Int(value) => format!("'{}'", value),
            Tok::Float(value) => format!("'{}'", crate::float::format(*value)),
            Tok::Str(_) => String::from("a string"),
            Tok::Bytes(_) => String::from("a bytes literal"),
            Tok::FStr(_) => String::from("an f-string"),
            Tok::Newline => String::from("end of line"),
            Tok::Indent => String::from("indent"),
            Tok::Dedent => String::from("dedent"),
            Tok::Eof => String::from("end of file"),
            Tok::Op(op) => format!("'{}'", op_text(*op)),
            Tok::Kw(kw) => format!("'{}'", kw_text(*kw)),
        }
    }

    fn found(&self) -> String {
        Self::describe(self.peek())
    }

    fn check_op(&self, op: Op) -> bool {
        matches!(self.peek(), Tok::Op(candidate) if *candidate == op)
    }

    fn check_kw(&self, kw: Kw) -> bool {
        matches!(self.peek(), Tok::Kw(candidate) if *candidate == kw)
    }

    fn eat_op(&mut self, op: Op) -> bool {
        if self.check_op(op) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: Kw) -> bool {
        if self.check_kw(kw) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_op(&mut self, op: Op, context: &str) -> Result<(), SyntaxError> {
        if self.eat_op(op) {
            Ok(())
        } else {
            self.error(format!(
                "expected '{}' {}, found {}",
                op_text(op),
                context,
                self.found()
            ))
        }
    }

    fn expect_kw(&mut self, kw: Kw, context: &str) -> Result<(), SyntaxError> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            self.error(format!(
                "expected '{}' {}, found {}",
                kw_text(kw),
                context,
                self.found()
            ))
        }
    }

    fn expect_newline(&mut self, context: &str) -> Result<(), SyntaxError> {
        if matches!(self.peek(), Tok::Newline) {
            self.advance();
            Ok(())
        } else {
            self.error(format!(
                "expected the end of the line after {}, found {}",
                context,
                self.found()
            ))
        }
    }

    fn expect_name(&mut self, context: &str) -> Result<Name, SyntaxError> {
        match self.peek() {
            Tok::Name(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => self.error(format!(
                "expected {}, found {}",
                context,
                self.found()
            )),
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Tok::Newline) {
            self.advance();
        }
    }

    // ── module and statements ──────────────────────────────────────────────

    fn parse_module(&mut self) -> Result<Module, SyntaxError> {
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                Tok::Eof => break,
                Tok::Indent => return self.error(String::from("unexpected indent")),
                Tok::Dedent => return self.error(String::from("unexpected dedent")),
                _ => {
                    let mut statements = self.parse_statement_line()?;
                    body.append(&mut statements);
                }
            }
        }
        Ok(Module { body })
    }

    fn parse_statement_line(&mut self) -> Result<Vec<Stmt>, SyntaxError> {
        if matches!(
            self.peek(),
            Tok::Kw(Kw::If | Kw::While | Kw::For | Kw::Def | Kw::Class | Kw::Try | Kw::With)
        ) {
            Ok(alloc::vec![self.parse_compound()?])
        } else {
            self.parse_simple_statements()
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, SyntaxError> {
        // Called after the ':' of a compound statement.
        if matches!(self.peek(), Tok::Newline) {
            self.advance();
            if !matches!(self.peek(), Tok::Indent) {
                return self.error(format!(
                    "expected an indented block, found {}",
                    self.found()
                ));
            }
            self.advance();
            let mut body = Vec::new();
            loop {
                match self.peek() {
                    Tok::Dedent => {
                        self.advance();
                        break;
                    }
                    Tok::Eof => {
                        return self.error(String::from(
                            "unexpected end of file: the indented block is not closed",
                        ))
                    }
                    Tok::Newline => self.advance(),
                    Tok::Indent => {
                        return self.error(String::from("unexpected indent"));
                    }
                    _ => {
                        let mut statements = self.parse_statement_line()?;
                        body.append(&mut statements);
                    }
                }
            }
            if body.is_empty() {
                return self.error(String::from("expected an indented block"));
            }
            Ok(body)
        } else {
            // A one-line suite: `if x: pass` or `if x: a = 1; b = 2`.
            self.parse_simple_statements()
        }
    }

    fn parse_simple_statements(&mut self) -> Result<Vec<Stmt>, SyntaxError> {
        let mut out = Vec::new();
        loop {
            out.push(self.parse_simple_statement()?);
            if self.eat_op(Op::Semi) {
                if matches!(self.peek(), Tok::Newline | Tok::Eof) {
                    break;
                }
                continue;
            }
            break;
        }
        if matches!(self.peek(), Tok::Eof) {
            // The lexer normally inserts a final NEWLINE; accept EOF here so a
            // file without a trailing newline still parses.
            Ok(out)
        } else {
            self.expect_newline("the statement")?;
            Ok(out)
        }
    }

    fn parse_simple_statement(&mut self) -> Result<Stmt, SyntaxError> {
        let line = self.line();
        if self.eat_kw(Kw::Pass) {
            return Ok(Stmt {
                line,
                kind: StmtKind::Pass,
            });
        }
        if self.eat_kw(Kw::Break) {
            return Ok(Stmt {
                line,
                kind: StmtKind::Break,
            });
        }
        if self.eat_kw(Kw::Continue) {
            return Ok(Stmt {
                line,
                kind: StmtKind::Continue,
            });
        }
        if self.eat_kw(Kw::Return) {
            let value = if matches!(self.peek(), Tok::Newline | Tok::Eof | Tok::Op(Op::Semi)) {
                None
            } else {
                Some(self.parse_testlist()?)
            };
            return Ok(Stmt {
                line,
                kind: StmtKind::Return(value),
            });
        }
        if self.eat_kw(Kw::Raise) {
            let value = if matches!(self.peek(), Tok::Newline | Tok::Eof | Tok::Op(Op::Semi)) {
                None
            } else {
                Some(self.parse_expr()?)
            };
            if self.eat_kw(Kw::From) {
                // The cause expression is parsed for syntax compatibility but
                // not linked into the traceback (documented gap).
                let _cause = self.parse_expr()?;
            }
            return Ok(Stmt {
                line,
                kind: StmtKind::Raise(value),
            });
        }
        if self.eat_kw(Kw::Import) {
            let mut names = Vec::new();
            loop {
                let module = self.parse_dotted_name()?;
                let alias = if self.eat_kw(Kw::As) {
                    Some(self.expect_name("a name after 'as'")?)
                } else {
                    None
                };
                names.push(ImportAlias {
                    name: Rc::from(module),
                    alias,
                });
                if !self.eat_op(Op::Comma) {
                    break;
                }
            }
            return Ok(Stmt {
                line,
                kind: StmtKind::Import(names),
            });
        }
        if self.eat_kw(Kw::From) {
            if self.check_op(Op::Dot) {
                return self.error(String::from(
                    "relative imports are not supported; use an absolute module name",
                ));
            }
            let module = self.parse_dotted_name()?;
            self.expect_kw(Kw::Import, "after the module name")?;
            let parenthesized = self.eat_op(Op::LParen);
            let mut names = Vec::new();
            if self.eat_op(Op::Star) {
                names.push(ImportAlias {
                    name: Rc::from("*"),
                    alias: None,
                });
            } else {
                loop {
                    let imported = self.expect_name("an imported name")?;
                    let alias = if self.eat_kw(Kw::As) {
                        Some(self.expect_name("a name after 'as'")?)
                    } else {
                        None
                    };
                    names.push(ImportAlias {
                        name: imported,
                        alias,
                    });
                    if !self.eat_op(Op::Comma) {
                        break;
                    }
                    if parenthesized && self.check_op(Op::RParen) {
                        break;
                    }
                }
            }
            if parenthesized {
                self.expect_op(Op::RParen, "after the imported names")?;
            }
            return Ok(Stmt {
                line,
                kind: StmtKind::FromImport { module, names },
            });
        }
        if self.eat_kw(Kw::Global) {
            let mut names = Vec::new();
            loop {
                names.push(self.expect_name("a variable name after 'global'")?);
                if !self.eat_op(Op::Comma) {
                    break;
                }
            }
            return Ok(Stmt {
                line,
                kind: StmtKind::Global(names),
            });
        }
        if self.eat_kw(Kw::Nonlocal) {
            let mut names = Vec::new();
            loop {
                names.push(self.expect_name("a variable name after 'nonlocal'")?);
                if !self.eat_op(Op::Comma) {
                    break;
                }
            }
            return Ok(Stmt {
                line,
                kind: StmtKind::Nonlocal(names),
            });
        }
        if self.eat_kw(Kw::Assert) {
            let test = self.parse_expr()?;
            let message = if self.eat_op(Op::Comma) {
                Some(self.parse_expr()?)
            } else {
                None
            };
            return Ok(Stmt {
                line,
                kind: StmtKind::Assert { test, message },
            });
        }
        if self.eat_kw(Kw::Del) {
            return self.error(String::from("'del' statements are not supported"));
        }
        if self.eat_kw(Kw::Yield) {
            return self.error(String::from("generators ('yield') are not supported"));
        }

        // Expression statement or assignment.
        let first = self.parse_testlist()?;
        if self.eat_op(Op::Assign) {
            let mut targets = alloc::vec![first];
            let value;
            loop {
                let next = self.parse_testlist()?;
                if self.eat_op(Op::Assign) {
                    targets.push(next);
                } else {
                    value = next;
                    break;
                }
            }
            for target in &targets {
                self.check_target(target)?;
            }
            return Ok(Stmt {
                line,
                kind: StmtKind::Assign { targets, value },
            });
        }
        if let Some(op) = compound_assignment_op(self.peek()) {
            self.advance();
            let value = self.parse_testlist()?;
            self.check_target(&first)?;
            return Ok(Stmt {
                line,
                kind: StmtKind::AugAssign {
                    target: first,
                    op,
                    value,
                },
            });
        }
        Ok(Stmt {
            line,
            kind: StmtKind::Expr(first),
        })
    }

    fn parse_compound(&mut self) -> Result<Stmt, SyntaxError> {
        if self.check_kw(Kw::If) {
            return self.parse_if();
        }
        if self.eat_kw(Kw::While) {
            let line = self.line();
            let condition = self.parse_expr()?;
            self.expect_op(Op::Colon, "after the while condition")?;
            let body = self.parse_block()?;
            return Ok(Stmt {
                line,
                kind: StmtKind::While { condition, body },
            });
        }
        if self.eat_kw(Kw::For) {
            let line = self.line();
            let target = self.parse_target()?;
            self.check_target(&target)?;
            self.expect_kw(Kw::In, "in a for statement")?;
            let iterable = self.parse_or()?;
            self.expect_op(Op::Colon, "after the for iterable")?;
            let body = self.parse_block()?;
            return Ok(Stmt {
                line,
                kind: StmtKind::For {
                    target,
                    iterable,
                    body,
                },
            });
        }
        if self.eat_kw(Kw::Def) {
            return self.parse_def();
        }
        if self.eat_kw(Kw::Class) {
            return self.parse_class();
        }
        if self.eat_kw(Kw::Try) {
            return self.parse_try();
        }
        if self.eat_kw(Kw::With) {
            return self.parse_with();
        }
        self.error(String::from("expected a compound statement"))
    }

    fn parse_if(&mut self) -> Result<Stmt, SyntaxError> {
        self.expect_kw(Kw::If, "at the start of an if statement")?;
        let line = self.line();
        let mut branches = Vec::new();
        let condition = self.parse_expr()?;
        self.expect_op(Op::Colon, "after the if condition")?;
        branches.push((condition, self.parse_block()?));
        while self.eat_kw(Kw::Elif) {
            let condition = self.parse_expr()?;
            self.expect_op(Op::Colon, "after the elif condition")?;
            branches.push((condition, self.parse_block()?));
        }
        let else_body = if self.eat_kw(Kw::Else) {
            self.expect_op(Op::Colon, "after 'else'")?;
            self.parse_block()?
        } else {
            Vec::new()
        };
        Ok(Stmt {
            line,
            kind: StmtKind::If {
                branches,
                else_body,
            },
        })
    }

    fn parse_def(&mut self) -> Result<Stmt, SyntaxError> {
        let line = self.line();
        let name = self.expect_name("a function name after 'def'")?;
        self.expect_op(Op::LParen, "after the function name")?;
        let mut params = Vec::new();
        if !self.eat_op(Op::RParen) {
            loop {
                if self.check_op(Op::Star) || self.check_op(Op::StarStar) {
                    return self.error(String::from(
                        "variadic parameters (*args / **kwargs) are not supported",
                    ));
                }
                let param_name = self.expect_name("a parameter name")?;
                let default = if self.eat_op(Op::Assign) {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                params.push(Param {
                    name: param_name,
                    default,
                });
                if self.eat_op(Op::Comma) {
                    if self.eat_op(Op::RParen) {
                        break;
                    }
                    continue;
                }
                self.expect_op(Op::RParen, "after the parameter list")?;
                break;
            }
        }
        self.expect_op(Op::Colon, "after the function signature")?;
        let body = self.parse_block()?;
        Ok(Stmt {
            line,
            kind: StmtKind::Def {
                name,
                params,
                body: Rc::new(body),
            },
        })
    }

    fn parse_class(&mut self) -> Result<Stmt, SyntaxError> {
        let line = self.line();
        let name = self.expect_name("a class name after 'class'")?;
        let mut bases = Vec::new();
        if self.eat_op(Op::LParen) {
            if !self.eat_op(Op::RParen) {
                loop {
                    bases.push(self.parse_expr()?);
                    if self.eat_op(Op::Comma) {
                        if self.eat_op(Op::RParen) {
                            break;
                        }
                        continue;
                    }
                    self.expect_op(Op::RParen, "after the base classes")?;
                    break;
                }
            }
        }
        self.expect_op(Op::Colon, "after the class header")?;
        let body = self.parse_block()?;
        Ok(Stmt {
            line,
            kind: StmtKind::Class { name, bases, body },
        })
    }

    fn parse_try(&mut self) -> Result<Stmt, SyntaxError> {
        let line = self.line();
        self.expect_op(Op::Colon, "after 'try'")?;
        let body = self.parse_block()?;
        let mut handlers = Vec::new();
        while self.eat_kw(Kw::Except) {
            let kind = if self.check_op(Op::Colon) {
                None
            } else {
                Some(self.parse_expr()?)
            };
            let name = if self.eat_kw(Kw::As) {
                Some(self.expect_name("an exception variable after 'as'")?)
            } else {
                None
            };
            self.expect_op(Op::Colon, "after the except clause")?;
            let handler_body = self.parse_block()?;
            handlers.push(Handler {
                kind,
                name,
                body: handler_body,
            });
        }
        let else_body = if self.eat_kw(Kw::Else) {
            self.expect_op(Op::Colon, "after 'else'")?;
            self.parse_block()?
        } else {
            Vec::new()
        };
        let finally_body = if self.eat_kw(Kw::Finally) {
            self.expect_op(Op::Colon, "after 'finally'")?;
            self.parse_block()?
        } else {
            Vec::new()
        };
        if handlers.is_empty() && finally_body.is_empty() {
            return self.error(String::from(
                "try must have at least one 'except' or a 'finally' clause",
            ));
        }
        Ok(Stmt {
            line,
            kind: StmtKind::Try {
                body,
                handlers,
                else_body,
                finally_body,
            },
        })
    }

    fn parse_with(&mut self) -> Result<Stmt, SyntaxError> {
        let line = self.line();
        let mut items = Vec::new();
        loop {
            let context = self.parse_expr()?;
            // `as` binds one target atom: an unparenthesized comma starts the
            // next with-item, exactly as in CPython's grammar.
            let target = if self.eat_kw(Kw::As) {
                let target = self.parse_target_atom()?;
                self.check_target(&target)?;
                Some(target)
            } else {
                None
            };
            items.push(WithItem { context, target });
            if !self.eat_op(Op::Comma) {
                break;
            }
        }
        self.expect_op(Op::Colon, "after the with items")?;
        let body = self.parse_block()?;
        Ok(Stmt {
            line,
            kind: StmtKind::With { items, body },
        })
    }

    // ── expressions ────────────────────────────────────────────────────────

    fn parse_testlist(&mut self) -> Result<Expr, SyntaxError> {
        let first = self.parse_expr()?;
        if !self.eat_op(Op::Comma) {
            return Ok(first);
        }
        let mut items = alloc::vec![first];
        loop {
            if self.at_testlist_end() {
                break;
            }
            items.push(self.parse_expr()?);
            if !self.eat_op(Op::Comma) {
                break;
            }
        }
        Ok(Expr::Tuple(items))
    }

    fn at_testlist_end(&self) -> bool {
        match self.peek() {
            Tok::Newline | Tok::Eof | Tok::Op(Op::Semi) => true,
            Tok::Op(op) => matches!(
                op,
                Op::RParen
                    | Op::RBracket
                    | Op::RBrace
                    | Op::Colon
                    | Op::Assign
                    | Op::PlusEq
                    | Op::MinusEq
                    | Op::StarEq
                    | Op::SlashEq
                    | Op::SlashSlashEq
                    | Op::PercentEq
                    | Op::StarStarEq
            ),
            Tok::Kw(kw) => matches!(kw, Kw::In | Kw::As),
            _ => false,
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, SyntaxError> {
        let body = self.parse_or()?;
        if self.eat_kw(Kw::If) {
            let condition = self.parse_or()?;
            self.expect_kw(Kw::Else, "in a conditional expression")?;
            let otherwise = self.parse_expr()?;
            return Ok(Expr::IfExpr {
                body: Box::new(body),
                condition: Box::new(condition),
                otherwise: Box::new(otherwise),
            });
        }
        Ok(body)
    }

    fn parse_or(&mut self) -> Result<Expr, SyntaxError> {
        let first = self.parse_and()?;
        if !self.check_kw(Kw::Or) {
            return Ok(first);
        }
        let mut values = alloc::vec![first];
        while self.eat_kw(Kw::Or) {
            values.push(self.parse_and()?);
        }
        Ok(Expr::BoolOp {
            op: BoolOp::Or,
            values,
        })
    }

    fn parse_and(&mut self) -> Result<Expr, SyntaxError> {
        let first = self.parse_not()?;
        if !self.check_kw(Kw::And) {
            return Ok(first);
        }
        let mut values = alloc::vec![first];
        while self.eat_kw(Kw::And) {
            values.push(self.parse_not()?);
        }
        Ok(Expr::BoolOp {
            op: BoolOp::And,
            values,
        })
    }

    fn parse_not(&mut self) -> Result<Expr, SyntaxError> {
        if self.eat_kw(Kw::Not) {
            return Ok(Expr::Unary {
                op: UnOp::Not,
                value: Box::new(self.parse_not()?),
            });
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, SyntaxError> {
        let left = self.parse_additive()?;
        let mut ops = Vec::new();
        let mut comparators = Vec::new();
        loop {
            let op = if self.eat_op(Op::EqEq) {
                Some(CmpOp::Eq)
            } else if self.eat_op(Op::NotEq) {
                Some(CmpOp::NotEq)
            } else if self.eat_op(Op::Le) {
                Some(CmpOp::Le)
            } else if self.eat_op(Op::Lt) {
                Some(CmpOp::Lt)
            } else if self.eat_op(Op::Ge) {
                Some(CmpOp::Ge)
            } else if self.eat_op(Op::Gt) {
                Some(CmpOp::Gt)
            } else if self.eat_kw(Kw::In) {
                Some(CmpOp::In)
            } else if self.check_kw(Kw::Is) {
                self.advance();
                if self.eat_kw(Kw::Not) {
                    Some(CmpOp::IsNot)
                } else {
                    Some(CmpOp::Is)
                }
            } else if self.check_kw(Kw::Not)
                && matches!(self.peek_at(1), Tok::Kw(Kw::In))
            {
                self.advance();
                self.advance();
                Some(CmpOp::NotIn)
            } else {
                None
            };
            let Some(op) = op else { break };
            ops.push(op);
            comparators.push(self.parse_additive()?);
        }
        if ops.is_empty() {
            Ok(left)
        } else {
            Ok(Expr::Compare {
                left: Box::new(left),
                ops,
                comparators,
            })
        }
    }

    fn parse_additive(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = if self.eat_op(Op::Plus) {
                Some(BinOp::Add)
            } else if self.eat_op(Op::Minus) {
                Some(BinOp::Sub)
            } else {
                None
            };
            let Some(op) = op else { break };
            let right = self.parse_multiplicative()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_factor()?;
        loop {
            let op = if self.eat_op(Op::StarStar) {
                // `**` was already consumed by parse_power in the common case;
                // this branch only triggers for malformed juxtaposition like
                // `a * *b`, so leave it to a clear error below.
                return self.error(String::from("unexpected '**' in an expression"));
            } else if self.eat_op(Op::Star) {
                Some(BinOp::Mul)
            } else if self.eat_op(Op::Slash) {
                Some(BinOp::Div)
            } else if self.eat_op(Op::SlashSlash) {
                Some(BinOp::FloorDiv)
            } else if self.eat_op(Op::Percent) {
                Some(BinOp::Mod)
            } else {
                None
            };
            let Some(op) = op else { break };
            let right = self.parse_factor()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<Expr, SyntaxError> {
        if self.eat_op(Op::Minus) {
            return Ok(Expr::Unary {
                op: UnOp::Minus,
                value: Box::new(self.parse_factor()?),
            });
        }
        if self.eat_op(Op::Plus) {
            return Ok(Expr::Unary {
                op: UnOp::Plus,
                value: Box::new(self.parse_factor()?),
            });
        }
        self.parse_power()
    }

    fn parse_power(&mut self) -> Result<Expr, SyntaxError> {
        let base = self.parse_postfix()?;
        if self.eat_op(Op::StarStar) {
            let exponent = self.parse_factor()?;
            return Ok(Expr::Binary {
                op: BinOp::Pow,
                left: Box::new(base),
                right: Box::new(exponent),
            });
        }
        Ok(base)
    }

    fn parse_postfix(&mut self) -> Result<Expr, SyntaxError> {
        let mut expression = self.parse_atom()?;
        loop {
            if self.eat_op(Op::LParen) {
                expression = self.parse_call(expression)?;
            } else if self.eat_op(Op::LBracket) {
                expression = self.parse_subscript(expression)?;
            } else if self.eat_op(Op::Dot) {
                let name = self.expect_name("an attribute name after '.'")?;
                expression = Expr::Attribute {
                    target: Box::new(expression),
                    name,
                };
            } else {
                break;
            }
        }
        Ok(expression)
    }

    fn parse_call(&mut self, function: Expr) -> Result<Expr, SyntaxError> {
        let mut args = Vec::new();
        let mut kwargs = Vec::new();
        if self.eat_op(Op::RParen) {
            return Ok(Expr::Call {
                function: Box::new(function),
                args,
                kwargs,
            });
        }
        loop {
            if self.check_op(Op::Star) || self.check_op(Op::StarStar) {
                return self.error(String::from(
                    "starred call arguments (*args / **kwargs) are not supported",
                ));
            }
            if matches!(self.peek(), Tok::Name(_)) && self.peek_at(1) == &Tok::Op(Op::Assign) {
                let name = self.expect_name("a keyword argument name")?;
                self.expect_op(Op::Assign, "after the keyword argument name")?;
                let value = self.parse_expr()?;
                kwargs.push((name, value));
            } else {
                if !kwargs.is_empty() {
                    return self.error(String::from(
                        "positional argument follows keyword argument",
                    ));
                }
                args.push(self.parse_expr()?);
            }
            if self.eat_op(Op::Comma) {
                if self.eat_op(Op::RParen) {
                    break;
                }
                continue;
            }
            self.expect_op(Op::RParen, "after the call arguments")?;
            break;
        }
        Ok(Expr::Call {
            function: Box::new(function),
            args,
            kwargs,
        })
    }

    fn parse_subscript(&mut self, target: Expr) -> Result<Expr, SyntaxError> {
        if self.eat_op(Op::Colon) {
            return self.parse_slice_tail(target, None);
        }
        let first = self.parse_expr()?;
        if self.eat_op(Op::Colon) {
            return self.parse_slice_tail(target, Some(first));
        }
        self.expect_op(Op::RBracket, "after the index expression")?;
        Ok(Expr::Index {
            target: Box::new(target),
            index: Box::new(first),
        })
    }

    fn parse_slice_tail(
        &mut self,
        target: Expr,
        start: Option<Expr>,
    ) -> Result<Expr, SyntaxError> {
        let stop = if self.check_op(Op::Colon) || self.check_op(Op::RBracket) {
            None
        } else {
            Some(Box::new(self.parse_expr()?))
        };
        let step = if self.eat_op(Op::Colon) {
            if self.check_op(Op::RBracket) {
                None
            } else {
                Some(Box::new(self.parse_expr()?))
            }
        } else {
            None
        };
        self.expect_op(Op::RBracket, "after the slice")?;
        Ok(Expr::Slice {
            target: Box::new(target),
            start: start.map(Box::new),
            stop,
            step,
        })
    }

    fn parse_atom(&mut self) -> Result<Expr, SyntaxError> {
        match self.peek() {
            Tok::Int(value) => {
                let value = *value;
                self.advance();
                Ok(Expr::Int(value))
            }
            Tok::Float(value) => {
                let value = *value;
                self.advance();
                Ok(Expr::Float(value))
            }
            Tok::Str(text) => {
                let first = text.clone();
                self.advance();
                if !matches!(self.peek(), Tok::Str(_)) {
                    return Ok(Expr::Str(first));
                }
                let mut joined = String::from(&*first);
                while let Tok::Str(next) = self.peek() {
                    joined.push_str(next);
                    self.advance();
                }
                Ok(Expr::Str(Rc::from(joined)))
            }
            Tok::Bytes(bytes) => {
                let bytes = bytes.clone();
                self.advance();
                Ok(Expr::Bytes(bytes))
            }
            Tok::FStr(parts) => {
                let parts = parts.clone();
                let line = self.line();
                let column = self.column();
                self.advance();
                let mut out = self.fstring_parts(&parts, line, column)?;
                // Adjacent literals concatenate, including f-strings.
                loop {
                    match self.peek() {
                        Tok::Str(text) => {
                            let text = text.clone();
                            self.advance();
                            out.push(FStringPart::Text(text));
                        }
                        Tok::FStr(next) => {
                            let next = next.clone();
                            let next_line = self.line();
                            let next_column = self.column();
                            self.advance();
                            out.extend(self.fstring_parts(&next, next_line, next_column)?);
                        }
                        _ => break,
                    }
                }
                Ok(Expr::FString(out))
            }
            Tok::Name(name) => {
                let name = name.clone();
                self.advance();
                Ok(Expr::Name(name))
            }
            Tok::Kw(Kw::None) => {
                self.advance();
                Ok(Expr::None)
            }
            Tok::Kw(Kw::True) => {
                self.advance();
                Ok(Expr::True)
            }
            Tok::Kw(Kw::False) => {
                self.advance();
                Ok(Expr::False)
            }
            Tok::Kw(Kw::Lambda) => self.parse_lambda(),
            Tok::Kw(Kw::Yield) => {
                self.error(String::from("generators ('yield') are not supported"))
            }
            Tok::Op(Op::LParen) => {
                self.advance();
                if self.eat_op(Op::RParen) {
                    return Ok(Expr::Tuple(Vec::new()));
                }
                let value = self.parse_testlist()?;
                self.expect_op(Op::RParen, "after the parenthesized expression")?;
                Ok(value)
            }
            Tok::Op(Op::LBracket) => {
                self.advance();
                if self.eat_op(Op::RBracket) {
                    return Ok(Expr::List(Vec::new()));
                }
                let first = self.parse_expr()?;
                if self.check_kw(Kw::For) {
                    return self.parse_list_comprehension(first);
                }
                let mut items = alloc::vec![first];
                while self.eat_op(Op::Comma) {
                    if self.eat_op(Op::RBracket) {
                        return Ok(Expr::List(items));
                    }
                    items.push(self.parse_expr()?);
                }
                self.expect_op(Op::RBracket, "after the list elements")?;
                Ok(Expr::List(items))
            }
            Tok::Op(Op::LBrace) => {
                self.advance();
                if self.eat_op(Op::RBrace) {
                    // `{}` is an empty dict; `set()` is the empty set.
                    return Ok(Expr::Dict(Vec::new()));
                }
                let first = self.parse_expr()?;
                if self.check_kw(Kw::For) {
                    let generators = self.parse_comp_generators()?;
                    self.expect_op(Op::RBrace, "after the set comprehension")?;
                    return Ok(Expr::SetComp {
                        element: Box::new(first),
                        generators,
                    });
                }
                if self.eat_op(Op::Colon) {
                    let value = self.parse_expr()?;
                    if self.check_kw(Kw::For) {
                        let generators = self.parse_comp_generators()?;
                        self.expect_op(Op::RBrace, "after the dict comprehension")?;
                        return Ok(Expr::DictComp {
                            key: Box::new(first),
                            value: Box::new(value),
                            generators,
                        });
                    }
                    let mut entries = alloc::vec![(first, value)];
                    loop {
                        if self.eat_op(Op::Comma) {
                            if self.eat_op(Op::RBrace) {
                                break;
                            }
                            let key = self.parse_expr()?;
                            self.expect_op(Op::Colon, "after a dict key")?;
                            let value = self.parse_expr()?;
                            entries.push((key, value));
                            continue;
                        }
                        self.expect_op(Op::RBrace, "after the dict entries")?;
                        break;
                    }
                    return Ok(Expr::Dict(entries));
                }
                let mut items = alloc::vec![first];
                loop {
                    if self.eat_op(Op::Comma) {
                        if self.eat_op(Op::RBrace) {
                            break;
                        }
                        items.push(self.parse_expr()?);
                        continue;
                    }
                    self.expect_op(Op::RBrace, "after the set elements")?;
                    break;
                }
                Ok(Expr::Set(items))
            }
            _ => self.error(format!(
                "expected an expression, found {}",
                self.found()
            )),
        }
    }

    fn parse_list_comprehension(&mut self, element: Expr) -> Result<Expr, SyntaxError> {
        let generators = self.parse_comp_generators()?;
        self.expect_op(Op::RBracket, "after the comprehension")?;
        Ok(Expr::ListComp {
            element: Box::new(element),
            generators,
        })
    }

    fn parse_comp_generators(&mut self) -> Result<Vec<CompGen>, SyntaxError> {
        let mut generators = Vec::new();
        while self.eat_kw(Kw::For) {
            let target = self.parse_target()?;
            self.check_target(&target)?;
            self.expect_kw(Kw::In, "in a comprehension")?;
            let iterable = self.parse_or()?;
            let mut conditions = Vec::new();
            while self.eat_kw(Kw::If) {
                conditions.push(self.parse_or()?);
            }
            generators.push(CompGen {
                target,
                iterable,
                conditions,
            });
        }
        Ok(generators)
    }

    fn parse_lambda(&mut self) -> Result<Expr, SyntaxError> {
        self.expect_kw(Kw::Lambda, "at the start of a lambda expression")?;
        let mut params = Vec::new();
        if !self.check_op(Op::Colon) {
            loop {
                if self.check_op(Op::Star) || self.check_op(Op::StarStar) {
                    return self.error(String::from(
                        "variadic parameters (*args / **kwargs) are not supported",
                    ));
                }
                let name = self.expect_name("a parameter name after 'lambda'")?;
                let default = if self.eat_op(Op::Assign) {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                params.push(Param { name, default });
                if !self.eat_op(Op::Comma) {
                    break;
                }
                if self.check_op(Op::Colon) {
                    break;
                }
            }
        }
        self.expect_op(Op::Colon, "after the lambda parameters")?;
        let body = self.parse_expr()?;
        Ok(Expr::Lambda {
            params,
            body: Rc::new(body),
        })
    }

    fn fstring_parts(
        &mut self,
        parts: &[FStringLexPart],
        line: u32,
        column: u32,
    ) -> Result<Vec<FStringPart>, SyntaxError> {
        let mut out = Vec::with_capacity(parts.len());
        for part in parts {
            match part {
                FStringLexPart::Text(text) => out.push(FStringPart::Text(text.clone())),
                FStringLexPart::Value {
                    text,
                    spec,
                    conversion,
                } => {
                    let expression = parse_expression_fragment(text, line, column)?;
                    out.push(FStringPart::Value {
                        expression: Box::new(expression),
                        spec: spec.clone(),
                        conversion: *conversion,
                    });
                }
            }
        }
        Ok(out)
    }

    // ── assignment targets ─────────────────────────────────────────────────

    fn parse_target(&mut self) -> Result<Expr, SyntaxError> {
        let first = self.parse_target_atom()?;
        if !self.eat_op(Op::Comma) {
            return Ok(first);
        }
        let mut items = alloc::vec![first];
        loop {
            if matches!(self.peek(), Tok::Kw(Kw::In) | Tok::Newline | Tok::Eof) {
                break;
            }
            items.push(self.parse_target_atom()?);
            if !self.eat_op(Op::Comma) {
                break;
            }
        }
        Ok(Expr::Tuple(items))
    }

    fn parse_target_atom(&mut self) -> Result<Expr, SyntaxError> {
        if self.eat_op(Op::LParen) {
            if self.check_op(Op::RParen) {
                return self.error(String::from("cannot assign to an empty tuple"));
            }
            let target = self.parse_target()?;
            self.expect_op(Op::RParen, "after the tuple assignment target")?;
            return Ok(target);
        }
        if self.eat_op(Op::LBracket) {
            if self.check_op(Op::RBracket) {
                return self.error(String::from("cannot assign to an empty list"));
            }
            let target = self.parse_target()?;
            self.expect_op(Op::RBracket, "after the list assignment target")?;
            return Ok(target);
        }
        self.parse_postfix()
    }

    fn check_target(&self, expression: &Expr) -> Result<(), SyntaxError> {
        match expression {
            Expr::Name(_) | Expr::Attribute { .. } | Expr::Index { .. } => Ok(()),
            Expr::Tuple(items) | Expr::List(items) => {
                for item in items {
                    self.check_target(item)?;
                }
                Ok(())
            }
            Expr::Slice { .. } => self.error(String::from(
                "assigning to a slice is not supported",
            )),
            _ => self.error(String::from("cannot assign to this expression")),
        }
    }

    // ── dotted module names ────────────────────────────────────────────────

    fn parse_dotted_name(&mut self) -> Result<String, SyntaxError> {
        let first = self.expect_name("a module name")?;
        let mut name = String::from(&*first);
        while self.eat_op(Op::Dot) {
            let component = self.expect_name("a module name component")?;
            name.push('.');
            name.push_str(&component);
        }
        Ok(name)
    }
}

/// Parse the source text of one f-string expression. Positions in errors are
/// reported at the f-string's opening token: the fragment is a separate token
/// stream, and its offsets would otherwise look like line 1.
fn parse_expression_fragment(text: &str, line: u32, column: u32) -> Result<Expr, SyntaxError> {
    let tokens = tokenize(text).map_err(|error| {
        SyntaxError::new(
            format!("in f-string expression: {}", error.message),
            line,
            column,
        )
    })?;
    let mut parser = Parser { tokens, pos: 0 };
    let expression = parser.parse_testlist().map_err(|error| {
        SyntaxError::new(
            format!("in f-string expression: {}", error.message),
            line,
            column,
        )
    })?;
    parser.skip_newlines();
    if !matches!(parser.peek(), Tok::Eof) {
        return Err(SyntaxError::new(
            format!(
                "in f-string expression: unexpected {}",
                Parser::describe(parser.peek())
            ),
            line,
            column,
        ));
    }
    Ok(expression)
}

fn compound_assignment_op(tok: &Tok) -> Option<BinOp> {
    match tok {
        Tok::Op(Op::PlusEq) => Some(BinOp::Add),
        Tok::Op(Op::MinusEq) => Some(BinOp::Sub),
        Tok::Op(Op::StarEq) => Some(BinOp::Mul),
        Tok::Op(Op::SlashEq) => Some(BinOp::Div),
        Tok::Op(Op::SlashSlashEq) => Some(BinOp::FloorDiv),
        Tok::Op(Op::PercentEq) => Some(BinOp::Mod),
        Tok::Op(Op::StarStarEq) => Some(BinOp::Pow),
        _ => None,
    }
}

fn op_text(op: Op) -> &'static str {
    match op {
        Op::Plus => "+",
        Op::Minus => "-",
        Op::Star => "*",
        Op::Slash => "/",
        Op::SlashSlash => "//",
        Op::Percent => "%",
        Op::StarStar => "**",
        Op::EqEq => "==",
        Op::NotEq => "!=",
        Op::Lt => "<",
        Op::Le => "<=",
        Op::Gt => ">",
        Op::Ge => ">=",
        Op::Assign => "=",
        Op::PlusEq => "+=",
        Op::MinusEq => "-=",
        Op::StarEq => "*=",
        Op::SlashEq => "/=",
        Op::SlashSlashEq => "//=",
        Op::PercentEq => "%=",
        Op::StarStarEq => "**=",
        Op::LParen => "(",
        Op::RParen => ")",
        Op::LBracket => "[",
        Op::RBracket => "]",
        Op::LBrace => "{",
        Op::RBrace => "}",
        Op::Comma => ",",
        Op::Colon => ":",
        Op::Dot => ".",
        Op::Semi => ";",
    }
}

fn kw_text(kw: Kw) -> &'static str {
    match kw {
        Kw::And => "and",
        Kw::As => "as",
        Kw::Assert => "assert",
        Kw::Break => "break",
        Kw::Class => "class",
        Kw::Continue => "continue",
        Kw::Def => "def",
        Kw::Del => "del",
        Kw::Elif => "elif",
        Kw::Else => "else",
        Kw::Except => "except",
        Kw::False => "False",
        Kw::Finally => "finally",
        Kw::For => "for",
        Kw::From => "from",
        Kw::Global => "global",
        Kw::If => "if",
        Kw::Import => "import",
        Kw::In => "in",
        Kw::Is => "is",
        Kw::Lambda => "lambda",
        Kw::None => "None",
        Kw::Nonlocal => "nonlocal",
        Kw::Not => "not",
        Kw::Or => "or",
        Kw::Pass => "pass",
        Kw::Raise => "raise",
        Kw::Return => "return",
        Kw::True => "True",
        Kw::Try => "try",
        Kw::While => "while",
        Kw::With => "with",
        Kw::Yield => "yield",
    }
}