//! Value-domain inference (E0212).
//!
//! rigid has no type system, and this is not one. It is the same idea the
//! extern boundary already uses — a *value domain* — pushed through the
//! program: integers, strings, lists and records are distinguishable, and the
//! operators that accept them are not interchangeable.
//!
//! What it knows:
//!   * a parameter's domain, from uses admitting exactly one;
//!   * a local's domain, from its initializer;
//!   * a function's return domain, when every `return` agrees.
//!
//! What it reports: a call argument, an operator operand, or a field access
//! whose domain is *known* and wrong. Anything less than certain infers
//! nothing and reports nothing, so a program that runs today still checks.
//!
//! Domains are inferred, never declared, and never enter canonical form.

use crate::ast::*;
use crate::diag::*;
use crate::resolve::{Binding, DefId, DefKind, Workspace};
use serde_json::json;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Int,
    Float,
    Str,
    List,
    Record,
}

impl Domain {
    /// Int and Float are one domain to an operator and two to a reader.
    /// Arithmetic accepts either and promotes; a string in the same place is
    /// still a category error, which is the whole point of this pass.
    fn numeric(self) -> bool {
        matches!(self, Domain::Int | Domain::Float)
    }

    /// Can a value of this domain stand where `other` was expected?
    fn compatible(self, other: Domain) -> bool {
        self == other || (self.numeric() && other.numeric())
    }

    /// "an integer", "a string" — messages read badly without this.
    fn with_article(self) -> &'static str {
        match self {
            Domain::Int => "an integer",
            Domain::Float => "a float",
            Domain::Str => "a string",
            Domain::List => "a list",
            Domain::Record => "a record",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Domain::Int => "integer",
            Domain::Float => "float",
            Domain::Str => "string",
            Domain::List => "list",
            Domain::Record => "record",
        }
    }
}

/// A slot's inferred domain. `Conflict` means two sources disagreed, which
/// disables checking for that slot rather than being an error in itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Unknown,
    Known(Domain),
    Conflict,
}

impl Slot {
    fn meet(self, d: Domain) -> Slot {
        match self {
            Slot::Unknown => Slot::Known(d),
            Slot::Known(x) if x == d => self,
            // A slot used as both int and float is numeric, not conflicted.
            // Widening to Float keeps the message honest: the value can hold
            // a fraction, so an int-only claim about it would be wrong.
            Slot::Known(x) if x.numeric() && d.numeric() => Slot::Known(Domain::Float),
            _ => Slot::Conflict,
        }
    }
    fn get(self) -> Option<Domain> {
        match self {
            Slot::Known(d) => Some(d),
            _ => None,
        }
    }
}

/// Operators that need a number on both sides — either number.
fn int_only(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem
            | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge
    )
}

struct Ctx<'a> {
    ws: &'a Workspace,
    def: DefId,
    /// How each slot is *used* — the only evidence a parameter has.
    slots: Vec<Slot>,
    /// What a local was *defined* as, by its initializer. A definition
    /// outranks a use: `let n = 5; n.kind` is a bad field access, not an
    /// ambiguous slot.
    defs: Vec<Slot>,
    /// Return domain of each definition, from the previous round.
    returns: &'a HashMap<DefId, Option<Domain>>,
}

impl<'a> Ctx<'a> {
    fn bind(&self, span: Span, name: &str) -> Binding {
        self.ws
            .bindings
            .get(&(self.def, span.line, span.col, name.to_string()))
            .copied()
            .unwrap_or(Binding::Broken)
    }

    fn slot_of(&self, id: &Ident) -> Option<usize> {
        match self.bind(id.span, &id.name) {
            Binding::Local(s) => Some(s as usize),
            _ => None,
        }
    }

    fn ensure(&mut self, slot: usize) {
        if self.slots.len() <= slot {
            self.slots.resize(slot + 1, Slot::Unknown);
        }
        if self.defs.len() <= slot {
            self.defs.resize(slot + 1, Slot::Unknown);
        }
    }

