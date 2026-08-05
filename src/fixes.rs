//! `fix --apply`: mechanically apply typed `rename_reference` fixes.
//! This is the agent repair loop closed: structured diagnostic in, source
//! edit out, converging `check`. `rebind` fixes are never auto-applied —
//! accepting a changed meaning is a decision, so it is its own command.

use crate::ast::Span;
use crate::diag::Diagnostic;
use crate::resolve::{Binding, Workspace};
use crate::sidecar;
use std::collections::HashMap;
use std::fs;

/// Collect textual quarantine fixes.
/// quarantine_call: (module, line, col, callee, argc, targets)
/// add_dyn_targets: (module, line, col, after_name, targets)
pub enum QFix {
    QuarantineCall {
        module: String,
        line: u32,
        col: u32,
        callee: String,
        argc: usize,
        targets: Vec<String>,
    },
    AddDynTargets {
        module: String,
        line: u32,
        col: u32,
        after: String,
        targets: Vec<String>,
    },
}

pub fn quarantine_fixes(diags: &[Diagnostic]) -> Vec<QFix> {
    let mut out = Vec::new();
    for d in diags {
        for fx in &d.fixes {
            let strv = |k: &str| fx[k].as_str().map(|s| s.to_string());
            let tgts = || -> Option<Vec<String>> {
                fx["targets"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            };
            if fx["kind"] == "quarantine_call" {
                if let (Some(module), Some(callee), Some(targets)) =
                    (strv("module"), strv("callee"), tgts())
                {
                    out.push(QFix::QuarantineCall {
                        module,
                        line: fx["line"].as_u64().unwrap_or(0) as u32,
                        col: fx["col"].as_u64().unwrap_or(0) as u32,
                        callee,
                        argc: fx["argc"].as_u64().unwrap_or(0) as usize,
                        targets,
                    });
                }
            } else if fx["kind"] == "add_dyn_targets" {
                if let (Some(module), Some(after), Some(targets)) =
                    (strv("module"), strv("after"), tgts())
                {
                    out.push(QFix::AddDynTargets {
                        module,
                        line: fx["line"].as_u64().unwrap_or(0) as u32,
                        col: fx["col"].as_u64().unwrap_or(0) as u32,
                        after,
                        targets,
                    });
                }
            }
        }
    }
    out
}

/// Apply one quarantine fix by textual surgery at a resolved span.
pub fn apply_quarantine(ws: &Workspace, q: &QFix) -> std::io::Result<bool> {
    let (module, line, col) = match q {
        QFix::QuarantineCall { module, line, col, .. }
        | QFix::AddDynTargets { module, line, col, .. } => (module, *line, *col),
    };
    let Some(m) = ws.modules.iter().find(|m| &m.path == module) else {
        return Ok(false);
    };
    let disk = ws.root.join(&m.path);
    let src = fs::read_to_string(&disk)?;
    let mut lines: Vec<String> = src.lines().map(|s| s.to_string()).collect();
    let Some(l) = lines.get_mut(line as usize - 1) else {
        return Ok(false);
    };
    let idx = (col as usize).saturating_sub(1);
    match q {
        QFix::QuarantineCall {
            callee,
            argc,
            targets,
            ..
        } => {
            let pat_args = format!("{}(", callee);
            let pat_zero = format!("{}()", callee);
            if l[idx..].starts_with(&pat_zero) && *argc == 0 {
                let repl = format!("dyn {{ {} }}({})", targets.join(", "), callee);
                l.replace_range(idx..idx + pat_zero.len(), &repl);
            } else if l[idx..].starts_with(&pat_args) {
                let repl = format!("dyn {{ {} }}({}, ", targets.join(", "), callee);
                l.replace_range(idx..idx + pat_args.len(), &repl);
            } else {
                return Ok(false);
            }
        }
        QFix::AddDynTargets { after, targets, .. } => {
            if !l[idx..].starts_with(after.as_str()) {
                return Ok(false);
            }
            let end = idx + after.len();
            let ins = format!(", {}", targets.join(", "));
            l.insert_str(end, &ins);
        }
    }
    let mut out = lines.join("\n");
    if src.ends_with('\n') {
        out.push('\n');
    }
    fs::write(&disk, out)?;
    Ok(true)
}

/// Collect rename_reference fixes as (module_path, from, to).
pub fn rename_fixes(diags: &[Diagnostic]) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for d in diags {
        for f in &d.fixes {
            if f["kind"] == "rename_reference" {
                if let (Some(m), Some(from), Some(to)) = (
                    f["module"].as_str(),
                    f["from"].as_str(),
                    f["to"].as_str(),
                ) {
                    let t = (m.to_string(), from.to_string(), to.to_string());
                    if !out.contains(&t) {
                        out.push(t);
                    }
                }
            }
        }
    }
    out
}

