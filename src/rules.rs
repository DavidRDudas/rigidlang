//! Declared relationships, checked exactly.
//!
//! Every large codebase has architectural rules — the domain layer must not
//! reach the transport layer, parsers must not touch the filesystem, every
//! request handler must be tested. Everywhere else those rules are enforced
//! by import linters that approximate the graph, or by reviewers who
//! remember. Here the graph *is* the compiler's resolution, so a rule is a
//! decidable question rather than a heuristic.
//!
//! That matters most when the code is being written by something that has no
//! memory of the last conversation: an agent that wires the domain layer to
//! HTTP fails `check`, immediately, rather than being caught three days
//! later by whoever happens to read the diff.
//!
//! Rules live in `rigid.rules` at the workspace root, one per line:
//!
//! ```text
//! # nothing under domain/ may reach anything under http/
//! deny    domain/**  ->  http/**
//! # the parser must not touch the outside world
//! deny    parse.rg#parse_untrusted -> !fs, !net
//! # every handler needs a test that reaches it
//! require test  handlers.rg#**
//! # and a human must have signed off on the public surface
//! require attest reviewed  api.rg#**
//! # two definitions that mean exactly the same thing are a mistake
//! deny    duplicates
//! ```
//!
//! A selector is a glob over `module/path.rg#name`, where `*` matches within
//! a segment and `**` matches across segments. `**` alone is everything.

use crate::canon::{hex, Hash};
use crate::ast::Span;
use crate::diag::*;
use crate::graph;
use crate::resolve::{DefId, Workspace};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    /// Nothing matching `from` may reach anything matching `to`.
    DenyReach { from: String, to: String, line: u32 },
    /// Nothing matching `from` may reach any of these capabilities.
    DenyCaps { from: String, caps: Vec<String>, line: u32 },
    /// Every definition matching `what` must be reached by some test.
    RequireTest { what: String, line: u32 },
    /// Every definition matching `what` must carry a live claim of `kind`.
    RequireAttest { kind: String, what: String, line: u32 },
    /// No two definitions may share an identity.
    DenyDuplicates { line: u32 },
}

pub fn rules_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join("rigid.rules")
}

pub fn load(root: &std::path::Path) -> Result<Vec<Rule>, Vec<Diagnostic>> {
    let p = rules_path(root);
    if !p.exists() {
        return Ok(Vec::new());
    }
    let src = match std::fs::read_to_string(&p) {
        Ok(s) => s,
        Err(e) => {
            return Err(vec![Diagnostic::new(
                E0214_RULE,
                Severity::Error,
                "rigid.rules",
                None,
                format!("cannot read rules: {}", e),
            )])
        }
    };
    parse(&src)
}

pub fn parse(src: &str) -> Result<Vec<Rule>, Vec<Diagnostic>> {
    let mut out = Vec::new();
    let mut errs = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let line = (i + 1) as u32;
        // A `#` inside a selector is part of a reference (`util.rg#clamp`),
        // so only a line that *starts* with `#` is a comment.
        let s = if raw.trim_start().starts_with('#') {
            ""
        } else {
            raw.trim()
        };
        if s.is_empty() {
            continue;
        }
        let toks: Vec<&str> = s.split_whitespace().collect();
        let bad = |msg: &str| {
            Diagnostic::new(
                E0214_RULE,
                Severity::Error,
                "rigid.rules",
                Some(Span { line, col: 1 }),
                msg.to_string(),
            )
        };
        match toks.as_slice() {
            ["deny", "duplicates"] => out.push(Rule::DenyDuplicates { line }),
            ["deny", from, "->", rest @ ..] if !rest.is_empty() => {
                let joined = rest.join(" ");
                if joined.trim_start().starts_with('!') {
                    let caps: Vec<String> = joined
                        .split(',')
                        .map(|c| c.trim().trim_start_matches('!').to_string())
                        .filter(|c| !c.is_empty())
                        .collect();
                    for c in &caps {
                        if crate::ast::cap_from_name(c).is_none() {
                            errs.push(bad(&format!(
                                "unknown capability `!{}` — expected one of {}",
                                c,
                                crate::ast::cap_vocabulary()
                            )));
                        }
                    }
                    out.push(Rule::DenyCaps {
                        from: from.to_string(),
                        caps,
                        line,
                    });
                } else if rest.len() == 1 {
                    out.push(Rule::DenyReach {
                        from: from.to_string(),
                        to: rest[0].to_string(),
                        line,
                    });
                } else {
                    errs.push(bad("a `deny A -> B` rule takes one selector on each side"));
                }
            }
            ["require", "test", what] => out.push(Rule::RequireTest {
                what: what.to_string(),
                line,
            }),
            ["require", "attest", kind, what] => out.push(Rule::RequireAttest {
                kind: kind.to_string(),
                what: what.to_string(),
                line,
            }),
            _ => errs.push(bad(
                "expected `deny A -> B`, `deny A -> !cap`, `deny duplicates`, \
                 `require test A`, or `require attest KIND A`",
            )),
        }
    }
    if errs.is_empty() {
        Ok(out)
    } else {
        Err(errs)
    }
}

