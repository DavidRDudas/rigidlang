//! Execution engine (v9): each definition is compiled once to a stack
//! bytecode; semantics are identical to the former tree-walker. The error
//! channel carries values (`fail`/`try`); `try` operands compile to
//! sub-chunks executed with the same locals.

use crate::ast::*;
use crate::canon::Hash;
use crate::resolve::{Binding, DefId, DefKind, Workspace};
use std::collections::{BTreeMap, HashMap};

/// On wasm there is no stdout, so program output is collected here and
/// drained by the host after a run. Native builds write straight through.
#[cfg(target_arch = "wasm32")]
pub mod host {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    thread_local! {
        pub static OUT: RefCell<String> = RefCell::new(String::new());
        /// read_file / write_file operate on this in-memory filesystem.
        pub static FILES: RefCell<BTreeMap<String, String>> = RefCell::new(BTreeMap::new());
        /// The test cache lives for the life of the worker, so running the
        /// tests twice in the playground demonstrates a real cache hit.
        pub static TESTCACHE: RefCell<BTreeMap<String, bool>> = RefCell::new(BTreeMap::new());
        /// Same for `memo`: on the CLI the store is a file, in the browser it
        /// is this, so a memoized result survives from one Run to the next.
        pub static MEMOSTORE: RefCell<
            std::collections::HashMap<String, serde_json::Value>,
        > = RefCell::new(std::collections::HashMap::new());
    }

    pub fn emit(s: &str) {
        OUT.with(|o| {
            let mut o = o.borrow_mut();
            o.push_str(s);
            o.push('\n');
        });
    }

    pub fn take_output() -> String {
        OUT.with(|o| std::mem::take(&mut *o.borrow_mut()))
    }
}

#[cfg(target_arch = "wasm32")]
macro_rules! sayln {
    ($($t:tt)*) => {{
        crate::eval::host::emit(&format!($($t)*));
    }};
}