    /// The domain of `e`, when it is knowable. Never guesses.
    fn domain(&self, e: &Expr) -> Option<Domain> {
        match e {
            Expr::Int(_) | Expr::Len(_) | Expr::IntConv(_) => Some(Domain::Int),
            // `-x` keeps x's domain rather than claiming integer.
            Expr::Neg(x) => self.domain(x),
            Expr::Float(_) | Expr::FloatConv(_) => Some(Domain::Float),
            Expr::Str(_) | Expr::StrConv(_) | Expr::Join(..) | Expr::Substr(..) => Some(Domain::Str),
            Expr::ListLit(_) | Expr::Chars(_) | Expr::Push(..) => Some(Domain::List),
            Expr::RecordLit(_) | Expr::Try(_) => Some(Domain::Record),
            Expr::Var(id) => self.slot_of(id).and_then(|s| {
                self.defs
                    .get(s)
                    .copied()
                    .and_then(Slot::get)
                    .or_else(|| self.slots.get(s).copied().and_then(Slot::get))
            }),
            Expr::Binop { op, l, r } => {
                // A comparison answers a question; arithmetic carries its
                // operands' domain forward, promoting when they differ.
                if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge)
                {
                    Some(Domain::Int)
                } else {
                    match (self.domain(l), self.domain(r)) {
                        (Some(a), Some(b)) if a == b => Some(a),
                        (Some(a), Some(b)) if a.numeric() && b.numeric() => Some(Domain::Float),
                        (Some(a), None) | (None, Some(a)) => Some(a),
                        _ => None,
                    }
                }
            }
            Expr::If { t, e: el, .. } => match (self.domain(t), self.domain(el)) {
                (Some(a), Some(b)) if a == b => Some(a),
                (Some(a), Some(b)) if a.numeric() && b.numeric() => Some(Domain::Float),
                _ => None,
            },
            Expr::Call { callee, .. } => match self.bind(callee.span, &callee.name) {
                Binding::Def(d) => *self.returns.get(&d)?,
                _ => None,
            },
            _ => None,
        }
    }

    /// Record the domain forced on a parameter or local by how it is used.
    fn force(&mut self, e: &Expr, d: Domain) {
        if let Expr::Var(id) = e {
            if let Some(s) = self.slot_of(id) {
                self.ensure(s);
                self.slots[s] = self.slots[s].meet(d);
            }
        }
    }

    /// Pass over a body recording what uses force, and what `let`s bind.
    fn learn_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, value } => {
                self.learn_expr(value);
                if let Some(slot) = self.slot_of(name) {
                    self.ensure(slot);
                    if let Some(d) = self.domain(value) {
                        self.defs[slot] = self.defs[slot].meet(d);
                    }
                }
            }
            Stmt::IfStmt { cond, then, els } => {
                self.learn_expr(cond);
                for s in then.iter().chain(els.iter()) {
                    self.learn_stmt(s);
                }
            }
            Stmt::For { var, iter, body } => {
                self.learn_expr(iter);
                // The loop variable's domain is the element domain, which is
                // not knowable from a list literal alone, so nothing is
                // claimed about it — the pass never guesses.
                let _ = var;
                for s in body {
                    self.learn_stmt(s);
                }
            }
            Stmt::Assign { name, value } => {
                self.learn_expr(value);
                if let Some(slot) = self.slot_of(name) {
                    self.ensure(slot);
                    if let Some(d) = self.domain(value) {
                        self.defs[slot] = self.defs[slot].meet(d);
                    }
                }
            }
            Stmt::Return(v) | Stmt::Expr(v) => self.learn_expr(v),
            Stmt::While { cond, body } => {
                self.learn_expr(cond);
                for s in body {
                    self.learn_stmt(s);
                }
            }
        }
    }

    fn learn_expr(&mut self, e: &Expr) {
        match e {
            Expr::Binop { op, l, r } => {
                if int_only(*op) {
                    self.force(l, Domain::Int);
                    self.force(r, Domain::Int);
                } else if matches!(op, BinOp::Add) {
                    if let Some(d) = self.domain(r) {
                        self.force(l, d);
                    }
                    if let Some(d) = self.domain(l) {
                        self.force(r, d);
                    }
                }
            }
            Expr::Get { base, .. } => self.force(base, Domain::Record),
            Expr::Substr(s, _, _) | Expr::Chars(s) => self.force(s, Domain::Str),
            Expr::Push(xs, _) => self.force(xs, Domain::List),
            Expr::Join(xs, sep) => {
                self.force(xs, Domain::List);
                self.force(sep, Domain::Str);
            }

            _ => {}
        }
        walk(e, &mut |c| self.learn_expr(c));
    }
}

/// Visit an expression's immediate children.
pub fn walk_expr_children(e: &Expr, f: &mut impl FnMut(&Expr)) {
    walk(e, f)
}

