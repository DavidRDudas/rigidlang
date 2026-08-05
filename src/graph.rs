//! deps / rdeps: blast radius as a language operation, not an IDE feature.

use crate::canon::{hex, Hash};
use crate::resolve::{DefId, Workspace};
use serde_json::json;
use std::collections::HashMap;

pub fn find_ref(ws: &Workspace, r: &str) -> Option<DefId> {
    let (path, name) = r.split_once('#')?;
    ws.def_index.get(&(path.to_string(), name.to_string())).copied()
}

pub fn ref_str(ws: &Workspace, d: DefId) -> String {
    format!("{}#{}", ws.defs[d].module_path, ws.defs[d].name)
}

fn entry(ws: &Workspace, hashes: &HashMap<DefId, Hash>, d: DefId) -> serde_json::Value {
    json!({
        "ref": ref_str(ws, d),
        "hash": hashes.get(&d).map(hex),
    })
}

pub fn deps(ws: &Workspace, hashes: &HashMap<DefId, Hash>, d: DefId) -> serde_json::Value {
    let out: Vec<_> = ws.edges[d]
        .iter()
        .filter(|&&t| t != d)
        .map(|&t| entry(ws, hashes, t))
        .collect();
    json!({ "query": "deps", "of": entry(ws, hashes, d), "results": out })
}

pub fn rdeps(ws: &Workspace, hashes: &HashMap<DefId, Hash>, d: DefId) -> serde_json::Value {
    let mut out = Vec::new();
    for (src, es) in ws.edges.iter().enumerate() {
        if src != d && es.contains(&d) {
            out.push(entry(ws, hashes, src));
        }
    }
    json!({ "query": "rdeps", "of": entry(ws, hashes, d), "results": out })
}

/// For each definition, the tests that transitively reach it.
///
/// `describe` reports this per definition; the coverage check below uses the
/// same walk, so the two can never disagree about what "covered" means.
pub fn test_coverage(ws: &Workspace) -> Vec<Vec<DefId>> {
    use crate::resolve::DefKind;
    let mut covered = vec![Vec::new(); ws.defs.len()];
    let tests: Vec<DefId> = (0..ws.defs.len())
        .filter(|&d| ws.defs[d].kind == DefKind::Fn && ws.defs[d].name.starts_with("test_"))
        .collect();
    for &t in &tests {
        let mut seen = vec![false; ws.defs.len()];
        let mut stack = vec![t];
        while let Some(x) = stack.pop() {
            if seen[x] {
                continue;
            }
            seen[x] = true;
            for n in &ws.edges[x] {
                stack.push(*n);
            }
        }
        for (d, s) in seen.iter().enumerate() {
            if *s && d != t {
                covered[d].push(t);
            }
        }
    }
    covered
}

/// I0407 — a dispatch table no test reaches.
///
/// Which key maps to which handler is a statement of intent, so no checker can
/// tell a wrong mapping from a right one: swapping two entries produces a
/// valid program. What *can* be said is that nothing asserts the mapping, so a
/// swap would pass unnoticed. Reported only in workspaces that already have
/// tests — telling a program with no tests that it has no tests is noise.
pub fn coverage_diags(ws: &Workspace) -> Vec<crate::diag::Diagnostic> {
    use crate::diag::{Diagnostic, Severity, I0407_UNTESTED_TABLE};
    use crate::resolve::DefKind;

    let has_tests = (0..ws.defs.len())
        .any(|d| ws.defs[d].kind == DefKind::Fn && ws.defs[d].name.starts_with("test_"));
    if !has_tests {
        return Vec::new();
    }

    let covered = test_coverage(ws);
    let mut out = Vec::new();
    for d in 0..ws.defs.len() {
        if ws.defs[d].kind != DefKind::Table || !covered[d].is_empty() {
            continue;
        }
        let info = &ws.defs[d];
        let t = &ws.modules[info.module_idx].tables[info.item_idx];
        let keys: Vec<&str> = t.entries.iter().map(|e| e.key.as_str()).collect();
        out.push(
            Diagnostic::new(
                I0407_UNTESTED_TABLE,
                Severity::Info,
                &info.module_path,
                Some(t.name.span),
                format!(
                    "no test reaches table '{}', so nothing asserts which handler each key selects \
                     ({} entries: {}). Swapping two of them would still check clean.",
                    info.name,
                    t.entries.len(),
                    keys.join(", ")
                ),
            )
            .with_data(json!({ "table": info.name, "keys": keys })),
        );
    }
    out
}

