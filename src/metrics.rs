//! Reference exactness (docs/PAPER.md §5): the fraction of call sites whose
//! candidate-target set is a singleton. Direct calls contribute width 1;
//! `dyn` sites contribute their declared-set width; table dispatches
//! contribute the table's value-set width. Bare lookups and reference
//! creation are not call sites.

use crate::ast::*;
use crate::resolve::{Binding, DefKind, Workspace};

#[derive(Debug, Default)]
pub struct Exactness {
    pub sites: usize,
    pub singleton: usize,
    pub dyn_widths: Vec<usize>,
    pub table_widths: Vec<usize>,
}

impl Exactness {
    pub fn score(&self) -> f64 {
        if self.sites == 0 {
            1.0
        } else {
            self.singleton as f64 / self.sites as f64
        }
    }
}

pub fn measure(ws: &Workspace) -> Exactness {
    let mut ex = Exactness::default();
    for (mi, m) in ws.modules.iter().enumerate() {
        for f in &m.fns {
            let Some(&id) = ws.def_index.get(&(m.path.clone(), f.name.name.clone())) else {
                continue;
            };
            if ws.defs[id].module_idx != mi {
                continue; // shadowed duplicate
            }
            for s in &f.body {
                stmt(ws, id, s, &mut ex);
            }
        }
    }
    ex
}

fn stmt(ws: &Workspace, id: usize, s: &Stmt, ex: &mut Exactness) {
    match s {
        Stmt::Let { value, .. } | Stmt::Assign { value, .. } => expr(ws, id, value, ex),
        Stmt::Return(e) | Stmt::Expr(e) => expr(ws, id, e, ex),
        Stmt::While { cond, body } => {
            expr(ws, id, cond, ex);
            for b in body {
                stmt(ws, id, b, ex);
            }
        }
        Stmt::IfStmt { cond, then, els } => {
            expr(ws, id, cond, ex);
            for b in then.iter().chain(els.iter()) {
                stmt(ws, id, b, ex);
            }
        }
        Stmt::For { iter, body, .. } => {
            expr(ws, id, iter, ex);
            for b in body {
                stmt(ws, id, b, ex);
            }
        }
    }
}

fn expr(ws: &Workspace, id: usize, e: &Expr, ex: &mut Exactness) {
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
                expr(ws, id, a, ex);
            }
        }
        Expr::Call { args, .. } => {
            ex.sites += 1;
            ex.singleton += 1;
            for a in args {
                expr(ws, id, a, ex);
            }
        }
        Expr::DynCall {
            targets,
            callee,
            args,
        } => {
            ex.sites += 1;
            let w = targets.len();
            ex.dyn_widths.push(w);
            if w == 1 {
                ex.singleton += 1;
            }
            expr(ws, id, callee, ex);
            for a in args {
                expr(ws, id, a, ex);
            }
        }
        Expr::TableDispatch { table, key, args } => {
            if let Some(Binding::Def(d)) = ws.bindings.get(&(
                id,
                table.span.line,
                table.span.col,
                table.name.clone(),
            )) {
                if ws.defs[*d].kind == DefKind::Table && args.is_some() {
                    ex.sites += 1;
                    let w = ws.table_entries.get(d).map(|v| v.len()).unwrap_or(0);
                    ex.table_widths.push(w);
                    if w == 1 {
                        ex.singleton += 1;
                    }
                }
            }
            expr(ws, id, key, ex);
            if let Some(a) = args {
                for x in a {
                    expr(ws, id, x, ex);
                }
            }
        }
        Expr::Binop { l, r, .. } => {
            expr(ws, id, l, ex);
            expr(ws, id, r, ex);
        }
        Expr::If { c, t, e: el } => {
            for x in [c, t, el] {
                expr(ws, id, x, ex);
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
        | Expr::ReadFile { path: x, .. } => expr(ws, id, x, ex),
        Expr::Substr(a, b, c) => {
            for x in [a, b, c] {
                expr(ws, id, x, ex);
            }
        }
        Expr::Push(a, b) | Expr::Join(a, b) => {
            expr(ws, id, a, ex);
            expr(ws, id, b, ex);
        }
        Expr::WriteFile { path, contents, .. } => {
            expr(ws, id, path, ex);
            expr(ws, id, contents, ex);
        }
        Expr::ListLit(xs) => {
            for x in xs {
                expr(ws, id, x, ex);
            }
        }
        Expr::RecordLit(fs) => {
            for (_, v) in fs {
                expr(ws, id, v, ex);
            }
        }
        Expr::Get { base, .. } => expr(ws, id, base, ex),
        Expr::IndexE { base, key } => {
            expr(ws, id, base, ex);
            expr(ws, id, key, ex);
        }
    }
}
