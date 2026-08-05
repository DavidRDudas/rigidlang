//! Funref flow inference (docs/QUARANTINE.md §6.2): a conservative,
//! intra-procedural may-reach analysis over function-reference values.
//! It powers two things, and never changes semantics:
//!   * I0405 — an unquarantined indirect call (E0204) gets an *inferred*
//!     minimal set, as a typed `quarantine_call` fix `fix --apply` can
//!     perform.
//!   * W0403 — a dyn site whose declared set provably misses a funref
//!     that may reach it, with a typed `add_dyn_targets` fix.
//! Inference proposes; the declaration in source remains the truth.

use crate::ast::*;
use crate::diag::*;
use crate::resolve::{Binding, DefId, DefKind, Workspace};
use serde_json::json;
use std::collections::BTreeSet;

/// May-reach set: None = unknown (top), Some(s) = at most these targets.
type Reach = Option<BTreeSet<DefId>>;

fn join(a: Reach, b: Reach) -> Reach {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.union(&y).cloned().collect()),
        _ => None,
    }
}

struct Fx<'a> {
    ws: &'a Workspace,
    def: DefId,
    slots: Vec<Reach>,
    summaries: &'a std::collections::HashMap<DefId, Reach>,
    /// summary-collection mode: joins return-expr reaches here
    ret: Option<Reach>,
    diags: Vec<Diagnostic>,
}

impl<'a> Fx<'a> {
    fn bind(&self, span: Span, name: &str) -> Binding {
        self.ws
            .bindings
            .get(&(self.def, span.line, span.col, name.to_string()))
            .copied()
            .unwrap_or(Binding::Broken)
    }

    /// A name for `d` writable in this module's scope, if any.
    fn scoped_name(&self, d: DefId) -> Option<String> {
        let me = &self.ws.defs[self.def];
        let t = &self.ws.defs[d];
        if t.module_path == me.module_path {
            return Some(t.name.clone());
        }
        // imported under its own name?
        if self
            .ws
            .def_index
            .get(&(t.module_path.clone(), t.name.clone()))
            == Some(&d)
        {
            // check this module imports it (binding by name from anywhere in
            // this def would be Def(d)); cheap test: def_index alone is not
            // enough, so probe the import map via a synthetic lookup is not
            // available here — accept the name if any binding in this def
            // already resolves it, else decline.
            for ((dd, _, _, n), b) in &self.ws.bindings {
                if *dd == self.def && n == &t.name && *b == Binding::Def(d) {
                    return Some(t.name.clone());
                }
            }
        }
        None
    }

    fn expr_reach(&mut self, e: &Expr) -> Reach {
        match e {
            Expr::FunRef(id) => match self.bind(id.span, &id.name) {
                Binding::Def(d) => Some(BTreeSet::from([d])),
                _ => None,
            },
            Expr::Var(id) => match self.bind(id.span, &id.name) {
                Binding::Local(s) => self
                    .slots
                    .get(s as usize)
                    .cloned()
                    .unwrap_or(Some(BTreeSet::new())),
                _ => None,
            },
            Expr::If { c, t, e: el } => {
                self.visit(c);
                let a = self.expr_reach(t);
                let b = self.expr_reach(el);
                join(a, b)
            }
            Expr::TableDispatch { table, key, args } => {
                self.visit(key);
                if let Some(a) = args {
                    for x in a {
                        self.visit(x);
                    }
                }
                // bare lookup yields a ref from the table's closed set
                if args.is_none() {
                    if let Binding::Def(d) = self.bind(table.span, &table.name) {
                        if self.ws.defs[d].kind == DefKind::Table {
                            if let Some(es) = self.ws.table_entries.get(&d) {
                                return Some(es.iter().map(|(_, t)| *t).collect());
                            }
                        }
                    }
                }
                None
            }
            Expr::TimeCall { args, .. } => {
                for a in args {
                    self.visit(a);
                }
                None
            }
            Expr::Call { callee, args } => {
                for a in args {
                    self.visit(a);
                }
                match self.bind(callee.span, &callee.name) {
                    Binding::Def(d) => self
                        .summaries
                        .get(&d)
                        .cloned()
                        .unwrap_or(Some(BTreeSet::new())),
                    _ => None,
                }
            }
            Expr::Try(x) => {
                self.visit(x);
                None
            }
            other => {
                self.visit(other);
                if matches!(other, Expr::Int(_) | Expr::Float(_) | Expr::Str(_)) {
                    Some(BTreeSet::new())
                } else {
                    None
                }
            }
        }
    }

