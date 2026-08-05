//! Canonical formatter, now comment-preserving. One rendering per AST;
//! comments are reattached by original-line association: a comment goes
//! before the first construct whose source line follows it, and a comment
//! sharing a construct's line trails it. Anchoring is heuristic (v2) but
//! lossless: every comment is emitted exactly once.

use crate::ast::*;

/// The one canonical rendering of a float: Rust's shortest round-trip form,
/// with `.0` forced onto whole numbers so `2.0` never renders as `2` and
/// silently reads back as an int.
pub fn fmt_f64(v: f64) -> String {
    if v.is_nan() {
        return "(0.0 / 0.0)".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "(1.0 / 0.0)".to_string() } else { "(-1.0 / 0.0)".to_string() };
    }
    let s = format!("{}", v);
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{}.0", s)
    }
}

pub fn render(m: &Module) -> String {
    let mut out = String::new();
    let mut comments: Vec<Comment> = m.comments.clone();
    comments.sort_by_key(|c| c.line);
    let mut ci = 0usize;

    let lead = |out: &mut String, upto: u32, indent: &str, ci: &mut usize| {
        while *ci < comments.len() && comments[*ci].line < upto {
            out.push_str(indent);
            out.push_str("// ");
            out.push_str(&comments[*ci].text);
            out.push('\n');
            *ci += 1;
        }
    };
    let trail = |out: &mut String, line: u32, ci: &mut usize, comments: &[Comment]| {
        if *ci < comments.len() && comments[*ci].line == line {
            out.push_str("  // ");
            out.push_str(&comments[*ci].text);
            *ci += 1;
        }
    };

    for imp in &m.imports {
        lead(&mut out, imp.span.line, "", &mut ci);
        out.push_str("import { ");
        out.push_str(
            &imp.names
                .iter()
                .map(|n| n.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push_str(" } from \"");
        out.push_str(&imp.from);
        out.push_str("\";");
        trail(&mut out, imp.span.line, &mut ci, &comments);
        out.push('\n');
    }

    enum It<'a> {
        F(&'a FnDef),
        T(&'a TableDef),
    }
    // Synthetic definitions (lifted lambdas) have no source of their own;
    // the `fn(...) { ... }` the author wrote is rendered where it stands.
    let mut items: Vec<(u32, It)> = m
        .fns
        .iter()
        .filter(|f| !f.synthetic)
        .map(|f| (f.line, It::F(f)))
        .collect();
    items.extend(m.tables.iter().map(|t| (t.line, It::T(t))));
    items.sort_by_key(|(l, _)| *l);
    for x in &m.externs {
        lead(&mut out, x.span.line, "", &mut ci);
        out.push_str("extern fn ");
        out.push_str(&x.name.name);
        out.push('(');
        out.push_str(
            &x.params
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push(')');
        if x.caps != 0 {
            out.push(' ');
            out.push_str(&crate::ast::caps_str(x.caps));
        }
        out.push_str(" = ");
        out.push_str(&format!("{:?}", x.host));
        out.push(';');
        trail(&mut out, x.span.line, &mut ci, &comments);
        out.push('\n');
    }

    for (i, (_, item)) in items.iter().enumerate() {
        let It::F(f) = item else {
            let It::T(t) = item else { unreachable!() };
            lead(&mut out, t.line, "", &mut ci);
            if i > 0 || !m.imports.is_empty() {
                out.push('\n');
            }
            out.push_str("table ");
            out.push_str(&t.name.name);
            out.push_str(" {");
            trail(&mut out, t.line, &mut ci, &comments);
            out.push('\n');
            let last = t.entries.len().saturating_sub(1);
            for (ei, e) in t.entries.iter().enumerate() {
                let eline = t.entry_lines.get(ei).copied().unwrap_or(t.line);
                lead(&mut out, eline, "  ", &mut ci);
                out.push_str("  ");
                out.push_str(&format!("{:?} -> {}", e.key, e.target.name));
                if ei != last {
                    out.push(',');
                }
                trail(&mut out, eline, &mut ci, &comments);
                out.push('\n');
            }
            out.push_str("}\n");
            continue;
        };
        lead(&mut out, f.line, "", &mut ci);
        if i > 0 || !m.imports.is_empty() {
            out.push('\n');
        }
        if f.memo { out.push_str("memo "); }
        out.push_str("fn ");
        out.push_str(&f.name.name);
        out.push('(');
        out.push_str(
            &f.params
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push(')');
        if f.caps != 0 {
            out.push(' ');
            out.push_str(&crate::ast::caps_str(f.caps));
        }
        out.push_str(" {");
        trail(&mut out, f.line, &mut ci, &comments);
        out.push('\n');
        for (si, s) in f.body.iter().enumerate() {
            let sline = f.stmt_lines.get(si).copied().unwrap_or(f.line);
            lead(&mut out, sline, "  ", &mut ci);
            out.push_str("  ");
            out.push_str(&stmt(s));
            trail(&mut out, sline, &mut ci, &comments);
            out.push('\n');
        }
        out.push_str("}\n");
    }

    // leftovers (anything after the last construct)
    while ci < comments.len() {
        out.push_str("// ");
        out.push_str(&comments[ci].text);
        out.push('\n');
        ci += 1;
    }
    out
}

fn stmt(s: &Stmt) -> String {
    stmt_ind(s, 1)
}

fn stmt_ind(s: &Stmt, level: usize) -> String {
    let pad = "  ".repeat(level);
    match s {
        Stmt::Let { name, value } => format!("let {} = {};", name.name, expr(value, 0)),
        Stmt::Assign { name, value } => format!("{} = {};", name.name, expr(value, 0)),
        Stmt::Return(e) => format!("return {};", expr(e, 0)),
        Stmt::Expr(e) => format!("{};", expr(e, 0)),
        Stmt::IfStmt { cond, then, els } => {
            let mut out = format!("if ({}) {{", expr(cond, 0));
            for s in then {
                out.push('\n');
                out.push_str(&pad);
                out.push_str("  ");
                out.push_str(&stmt_ind(s, level + 1));
            }
            out.push('\n');
            out.push_str(&pad);
            out.push('}');
            if !els.is_empty() {
                // `else if` stays on one line; a plain else opens a block.
                if els.len() == 1 && matches!(els[0], Stmt::IfStmt { .. }) {
                    out.push_str(" else ");
                    out.push_str(&stmt_ind(&els[0], level));
                } else {
                    out.push_str(" else {");
                    for s in els {
                        out.push('\n');
                        out.push_str(&pad);
                        out.push_str("  ");
                        out.push_str(&stmt_ind(s, level + 1));
                    }
                    out.push('\n');
                    out.push_str(&pad);
                    out.push('}');
                }
            }
            out
        }
        Stmt::For { var, iter, body } => {
            let mut out = format!("for ({} in {}) {{", var.name, expr(iter, 0));
            for s in body {
                out.push('\n');
                out.push_str(&pad);
                out.push_str("  ");
                out.push_str(&stmt_ind(s, level + 1));
            }
            out.push('\n');
            out.push_str(&pad);
            out.push('}');
            out
        }
        Stmt::While { cond, body } => {
            let mut out = format!("while ({}) {{", expr(cond, 0));
            for bs in body {
                out.push('\n');
                out.push_str(&pad);
                out.push_str("  ");
                out.push_str(&stmt_ind(bs, level + 1));
            }
            out.push('\n');
            out.push_str(&pad);
            out.push('}');
            out
        }
    }
}

fn prec(op: BinOp) -> u8 {
    use BinOp::*;
    match op {
        Eq | Ne | Lt | Gt | Le | Ge => 1,
        Add | Sub => 2,
        Mul | Div | Rem => 3,
    }
}

fn op_str(op: BinOp) -> &'static str {
    use BinOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        Rem => "%",
        Eq => "==",
        Ne => "!=",
        Lt => "<",
        Gt => ">",
        Le => "<=",
        Ge => ">=",
    }
}

fn expr(e: &Expr, ctx: u8) -> String {
    match e {
        Expr::Int(v) => format!("{}", v),
        Expr::Float(v) => fmt_f64(*v),
        Expr::Str(s) => format!("{:?}", s),
        Expr::Var(id) => id.name.clone(),
        Expr::FunRef(id) => format!("&{}", id.name),
        Expr::TimeCall { hash, args, .. } => format!(
            "@{:?}({})",
            hash,
            args.iter().map(|a| expr(a, 0)).collect::<Vec<_>>().join(", ")
        ),
        Expr::Call { callee, args } => format!(
            "{}({})",
            callee.name,
            args.iter().map(|a| expr(a, 0)).collect::<Vec<_>>().join(", ")
        ),
        Expr::DynCall {
            targets,
            callee,
            args,
        } => {
            let mut parts = vec![expr(callee, 0)];
            parts.extend(args.iter().map(|a| expr(a, 0)));
            format!(
                "dyn {{ {} }}({})",
                targets
                    .iter()
                    .map(|t| t.name.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
                parts.join(", ")
            )
        }
        Expr::Print { arg, .. } => format!("print({})", expr(arg, 0)),
        Expr::ListLit(elems) => format!(
            "[{}]",
            elems.iter().map(|x| expr(x, 0)).collect::<Vec<_>>().join(", ")
        ),
        Expr::RecordLit(fields) => {
            let mut fs: Vec<(&String, &Expr)> =
                fields.iter().map(|(n, v)| (&n.name, v)).collect();
            fs.sort_by(|a, b| a.0.cmp(b.0));
            format!(
                "{{ {} }}",
                fs.iter()
                    .map(|(n, v)| format!("{}: {}", n, expr(v, 0)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        Expr::Get { base, field } => format!("{}.{}", expr(base, 4), field.name),
        Expr::IndexE { base, key } => format!("{}[{}]", expr(base, 4), expr(key, 0)),
        Expr::Len(x) => format!("len({})", expr(x, 0)),
        Expr::Push(a, b) => format!("push({}, {})", expr(a, 0), expr(b, 0)),
        Expr::Input { .. } => "input()".to_string(),
        Expr::Fail(x) => format!("fail({})", expr(x, 0)),
        Expr::StrConv(x) => format!("str({})", expr(x, 0)),
        Expr::IntConv(x) => format!("int({})", expr(x, 0)),
        Expr::FloatConv(x) => format!("float({})", expr(x, 0)),
        Expr::Lambda { params, body, .. } => {
            let mut out = format!(
                "fn({}) {{",
                params.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ")
            );
            for s in body {
                out.push(' ');
                out.push_str(&stmt(s));
            }
            out.push_str(" }");
            out
        }
        Expr::Substr(a, b, c) => format!("substr({}, {}, {})", expr(a, 0), expr(b, 0), expr(c, 0)),
        Expr::Chars(x) => format!("chars({})", expr(x, 0)),
        Expr::Join(a, b) => format!("join({}, {})", expr(a, 0), expr(b, 0)),
        Expr::ReadFile { path, .. } => format!("read_file({})", expr(path, 0)),
        Expr::WriteFile { path, contents, .. } => format!("write_file({}, {})", expr(path, 0), expr(contents, 0)),
        Expr::Try(x) => format!("try({})", expr(x, 0)),
        Expr::TableDispatch { table, key, args } => match args {
            None => format!("{}[{}]", table.name, expr(key, 0)),
            Some(a) => format!(
                "{}[{}]({})",
                table.name,
                expr(key, 0),
                a.iter().map(|x| expr(x, 0)).collect::<Vec<_>>().join(", ")
            ),
        },
        Expr::Binop { op, l, r } => {
            let p = prec(*op);
            let s = format!("{} {} {}", expr(l, p), op_str(*op), expr(r, p + 1));
            if p < ctx {
                format!("({})", s)
            } else {
                s
            }
        }
        Expr::If { c, t, e } => {
            let s = format!("if ({}) {} else {}", expr(c, 0), expr(t, 0), expr(e, 0));
            if ctx > 0 {
                format!("({})", s)
            } else {
                s
            }
        }
        Expr::Neg(x) => format!("-{}", expr(x, 4)),
    }
}