/// I0408 — a key that names a different target of the same table.
///
/// Keys are data and need no relationship to the names of the functions they
/// select: `"light" -> slash` is ordinary. But when a key is *exactly* the
/// name of another function this same table dispatches to, and points
/// somewhere else, the entries have most likely been transposed. That is the
/// one shape of a wrong pairing a checker can recognise without being told
/// what you meant, so it is reported — as information, never as an error,
/// because it is a guess about intent and the code is valid either way.
pub fn key_shadow_diags(ws: &Workspace) -> Vec<crate::diag::Diagnostic> {
    use crate::diag::{Diagnostic, Severity, I0408_KEY_SHADOWS};
    use crate::resolve::DefKind;
    use std::collections::HashMap;

    let mut out = Vec::new();
    for d in 0..ws.defs.len() {
        if ws.defs[d].kind != DefKind::Table {
            continue;
        }
        let Some(entries) = ws.table_entries.get(&d) else { continue };
        // name of every function this table can select -> that function
        let targets: HashMap<&str, DefId> = entries
            .iter()
            .map(|(_, t)| (ws.defs[*t].name.as_str(), *t))
            .collect();

        let info = &ws.defs[d];
        let tbl = &ws.modules[info.module_idx].tables[info.item_idx];
        for (key, target) in entries {
            let Some(&named) = targets.get(key.as_str()) else { continue };
            if named == *target {
                continue; // the key names what it selects: nothing to say
            }
            out.push(
                Diagnostic::new(
                    I0408_KEY_SHADOWS,
                    Severity::Info,
                    &info.module_path,
                    Some(tbl.name.span),
                    format!(
                        "in table '{}', the key {:?} is also the name of another target of this \
                         table, but selects '{}'. If that is deliberate, nothing is wrong; if the \
                         entries were transposed, no other check will notice.",
                        info.name,
                        key,
                        ws.defs[*target].name
                    ),
                )
                .with_data(json!({
                    "table": info.name,
                    "key": key,
                    "selects": ws.defs[*target].name,
                    "shadows": ws.defs[named].name,
                })),
            );
        }
    }
    out
}

/// Shortest reference path from `src` to `dst`, as definition ids.
///
/// Answers "how does this test reach that definition?" — the question behind
/// `rigid why`. Edges are the compiler's own resolution, so the path is exact
/// rather than a guess, including hops through a dispatch table.
pub fn path_between(ws: &Workspace, src: DefId, dst: DefId) -> Option<Vec<DefId>> {
    use std::collections::VecDeque;
    if src == dst {
        return Some(vec![src]);
    }
    let mut prev: HashMap<DefId, DefId> = HashMap::new();
    let mut seen = vec![false; ws.defs.len()];
    let mut q = VecDeque::new();
    seen[src] = true;
    q.push_back(src);
    while let Some(x) = q.pop_front() {
        for &n in &ws.edges[x] {
            if seen[n] {
                continue;
            }
            seen[n] = true;
            prev.insert(n, x);
            if n == dst {
                let mut path = vec![dst];
                let mut cur = dst;
                while let Some(&p) = prev.get(&cur) {
                    path.push(p);
                    cur = p;
                }
                path.reverse();
                return Some(path);
            }
            q.push_back(n);
        }
    }
    None
}

/// Everything that would have to be re-checked if `d` changed: the tests that
/// reach it, and every definition whose own hash moves because it depends on
/// it (transitively).
pub fn blast(ws: &Workspace, d: DefId) -> (Vec<DefId>, Vec<DefId>) {
    use crate::resolve::DefKind;
    let mut affected = Vec::new();
    let mut seen = vec![false; ws.defs.len()];
    let mut stack = vec![d];
    while let Some(x) = stack.pop() {
        if seen[x] {
            continue;
        }
        seen[x] = true;
        if x != d {
            affected.push(x);
        }
        for (src, es) in ws.edges.iter().enumerate() {
            if !seen[src] && es.contains(&x) {
                stack.push(src);
            }
        }
    }
    affected.sort();
    let tests = affected
        .iter()
        .copied()
        .filter(|&t| ws.defs[t].kind == DefKind::Fn && ws.defs[t].name.starts_with("test_"))
        .collect();
    (affected, tests)
}