    /// Walk for side-effect checks (dyn-site width, E0204 inference)
    /// without contributing reach.
    fn visit(&mut self, e: &Expr) {
        match e {
            Expr::Int(_)
            | Expr::Float(_)
            | Expr::Str(_)
            | Expr::Var(_)
            | Expr::FunRef(_)
            | Expr::Lambda { .. }
            | Expr::Input { .. } => {}
            Expr::TimeCall { args, .. } => {
                for a in args {
                    self.visit(a);
                }
            }
            Expr::Call { callee, args } => {
                for a in args {
                    self.visit(a);
                }
                // an E0204 site: callee bound to a local — infer its set
                if let Binding::Local(_) = self.bind(callee.span, &callee.name) {
                    let reach = self.expr_reach(&Expr::Var(callee.clone()));
                    if let Some(set) = reach {
                        if !set.is_empty() {
                            let names: Option<Vec<String>> =
                                set.iter().map(|&d| self.scoped_name(d)).collect();
                            if let Some(mut names) = names {
                                names.sort();
                                let me = &self.ws.defs[self.def];
                                let diag = Diagnostic::new(
                                    I0405_INFERRED_SET,
                                    Severity::Info,
                                    &me.module_path,
                                    Some(callee.span),
                                    format!(
                                        "inferred quarantine for this call: dyn {{ {} }}({}, ...)",
                                        names.join(", "),
                                        callee.name
                                    ),
                                );
                                let diag = diag.with_fix(json!({
                                    "kind": "quarantine_call",
                                    "module": me.module_path,
                                    "line": callee.span.line,
                                    "col": callee.span.col,
                                    "callee": callee.name,
                                    "argc": args.len(),
                                    "targets": names,
                                }));
                                self.diags.push(diag);
                            }
                        }
                    }
                }
            }
            Expr::DynCall {
                targets,
                callee,
                args,
            } => {
                for a in args {
                    self.visit(a);
                }
                let reach = self.expr_reach(callee);
                if let (Some(reach), Some(last)) = (reach, targets.last()) {
                    let declared: BTreeSet<DefId> = targets
                        .iter()
                        .filter_map(|t| match self.bind(t.span, &t.name) {
                            Binding::Def(d) => Some(d),
                            _ => None,
                        })
                        .collect();
                    let missing: Vec<DefId> =
                        reach.difference(&declared).copied().collect();
                    if !missing.is_empty() {
                        let names: Option<Vec<String>> =
                            missing.iter().map(|&d| self.scoped_name(d)).collect();
                        if let Some(mut names) = names {
                            names.sort();
                            let me = &self.ws.defs[self.def];
                            let diag = Diagnostic::new(
                                W0403_NARROW_DYN,
                                Severity::Warning,
                                &me.module_path,
                                Some(last.span),
                                format!(
                                    "a reference to {{ {} }} may reach this dyn site but is outside its declared set",
                                    names.join(", ")
                                ),
                            );
                            let diag = diag.with_fix(json!({
                                "kind": "add_dyn_targets",
                                "module": me.module_path,
                                "line": last.span.line,
                                "col": last.span.col,
                                "after": last.name,
                                "targets": names,
                            }));
                            self.diags.push(diag);
                        }
                    }
                }
            }
            Expr::TableDispatch { key, args, .. } => {
                self.visit(key);
                if let Some(a) = args {
                    for x in a {
                        self.visit(x);
                    }
                }
            }
            Expr::Binop { l, r, .. } => {
                self.visit(l);
                self.visit(r);
            }
            Expr::If { c, t, e: el } => {
                for x in [c, t, el] {
                    self.visit(x);
                }
            }
            Expr::Neg(x)
            | Expr::Print { arg: x, .. }
            | Expr::Fail(x)
            | Expr::Try(x)
            | Expr::StrConv(x)
            | Expr::IntConv(x)
            | Expr::FloatConv(x)
            | Expr::Chars(x)
            | Expr::Len(x)
            | Expr::ReadFile { path: x, .. } => self.visit(x),
            Expr::Substr(a, b, c) => {
                for x in [a, b, c] {
                    self.visit(x);
                }
            }
            Expr::Push(a, b) | Expr::Join(a, b) => {
                self.visit(a);
                self.visit(b);
            }
            Expr::WriteFile { path, contents, .. } => {
                self.visit(path);
                self.visit(contents);
            }
            Expr::ListLit(xs) => {
                for x in xs {
                    self.visit(x);
                }
            }
            Expr::RecordLit(fs) => {
                for (_, v) in fs {
                    self.visit(v);
                }
            }
            Expr::Get { base, .. } => self.visit(base),
            Expr::IndexE { base, key } => {
                self.visit(base);
                self.visit(key);
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, value } | Stmt::Assign { name, value } => {
                let r = self.expr_reach(value);
                if let Binding::Local(sl) = self.bind(name.span, &name.name) {
                    let sl = sl as usize;
                    while self.slots.len() <= sl {
                        self.slots.push(Some(BTreeSet::new()));
                    }
                    // joins with prior contents so loop re-assignment widens
                    let prev = self.slots[sl].clone();
                    self.slots[sl] = join(prev, r);
                }
            }
            Stmt::Return(e) => {
                if self.ret.is_some() {
                    let r = self.expr_reach(e);
                    let cur = self.ret.take().unwrap();
                    self.ret = Some(join_ret(cur, r));
                } else {
                    self.visit(e);
                }
            }
            Stmt::Expr(e) => {
                self.visit(e);
            }
            Stmt::IfStmt { cond, then, els } => {
                self.visit(cond);
                // Both arms are possible, so both widen the may-reach set.
                for b in then.iter().chain(els.iter()) {
                    self.stmt(b);
                }
            }
            Stmt::For { var, iter, body } => {
                self.visit(iter);
                let _ = var;
                // Two passes, as with `while`: loop-carried references have
                // to stabilize, and joins only ever widen.
                for _ in 0..2 {
                    for b in body {
                        self.stmt(b);
                    }
                }
            }
            Stmt::While { cond, body } => {
                self.visit(cond);
                // two passes so loop-carried refs stabilize (joins only widen)
                for _ in 0..2 {
                    for b in body {
                        self.stmt(b);
                    }
                }
            }
        }
    }
}