#[cfg(not(target_arch = "wasm32"))]
macro_rules! sayln {
    ($($t:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Record(BTreeMap<String, Value>),
    Fun(DefId),
    /// A lifted lambda together with the values it closed over. The
    /// DefId is a real definition with a real hash — a closure is a
    /// partially applied one, not an opaque function object.
    Closure(DefId, Vec<Value>),
}

pub fn show(ws: &Workspace, v: &Value) -> String {
    match v {
        Value::Int(x) => format!("{}", x),
        Value::Float(x) => crate::fmt::fmt_f64(*x),
        Value::Str(s) => quote(s),
        // A function value shows as its identity, not its name: a rename
        // moves no hash, so it must not change what a program prints.
        Value::Fun(d) => format!("&{}", crate::canon::tag(ws, *d)),
        Value::Closure(d, caught) => {
            if caught.is_empty() {
                format!("&{}", crate::canon::tag(ws, *d))
            } else {
                format!(
                    "&{}[{}]",
                    crate::canon::tag(ws, *d),
                    caught.iter().map(|v| show(ws, v)).collect::<Vec<_>>().join(", ")
                )
            }
        }
        Value::List(xs) => format!(
            "[{}]",
            xs.iter().map(|x| show(ws, x)).collect::<Vec<_>>().join(", ")
        ),
        Value::Record(fs) => format!(
            "{{{}}}",
            fs.iter()
                .map(|(k, v)| format!("{}: {}", k, show(ws, v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn binding(ws: &Workspace, def: DefId, span: Span, name: &str) -> Binding {
    ws.bindings
        .get(&(def, span.line, span.col, name.to_string()))
        .copied()
        .unwrap_or(Binding::Broken)
}

// ---------------------------------------------------------------- bytecode

#[derive(Debug, Clone)]
enum Op {
    TimeCall(String, usize),
    Const(Value),
    Load(u32),
    Store(u32),
    Pop,
    Bin(BinOp),
    Neg,
    Jump(usize),
    JumpIfZero(usize),
    Call(DefId, u8),
    DynCall(Vec<DefId>, u8),
    TCall(DefId, u8),
    TLookup(DefId),
    Index,
    GetField(String),
    MakeList(u32),
    MakeRecord(Vec<String>),
    Len,
    PushElem,
    StrConv,
    IntConv,
    FloatConv,
    /// Replaces the iterable with a cursor: the sequence, then an index.
    /// Pops n captured values and pushes a closure over the lifted def.
    MakeClosure(DefId, u32),
    ForInit,
    /// Stores the next element in a slot and advances, or unwinds and jumps.
    ForNext(u32, usize),
    Substr,
    Chars,
    Join,
    Print,
    Input,
    ReadFile,
    WriteFile,
    Fail,
    TryBlock(usize),
    Return,
}

#[derive(Debug, Clone, Default)]
struct Chunk {
    ops: Vec<Op>,
    subs: Vec<Chunk>,
}

struct C<'a> {
    ws: &'a Workspace,
    /// def -> the first definition sharing its hash (see Engine::rep)
    rep: &'a [DefId],
    def: DefId,
    ops: Vec<Op>,
    subs: Vec<Chunk>,
}

impl<'a> C<'a> {
    fn slot(&self, span: Span, name: &str) -> u32 {
        match binding(self.ws, self.def, span, name) {
            Binding::Local(s) => s,
            _ => u32::MAX, // unreachable for non-poisoned defs
        }
    }
    fn defid(&self, span: Span, name: &str) -> DefId {
        match binding(self.ws, self.def, span, name) {
            Binding::Def(d) => d,
            _ => usize::MAX,
        }
    }
    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, value } | Stmt::Assign { name, value } => {
                self.expr(value);
                let sl = self.slot(name.span, &name.name);
                self.ops.push(Op::Store(sl));
            }
            Stmt::Return(e) => {
                self.expr(e);
                self.ops.push(Op::Return);
            }
            Stmt::Expr(e) => {
                self.expr(e);
                self.ops.push(Op::Pop);
            }
            Stmt::IfStmt { cond, then, els } => {
                self.expr(cond);
                let to_else = self.ops.len();
                self.ops.push(Op::JumpIfZero(usize::MAX));
                for s in then {
                    self.stmt(s);
                }
                let over_else = self.ops.len();
                self.ops.push(Op::Jump(usize::MAX));
                let else_start = self.ops.len();
                for s in els {
                    self.stmt(s);
                }
                let end = self.ops.len();
                self.ops[to_else] = Op::JumpIfZero(else_start);
                self.ops[over_else] = Op::Jump(end);
            }
            Stmt::For { var, iter, body } => {
                // The iterable is evaluated once and its cursor lives on the
                // stack as [sequence, index]. Statements are stack-balanced,
                // so the cursor is still on top when the body jumps back.
                let vslot = self.slot(var.span, &var.name);
                self.expr(iter);
                self.ops.push(Op::ForInit);
                let next = self.ops.len();
                self.ops.push(Op::ForNext(vslot, usize::MAX));
                for s in body {
                    self.stmt(s);
                }
                self.ops.push(Op::Jump(next));
                let end = self.ops.len();
                self.ops[next] = Op::ForNext(vslot, end);
            }
            Stmt::While { cond, body } => {
                let start = self.ops.len();
                self.expr(cond);
                let exit = self.ops.len();
                self.ops.push(Op::JumpIfZero(usize::MAX));
                for b in body {
                    self.stmt(b);
                }
                self.ops.push(Op::Jump(start));
                let end = self.ops.len();
                self.ops[exit] = Op::JumpIfZero(end);
            }
        }
    }
    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::TimeCall { hash, args, .. } => {
                for a in args {
                    self.expr(a);
                }
                self.ops.push(Op::TimeCall(hash.clone(), args.len()));
            }
            Expr::Int(v) => self.ops.push(Op::Const(Value::Int(*v))),
            Expr::Float(v) => self.ops.push(Op::Const(Value::Float(*v))),
            Expr::Str(s) => self.ops.push(Op::Const(Value::Str(s.clone()))),
            Expr::Var(id) => {
                let sl = self.slot(id.span, &id.name);
                self.ops.push(Op::Load(sl));
            }
            Expr::FunRef(id) => {
                let d = self.defid(id.span, &id.name);
                self.ops.push(Op::Const(Value::Fun(self.rep.get(d).copied().unwrap_or(d))));
            }
            Expr::Lambda {
                lifted, captures, ..
            } => {
                for c in captures {
                    let sl = self.slot(c.span, &c.name);
                    self.ops.push(Op::Load(sl));
                }
                let d = match lifted {
                    Some(id) => self.defid(id.span, &id.name),
                    None => usize::MAX,
                };
                let d = self.rep.get(d).copied().unwrap_or(d);
                self.ops.push(Op::MakeClosure(d, captures.len() as u32));
            }
            Expr::Call { callee, args } => {
                for a in args {
                    self.expr(a);
                }
                let d = self.defid(callee.span, &callee.name);
                self.ops.push(Op::Call(d, args.len() as u8));
            }
            Expr::DynCall {
                targets,
                callee,
                args,
            } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
                let set: Vec<DefId> = targets
                    .iter()
                    .map(|t| self.defid(t.span, &t.name))
                    .map(|d| self.rep.get(d).copied().unwrap_or(d))
                    .collect();
                self.ops.push(Op::DynCall(set, args.len() as u8));
            }
            Expr::TableDispatch { table, key, args } => {
                match binding(self.ws, self.def, table.span, &table.name) {
                    Binding::Local(sl) => {
                        self.ops.push(Op::Load(sl));
                        self.expr(key);
                        self.ops.push(Op::Index);
                    }
                    Binding::Def(d) => match args {
                        Some(a) => {
                            self.expr(key);
                            for x in a {
                                self.expr(x);
                            }
                            self.ops.push(Op::TCall(d, a.len() as u8));
                        }
                        None => {
                            self.expr(key);
                            self.ops.push(Op::TLookup(d));
                        }
                    },
                    Binding::Broken => self.ops.push(Op::Const(Value::Int(0))),
                }
            }
            Expr::Binop { op, l, r } => {
                self.expr(l);
                self.expr(r);
                self.ops.push(Op::Bin(*op));
            }
            Expr::If { c, t, e: el } => {
                self.expr(c);
                let j1 = self.ops.len();
                self.ops.push(Op::JumpIfZero(usize::MAX));
                self.expr(t);
                let j2 = self.ops.len();
                self.ops.push(Op::Jump(usize::MAX));
                let else_at = self.ops.len();
                self.ops[j1] = Op::JumpIfZero(else_at);
                self.expr(el);
                let end = self.ops.len();
                self.ops[j2] = Op::Jump(end);
            }
            Expr::Neg(x) => {
                self.expr(x);
                self.ops.push(Op::Neg);
            }
            Expr::Print { arg, .. } => {
                self.expr(arg);
                self.ops.push(Op::Print);
            }
            Expr::Input { .. } => self.ops.push(Op::Input),
            Expr::ListLit(xs) => {
                for x in xs {
                    self.expr(x);
                }
                self.ops.push(Op::MakeList(xs.len() as u32));
            }
            Expr::RecordLit(fs) => {
                // Fields evaluate in canonical (name-sorted) order, not
                // source order. Canonical form sorts them, so two literals
                // that differ only in field order share a hash; if they also
                // evaluated in different orders, a `fail` or `print` in two
                // fields would make one hash mean two behaviours.
                let mut order: Vec<&(Ident, Expr)> = fs.iter().collect();
                order.sort_by(|a, b| a.0.name.cmp(&b.0.name));
                for (_, v) in &order {
                    self.expr(v);
                }
                self.ops.push(Op::MakeRecord(
                    order.iter().map(|(n, _)| n.name.clone()).collect(),
                ));
            }
            Expr::Get { base, field } => {
                self.expr(base);
                self.ops.push(Op::GetField(field.name.clone()));
            }
            Expr::IndexE { base, key } => {
                self.expr(base);
                self.expr(key);
                self.ops.push(Op::Index);
            }
            Expr::Len(x) => {
                self.expr(x);
                self.ops.push(Op::Len);
            }
            Expr::Push(a, b) => {
                self.expr(a);
                self.expr(b);
                self.ops.push(Op::PushElem);
            }
            Expr::StrConv(x) => {
                self.expr(x);
                self.ops.push(Op::StrConv);
            }
            Expr::IntConv(x) => {
                self.expr(x);
                self.ops.push(Op::IntConv);
            }
            Expr::FloatConv(x) => {
                self.expr(x);
                self.ops.push(Op::FloatConv);
            }
            Expr::Substr(a, b, c) => {
                self.expr(a);
                self.expr(b);
                self.expr(c);
                self.ops.push(Op::Substr);
            }
            Expr::Chars(x) => {
                self.expr(x);
                self.ops.push(Op::Chars);
            }
            Expr::Join(a, b) => {
                self.expr(a);
                self.expr(b);
                self.ops.push(Op::Join);
            }
            Expr::ReadFile { path, .. } => {
                self.expr(path);
                self.ops.push(Op::ReadFile);
            }
            Expr::WriteFile { path, contents, .. } => {
                self.expr(path);
                self.expr(contents);
                self.ops.push(Op::WriteFile);
            }
            Expr::Fail(x) => {
                self.expr(x);
                self.ops.push(Op::Fail);
            }
            Expr::Try(x) => {
                let mut sub = C {
                    ws: self.ws,
                    rep: self.rep,
                    def: self.def,
                    ops: Vec::new(),
                    subs: Vec::new(),
                };
                sub.expr(x);
                let ch = Chunk {
                    ops: sub.ops,
                    subs: sub.subs,
                };
                self.subs.push(ch);
                self.ops.push(Op::TryBlock(self.subs.len() - 1));
            }
        }
    }
}

// ---------------------------------------------------------------- engine

/// The version of what the evaluator does. Cached results (`test`, `memo`)
/// are keyed by it as well as by content hash: the hash pins what the code
/// says, this pins what running it means, so a release that changes
/// behaviour cannot serve answers computed under the old rules. Bump it with
/// any change a program could observe.
///
/// 2: checked integer arithmetic, record fields evaluated in canonical
///    order, functions shown by hash, -0.0 kept distinct in memo keys.
pub const SEMANTICS: &str = "rigid-semantics/2";

/// How many calls may be in progress at once. Past it, the call fails with
/// an ordinary, catchable error instead of exhausting the host's stack — and
/// the JavaScript target enforces the same number, so the two engines fail at
/// the same depth with the same message.
///
/// Ten thousand is what every host can actually provide: the CLI runs on a
/// thread sized for it, the playground's wasm module is linked with a stack
/// for it, and JavaScript engines reach it — or, on a small host stack, fail
/// earlier with this same error.
pub const MAX_CALL_DEPTH: u32 = 10_000;

pub struct Engine<'a> {
    /// calls currently in progress (see MAX_CALL_DEPTH)
    depth: std::cell::Cell<u32>,
    memo_hash: Option<std::collections::HashMap<DefId, String>>,
    memo_path: Option<std::path::PathBuf>,
    memo_store: std::cell::RefCell<std::collections::HashMap<String, serde_json::Value>>,
    pub ws: &'a Workspace,
    chunks: Vec<Option<Chunk>>,
    arities: Vec<usize>,
    /// def -> the first definition with the same hash. Function values are
    /// built from this, so `&f == &g` and dyn membership compare identities
    /// rather than names when f and g are the same code.
    rep: Vec<DefId>,
}

