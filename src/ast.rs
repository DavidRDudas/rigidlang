//! AST for the v0 core: definitions, calls, imports, a few expressions.
//! Deliberately tiny. See docs/SPEC.md §2.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl BinOp {
    pub fn tag(self) -> u8 {
        match self {
            BinOp::Add => 1,
            BinOp::Sub => 2,
            BinOp::Mul => 3,
            BinOp::Div => 4,
            BinOp::Rem => 5,
            BinOp::Eq => 6,
            BinOp::Ne => 7,
            BinOp::Lt => 8,
            BinOp::Gt => 9,
            BinOp::Le => 10,
            BinOp::Ge => 11,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    TimeCall { hash: String, args: Vec<Expr>, span: Span },
    Int(i64),
    Float(f64),
    Str(String),
    Var(Ident),
    Call { callee: Ident, args: Vec<Expr> },
    Binop { op: BinOp, l: Box<Expr>, r: Box<Expr> },
    If { c: Box<Expr>, t: Box<Expr>, e: Box<Expr> },
    Neg(Box<Expr>),
    /// `&name` — a first-class reference to a definition. Creating one is a
    /// static edge; only a `dyn` block can invoke it.
    FunRef(Ident),
    /// `dyn { a, b } (callee, args...)` — quarantined indirect call. The
    /// declared set is the exact over-approximation the graph records.
    DynCall { targets: Vec<Ident>, callee: Box<Expr>, args: Vec<Expr> },
    /// `print(e)` — builtin effect; requires `!io` on the enclosing fn.
    Print { span: Span, arg: Box<Expr> },
    /// `name[key]` or `name[key](args...)` — dispatch through a declared
    /// table. The site's static edge is the table; the table's edges are
    /// its whole value set.
    TableDispatch {
        table: Ident,
        key: Box<Expr>,
        args: Option<Vec<Expr>>,
    },
    /// `[a, b, c]` — immutable list value.
    ListLit(Vec<Expr>),
    /// `{ x: 1, y: 2 }` — immutable record value. Field order is
    /// presentation; identity and rendering sort by field name.
    RecordLit(Vec<(Ident, Expr)>),
    /// `base.field`
    Get { base: Box<Expr>, field: Ident },
    /// `base[key]` where base is an arbitrary expression (value indexing).
    IndexE { base: Box<Expr>, key: Box<Expr> },
    /// `len(x)` — length of a list or string.
    Len(Box<Expr>),
    /// `push(xs, v)` — new list with v appended.
    Push(Box<Expr>, Box<Expr>),
    /// `input()` — io builtin: read one line from stdin.
    Input { span: Span },
    /// `fail(e)` — raise e (any value) as a recoverable error.
    Fail(Box<Expr>),
    /// `try(e)` — evaluate e; success -> { ok: 1, value: v },
    /// any runtime error -> { ok: 0, error: payload }.
    Try(Box<Expr>),
    /// `str(x)` — Int -> decimal string (Str passes through).
    StrConv(Box<Expr>),
    /// `int(s)` — parse a string; failure is a recoverable error.
    IntConv(Box<Expr>),
    /// `float(e)` — widen an int, or parse a string; failure is catchable.
    FloatConv(Box<Expr>),
    /// `fn(a, b) { ... }` — a closure.
    ///
    /// The body is lifted to a synthetic top-level definition so that it has
    /// a content identity like everything else; free locals become that
    /// definition's leading parameters. This node keeps the source form for
    /// `fmt`, and canonically encodes as a *reference* to the lifted
    /// definition plus the slots it captures — so the exact graph survives
    /// anonymous functions, which is the one thing a quarantine over
    /// first-class references could not otherwise promise.
    Lambda {
        params: Vec<Ident>,
        body: Vec<Stmt>,
        /// Filled in by the lifting pass: the synthetic definition's name.
        lifted: Option<Ident>,
        /// Enclosing locals the body reads, in canonical (sorted) order.
        captures: Vec<Ident>,
        line: u32,
    },
    /// `substr(s, start, count)` — chars-based slice.
    Substr(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `chars(s)` — list of 1-char strings.
    Chars(Box<Expr>),
    /// `join(xs, sep)` — join a list of strings.
    Join(Box<Expr>, Box<Expr>),
    /// `read_file(path)` — io; missing file is a recoverable error.
    ReadFile { span: Span, path: Box<Expr> },
    /// `write_file(path, contents)` — io; returns contents.
    WriteFile { span: Span, path: Box<Expr>, contents: Box<Expr> },
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let { name: Ident, value: Expr },
    /// `x = expr;` — rebind an existing local slot. Definitions are not
    /// assignable (E0210).
    Assign { name: Ident, value: Expr },
    Return(Expr),
    Expr(Expr),
    /// `while (cond) { stmts }` — scope is flat per function; a `let`
    /// inside the body keeps one stable slot across iterations.
    While { cond: Expr, body: Vec<Stmt> },
    /// `if (cond) { stmts } [else { stmts } | else if ...]` as a *statement*.
    /// Distinct from the `if` expression, which yields a value and requires
    /// an `else`. This one runs statements and its `else` is optional.
    IfStmt {
        cond: Expr,
        then: Vec<Stmt>,
        els: Vec<Stmt>,
    },
    /// `for (x in xs) { stmts }` — iterate a list or the characters of a
    /// string. The loop variable is an ordinary local slot.
    For {
        var: Ident,
        iter: Expr,
        body: Vec<Stmt>,
    },
}

/// A capability set: which parts of the outside world a definition may touch.
///
/// `!io` is rigid's own console and file builtins and predates the rest, so it
/// keeps its meaning exactly. The finer capabilities describe the *host*
/// boundary — they are declared on `extern` and propagate along the exact
/// graph, which is what makes "this dependency now reaches the network" a
/// compile-time fact rather than an audit.
pub type Caps = u16;

pub const CAP_IO: Caps = 1 << 0;
pub const CAP_FS: Caps = 1 << 1;
pub const CAP_NET: Caps = 1 << 2;
pub const CAP_ENV: Caps = 1 << 3;
pub const CAP_TIME: Caps = 1 << 4;
pub const CAP_RAND: Caps = 1 << 5;
pub const CAP_PROC: Caps = 1 << 6;

/// Declaration order is canonical order: rendering and encoding both use it.
pub const CAP_NAMES: &[(Caps, &str)] = &[
    (CAP_IO, "io"),
    (CAP_FS, "fs"),
    (CAP_NET, "net"),
    (CAP_ENV, "env"),
    (CAP_TIME, "time"),
    (CAP_RAND, "rand"),
    (CAP_PROC, "proc"),
];

pub fn cap_from_name(s: &str) -> Option<Caps> {
    CAP_NAMES.iter().find(|(_, n)| *n == s).map(|(c, _)| *c)
}

pub fn cap_vocabulary() -> String {
    CAP_NAMES
        .iter()
        .map(|(_, n)| format!("!{}", n))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `""` when empty, else `"!io !net"` in canonical order.
pub fn caps_str(c: Caps) -> String {
    CAP_NAMES
        .iter()
        .filter(|(b, _)| c & b != 0)
        .map(|(_, n)| format!("!{}", n))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn caps_list(c: Caps) -> Vec<&'static str> {
    CAP_NAMES
        .iter()
        .filter(|(b, _)| c & b != 0)
        .map(|(_, n)| *n)
        .collect()
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub memo: bool,
    /// `property fn f(xs) { ... }` — a claim that must hold for *every*
    /// input, not an example. Like `memo`, it is an execution annotation and
    /// does not enter canonical form: adding or removing it never moves the
    /// definition's hash.
    pub is_property: bool,
    /// Optional per-parameter generator hints, legal only on a `property`.
    ///
    /// This is not a type: nothing checks it, nothing is coerced, and it
    /// does not enter canonical form. It exists because `for (c in s)` and
    /// `len(x)` accept both lists and strings, so inference genuinely cannot
    /// tell which a parameter wants — and a generator that guesses wrong
    /// wastes every case. `int`, `float`, `str`, `list`.
    pub gen_hints: Vec<Option<String>>,
    /// True for a definition the compiler created by lifting a lambda out of
    /// an enclosing body. It is a real definition — it has a hash, it is in
    /// the graph, `rdeps` reaches it — but it has no source of its own, so
    /// `fmt` must not print it: the enclosing `Expr::Lambda` is what the
    /// author wrote and what gets rendered back.
    pub synthetic: bool,
    /// How many of `params` are captured values rather than arguments. Only
    /// nonzero on a lifted lambda: a closure arrives with these already
    /// bound, so the arity a call site sees is `params.len() - captured`.
    pub captured: usize,
    pub name: Ident,
    pub params: Vec<Ident>,
    pub caps: Caps,
    pub body: Vec<Stmt>,
    /// original source line of each stmt (for comment reattachment in fmt)
    pub stmt_lines: Vec<u32>,
    /// original source line of the `fn` keyword
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct TableEntry {
    pub key: String,
    pub key_span: Span,
    pub target: Ident,
}

#[derive(Debug, Clone)]
pub struct TableDef {
    pub name: Ident,
    pub entries: Vec<TableEntry>,
    /// source line of the `table` keyword
    pub line: u32,
    /// source line of each entry (for comment reattachment)
    pub entry_lines: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub names: Vec<Ident>,
    pub from: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub line: u32,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ExternDef {
    pub name: Ident,
    pub params: Vec<Ident>,
    pub caps: Caps,
    /// host binding: a dotted global path for the JS target (e.g. "console.log")
    pub host: String,
    pub span: Span,
}

#[derive(Debug)]
pub struct Module {
    /// Path relative to workspace root, forward slashes (e.g. "src/util.rg").
    pub path: String,
    pub imports: Vec<Import>,
    pub fns: Vec<FnDef>,
    pub tables: Vec<TableDef>,
    pub externs: Vec<ExternDef>,
    pub comments: Vec<Comment>,
}