fn walk(e: &Expr, f: &mut impl FnMut(&Expr)) {
    match e {
        Expr::Binop { l, r, .. } => {
            f(l);
            f(r);
        }
        Expr::If { c, t, e: el } => {
            f(c);
            f(t);
            f(el);
        }
        Expr::Neg(x) | Expr::Len(x) | Expr::Fail(x) | Expr::Try(x) | Expr::Chars(x) => f(x),
        Expr::StrConv(x) | Expr::IntConv(x) | Expr::FloatConv(x) => f(x),
        Expr::ReadFile { path, .. } => f(path),
        Expr::Print { arg, .. } => f(arg),
        Expr::Call { args, .. } | Expr::ListLit(args) | Expr::TimeCall { args, .. } => {
            for a in args {
                f(a)
            }
        }
        Expr::DynCall { callee, args, .. } => {
            f(callee);
            for a in args {
                f(a)
            }
        }
        Expr::TableDispatch { key, args, .. } => {
            f(key);
            if let Some(args) = args {
                for a in args {
                    f(a)
                }
            }
        }
        Expr::RecordLit(fields) => {
            for (_, v) in fields {
                f(v)
            }
        }
        Expr::Get { base, .. } => f(base),
        Expr::IndexE { base, key } => {
            f(base);
            f(key);
        }
        Expr::Push(a, b) | Expr::Join(a, b) => {
            f(a);
            f(b);
        }
        Expr::WriteFile { path, contents, .. } => {
            f(path);
            f(contents);
        }
        Expr::Substr(a, b, c) => {
            f(a);
            f(b);
            f(c);
        }
        Expr::Int(_)
        | Expr::Float(_)
        | Expr::Str(_)
        | Expr::Var(_)
        | Expr::FunRef(_)
        | Expr::Lambda { .. }
        | Expr::Input { .. } => {}
    }
}

fn fn_of(ws: &Workspace, d: DefId) -> &FnDef {
    let i = &ws.defs[d];
    &ws.modules[i.module_idx].fns[i.item_idx]
}

fn fns(ws: &Workspace) -> Vec<DefId> {
    (0..ws.defs.len())
        .filter(|&d| ws.defs[d].kind == DefKind::Fn)
        .collect()
}

/// Build slot domains for one definition, given the current return summaries.
fn slots_for(
    ws: &Workspace,
    d: DefId,
    returns: &HashMap<DefId, Option<Domain>>,
) -> (Vec<Slot>, Vec<Slot>) {
    let f = fn_of(ws, d);
    let mut cx = Ctx {
        ws,
        def: d,
        slots: vec![Slot::Unknown; f.params.len()],
        defs: vec![Slot::Unknown; f.params.len()],
        returns,
    };
    // Two sweeps: the first records forced domains, the second lets a local
    // initialised from an earlier local pick that domain up.
    for _ in 0..2 {
        for s in &f.body {
            cx.learn_stmt(s);
        }
    }
    (cx.slots, cx.defs)
}

/// Inferred parameter domains per definition (parameters only).
pub fn infer_all(ws: &Workspace) -> Vec<Vec<Option<Domain>>> {
    let returns = return_domains(ws);
    let mut out = vec![Vec::new(); ws.defs.len()];
    for d in fns(ws) {
        let n = fn_of(ws, d).params.len();
        let (slots, _) = slots_for(ws, d, &returns);
        out[d] = slots.iter().take(n).map(|s| s.get()).collect();
    }
    out
}

/// A function's return domain, when every `return` in it agrees.
fn return_domains(ws: &Workspace) -> HashMap<DefId, Option<Domain>> {
    let all = fns(ws);
    let mut returns: HashMap<DefId, Option<Domain>> = all.iter().map(|&d| (d, None)).collect();

    // Three rounds let a domain travel a short call chain. Conservative
    // throughout: a single unknown or disagreeing `return` yields None.
    for _ in 0..3 {
        let prev = returns.clone();
        for &d in &all {
            let (slots, defs) = slots_for(ws, d, &prev);
            let cx = Ctx { ws, def: d, slots, defs, returns: &prev };
            let mut acc: Option<Option<Domain>> = None;
            collect_returns(&fn_of(ws, d).body, &mut |e| {
                let here = cx.domain(e);
                acc = Some(match acc {
                    None => here,
                    Some(Some(a)) if here == Some(a) => Some(a),
                    _ => None,
                });
            });
            returns.insert(d, acc.flatten());
        }
    }
    returns
}

