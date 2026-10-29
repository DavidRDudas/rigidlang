//! Lambda lifting: give every anonymous function a content identity.
//!
//! rigid's exactness rests on one property — every callable is a definition
//! with a hash, so the reference graph is the compiler's own resolution
//! rather than a guess. An anonymous function is the standard way that
//! property is lost, which is why the language went this long without one.
//!
//! Lifting restores it. Each `fn(a) { ... }` becomes a real top-level
//! definition, `<enclosing>$<n>`, whose leading parameters are the enclosing
//! locals its body reads. The expression that remains is a *reference* to
//! that definition plus the values to close over — so `rdeps` sees through a
//! lambda exactly as it sees through `&name`.
//!
//! `$` cannot appear in a source identifier, so a synthetic name can never
//! collide with one an author wrote.
//!
//! Free variables are found syntactically, before resolution: a name used in
//! the body counts as captured when the enclosing function binds it as a
//! parameter or a `let`, and is left alone otherwise — globals, imports and
//! builtins resolve normally from inside the lifted definition.

use crate::ast::*;
use std::collections::BTreeSet;

pub fn lift_module(m: &mut Module) {
    let mut created: Vec<FnDef> = Vec::new();
    for i in 0..m.fns.len() {
        let owner = m.fns[i].name.name.clone();
        let mut bound: BTreeSet<String> = m.fns[i].params.iter().map(|p| p.name.clone()).collect();
        collect_lets(&m.fns[i].body, &mut bound);
        let mut body = std::mem::take(&mut m.fns[i].body);
        let mut n = 0usize;
        for s in body.iter_mut() {
            lift_stmt(s, &owner, &bound, &mut n, &mut created, m.fns[i].caps);
        }
        m.fns[i].body = body;
    }
    m.fns.extend(created);
}

fn collect_lets(body: &[Stmt], out: &mut BTreeSet<String>) {
    for s in body {
        match s {
            Stmt::Let { name, .. } => {
                out.insert(name.name.clone());
            }
            Stmt::For { var, body, .. } => {
                out.insert(var.name.clone());
                collect_lets(body, out);
            }
            Stmt::While { body, .. } => collect_lets(body, out),
            Stmt::IfStmt { then, els, .. } => {
                collect_lets(then, out);
                collect_lets(els, out);
            }
            _ => {}
        }
    }
}

fn lift_stmt(
    s: &mut Stmt,
    owner: &str,
    bound: &BTreeSet<String>,
    n: &mut usize,
    created: &mut Vec<FnDef>,
    caps: Caps,
) {
    match s {
        Stmt::Let { value, .. } | Stmt::Assign { value, .. } => {
            lift_expr(value, owner, bound, n, created, caps)
        }
        Stmt::Return(e) | Stmt::Expr(e) => lift_expr(e, owner, bound, n, created, caps),
        Stmt::While { cond, body } => {
            lift_expr(cond, owner, bound, n, created, caps);
            for b in body {
                lift_stmt(b, owner, bound, n, created, caps);
            }
        }
        Stmt::IfStmt { cond, then, els } => {
            lift_expr(cond, owner, bound, n, created, caps);
            for b in then.iter_mut().chain(els.iter_mut()) {
                lift_stmt(b, owner, bound, n, created, caps);
            }
        }
        Stmt::For { iter, body, .. } => {
            lift_expr(iter, owner, bound, n, created, caps);
            for b in body {
                lift_stmt(b, owner, bound, n, created, caps);
            }
        }
    }
}

fn lift_expr(
    e: &mut Expr,
    owner: &str,
    bound: &BTreeSet<String>,
    n: &mut usize,
    created: &mut Vec<FnDef>,
    caps: Caps,
) {
    if let Expr::Lambda {
        params,
        body,
        lifted,
        captures,
        line,
    } = e
    {
        // Inner lambdas first, so a nested one is already a reference by the
        // time this one's free variables are read.
        let mut inner_bound: BTreeSet<String> = bound.clone();
        for p in params.iter() {
            inner_bound.insert(p.name.clone());
        }
        collect_lets(body, &mut inner_bound);
        for s in body.iter_mut() {
            lift_stmt(s, owner, &inner_bound, n, created, caps);
        }

        let own: BTreeSet<String> = {
            let mut b: BTreeSet<String> = params.iter().map(|p| p.name.clone()).collect();
            collect_lets(body, &mut b);
            b
        };
        // Captures in order of first use, not by name: they become the
        // lifted definition's leading parameters, so a name order would let
        // renaming two captured locals reorder them and move the hash.
        let mut free: Vec<String> = Vec::new();
        for s in body.iter() {
            free_stmt(s, &own, &mut free);
        }
        let caught: Vec<Ident> = free
            .iter()
            .filter(|f| bound.contains(*f))
            .map(|f| Ident {
                name: f.clone(),
                span: Span {
                    line: *line,
                    col: 0,
                },
            })
            .collect();

        let name = format!("{}${}", owner, *n);
        *n += 1;
        let sig: Vec<Ident> = caught.iter().cloned().chain(params.iter().cloned()).collect();
        let stmt_lines = vec![*line; body.len()];
        created.push(FnDef {
            memo: false,
            is_property: false,
            gen_hints: vec![None; caught.len() + params.len()],
            synthetic: true,
            captured: caught.len(),
            name: Ident {
                name: name.clone(),
                span: Span {
                    line: *line,
                    col: 0,
                },
            },
            params: sig,
            // A lambda inherits the capability set of the function it was
            // written in: it runs there, and nothing else could be sound.
            caps,
            body: body.clone(),
            stmt_lines,
            line: *line,
        });
        *lifted = Some(Ident {
            name,
            span: Span {
                line: *line,
                col: 0,
            },
        });
        *captures = caught;
        return;
    }
    walk_mut(e, &mut |c| lift_expr(c, owner, bound, n, created, caps));
}