/// Apply one rename inside one module's source text. Only occurrences the
/// resolver marked Broken (plus the import-list identifier) are touched —
/// a local variable that happens to share the name is left alone.
pub fn apply_rename(
    ws: &Workspace,
    module_path: &str,
    from: &str,
    to: &str,
) -> std::io::Result<usize> {
    let mi = ws
        .modules
        .iter()
        .position(|m| m.path == module_path)
        .expect("module exists");

    let mut spans: Vec<Span> = Vec::new();
    // broken occurrences inside definitions of this module
    for ((def, line, col, name), b) in &ws.bindings {
        if *b == Binding::Broken
            && name == from
            && ws.defs[*def].module_idx == mi
        {
            spans.push(Span {
                line: *line,
                col: *col,
            });
        }
    }
    // the identifier in the import list itself
    for b in &ws.imports[mi].broken {
        if b.name == from {
            spans.push(b.span);
        }
    }
    spans.sort_by_key(|s| (s.line, s.col));
    spans.dedup();

    let disk = ws.root.join(module_path);
    let src = fs::read_to_string(&disk)?;
    let chars: Vec<char> = src.chars().collect();

    // line/col -> char offset
    let mut line_starts: Vec<usize> = vec![0];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            line_starts.push(i + 1);
        }
    }
    let mut edits: Vec<(usize, usize)> = spans
        .iter()
        .filter_map(|s| {
            let ls = *line_starts.get((s.line as usize).saturating_sub(1))?;
            let start = ls + (s.col as usize) - 1;
            let end = start + from.chars().count();
            // sanity: the text there must actually be `from`
            if chars.get(start..end).map(|w| w.iter().collect::<String>())
                == Some(from.to_string())
            {
                Some((start, end))
            } else {
                None
            }
        })
        .collect();
    edits.sort();
    edits.reverse();

    let mut out = chars;
    let count = edits.len();
    for (start, end) in edits {
        out.splice(start..end, to.chars());
    }
    fs::write(&disk, out.into_iter().collect::<String>())?;
    Ok(count)
}

/// `rigid rebind MODULE#NAME`: accept the name's *current* meaning, repinning
/// the sidecar to whatever the name resolves to right now.
pub fn rebind(
    ws: &Workspace,
    hashes: &HashMap<crate::resolve::DefId, crate::canon::Hash>,
    module_path: &str,
    name: &str,
) -> Result<String, String> {
    let mi = ws
        .modules
        .iter()
        .position(|m| m.path == module_path)
        .ok_or_else(|| format!("no such module: {}", module_path))?;
    let (target, _) = ws.imports[mi]
        .map
        .get(name)
        .ok_or_else(|| format!("'{}' is not imported in {}", name, module_path))?
        .clone();
    let d = ws
        .def_index
        .get(&(target.clone(), name.to_string()))
        .ok_or_else(|| format!("'{}' does not currently exist in \"{}\"", name, target))?;
    let h = hashes
        .get(d)
        .ok_or_else(|| format!("'{}' is not hashed (upstream errors?)", name))?;
    let mut bf = sidecar::load(ws, module_path).unwrap_or_else(|| sidecar::BindFile {
        format: sidecar::FORMAT.to_string(),
        definitions: Default::default(),
        references: Default::default(),
        quarantine: Default::default(),
        module_hash: Default::default(),
    });
    let hexh = crate::canon::hex(h);
    bf.references.insert(
        name.to_string(),
        sidecar::RefPin {
            target: format!("{}#{}", target, name),
            hash: hexh.clone(),
            // Rebinding accepts the new meaning, capabilities included — so
            // record them, or the next check would report the widening again.
            caps: ws
                .def_index
                .get(&(target.to_string(), name.to_string()))
                .map(|&d| {
                    let info = &ws.defs[d];
                    let caps = match info.kind {
                        crate::resolve::DefKind::Fn => {
                            ws.modules[info.module_idx].fns[info.item_idx].caps
                        }
                        crate::resolve::DefKind::Extern => {
                            ws.modules[info.module_idx].externs[info.item_idx].caps
                        }
                        crate::resolve::DefKind::Table => 0,
                    };
                    crate::ast::caps_list(caps)
                        .into_iter()
                        .map(|s| s.to_string())
                        .collect()
                }),
        },
    );
    sidecar::save(ws, module_path, &bf).map_err(|e| e.to_string())?;
    Ok(format!(
        "rebound '{}' in {} -> {}#{} ({})",
        name, module_path, target, name, hexh
    ))
}