/// Glob over `module/path.rg#name`. `*` stops at `/` and `#`; `**` does not.
pub fn matches(pat: &str, s: &str) -> bool {
    fn go(p: &[u8], s: &[u8]) -> bool {
        if p.is_empty() {
            return s.is_empty();
        }
        if p[0] == b'*' {
            if p.len() > 1 && p[1] == b'*' {
                let rest = &p[2..];
                for i in 0..=s.len() {
                    if go(rest, &s[i..]) {
                        return true;
                    }
                }
                return false;
            }
            let rest = &p[1..];
            for i in 0..=s.len() {
                if go(rest, &s[i..]) {
                    return true;
                }
                if i < s.len() && (s[i] == b'/' || s[i] == b'#') {
                    break;
                }
            }
            return false;
        }
        if s.is_empty() || p[0] != s[0] {
            return false;
        }
        go(&p[1..], &s[1..])
    }
    go(pat.as_bytes(), s.as_bytes())
}

fn select(ws: &Workspace, pat: &str) -> Vec<DefId> {
    (0..ws.defs.len())
        .filter(|&d| !ws.defs[d].poisoned && matches(pat, &graph::ref_str(ws, d)))
        .collect()
}

/// Everything `d` can reach, `d` excluded.
fn reach(ws: &Workspace, d: DefId) -> HashSet<DefId> {
    let mut seen = HashSet::new();
    let mut stack: Vec<DefId> = ws.edges[d].clone();
    while let Some(x) = stack.pop() {
        if !seen.insert(x) {
            continue;
        }
        for &n in &ws.edges[x] {
            stack.push(n);
        }
    }
    seen
}

pub struct Ctx<'a> {
    pub ws: &'a Workspace,
    pub hashes: &'a HashMap<DefId, Hash>,
    /// Definitions carrying a live claim of each kind.
    pub attested: HashMap<String, HashSet<DefId>>,
}

