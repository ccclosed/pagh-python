//! The Python abstract syntax tree produced by [`crate::parser`].
//!
//! Names are `Rc<str>` so cloning one (for a frame slot, a class attribute or
//! a function object) never copies the text. Function bodies are
//! `Rc<Vec<Stmt>>` for the same reason: defining a function clones a pointer,
//! not a statement tree. Everything is owned by the module AST; the
//! interpreter only ever borrows it.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

/// An identifier, also used for keyword-argument and attribute names.
pub type Name = Rc<str>;

pub struct Module {
    pub body: Vec<Stmt>,
}

pub struct Stmt {
    pub line: u32,
    pub kind: StmtKind,
}

pub enum StmtKind {
    Expr(Expr),
    /// `a = b = value`; every target is validated by the parser.
    Assign { targets: Vec<Expr>, value: Expr },
    AugAssign { target: Expr, op: BinOp, value: Expr },
    If {
        /// `if` plus every `elif`, in source order.
        branches: Vec<(Expr, Vec<Stmt>)>,
        else_body: Vec<Stmt>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    For {
        target: Expr,
        iterable: Expr,
        body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    Break,
    Continue,
    /// `del a, b[0], c.attr` — targets are validated by the parser.
    Del(Vec<Expr>),
    Pass,
    Def {
        name: Name,
        params: Vec<Param>,
        body: Rc<Vec<Stmt>>,
    },
    Return(Option<Expr>),
    Class {
        name: Name,
        bases: Vec<Expr>,
        body: Vec<Stmt>,
    },
    Try {
        body: Vec<Stmt>,
        handlers: Vec<Handler>,
        else_body: Vec<Stmt>,
        finally_body: Vec<Stmt>,
    },
    /// `with a as x, b as y: ...`. The items are entered left to right and
    /// exited right to left.
    With {
        items: Vec<WithItem>,
        body: Vec<Stmt>,
    },
    Raise(Option<Expr>),
    Import(Vec<ImportAlias>),
    FromImport {
        module: String,
        names: Vec<ImportAlias>,
    },
    Global(Vec<Name>),
    Nonlocal(Vec<Name>),
    Assert {
        test: Expr,
        message: Option<Expr>,
    },
}

pub struct Param {
    pub name: Name,
    /// The default is evaluated once, when the `def` executes (Python's
    /// mutable-default semantics), and stored in the function object.
    pub default: Option<Expr>,
}

pub struct WithItem {
    pub context: Expr,
    pub target: Option<Expr>,
}

pub struct Handler {
    /// `None` is a bare `except:`.
    pub kind: Option<Expr>,
    pub name: Option<Name>,
    pub body: Vec<Stmt>,
}

pub struct ImportAlias {
    /// The dotted module path for `import a.b`, or the dotted source module in
    /// `from a.b import c` (unused for the imported-name entry itself).
    pub name: Name,
    pub alias: Option<Name>,
}

/// One piece of an f-string literal. Text pieces are already unescaped;
/// expression pieces keep the source text until the parser turns it into an
/// `Expr`, so the lexer never depends on the parser.
pub enum FStringPart {
    Text(Rc<str>),
    Value {
        expression: Box<Expr>,
        /// The text after `:`, without the colon.
        spec: Option<Rc<str>>,
        /// `!r`, `!s` or `!a`.
        conversion: Option<char>,
    },
}

pub struct CompGen {
    pub target: Expr,
    pub iterable: Expr,
    pub conditions: Vec<Expr>,
}

pub enum Expr {
    None,
    True,
    False,
    Int(i64),
    /// Decimal/exponent literals are parsed to f64 by the lexer. The kernel
    /// keeps the U-mode FPU enabled (`sstatus.FS`), so evaluation is a plain
    /// `Value::Float`.
    Float(f64),
    Str(Rc<str>),
    Name(Name),
    List(Vec<Expr>),
    Tuple(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    Set(Vec<Expr>),
    Bytes(Rc<Vec<u8>>),
    FString(Vec<FStringPart>),
    Lambda {
        params: Vec<Param>,
        body: Rc<Expr>,
    },
    Unary {
        op: UnOp,
        value: Box<Expr>,
    },
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    BoolOp {
        op: BoolOp,
        values: Vec<Expr>,
    },
    Compare {
        left: Box<Expr>,
        ops: Vec<CmpOp>,
        comparators: Vec<Expr>,
    },
    Call {
        function: Box<Expr>,
        args: Vec<Expr>,
        kwargs: Vec<(Name, Expr)>,
    },
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
    },
    Slice {
        target: Box<Expr>,
        start: Option<Box<Expr>>,
        stop: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
    },
    Attribute {
        target: Box<Expr>,
        name: Name,
    },
    ListComp {
        element: Box<Expr>,
        generators: Vec<CompGen>,
    },
    SetComp {
        element: Box<Expr>,
        generators: Vec<CompGen>,
    },
    DictComp {
        key: Box<Expr>,
        value: Box<Expr>,
        generators: Vec<CompGen>,
    },
    IfExpr {
        body: Box<Expr>,
        condition: Box<Expr>,
        otherwise: Box<Expr>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Minus,
    Plus,
    Not,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
    And,
    Or,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
    Is,
    IsNot,
}