fn errs(s: String) -> Value {
    Value::Str(s)
}

/// A file error by kind, not in the operating system's own words: those
/// differ between platforms, and a program can read them through `try`.
#[cfg(not(target_arch = "wasm32"))]
fn io_kind(e: &std::io::Error, verb: &str) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "no such file".to_string(),
        std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        _ => format!("cannot {}", verb),
    }
}

/// The message for a call past MAX_CALL_DEPTH; the JS prelude spells it the
/// same way.
pub fn depth_error() -> String {
    format!("call depth limit exceeded ({} nested calls)", MAX_CALL_DEPTH)
}

impl<'a> Engine<'a> {
    pub fn new(ws: &'a Workspace) -> Engine<'a> {
        assert_eq!(
            ws.ids.len(),
            ws.defs.len(),
            "Engine::new needs a hashed workspace (canon::hash_into)"
        );
        let mut first: HashMap<Hash, DefId> = HashMap::new();
        let rep: Vec<DefId> = (0..ws.defs.len())
            .map(|d| match ws.ids[d] {
                Some(h) => *first.entry(h).or_insert(d),
                None => d,
            })
            .collect();
        let mut chunks: Vec<Option<Chunk>> = vec![None; ws.defs.len()];
        let mut arities = vec![0usize; ws.defs.len()];
        for d in 0..ws.defs.len() {
            if ws.defs[d].kind != DefKind::Fn || ws.defs[d].poisoned {
                continue;
            }
            let info = &ws.defs[d];
            let f = &ws.modules[info.module_idx].fns[info.item_idx];
            arities[d] = f.params.len();
            let mut c = C {
                ws,
                rep: &rep,
                def: d,
                ops: Vec::new(),
                subs: Vec::new(),
            };
            for s in &f.body {
                c.stmt(s);
            }
            chunks[d] = Some(Chunk {
                ops: c.ops,
                subs: c.subs,
            });
        }
        Engine {
            depth: std::cell::Cell::new(0),
            memo_hash: None,
            memo_path: None,
            memo_store: std::cell::RefCell::new(std::collections::HashMap::new()),
            ws, chunks, arities, rep,
        }
    }

    /// Run one call's body, counted against MAX_CALL_DEPTH.
    fn enter(&self, chunk: &Chunk, env: &mut Vec<Value>) -> Result<Option<Value>, Value> {
        let d = self.depth.get();
        if d >= MAX_CALL_DEPTH {
            return Err(errs(depth_error()));
        }
        self.depth.set(d + 1);
        let out = self.run(chunk, env);
        self.depth.set(d);
        out
    }

    /// Fast path used by compiled call ops: the target is known to be a
    /// non-poisoned function; only arity needs a runtime check.
    fn call_fast(&self, def: DefId, mut args: Vec<Value>) -> Result<Value, Value> {
        match &self.chunks[def] {
            Some(chunk) => {
                let want = self.arities[def];
                if args.len() != want {
                    return Err(errs(format!(
                        "{} expects {} argument(s), got {}",
                        crate::canon::tag(self.ws, def),
                        want,
                        args.len()
                    )));
                }
                match self.enter(chunk, &mut args)? {
                    Some(v) => Ok(v),
                    None => Err(errs(format!(
                        "{} finished without `return`",
                        crate::canon::tag(self.ws, def)
                    ))),
                }
            }
            None => self.call(def, args),
        }
    }

    pub fn with_memo(
        mut self,
        hashes: &std::collections::HashMap<DefId, crate::canon::Hash>,
        root: &std::path::Path,
    ) -> Engine<'a> {
        let mut m = std::collections::HashMap::new();
        for (d, h) in hashes {
            m.insert(*d, crate::canon::hex(h));
        }
        let p = match std::env::var("RIGID_CACHE_DIR") {
            Ok(d) if !d.is_empty() => {
                let d = std::path::PathBuf::from(d);
                let _ = std::fs::create_dir_all(&d);
                d.join("memo.json")
            }
            _ => root.join(".rigid-memo.json"),
        };
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str::<std::collections::HashMap<String, serde_json::Value>>(&s) {
                *self.memo_store.borrow_mut() = v;
            }
        }
        self.memo_hash = Some(m);
        self.memo_path = Some(p);
        self
    }

    /// Browser equivalent of `with_memo`: no file, but the store survives
    /// between runs, so `memo` behaves as it does on the CLI.
    #[cfg(target_arch = "wasm32")]
    pub fn with_memo_host(
        mut self,
        hashes: &std::collections::HashMap<DefId, crate::canon::Hash>,
    ) -> Engine<'a> {
        let mut m = std::collections::HashMap::new();
        for (d, h) in hashes {
            m.insert(*d, crate::canon::hex(h));
        }
        *self.memo_store.borrow_mut() = host::MEMOSTORE.with(|s| s.borrow().clone());
        self.memo_hash = Some(m);
        self
    }

    /// Publish this engine's memo store back to the browser-durable one.
    #[cfg(target_arch = "wasm32")]
    pub fn publish_memo(&self) {
        host::MEMOSTORE.with(|s| *s.borrow_mut() = self.memo_store.borrow().clone());
    }

    fn venc(v: &Value, out: &mut Vec<u8>) -> bool {
        match v {
            Value::Int(i) => { out.push(1); out.extend_from_slice(&i.to_be_bytes()); true }
            // The same bits the hash uses: -0.0 and +0.0 stay distinct,
            // because a function can tell them apart (see canon_f64).
            Value::Float(f) => { out.push(5); out.extend_from_slice(&crate::canon::canon_f64(*f).to_be_bytes()); true }
            Value::Str(s) => { out.push(2); out.extend_from_slice(&(s.len() as u32).to_be_bytes()); out.extend_from_slice(s.as_bytes()); true }
            Value::List(xs) => { out.push(3); out.extend_from_slice(&(xs.len() as u32).to_be_bytes()); xs.iter().all(|x| Self::venc(x, out)) }
            Value::Record(m) => { out.push(4); out.extend_from_slice(&(m.len() as u32).to_be_bytes());
                m.iter().all(|(k, x)| { out.extend_from_slice(&(k.len() as u32).to_be_bytes()); out.extend_from_slice(k.as_bytes()); Self::venc(x, out) }) }
            // A closure cannot be a memo key: it is code, not data.
            Value::Closure(..) | Value::Fun(_) => false,
        }
    }
    fn v2j(v: &Value) -> Option<serde_json::Value> {
        Some(match v {
            Value::Int(i) => serde_json::json!(i),
            // Bits, not a decimal: exact round-trip, and NaN and the
            // infinities survive a format that has no syntax for them.
            Value::Float(f) => serde_json::json!({"f": format!("{:016x}", crate::canon::canon_f64(*f))}),
            Value::Str(s) => serde_json::json!({"s": s}),
            Value::List(xs) => serde_json::Value::Array(xs.iter().map(Self::v2j).collect::<Option<Vec<_>>>()?),
            Value::Record(m) => {
                let mut o = serde_json::Map::new();
                for (k, x) in m { o.insert(k.clone(), Self::v2j(x)?); }
                serde_json::json!({"r": o})
            }
            Value::Closure(..) | Value::Fun(_) => return None,
        })
    }
    fn j2v(j: &serde_json::Value) -> Value {
        if let Some(i) = j.as_i64() { return Value::Int(i); }
        if let Some(a) = j.as_array() { return Value::List(a.iter().map(Self::j2v).collect()); }
        let o = j.as_object().unwrap();
        if let Some(s) = o.get("s") { return Value::Str(s.as_str().unwrap().to_string()); }
        if let Some(f) = o.get("f") {
            let bits = u64::from_str_radix(f.as_str().unwrap_or("0"), 16).unwrap_or(0);
            return Value::Float(f64::from_bits(bits));
        }
        let r = o.get("r").unwrap().as_object().unwrap();
        Value::Record(r.iter().map(|(k, x)| (k.clone(), Self::j2v(x))).collect())
    }

    /// Call a prior version by hash prefix, from the content store written on
    /// every green `check`. The past cone is materialized into a synthetic
    /// workspace and executed there — old code runs against old meanings.
    pub fn recall(&self, prefix: &str, args: Vec<Value>) -> Result<Value, Value> {
        let p = prefix.strip_prefix("sha256:").unwrap_or(prefix);
        self.timecall(p, args)
    }

    fn timecall(&self, prefix: &str, args: Vec<Value>) -> Result<Value, Value> {
        let root = self
            .memo_path
            .as_ref()
            .and_then(|p| p.parent().map(|x| x.to_path_buf()))
            .ok_or_else(|| errs("temporal calls need workspace context".into()))?;
        let store = root.join(".rigid-store");
        let idx: std::collections::HashMap<String, Vec<String>> =
            std::fs::read_to_string(store.join("index.json"))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .ok_or_else(|| errs("no .rigid-store (run `rigid check` first)".into()))?;
        let hits: Vec<&String> = idx.keys().filter(|k| k.starts_with(prefix)).collect();
        let h = match hits.as_slice() {
            [one] => (*one).clone(),
            [] => return Err(errs(format!("no stored version matches @{}", quote(prefix)))),
            _ => return Err(errs(format!("@{} is ambiguous ({} matches)", quote(prefix), hits.len()))),
        };
        let mut need = Vec::new();
        let mut st = vec![h.clone()];
        while let Some(x) = st.pop() {
            if need.contains(&x) {
                continue;
            }
            for d in idx.get(&x).cloned().unwrap_or_default() {
                st.push(d);
            }
            need.push(x);
        }
        need.sort();
        let mut src = String::new();
        for n in &need {
            let f = store.join(format!("d_{}.rg", n));
            src.push_str(&std::fs::read_to_string(&f).map_err(|_| {
                errs(format!("store is missing d_{}.rg", n))
            })?);
            src.push('\n');
        }
        let tmp = std::env::temp_dir().join(format!("rigid-tc-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(tmp.join("past.rg"), src).map_err(|x| errs(x.to_string()))?;
        let mut ws2 = crate::resolve::load_workspace(&tmp)
            .map_err(|_| errs("stored code failed to parse".into()))?;
        let diags = crate::resolve::resolve(&mut ws2);
        if diags.iter().any(|d| d.severity == crate::diag::Severity::Error) {
            return Err(errs(format!("stored cone for {} does not resolve", &h)));
        }
        let id = *ws2
            .def_index
            .get(&("past.rg".to_string(), format!("d_{}", h)))
            .ok_or_else(|| errs("stored def not found after load".into()))?;
        crate::canon::hash_into(&mut ws2);
        let eng2 = Engine::new(&ws2);
        // the past version runs nested inside this call, so it counts
        // against the same depth limit
        eng2.depth.set(self.depth.get());
        eng2.call(id, args)
    }

    pub fn call(&self, def: DefId, args: Vec<Value>) -> Result<Value, Value> {
        let info = &self.ws.defs[def];
        if info.kind == DefKind::Extern {
            let x = &self.ws.modules[info.module_idx].externs[info.item_idx];
            return Err(errs(format!(
                "extern {} has no binding in the rigid VM; compile with `rigid build --target js` to run it",
                x.host
            )));
        }
        if info.kind != DefKind::Fn {
            return Err(errs(format!(
                "{} is a table; dispatch it with a key: table[key](...)",
                crate::canon::tag(self.ws, def)
            )));
        }
        let f = &self.ws.modules[info.module_idx].fns[info.item_idx];
        if args.len() != f.params.len() {
            return Err(errs(format!(
                "{} expects {} argument(s), got {}",
                crate::canon::tag(self.ws, def),
                f.params.len(),
                args.len()
            )));
        }
        let Some(chunk) = self.chunks[def].as_ref() else {
            return Err(errs(format!(
                "{}#{} is not executable (unresolved or poisoned)",
                info.module_path, info.name
            )));
        };
        // memo: (def-hash, canonical args) -> value, persisted across runs.
        // Sound because purity is checked (E0211) and the hash covers the cone.
        let mkey = if f.memo {
            self.memo_hash.as_ref().and_then(|hm| hm.get(&def)).and_then(|h| {
                let mut buf = Vec::new();
                buf.extend_from_slice(SEMANTICS.as_bytes());
                buf.extend_from_slice(h.as_bytes());
                if args.iter().all(|a| Self::venc(a, &mut buf)) {
                    use sha2::{Digest, Sha256};
                    let mut hs = Sha256::new();
                    hs.update(&buf);
                    Some(format!("{:x}", hs.finalize()))
                } else {
                    None
                }
            })
        } else {
            None
        };
        if let Some(k) = &mkey {
            if let Some(j) = self.memo_store.borrow().get(k) {
                return Ok(Self::j2v(j));
            }
        }
        let mut env = args;
        let out = match self.enter(chunk, &mut env)? {
            Some(v) => Ok(v),
            None => Err(errs(format!(
                "{} finished without `return`",
                crate::canon::tag(self.ws, def)
            ))),
        };
        if let (Some(k), Ok(v)) = (&mkey, &out) {
            if let Some(j) = Self::v2j(v) {
                self.memo_store.borrow_mut().insert(k.clone(), j);
                if let Some(p) = &self.memo_path {
                    let _ = std::fs::write(
                        p,
                        serde_json::to_string(&*self.memo_store.borrow()).unwrap(),
                    );
                }
            }
        }
        out
    }

    fn run(&self, chunk: &Chunk, env: &mut Vec<Value>) -> Result<Option<Value>, Value> {
        let mut stack: Vec<Value> = Vec::with_capacity(8);
        let mut pc = 0usize;
        macro_rules! pop {
            () => {
                stack.pop().expect("stack underflow")
            };
        }
        while pc < chunk.ops.len() {
            match &chunk.ops[pc] {
                Op::Const(v) => stack.push(v.clone()),
                Op::Load(sl) => {
                    let i = *sl as usize;
                    stack.push(if i < env.len() {
                        env[i].clone()
                    } else {
                        Value::Int(0)
                    });
                }
                Op::Store(sl) => {
                    let v = pop!();
                    let i = *sl as usize;
                    while env.len() <= i {
                        env.push(Value::Int(0));
                    }
                    env[i] = v;
                }
                Op::Pop => {
                    pop!();
                }
                Op::Bin(op) => {
                    let r = pop!();
                    let l = pop!();
                    stack.push(binop(self.ws, *op, l, r)?);
                }
                Op::Neg => match pop!() {
                    Value::Int(v) => match v.checked_neg() {
                        Some(n) => stack.push(Value::Int(n)),
                        None => return Err(errs(format!("integer overflow: -({})", v))),
                    },
                    Value::Float(v) => stack.push(Value::Float(-v)),
                    _ => return Err(errs("`-` needs a number".to_string())),
                },
                Op::MakeClosure(d, n) => {
                    let mut caught = Vec::with_capacity(*n as usize);
                    for _ in 0..*n {
                        caught.push(pop!());
                    }
                    caught.reverse();
                    stack.push(Value::Closure(*d, caught));
                }
                Op::ForInit => {
                    let seq = match pop!() {
                        Value::List(xs) => Value::List(xs),
                        Value::Str(s) => {
                            Value::List(s.chars().map(|c| Value::Str(c.to_string())).collect())
                        }
                        other => {
                            return Err(errs(format!(
                                "`for` needs a list or string, got {}",
                                show(self.ws, &other)
                            )))
                        }
                    };
                    stack.push(seq);
                    stack.push(Value::Int(0));
                }
                Op::ForNext(slot, end) => {
                    let i = match stack.last() {
                        Some(Value::Int(i)) => *i,
                        _ => return Err(errs("corrupt for-loop cursor".into())),
                    };
                    let len = match stack.get(stack.len().wrapping_sub(2)) {
                        Some(Value::List(xs)) => xs.len() as i64,
                        _ => return Err(errs("corrupt for-loop cursor".into())),
                    };
                    if i >= len {
                        stack.pop();
                        stack.pop();
                        pc = *end;
                        continue;
                    }
                    let item = match stack.get(stack.len() - 2) {
                        Some(Value::List(xs)) => xs[i as usize].clone(),
                        _ => unreachable!(),
                    };
                    let n = stack.len();
                    stack[n - 1] = Value::Int(i + 1);
                    let s = *slot as usize;
                    while env.len() <= s {
                        env.push(Value::Int(0));
                    }
                    env[s] = item;
                }
                Op::Jump(t) => {
                    pc = *t;
                    continue;
                }
                Op::JumpIfZero(t) => match pop!() {
                    Value::Int(0) => {
                        pc = *t;
                        continue;
                    }
                    Value::Int(_) => {}
                    other => {
                        return Err(errs(format!(
                            "condition must be an integer, got {}",
                            show(self.ws, &other)
                        )))
                    }
                },
                Op::TimeCall(h, argc) => {
                    let n = *argc;
                    let args = stack.split_off(stack.len() - n);
                    stack.push(self.timecall(h, args)?);
                }
                Op::Call(d, argc) => {
                    let n = *argc as usize;
                    let args = stack.split_off(stack.len() - n);
                    let di = &self.ws.defs[*d];
                    let is_memo = di.kind == crate::resolve::DefKind::Fn
                        && self.ws.modules[di.module_idx].fns[di.item_idx].memo;
                    if is_memo {
                        stack.push(self.call(*d, args)?);
                    } else {
                        stack.push(self.call_fast(*d, args)?);
                    }
                }
                Op::DynCall(set, argc) => {
                    let n = *argc as usize;
                    let args = stack.split_off(stack.len() - n);
                    let cv = pop!();
                    // A closure arrives with its captured values already
                    // bound; they lead the argument list, exactly as the
                    // lifted definition's signature declares.
                    let (d, args) = match cv {
                        Value::Fun(d) => (d, args),
                        Value::Closure(d, caught) => {
                            let mut a = caught;
                            a.extend(args);
                            (d, a)
                        }
                        other => {
                            return Err(errs(format!(
                                "dyn callee must be a function reference, got {}",
                                show(self.ws, &other)
                            )))
                        }
                    };
                    if !set.contains(&d) {
                        // sorted, so the message is as order-free as the
                        // set's canonical form
                        let mut declared: Vec<String> = set
                            .iter()
                            .map(|&t| crate::canon::tag(self.ws, t))
                            .collect();
                        declared.sort();
                        return Err(errs(format!(
                            "quarantine violation: {} is not in the declared set {{ {} }}",
                            show(self.ws, &Value::Fun(d)),
                            declared.join(", ")
                        )));
                    }
                    stack.push(self.call_fast(d, args)?);
                }
                Op::TCall(td, argc) => {
                    let n = *argc as usize;
                    let args = stack.split_off(stack.len() - n);
                    let k = pop!();
                    let d = self.table_target(*td, &k)?;
                    stack.push(self.call_fast(d, args)?);
                }
                Op::TLookup(td) => {
                    let k = pop!();
                    let d = self.table_target(*td, &k)?;
                    stack.push(Value::Fun(self.rep[d]));
                }
                Op::Index => {
                    let k = pop!();
                    let b = pop!();
                    stack.push(index_value(self.ws, &b, &k)?);
                }
                Op::GetField(name) => {
                    let b = pop!();
                    match b {
                        Value::Record(m) => match m.get(name) {
                            Some(v) => stack.push(v.clone()),
                            None => {
                                let keys: Vec<&str> = m.keys().map(|k| k.as_str()).collect();
                                return Err(errs(format!(
                                    "no field '{}' (fields: {})",
                                    name,
                                    keys.join(", ")
                                )));
                            }
                        },
                        other => {
                            return Err(errs(format!(
                                "cannot access field on {}",
                                show(self.ws, &other)
                            )))
                        }
                    }
                }
                Op::MakeList(n) => {
                    let xs = stack.split_off(stack.len() - *n as usize);
                    stack.push(Value::List(xs));
                }
                Op::MakeRecord(names) => {
                    let vals = stack.split_off(stack.len() - names.len());
                    let mut m = BTreeMap::new();
                    for (n, v) in names.iter().zip(vals) {
                        if m.insert(n.clone(), v).is_some() {
                            return Err(errs(format!("duplicate field '{}' in record", n)));
                        }
                    }
                    stack.push(Value::Record(m));
                }
                Op::Len => match pop!() {
                    Value::List(xs) => stack.push(Value::Int(xs.len() as i64)),
                    Value::Str(s) => stack.push(Value::Int(s.chars().count() as i64)),
                    other => {
                        return Err(errs(format!(
                            "len needs a list or string, got {}",
                            show(self.ws, &other)
                        )))
                    }
                },
                Op::PushElem => {
                    let v = pop!();
                    match pop!() {
                        Value::List(mut xs) => {
                            xs.push(v);
                            stack.push(Value::List(xs));
                        }
                        other => {
                            return Err(errs(format!(
                                "push needs a list, got {}",
                                show(self.ws, &other)
                            )))
                        }
                    }
                }
                Op::StrConv => match pop!() {
                    Value::Int(v) => stack.push(Value::Str(v.to_string())),
                    Value::Float(v) => stack.push(Value::Str(crate::fmt::fmt_f64(v))),
                    Value::Str(s) => stack.push(Value::Str(s)),
                    other => {
                        return Err(errs(format!(
                            "str needs a number or string, got {}",
                            show(self.ws, &other)
                        )))
                    }
                },
                Op::FloatConv => match pop!() {
                    Value::Float(v) => stack.push(Value::Float(v)),
                    Value::Int(v) => stack.push(Value::Float(v as f64)),
                    Value::Str(s) => match s.trim().parse::<f64>() {
                        Ok(v) => stack.push(Value::Float(v)),
                        Err(_) => return Err(errs(format!("cannot parse {} as a float", quote(&s)))),
                    },
                    other => {
                        return Err(errs(format!(
                            "float needs a string, int, or float, got {}",
                            show(self.ws, &other)
                        )))
                    }
                },
                Op::IntConv => match pop!() {
                    Value::Int(v) => stack.push(Value::Int(v)),
                    // Truncation toward zero, which is what `int` has always
                    // meant here; rounding would be a different builtin. A
                    // float no i64 can hold is an error, not a saturated
                    // or zeroed integer.
                    Value::Float(v) => stack.push(Value::Int(float_to_int(v).map_err(errs)?)),
                    Value::Str(s) => match s.trim().parse::<i64>() {
                        Ok(v) => stack.push(Value::Int(v)),
                        Err(_) => {
                            return Err(errs(format!("cannot parse {} as an integer", quote(&s))))
                        }
                    },
                    other => {
                        return Err(errs(format!(
                            "int needs a string or int, got {}",
                            show(self.ws, &other)
                        )))
                    }
                },
                Op::Substr => {
                    let (c, b, a) = (pop!(), pop!(), pop!());
                    let Value::Str(s) = a else {
                        return Err(errs("substr needs a string".to_string()));
                    };
                    let (Value::Int(start), Value::Int(count)) = (b, c) else {
                        return Err(errs("substr start/count must be integers".to_string()));
                    };
                    let cs: Vec<char> = s.chars().collect();
                    if start < 0 || start as usize > cs.len() || count < 0 {
                        return Err(errs(format!(
                            "substr out of range (start {}, count {}, len {})",
                            start,
                            count,
                            cs.len()
                        )));
                    }
                    let end = (start.saturating_add(count) as usize).min(cs.len());
                    stack.push(Value::Str(cs[start as usize..end].iter().collect()));
                }
                Op::Chars => match pop!() {
                    Value::Str(s) => stack.push(Value::List(
                        s.chars().map(|ch| Value::Str(ch.to_string())).collect(),
                    )),
                    other => {
                        return Err(errs(format!(
                            "chars needs a string, got {}",
                            show(self.ws, &other)
                        )))
                    }
                },
                Op::Join => {
                    let sep = pop!();
                    let xs = pop!();
                    let (Value::List(xs), Value::Str(sep)) = (xs, sep) else {
                        return Err(errs("join needs a list and a string separator".to_string()));
                    };
                    let mut parts = Vec::with_capacity(xs.len());
                    for x in &xs {
                        match x {
                            Value::Str(s) => parts.push(s.clone()),
                            other => {
                                return Err(errs(format!(
                                    "join needs strings, found {}",
                                    show(self.ws, other)
                                )))
                            }
                        }
                    }
                    stack.push(Value::Str(parts.join(&sep)));
                }
                Op::Print => {
                    let v = pop!();
                    // strings print as their text; other values print in
                    // their canonical display form
                    match &v {
                        Value::Str(s) => sayln!("{}", s),
                        other => sayln!("{}", show(self.ws, other)),
                    }
                    stack.push(v);
                }
                Op::Input => {
                    #[cfg(target_arch = "wasm32")]
                    {
                        return Err(errs(
                            "input() is not available in the browser playground".to_string(),
                        ));
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        let mut line = String::new();
                        std::io::stdin()
                            .read_line(&mut line)
                            .map_err(|e| errs(format!("input failed: {}", e)))?;
                        while line.ends_with('\n') || line.ends_with('\r') {
                            line.pop();
                        }
                        stack.push(Value::Str(line));
                    }
                }
                Op::ReadFile => {
                    let p = pop!();
                    let Value::Str(p) = p else {
                        return Err(errs("read_file path must be a string".to_string()));
                    };
                    #[cfg(target_arch = "wasm32")]
                    {
                        match host::FILES.with(|f| f.borrow().get(&p).cloned()) {
                            Some(s) => stack.push(Value::Str(s)),
                            None => {
                                return Err(errs(format!("read_file {}: no such file", quote(&p))));
                            }
                        }
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    match std::fs::read_to_string(&p) {
                        Ok(s) => stack.push(Value::Str(s)),
                        Err(er) => return Err(errs(format!("read_file {}: {}", quote(&p), io_kind(&er, "read")))),
                    }
                }
                Op::WriteFile => {
                    let v = pop!();
                    let p = pop!();
                    let Value::Str(p) = p else {
                        return Err(errs("write_file path must be a string".to_string()));
                    };
                    let text = match &v {
                        Value::Str(s) => s.clone(),
                        other => show(self.ws, other),
                    };
                    #[cfg(target_arch = "wasm32")]
                    host::FILES.with(|f| f.borrow_mut().insert(p.clone(), text));
                    #[cfg(not(target_arch = "wasm32"))]
                    std::fs::write(&p, text)
                        .map_err(|er| errs(format!("write_file {}: {}", quote(&p), io_kind(&er, "write"))))?;
                    stack.push(v);
                }
                Op::Fail => {
                    let v = pop!();
                    return Err(v);
                }
                Op::TryBlock(i) => {
                    let mut m = BTreeMap::new();
                    match self.run(&chunk.subs[*i], env) {
                        Ok(Some(v)) => {
                            m.insert("ok".to_string(), Value::Int(1));
                            m.insert("value".to_string(), v);
                        }
                        Ok(None) => {
                            return Err(errs("try operand produced no value".to_string()))
                        }
                        Err(err) => {
                            m.insert("ok".to_string(), Value::Int(0));
                            m.insert("error".to_string(), err);
                        }
                    }
                    stack.push(Value::Record(m));
                }
                Op::Return => return Ok(Some(pop!())),
            }
            pc += 1;
        }
        // an expression sub-chunk ends here with its value on the stack
        Ok(stack.pop().map(Some).unwrap_or(None))
    }

    fn table_target(&self, td: DefId, k: &Value) -> Result<DefId, Value> {
        let Value::Str(k) = k else {
            return Err(errs(format!(
                "table key must be a string, got {}",
                show(self.ws, k)
            )));
        };
        let entries = self
            .ws
            .table_entries
            .get(&td)
            .ok_or_else(|| errs(format!("table {} has no entries at runtime", crate::canon::tag(self.ws, td))))?;
        match entries.iter().find(|(ek, _)| ek == k) {
            Some((_, t)) => Ok(*t),
            None => {
                let mut keys: Vec<&str> = entries.iter().map(|(ek, _)| ek.as_str()).collect();
                keys.sort();
                Err(errs(format!(
                    "no entry {} in table {} (keys: {})",
                    quote(k),
                    crate::canon::tag(self.ws, td),
                    keys.join(", ")
                )))
            }
        }
    }
}

fn index_value(ws: &Workspace, base: &Value, key: &Value) -> Result<Value, Value> {
    match (base, key) {
        (Value::List(xs), Value::Int(i)) => {
            let i = *i;
            if i < 0 || i as usize >= xs.len() {
                return Err(errs(format!("index {} out of bounds (len {})", i, xs.len())));
            }
            Ok(xs[i as usize].clone())
        }
        (Value::Str(s), Value::Int(i)) => {
            let i = *i;
            match s.chars().nth(i.max(0) as usize) {
                Some(c) if i >= 0 => Ok(Value::Str(c.to_string())),
                _ => Err(errs(format!(
                    "index {} out of bounds (len {})",
                    i,
                    s.chars().count()
                ))),
            }
        }
        (Value::List(_), k) | (Value::Str(_), k) => Err(errs(format!(
            "index must be an integer, got {}",
            show(ws, k)
        ))),
        (b, _) => Err(errs(format!("cannot index into {}", show(ws, b)))),
    }
}

/// Float arithmetic. Division by zero is an *error* for ints and an infinity
/// for floats, matching IEEE-754 — the two number types are deliberately not
/// pretending to be one thing.
fn float_op(op: BinOp, a: f64, b: f64) -> Value {
    use BinOp::*;
    match op {
        Add => Value::Float(a + b),
        Sub => Value::Float(a - b),
        Mul => Value::Float(a * b),
        Div => Value::Float(a / b),
        Rem => Value::Float(a % b),
        Lt => Value::Int((a < b) as i64),
        Gt => Value::Int((a > b) as i64),
        Le => Value::Int((a <= b) as i64),
        Ge => Value::Int((a >= b) as i64),
        Eq | Ne => unreachable!("handled before promotion"),
    }
}

/// Order an integer against a float exactly. Rounding the integer to a
/// float first would make 2^53 + 1 equal to 2^53 as a float, and equality
/// stop being transitive. None when the float is NaN, which is unordered.
fn cmp_int_float(a: i64, b: f64) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering::*;
    if b.is_nan() {
        return None;
    }
    // 2^63 is exactly representable, and every i64 is below it.
    if b >= 9223372036854775808.0 {
        return Some(Less);
    }
    if b < -9223372036854775808.0 {
        return Some(Greater);
    }
    // In range, so the truncation is an exact i64.
    let t = b.trunc();
    match a.cmp(&(t as i64)) {
        Equal if b > t => Some(Less),
        Equal if b < t => Some(Greater),
        o => Some(o),
    }
}

fn compare(op: BinOp, o: Option<std::cmp::Ordering>) -> Value {
    use std::cmp::Ordering::*;
    let r = match op {
        BinOp::Eq => o == Some(Equal),
        BinOp::Ne => o != Some(Equal),
        BinOp::Lt => o == Some(Less),
        BinOp::Gt => o == Some(Greater),
        BinOp::Le => matches!(o, Some(Less | Equal)),
        BinOp::Ge => matches!(o, Some(Greater | Equal)),
        _ => unreachable!("not a comparison"),
    };
    Value::Int(r as i64)
}

fn is_comparison(op: BinOp) -> bool {
    use BinOp::*;
    matches!(op, Eq | Ne | Lt | Gt | Le | Ge)
}

fn binop(ws: &Workspace, op: BinOp, l: Value, r: Value) -> Result<Value, Value> {
    use BinOp::*;
    match (op, l, r) {
        // `1 == 1.0` is true: comparison follows the value, not the variant,
        // and it is exact rather than rounded.
        (op, Value::Int(a), Value::Float(b)) if is_comparison(op) => {
            Ok(compare(op, cmp_int_float(a, b)))
        }
        (op, Value::Float(a), Value::Int(b)) if is_comparison(op) => {
            Ok(compare(op, cmp_int_float(b, a).map(|o| o.reverse())))
        }
        (Eq, a, b) => Ok(Value::Int((a == b) as i64)),
        (Ne, a, b) => Ok(Value::Int((a != b) as i64)),
        (Add, Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{}{}", a, b))),
        (Add, Value::List(a), Value::List(b)) => {
            let mut out = a.clone();
            out.extend(b.iter().cloned());
            Ok(Value::List(out))
        }
        // Strings order lexicographically by Unicode scalar value, which is
        // what `sort` needs and what two string literals already compare as
        // under `==`. Only the ordering operators: `-` on strings stays an
        // error, because there is no sensible answer.
        (Lt, Value::Str(a), Value::Str(b)) => Ok(Value::Int((a < b) as i64)),
        (Gt, Value::Str(a), Value::Str(b)) => Ok(Value::Int((a > b) as i64)),
        (Le, Value::Str(a), Value::Str(b)) => Ok(Value::Int((a <= b) as i64)),
        (Ge, Value::Str(a), Value::Str(b)) => Ok(Value::Int((a >= b) as i64)),
        // A float anywhere in an arithmetic pair promotes the other side.
        // Int/Int stays exact — widening it would silently lose precision at
        // 2^53, which is the kind of quiet wrongness this language exists to
        // refuse.
        (_, Value::Float(a), Value::Float(b)) => Ok(float_op(op, a, b)),
        (_, Value::Float(a), Value::Int(b)) => Ok(float_op(op, a, b as f64)),
        (_, Value::Int(a), Value::Float(b)) => Ok(float_op(op, a as f64, b)),
        // Integers are 64-bit and checked: a result that does not fit is an
        // error, never a wrapped number. Wrapping is the same quiet
        // wrongness as rounding.
        (_, Value::Int(a), Value::Int(b)) => {
            let overflow = || errs(format!("integer overflow: {} {} {}", a, op.symbol(), b));
            let v = match op {
                Add => a.checked_add(b).ok_or_else(overflow)?,
                Sub => a.checked_sub(b).ok_or_else(overflow)?,
                Mul => a.checked_mul(b).ok_or_else(overflow)?,
                Div => {
                    if b == 0 {
                        return Err(errs("division by zero".to_string()));
                    }
                    a.checked_div(b).ok_or_else(overflow)?
                }
                Rem => {
                    if b == 0 {
                        return Err(errs("remainder by zero".to_string()));
                    }
                    // MIN % -1 is 0, which fits; only the CPU instruction
                    // overflows, and wrapping_rem returns the true answer.
                    a.wrapping_rem(b)
                }
                Lt => (a < b) as i64,
                Gt => (a > b) as i64,
                Le => (a <= b) as i64,
                Ge => (a >= b) as i64,
                Eq | Ne => unreachable!(),
            };
            Ok(Value::Int(v))
        }
        (_, a, b) => Err(errs(format!(
            "operator needs numbers (got {} and {})",
            show(ws, &a),
            show(ws, &b)
        ))),
    }
}

/// An f64 truncated toward zero, or an error when no i64 holds it.
pub fn float_to_int(v: f64) -> Result<i64, String> {
    let t = v.trunc();
    if t.is_nan() || t < -9223372036854775808.0 || t >= 9223372036854775808.0 {
        return Err(format!(
            "cannot convert {} to an integer: out of range",
            crate::fmt::fmt_f64(v)
        ));
    }
    Ok(t as i64)
}

/// A command-line argument's value. `2` is an int, `2.0` a float, and
/// anything else a string — the same reading the lexer gives a literal, so
/// `rigid run f#area 2.0` and `area(2.0)` mean the same thing. The playground
/// and the JS target's CLI read arguments by this same rule.
pub fn cli_value(s: &str) -> Value {
    if let Ok(v) = s.parse::<i64>() {
        return Value::Int(v);
    }
    if s.contains('.') {
        if let Ok(v) = s.parse::<f64>() {
            return Value::Float(v);
        }
    }
    Value::Str(s.to_string())
}

/// A runtime message with a legend for the definitions it mentions.
///
/// Values and errors name definitions by hash (`#6ef3a5556d27`), because a
/// name is not part of what a definition is. People still want names, so the
/// CLIs append them, for each tag that belongs to this workspace:
/// `... [#6ef3a5556d27 = util.rg#clamp]`. This is presentation only — a
/// program never sees it — and the JS CLI prints the same legend.
pub fn annotate(ws: &Workspace, msg: &str) -> String {
    let mut tags: Vec<&str> = Vec::new();
    let b = msg.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#'
            && i + 13 <= b.len()
            && b[i + 1..i + 13].iter().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            && !(i + 13 < b.len() && b[i + 13].is_ascii_hexdigit())
        {
            let t = &msg[i..i + 13];
            if !tags.contains(&t) {
                tags.push(t);
            }
            i += 13;
        } else {
            i += 1;
        }
    }
    let mut legend = Vec::new();
    for t in tags {
        let mut names: Vec<String> = (0..ws.defs.len())
            .filter(|&d| crate::canon::tag(ws, d) == t)
            .map(|d| format!("{}#{}", ws.defs[d].module_path, ws.defs[d].name))
            .collect();
        if !names.is_empty() {
            names.sort();
            legend.push(format!("{} = {}", t, names.join(" | ")));
        }
    }
    if legend.is_empty() {
        msg.to_string()
    } else {
        format!("{}  [{}]", msg, legend.join(", "))
    }
}

/// A string in quotes, escaped by one fixed rule both engines share:
/// `\"` `\\` `\n` `\r` `\t` `\0` by name, every other control character
/// (C0, DEL, C1) as `\u{hex}`, and everything else as itself. Rust's own
/// Debug escaping depends on the compiler's Unicode tables, so a display
/// built on it could change with the toolchain.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)) => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// How many generated cases a property is checked against. High enough to
/// find real edge cases, low enough that a workspace full of properties is
/// still a fast `check`.
const PROPERTY_CASES: u32 = 200;

/// Check one property over generated inputs, shrinking any counterexample.
///
/// Returns the number of cases that passed, or a description of the smallest
/// input that broke it.
fn check_property(
    eng: &Engine,
    ws: &Workspace,
    d: DefId,
    hash: &Hash,
    domains: &[Option<crate::domains::Domain>],
) -> Result<u32, String> {
    use crate::property::{shrink_args, show_args, Gen};
    let arity = ws.modules[ws.defs[d].module_idx].fns[ws.defs[d].item_idx]
        .params
        .len();
    if arity == 0 {
        return match eng.call(d, vec![]) {
            Ok(Value::Int(v)) if v != 0 => Ok(1),
            Ok(v) => Err(format!("returned {}", show(ws, &v))),
            Err(e) => Err(format!("runtime error: {}", show(ws, &e))),
        };
    }
    if domains
        .iter()
        .take(arity)
        .any(|x| *x == Some(crate::domains::Domain::Record))
    {
        return Err(
            "a parameter's domain is a record, and a record's shape cannot be inferred from              how it is used — nothing honest can be generated for it"
                .to_string(),
        );
    }

    // Seeded from the property's own identity, so the same code always draws
    // the same inputs: the result is cacheable and a failure reproduces.
    // An explicit hint wins over inference: it was written precisely because
    // inference could not tell a list from a string.
    let f = &ws.modules[ws.defs[d].module_idx].fns[ws.defs[d].item_idx];
    let want: Vec<Option<crate::domains::Domain>> = (0..arity)
        .map(|k| match f.gen_hints.get(k).and_then(|h| h.as_deref()) {
            Some("int") => Some(crate::domains::Domain::Int),
            Some("float") => Some(crate::domains::Domain::Float),
            Some("str") => Some(crate::domains::Domain::Str),
            Some("list") => Some(crate::domains::Domain::List),
            _ => domains.get(k).copied().flatten(),
        })
        .collect();

    let mut g = Gen::from_hash(hash);
    let mut failing: Option<(Vec<Value>, String)> = None;
    let mut n = 0u32;
    for i in 0..PROPERTY_CASES {
        // Size ramps so the small inputs — where the bugs are — come first.
        let size = 1 + (i / 8);
        let args: Vec<Value> = (0..arity).map(|k| g.value(want[k], size)).collect();
        match eng.call(d, args.clone()) {
            Ok(Value::Int(v)) if v != 0 => n += 1,
            Ok(v) => {
                failing = Some((args, format!("returned {}", show(ws, &v))));
                break;
            }
            Err(e) => {
                failing = Some((args, format!("runtime error: {}", show(ws, &e))));
                break;
            }
        }
    }
    let Some((args, why)) = failing else {
        return Ok(n);
    };

    // Shrink: keep taking the first simpler input that still fails, until
    // nothing simpler does. Bounded, because a pathological property could
    // otherwise walk a very large space.
    let mut best = args;
    let mut best_why = why;
    for _ in 0..500 {
        let mut improved = false;
        for cand in shrink_args(&best) {
            let bad = match eng.call(d, cand.clone()) {
                Ok(Value::Int(v)) if v != 0 => None,
                Ok(v) => Some(format!("returned {}", show(ws, &v))),
                Err(e) => Some(format!("runtime error: {}", show(ws, &e))),
            };
            if let Some(w) = bad {
                best = cand;
                best_why = w;
                improved = true;
                break;
            }
        }
        if !improved {
            break;
        }
    }
    Err(format!(
        "falsified by ({}) — {}",
        show_args(ws, &best),
        best_why
    ))
}

// ---------------------------------------------------------------- tests

pub struct TestReport {
    pub executed: usize,
    pub skipped: usize,
    pub failed: usize,
}

pub fn run_tests(ws: &Workspace, hashes: &HashMap<DefId, Hash>, no_cache: bool) -> TestReport {
    run_tests_opts(ws, hashes, no_cache, false)
}

/// `quiet` reports only the tests that actually executed. `watch` uses it:
/// after a rename the interesting output is that nothing ran, and twenty-two
/// lines saying "cached" bury that.
pub fn run_tests_opts(
    ws: &Workspace,
    hashes: &HashMap<DefId, Hash>,
    no_cache: bool,
    quiet: bool,
) -> TestReport {
    let eng = Engine::new(ws);
    // RIGID_CACHE_DIR lets CI and a laptop share one cache. Keys are content
    // hashes plus SEMANTICS, so a result computed anywhere by the same
    // evaluator is valid everywhere.
    #[cfg(not(target_arch = "wasm32"))]
    let cache_path = match std::env::var("RIGID_CACHE_DIR") {
        Ok(d) if !d.is_empty() => {
            let p = std::path::PathBuf::from(d);
            let _ = std::fs::create_dir_all(&p);
            p.join("testcache.json")
        }
        _ => ws.root.join(".rigid-testcache.json"),
    };
    // On wasm the cache lives in the worker for the life of the page, so a
    // second run in the playground is a genuine content-addressed cache hit.
    #[cfg(target_arch = "wasm32")]
    let mut cache: BTreeMap<String, bool> = host::TESTCACHE.with(|c| c.borrow().clone());
    #[cfg(not(target_arch = "wasm32"))]
    let mut cache: BTreeMap<String, bool> = std::fs::read_to_string(&cache_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let param_domains = crate::domains::generation_domains(ws);
    let mut tests: Vec<DefId> = (0..ws.defs.len())
        .filter(|&d| {
            ws.defs[d].kind == DefKind::Fn
                && (ws.defs[d].name.starts_with("test_")
                    || ws.modules[ws.defs[d].module_idx].fns[ws.defs[d].item_idx].is_property)
        })
        .collect();
    tests.sort_by_key(|&d| format!("{}#{}", ws.defs[d].module_path, ws.defs[d].name));

    if no_cache {
        cache.clear();
    }
    let mut rep = TestReport {
        executed: 0,
        skipped: 0,
        failed: 0,
    };
    for d in tests {
        let r = format!("{}#{}", ws.defs[d].module_path, ws.defs[d].name);
        {
            let info = &ws.defs[d];
            if ws.modules[info.module_idx].fns[info.item_idx].caps != 0 {
                sayln!("skip  {} (!io — effects are not cacheable)", r);
                continue;
            }
        }
        let Some(h) = hashes.get(&d) else {
            sayln!("skip  {} (not hashed)", r);
            continue;
        };
        let key = format!("{}:{}:{}", SEMANTICS, r, crate::canon::hex(h));
        if let Some(passed) = cache.get(&key) {
            rep.skipped += 1;
            // In quiet mode a cached pass is the uninteresting case; a cached
            // failure still needs saying.
            if !quiet || !*passed {
                sayln!(
                    "{}  {}  [cached — content unchanged, rerun provably identical]",
                    if *passed { "PASS" } else { "FAIL" },
                    r
                );
            }
            if !*passed {
                rep.failed += 1;
            }
            continue;
        }
        // A property is checked over generated inputs rather than called
        // once; everything else about it — caching, reporting — is identical.
        if ws.modules[ws.defs[d].module_idx].fns[ws.defs[d].item_idx].is_property {
            rep.executed += 1;
            match check_property(&eng, ws, d, h, &param_domains[d]) {
                Ok(n) => {
                    sayln!("PASS  {}  ({} cases)", r, n);
                    cache.insert(key, true);
                }
                Err(msg) => {
                    rep.failed += 1;
                    sayln!("FAIL  {}  {}", r, msg);
                    cache.insert(key, false);
                }
            }
            continue;
        }
        match eng.call(d, vec![]) {
            Ok(Value::Int(v)) if v != 0 => {
                rep.executed += 1;
                sayln!("PASS  {}", r);
                cache.insert(key, true);
            }
            Ok(v) => {
                rep.executed += 1;
                rep.failed += 1;
                sayln!("FAIL  {}  (returned {})", r, show(ws, &v));
                cache.insert(key, false);
            }
            Err(e) => {
                rep.executed += 1;
                rep.failed += 1;
                sayln!("FAIL  {}  (runtime error: {})", r, annotate(ws, &show(ws, &e)));
                cache.insert(key, false);
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    host::TESTCACHE.with(|c| *c.borrow_mut() = cache.clone());
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(s) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(&cache_path, s);
    }
    rep
}