pub fn check(rules: &[Rule], cx: &Ctx) -> Vec<Diagnostic> {
    let ws = cx.ws;
    let mut out = Vec::new();
    let at = |line: u32| Some(Span { line, col: 1 });

    for r in rules {
        match r {
            Rule::DenyReach { from, to, line } => {
                let targets: HashSet<DefId> = select(ws, to).into_iter().collect();
                if targets.is_empty() {
                    continue;
                }
                for d in select(ws, from) {
                    let hit: Vec<DefId> = reach(ws, d)
                        .into_iter()
                        .filter(|x| targets.contains(x))
                        .collect();
                    if let Some(&first) = hit.iter().min() {
                        let path = graph::path_between(ws, d, first)
                            .map(|p| {
                                p.iter()
                                    .map(|&x| ws.defs[x].name.clone())
                                    .collect::<Vec<_>>()
                                    .join(" -> ")
                            })
                            .unwrap_or_default();
                        out.push(
                            Diagnostic::new(
                                E0214_RULE,
                                Severity::Error,
                                &ws.defs[d].module_path,
                                at(*line),
                                format!(
                                    "rule `deny {} -> {}`: {} reaches {} ({})",
                                    from,
                                    to,
                                    graph::ref_str(ws, d),
                                    graph::ref_str(ws, first),
                                    path
                                ),
                            )
                            .with_data(serde_json::json!({
                                "rule": "deny_reach", "from": from, "to": to,
                                "source": graph::ref_str(ws, d),
                                "target": graph::ref_str(ws, first),
                                "path": path,
                            })),
                        );
                    }
                }
            }
            Rule::DenyCaps { from, caps, line } => {
                let want: crate::ast::Caps = caps
                    .iter()
                    .filter_map(|c| crate::ast::cap_from_name(c))
                    .fold(0, |a, b| a | b);
                for d in select(ws, from) {
                    let mut cone = reach(ws, d);
                    cone.insert(d);
                    // Name where the capability comes from: a holder none of
                    // whose own dependencies supplies it (an extern, or the
                    // definition that prints). Chosen by id, never by hash-set
                    // order, so the same program always gets the same report.
                    let holders: Vec<DefId> = {
                        let mut v: Vec<DefId> = cone
                            .iter()
                            .copied()
                            .filter(|&x| def_caps(ws, x) & want != 0)
                            .collect();
                        v.sort();
                        v
                    };
                    let origin = holders
                        .iter()
                        .copied()
                        .find(|&x| {
                            ws.edges[x]
                                .iter()
                                .all(|&y| y == x || def_caps(ws, y) & def_caps(ws, x) & want == 0)
                        })
                        .or_else(|| holders.first().copied());
                    // everything denied that the cone reaches, not just what
                    // the named origin supplies
                    let got = holders.iter().fold(0, |a, &x| a | def_caps(ws, x)) & want;
                    if let Some(x) = origin {
                        {
                            out.push(
                                Diagnostic::new(
                                    E0214_RULE,
                                    Severity::Error,
                                    &ws.defs[d].module_path,
                                    at(*line),
                                    format!(
                                        "rule `deny {} -> {}`: {} reaches {} via {}",
                                        from,
                                        caps.iter()
                                            .map(|c| format!("!{}", c))
                                            .collect::<Vec<_>>()
                                            .join(", "),
                                        graph::ref_str(ws, d),
                                        crate::ast::caps_str(got),
                                        graph::ref_str(ws, x)
                                    ),
                                )
                                .with_data(serde_json::json!({
                                    "rule": "deny_caps", "from": from,
                                    "source": graph::ref_str(ws, d),
                                    "via": graph::ref_str(ws, x),
                                    "capabilities": crate::ast::caps_list(got),
                                })),
                            );
                        }
                    }
                }
            }
            Rule::RequireTest { what, line } => {
                let covered = graph::test_coverage(ws);
                for d in select(ws, what) {
                    if ws.defs[d].name.starts_with("test_") {
                        continue;
                    }
                    if covered[d].is_empty() {
                        out.push(
                            Diagnostic::new(
                                E0214_RULE,
                                Severity::Error,
                                &ws.defs[d].module_path,
                                at(*line),
                                format!(
                                    "rule `require test {}`: no test reaches {}",
                                    what,
                                    graph::ref_str(ws, d)
                                ),
                            )
                            .with_data(serde_json::json!({
                                "rule": "require_test", "what": what,
                                "definition": graph::ref_str(ws, d),
                            })),
                        );
                    }
                }
            }
            Rule::RequireAttest { kind, what, line } => {
                let have = cx.attested.get(kind);
                for d in select(ws, what) {
                    if ws.defs[d].name.starts_with("test_") {
                        continue;
                    }
                    if !have.map(|s| s.contains(&d)).unwrap_or(false) {
                        out.push(
                            Diagnostic::new(
                                E0214_RULE,
                                Severity::Error,
                                &ws.defs[d].module_path,
                                at(*line),
                                format!(
                                    "rule `require attest {} {}`: {} carries no live {} claim",
                                    kind,
                                    what,
                                    graph::ref_str(ws, d),
                                    kind
                                ),
                            )
                            .with_data(serde_json::json!({
                                "rule": "require_attest", "kind": kind,
                                "definition": graph::ref_str(ws, d),
                            })),
                        );
                    }
                }
            }
            Rule::DenyDuplicates { line } => {
                for (h, ds) in duplicates(ws, cx.hashes) {
                    let names: Vec<String> =
                        ds.iter().map(|&d| graph::ref_str(ws, d)).collect();
                    out.push(
                        Diagnostic::new(
                            E0214_RULE,
                            Severity::Error,
                            &ws.defs[ds[0]].module_path,
                            at(*line),
                            format!(
                                "rule `deny duplicates`: {} definitions mean exactly the same \
                                 thing ({}): {}",
                                ds.len(),
                                &h[..19.min(h.len())],
                                names.join(", ")
                            ),
                        )
                        .with_data(serde_json::json!({
                            "rule": "deny_duplicates", "hash": h, "definitions": names,
                        })),
                    );
                }
            }
        }
    }
    out
}

fn def_caps(ws: &Workspace, d: DefId) -> crate::ast::Caps {
    let i = &ws.defs[d];
    match i.kind {
        crate::resolve::DefKind::Fn => ws.modules[i.module_idx].fns[i.item_idx].caps,
        crate::resolve::DefKind::Extern => ws.modules[i.module_idx].externs[i.item_idx].caps,
        crate::resolve::DefKind::Table => 0,
    }
}

/// Definitions that share an identity, grouped.
///
/// Two definitions with the same hash are the same definition wearing two
/// names — the canonical form erased everything that differed. That is the
/// duplication a search for similar text cannot find and cannot prove: this
/// is not "these look alike", it is "these *are* alike".
pub fn duplicates(
    ws: &Workspace,
    hashes: &HashMap<DefId, Hash>,
) -> Vec<(String, Vec<DefId>)> {
    let mut by: BTreeMap<String, Vec<DefId>> = BTreeMap::new();
    for d in 0..ws.defs.len() {
        if ws.defs[d].poisoned || ws.defs[d].module_path.starts_with("std/") {
            continue;
        }
        if let Some(h) = hashes.get(&d) {
            by.entry(hex(h)).or_default().push(d);
        }
    }
    by.into_iter().filter(|(_, v)| v.len() > 1).collect()
}