/// Joining return reaches: an empty accumulator means "no returns seen",
/// which must not poison the join toward bottom.
fn join_ret(acc: Reach, r: Reach) -> Reach {
    join(acc, r)
}

pub fn analyze(ws: &Workspace) -> Vec<Diagnostic> {
    use std::collections::HashMap;
    let fns: Vec<DefId> = (0..ws.defs.len())
        .filter(|&d| ws.defs[d].kind == DefKind::Fn)
        .collect();

    // pass 1: one-level return summaries, fixpoint-lite (joins only widen;
    // three rounds cover the mutual-recursion shapes small programs have)
    let mut summaries: HashMap<DefId, Reach> = fns
        .iter()
        .map(|&d| (d, Some(BTreeSet::new())))
        .collect();
    for _ in 0..3 {
        let snapshot = summaries.clone();
        for &d in &fns {
            let info = &ws.defs[d];
            let f = &ws.modules[info.module_idx].fns[info.item_idx];
            let mut fx = Fx {
                ws,
                def: d,
                slots: vec![None; f.params.len()],
                summaries: &snapshot,
                ret: Some(Some(BTreeSet::new())),
                diags: Vec::new(),
            };
            for s in &f.body {
                fx.stmt(s);
            }
            summaries.insert(d, fx.ret.take().unwrap());
        }
    }

    // pass 2: site analysis with summaries available
    let mut out = Vec::new();
    for &d in &fns {
        let info = &ws.defs[d];
        let f = &ws.modules[info.module_idx].fns[info.item_idx];
        let mut fx = Fx {
            ws,
            def: d,
            slots: vec![None; f.params.len()],
            summaries: &summaries,
            ret: None,
            diags: Vec::new(),
        };
        for s in &f.body {
            fx.stmt(s);
        }
        out.extend(fx.diags);
    }
    out
}