fn collect_returns(body: &[Stmt], f: &mut impl FnMut(&Expr)) {
    for s in body {
        match s {
            Stmt::Return(e) => f(e),
            Stmt::While { body, .. } => collect_returns(body, f),
            _ => {}
        }
    }
}

/// Check call arguments, operator operands, and field accesses.
/// Parameter domains for *generation*, not for diagnostics.
///
/// `param_domains` only knows what a function's own body forces, which is
/// often nothing: in `len(sort(xs)) == len(xs)` nothing local pins `xs`,
/// because `len` accepts a list or a string. But `sort`'s parameter *is*
/// known to be a list, and `xs` is passed straight to it — so the domain can
/// be carried backwards across the call.
///
/// This is deliberately kept away from `analyze`. A checker that reports an
/// error must be certain; a generator only needs a good guess, and being
/// wrong here costs a wasted test case rather than a false diagnostic.
pub fn generation_domains(ws: &Workspace) -> Vec<Vec<Option<Domain>>> {
    let mut params = param_domains(ws);
    // Fixpoint: a parameter passed directly into a slot with a known domain
    // takes that domain. Bounded — each round can only fill unknowns.
    for _ in 0..8 {
        let mut changed = false;
        for d in fns(ws) {
            let f = fn_of(ws, d);
            let own: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
            let mut found: Vec<Option<Domain>> = params[d].clone();
            let mut visit = |e: &Expr| {
                if let Expr::Call { callee, args } = e {
                    let target = match ws
                        .bindings
                        .get(&(d, callee.span.line, callee.span.col, callee.name.clone()))
                    {
                        Some(Binding::Def(t)) => *t,
                        _ => return,
                    };
                    for (i, a) in args.iter().enumerate() {
                        let Expr::Var(id) = a else { continue };
                        let Some(k) = own.iter().position(|p| *p == id.name) else {
                            continue;
                        };
                        if found.get(k).copied().flatten().is_some() {
                            continue;
                        }
                        if let Some(Some(dom)) = params.get(target).and_then(|v| v.get(i)) {
                            while found.len() <= k {
                                found.push(None);
                            }
                            found[k] = Some(*dom);
                        }
                    }
                }
            };
            for s in &f.body {
                walk_stmt(s, &mut |e| deep(e, &mut visit));
            }
            if found != params[d] {
                params[d] = found;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    params
}

/// Each function's inferred parameter domains, from its own body alone.
pub fn param_domains(ws: &Workspace) -> Vec<Vec<Option<Domain>>> {
    let returns = return_domains(ws);
    let mut v = vec![Vec::new(); ws.defs.len()];
    for d in fns(ws) {
        let n = fn_of(ws, d).params.len();
        v[d] = slots_for(ws, d, &returns)
            .0
            .iter()
            .take(n)
            .map(|s| s.get())
            .collect();
    }
    v
}

pub fn analyze(ws: &Workspace) -> Vec<Diagnostic> {
    let returns = return_domains(ws);
    let params: Vec<Vec<Option<Domain>>> = {
        let mut v = vec![Vec::new(); ws.defs.len()];
        for d in fns(ws) {
            let n = fn_of(ws, d).params.len();
            v[d] = slots_for(ws, d, &returns)
                .0
                .iter()
                .take(n)
                .map(|s| s.get())
                .collect();
        }
        v
    };

    let mut out = Vec::new();
    for d in fns(ws) {
        let info = &ws.defs[d];
        let file = info.module_path.clone();
        let (slots, defs) = slots_for(ws, d, &returns);
        let cx = Ctx { ws, def: d, slots, defs, returns: &returns };
        let mut check = |e: &Expr| check_expr(&cx, e, &params, &file, &mut out);
        for s in &fn_of(ws, d).body {
            walk_stmt(s, &mut check);
        }
    }
    out
}

fn walk_stmt(s: &Stmt, f: &mut impl FnMut(&Expr)) {
    match s {
        Stmt::Let { value, .. } | Stmt::Assign { value, .. } | Stmt::Return(value) => {
            deep(value, f)
        }
        Stmt::Expr(e) => deep(e, f),
        Stmt::While { cond, body } => {
            deep(cond, f);
            for s in body {
                walk_stmt(s, f);
            }
        }
        Stmt::IfStmt { cond, then, els } => {
            deep(cond, f);
            for s in then.iter().chain(els.iter()) {
                walk_stmt(s, f);
            }
        }
        Stmt::For { iter, body, .. } => {
            deep(iter, f);
            for s in body {
                walk_stmt(s, f);
            }
        }
    }
}

fn deep(e: &Expr, f: &mut impl FnMut(&Expr)) {
    f(e);
    walk(e, &mut |c| deep(c, f));
}

fn report(
    out: &mut Vec<Diagnostic>,
    file: &str,
    span: Option<Span>,
    msg: String,
    data: serde_json::Value,
) {
    out.push(Diagnostic::new(E0212_DOMAIN, Severity::Error, file, span, msg).with_data(data));
}

fn check_expr(
    cx: &Ctx,
    e: &Expr,
    params: &[Vec<Option<Domain>>],
    file: &str,
    out: &mut Vec<Diagnostic>,
) {
    match e {
        // Integer-only operators applied to something known not to be one.
        Expr::Binop { op, l, r } if int_only(*op) => {
            // Ordering also accepts two strings; arithmetic does not.
            let ordering = matches!(op, BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge);
            if ordering {
                if let (Some(a), Some(b)) = (cx.domain(l), cx.domain(r)) {
                    if a == Domain::Str && b == Domain::Str {
                        return;
                    }
                }
            }
            for side in [l, r] {
                if let Some(got) = cx.domain(side) {
                    if !got.numeric() && !(ordering && got == Domain::Str) {
                        report(
                            out,
                            file,
                            span_of(side),
                            format!(
                                "this operator needs a number, but one side is {}",
                                got.with_article()
                            ),
                            json!({ "expected": "number", "found": got.name() }),
                        );
                    }
                }
            }
        }
        // `+` needs both sides in the same domain.
        Expr::Binop { op: BinOp::Add, l, r } => {
            if let (Some(a), Some(b)) = (cx.domain(l), cx.domain(r)) {
                if !a.compatible(b) {
                    report(
                        out,
                        file,
                        span_of(l).or_else(|| span_of(r)),
                        format!("cannot combine {} and {} with `+`", a.with_article(), b.with_article()),
                        json!({ "left": a.name(), "right": b.name() }),
                    );
                }
            }
        }
        // Field access on something known not to be a record.
        Expr::Get { base, field } => {
            if let Some(got) = cx.domain(base) {
                if got != Domain::Record {
                    report(
                        out,
                        file,
                        Some(field.span),
                        format!("'.{}' needs a record, but this is {}", field.name, got.with_article()),
                        json!({ "field": field.name, "found": got.name() }),
                    );
                }
            }
        }
        // Arguments at a direct call.
        Expr::Call { callee, args } => {
            let Binding::Def(target) = cx.bind(callee.span, &callee.name) else {
                return;
            };
            if cx.ws.defs[target].kind != DefKind::Fn {
                return;
            }
            for (i, a) in args.iter().enumerate() {
                let (Some(Some(expect)), Some(got)) = (params[target].get(i), cx.domain(a)) else {
                    continue;
                };
                if expect.compatible(got) {
                    continue;
                }
                let pname = fn_of(cx.ws, target).params[i].name.clone();
                report(
                    out,
                    file,
                    Some(callee.span),
                    format!(
                        "'{}' uses parameter '{}' as {} ({}), but this call passes {}",
                        callee.name,
                        pname,
                        expect.name(),
                        why(*expect),
                        got.name()
                    ),
                    json!({
                        "callee": callee.name, "param": pname, "index": i,
                        "expected": expect.name(), "found": got.name(),
                    }),
                );
            }
        }
        _ => {}
    }
}

fn span_of(e: &Expr) -> Option<Span> {
    match e {
        Expr::Var(id) | Expr::FunRef(id) => Some(id.span),
        Expr::Call { callee, .. } => Some(callee.span),
        Expr::Get { field, .. } => Some(field.span),
        Expr::Print { span, .. } | Expr::Input { span } => Some(*span),
        _ => None,
    }
}

fn why(d: Domain) -> &'static str {
    match d {
        Domain::Int => "arithmetic or ordering",
        Domain::Float => "arithmetic or ordering",
        Domain::Str => "concatenation or a string builtin",
        Domain::List => "a list builtin",
        Domain::Record => "field access",
    }
}