/// Names a body reads that it does not itself bind.
fn free_stmt(s: &Stmt, own: &BTreeSet<String>, out: &mut Vec<String>) {
    match s {
        Stmt::Let { value, .. } | Stmt::Assign { value, .. } => free_expr(value, own, out),
        Stmt::Return(e) | Stmt::Expr(e) => free_expr(e, own, out),
        Stmt::While { cond, body } => {
            free_expr(cond, own, out);
            for b in body {
                free_stmt(b, own, out);
            }
        }
        Stmt::IfStmt { cond, then, els } => {
            free_expr(cond, own, out);
            for b in then.iter().chain(els.iter()) {
                free_stmt(b, own, out);
            }
        }
        Stmt::For { iter, body, .. } => {
            free_expr(iter, own, out);
            for b in body {
                free_stmt(b, own, out);
            }
        }
    }
}

fn free_expr(e: &Expr, own: &BTreeSet<String>, out: &mut Vec<String>) {
    let note = |name: &String, out: &mut Vec<String>| {
        if !own.contains(name) && !out.contains(name) {
            out.push(name.clone());
        }
    };
    match e {
        Expr::Var(id) => note(&id.name, out),
        // An already-lifted inner lambda carries its captures explicitly;
        // they are the names it needs from out here.
        Expr::Lambda { captures, .. } => {
            for c in captures {
                note(&c.name, out);
            }
        }
        // `xs[i]` on a local parses as a dispatch whose base is a bare
        // name; it is a use of that local like any other.
        Expr::TableDispatch { table, .. } => {
            note(&table.name, out);
            walk(e, &mut |c| free_expr(c, own, out));
        }
        // Fields in canonical (name) order, the order they are hashed and
        // evaluated in: their order in the source is not part of identity,
        // so it must not decide which capture comes first.
        Expr::RecordLit(fs) => {
            let mut order: Vec<&(Ident, Expr)> = fs.iter().collect();
            order.sort_by(|a, b| a.0.name.cmp(&b.0.name));
            for (_, v) in order {
                free_expr(v, own, out);
            }
        }
        _ => walk(e, &mut |c| free_expr(c, own, out)),
    }
}

fn walk(e: &Expr, f: &mut impl FnMut(&Expr)) {
    crate::domains::walk_expr_children(e, f)
}

fn walk_mut(e: &mut Expr, f: &mut impl FnMut(&mut Expr)) {
    match e {
        Expr::Neg(x)
        | Expr::Len(x)
        | Expr::Fail(x)
        | Expr::Try(x)
        | Expr::StrConv(x)
        | Expr::IntConv(x)
        | Expr::FloatConv(x)
        | Expr::Chars(x)
        | Expr::Print { arg: x, .. }
        | Expr::ReadFile { path: x, .. }
        | Expr::Get { base: x, .. } => f(x),
        Expr::Binop { l, r, .. } | Expr::Push(l, r) | Expr::Join(l, r) => {
            f(l);
            f(r)
        }
        Expr::If { c, t, e: el } => {
            f(c);
            f(t);
            f(el)
        }
        Expr::Substr(a, b, c) => {
            f(a);
            f(b);
            f(c)
        }
        Expr::WriteFile { path, contents, .. } => {
            f(path);
            f(contents)
        }
        Expr::IndexE { base, key } => {
            f(base);
            f(key)
        }
        Expr::Call { args, .. } | Expr::ListLit(args) | Expr::TimeCall { args, .. } => {
            for a in args {
                f(a)
            }
        }
        Expr::RecordLit(fs) => {
            for (_, a) in fs {
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
        Expr::Lambda { body, .. } => {
            for s in body.iter_mut() {
                walk_stmt_mut(s, f)
            }
        }
        Expr::Int(_)
        | Expr::Float(_)
        | Expr::Str(_)
        | Expr::Var(_)
        | Expr::FunRef(_)
        | Expr::Input { .. } => {}
    }
}

fn walk_stmt_mut(s: &mut Stmt, f: &mut impl FnMut(&mut Expr)) {
    match s {
        Stmt::Let { value, .. } | Stmt::Assign { value, .. } => f(value),
        Stmt::Return(e) | Stmt::Expr(e) => f(e),
        Stmt::While { cond, body } => {
            f(cond);
            for b in body {
                walk_stmt_mut(b, f)
            }
        }
        Stmt::IfStmt { cond, then, els } => {
            f(cond);
            for b in then.iter_mut().chain(els.iter_mut()) {
                walk_stmt_mut(b, f)
            }
        }
        Stmt::For { iter, body, .. } => {
            f(iter);
            for b in body {
                walk_stmt_mut(b, f)
            }
        }
    }
}
