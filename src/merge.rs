//! Three-way merge over identities instead of lines.
//!
//! Git merges text, so it sees a rename and an edit to the same function as
//! two changes to the same lines and stops. They are not the same change:
//! one moved a *name*, the other moved a *meaning*, and those are separate
//! coordinates here. rigid can take both.
//!
//! The unit is a definition, paired across the three revisions by identity
//! first and name second:
//!
//!   - a hash that survives is the same definition, whatever it is called;
//!   - a name that survives is the same definition, whatever it now does.
//!
//! With the pairing settled, name and body are resolved independently. Two
//! sides that renamed differently conflict on the name; two sides that edited
//! differently conflict on the body; one of each does not conflict at all.
//!
//! Bodies move as text, comments and all — text is authoritative for
//! content, so a merge relocates it rather than regenerating it from the
//! graph. References inside a moved body are rewritten to the merged names,
//! skipping string literals, and the proof that it worked is that the result
//! checks.

use crate::semdiff::Snapshot;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    Ours,
    Theirs,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Neither side touched it.
    Unchanged,
    /// Exactly one side changed it; take that side whole.
    TakeOne(Side),
    /// Both sides made the same change.
    Agreed,
    /// One side renamed it, the other changed what it does. Both survive —
    /// the case git reports as a conflict and this does not.
    NameFrom(Side),
    /// Both renamed it, differently.
    ConflictName(String, String),
    /// Both changed what it does, differently.
    ConflictBody(String, String),
    /// One deleted it, the other changed it.
    ConflictDelete(Side),
    Added(Side),
    Removed(Side),
}

impl Outcome {
    pub fn is_conflict(&self) -> bool {
        matches!(
            self,
            Outcome::ConflictName(..) | Outcome::ConflictBody(..) | Outcome::ConflictDelete(_)
        )
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    /// Reference in the base revision, or in whichever side introduced it.
    pub base_ref: Option<String>,
    /// Reference the merged workspace should use.
    pub merged_ref: Option<String>,
    /// Which side supplies the body.
    pub body_from: Option<Side>,
    pub outcome: Outcome,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub items: Vec<Item>,
}

impl Plan {
    pub fn conflicts(&self) -> usize {
        self.items.iter().filter(|i| i.outcome.is_conflict()).count()
    }
    /// Merges git cannot do: a rename on one side, an edit on the other.
    pub fn rescued(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.outcome, Outcome::NameFrom(_)))
            .count()
    }
}

fn module_of(r: &str) -> &str {
    r.split('#').next().unwrap_or(r)
}

fn name_of(r: &str) -> &str {
    r.split('#').nth(1).unwrap_or(r)
}

/// Find `base_ref`'s counterpart in `side`: same identity under any name, or
/// the same name carrying a new identity.
fn counterpart(base_ref: &str, base_hash: &str, side: &Snapshot) -> Option<(String, String)> {
    if let Some(h) = side.defs.get(base_ref) {
        return Some((base_ref.to_string(), h.clone()));
    }
    side.defs
        .iter()
        .find(|(_, h)| *h == base_hash)
        .map(|(k, h)| (k.clone(), h.clone()))
}

pub fn plan(base: &Snapshot, ours: &Snapshot, theirs: &Snapshot) -> Plan {
    let mut items = Vec::new();
    let mut seen_ours: BTreeSet<String> = BTreeSet::new();
    let mut seen_theirs: BTreeSet<String> = BTreeSet::new();

    for (bref, bhash) in &base.defs {
        let o = counterpart(bref, bhash, ours);
        let t = counterpart(bref, bhash, theirs);
        if let Some((k, _)) = &o {
            seen_ours.insert(k.clone());
        }
        if let Some((k, _)) = &t {
            seen_theirs.insert(k.clone());
        }

        let (outcome, merged_ref, body_from) = match (&o, &t) {
            (None, None) => (Outcome::Removed(Side::Ours), None, None),
            (None, Some((tk, th))) => {
                if th == bhash {
                    (Outcome::TakeOne(Side::Ours), None, None) // deleted by us, untouched by them
                } else {
                    (Outcome::ConflictDelete(Side::Ours), Some(tk.clone()), Some(Side::Theirs))
                }
            }
            (Some((ok, oh)), None) => {
                if oh == bhash {
                    (Outcome::TakeOne(Side::Theirs), None, None) // deleted by them
                } else {
                    (Outcome::ConflictDelete(Side::Theirs), Some(ok.clone()), Some(Side::Ours))
                }
            }
            (Some((ok, oh)), Some((tk, th))) => {
                let o_named = ok != bref;
                let t_named = tk != bref;
                let o_bodied = oh != bhash;
                let t_bodied = th != bhash;

                // Name: whoever moved it, or a conflict if both did.
                let name = match (o_named, t_named) {
                    (false, false) => Ok(bref.clone()),
                    (true, false) => Ok(ok.clone()),
                    (false, true) => Ok(tk.clone()),
                    (true, true) if ok == tk => Ok(ok.clone()),
                    (true, true) => Err((ok.clone(), tk.clone())),
                };
                // Body: independently.
                let body = match (o_bodied, t_bodied) {
                    (false, false) => Ok(None),
                    (true, false) => Ok(Some(Side::Ours)),
                    (false, true) => Ok(Some(Side::Theirs)),
                    (true, true) if oh == th => Ok(Some(Side::Ours)),
                    (true, true) => Err((oh.clone(), th.clone())),
                };

                match (name, body) {
                    (Err((a, b)), _) => (Outcome::ConflictName(a, b), Some(bref.clone()), None),
                    (_, Err((a, b))) => (Outcome::ConflictBody(a, b), Some(bref.clone()), None),
                    (Ok(n), Ok(bf)) => {
                        let out = match (o_named || o_bodied, t_named || t_bodied) {
                            (false, false) => Outcome::Unchanged,
                            (true, false) => Outcome::TakeOne(Side::Ours),
                            (false, true) => Outcome::TakeOne(Side::Theirs),
                            (true, true) if oh == th && ok == tk => Outcome::Agreed,
                            // The interesting one: each side moved a
                            // different coordinate, so both survive.
                            (true, true) => Outcome::NameFrom(if o_named {
                                Side::Ours
                            } else {
                                Side::Theirs
                            }),
                        };
                        let bf = bf.or(Some(Side::Ours));
                        (out, Some(n), bf)
                    }
                }
            }
        };
        items.push(Item {
            base_ref: Some(bref.clone()),
            merged_ref,
            body_from,
            outcome,
        });
    }

    for (k, _) in &ours.defs {
        if !seen_ours.contains(k) {
            items.push(Item {
                base_ref: None,
                merged_ref: Some(k.clone()),
                body_from: Some(Side::Ours),
                outcome: Outcome::Added(Side::Ours),
            });
        }
    }
    for (k, _) in &theirs.defs {
        if !seen_theirs.contains(k) {
            items.push(Item {
                base_ref: None,
                merged_ref: Some(k.clone()),
                body_from: Some(Side::Theirs),
                outcome: Outcome::Added(Side::Theirs),
            });
        }
    }
    Plan { items }
}

// ------------------------------------------------------------- source slices

/// The source lines of one definition, located by brace matching from its
/// `fn`/`table`/`extern` line. Text is authoritative for content, so a merge
/// moves text — it does not regenerate it from the graph and lose comments.
pub fn slice_of(src: &str, name: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = src.lines().collect();
    let decl = |l: &str| -> bool {
        let l = l.trim_start();
        for kw in ["fn ", "memo fn ", "table ", "extern fn "] {
            if let Some(rest) = l.strip_prefix(kw) {
                let ident: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if ident == name {
                    return true;
                }
            }
        }
        false
    };
    let start = lines.iter().position(|l| decl(l))?;
    if lines[start].trim_start().starts_with("extern ") {
        let mut end = start;
        while end < lines.len() && !lines[end].contains(';') {
            end += 1;
        }
        return Some((start, end.min(lines.len() - 1)));
    }
    let mut depth = 0i32;
    let mut seen = false;
    for (li, line) in lines.iter().enumerate().skip(start) {
        let mut in_str = false;
        let mut esc = false;
        for c in line.chars() {
            if esc {
                esc = false;
                continue;
            }
            match c {
                '\\' if in_str => esc = true,
                '"' => in_str = !in_str,
                '{' if !in_str => {
                    depth += 1;
                    seen = true;
                }
                '}' if !in_str => {
                    depth -= 1;
                    if seen && depth == 0 {
                        return Some((start, li));
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Rewrite identifiers in one definition's source slice.
///
/// A body taken from one side may name definitions the other side renamed.
/// The rewrite is textual, but it is scoped to a single definition and it
/// skips string literals, so `print("clamp")` survives a `clamp` -> `clip`
/// rename that the code around it does not.
pub fn rename_idents(slice: &str, map: &BTreeMap<String, String>) -> String {
    if map.is_empty() {
        return slice.to_string();
    }
    let mut out = String::with_capacity(slice.len());
    let cs: Vec<char> = slice.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    while i < cs.len() {
        let c = cs[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < cs.len() {
                out.push(cs[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            let word: String = cs[start..i].iter().collect();
            // A word preceded by a name character is not a fresh identifier,
            // but the scan above already consumed those together.
            out.push_str(map.get(&word).unwrap_or(&word));
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

pub type Files = BTreeMap<String, String>;

/// Assemble the merged sources. Every definition's text comes from the side
/// the plan chose, carrying that side's comments and spacing with it.
pub fn assemble(
    plan: &Plan,
    ours_files: &Files,
    theirs_files: &Files,
) -> Result<Files, Vec<String>> {
    let mut errs = Vec::new();
    let mut out: Files = ours_files.clone();

    // What each side calls a definition, versus what the merge decided to
    // call it. A body spliced from one side is rewritten through its own map.
    let mut ren: BTreeMap<Side, BTreeMap<String, String>> = BTreeMap::new();
    for item in &plan.items {
        let (Some(mref), Some(bref)) = (&item.merged_ref, &item.base_ref) else {
            continue;
        };
        let merged = name_of(mref);
        // The side that did NOT rename still writes the base name.
        if let Outcome::NameFrom(namer) = item.outcome {
            let other = match namer {
                Side::Ours => Side::Theirs,
                Side::Theirs => Side::Ours,
            };
            if name_of(bref) != merged {
                ren.entry(other)
                    .or_default()
                    .insert(name_of(bref).to_string(), merged.to_string());
            }
        }
        // A rename by one side alone: the other side still uses the old name.
        if let Outcome::TakeOne(mover) = item.outcome {
            if name_of(bref) != merged {
                let other = match mover {
                    Side::Ours => Side::Theirs,
                    Side::Theirs => Side::Ours,
                };
                ren.entry(other)
                    .or_default()
                    .insert(name_of(bref).to_string(), merged.to_string());
            }
        }
    }

    for item in &plan.items {
        if item.outcome.is_conflict() {
            continue;
        }
        let (Some(mref), Some(side)) = (&item.merged_ref, item.body_from) else {
            continue;
        };
        // Nothing to move when our copy already is the answer.
        if side == Side::Ours && matches!(item.outcome, Outcome::Unchanged | Outcome::Added(Side::Ours)) {
            continue;
        }
        let src_files = match side {
            Side::Ours => ours_files,
            Side::Theirs => theirs_files,
        };
        // The name the body is filed under on its own side.
        let src_ref = match (side, &item.base_ref) {
            (Side::Ours, _) => mref.clone(),
            (Side::Theirs, Some(b)) => {
                // Their copy still uses their name for it.
                if src_files
                    .get(module_of(mref))
                    .map(|s| slice_of(s, name_of(mref)).is_some())
                    .unwrap_or(false)
                {
                    mref.clone()
                } else {
                    b.clone()
                }
            }
            (Side::Theirs, None) => mref.clone(),
        };
        let Some(src) = src_files.get(module_of(&src_ref)) else {
            errs.push(format!("{}: missing module on the {:?} side", src_ref, side));
            continue;
        };
        let Some((a, b)) = slice_of(src, name_of(&src_ref)) else {
            errs.push(format!("{}: could not locate its source", src_ref));
            continue;
        };
        let lines: Vec<&str> = src.lines().collect();
        let mut map = ren.get(&side).cloned().unwrap_or_default();
        if name_of(&src_ref) != name_of(mref) {
            map.insert(name_of(&src_ref).to_string(), name_of(mref).to_string());
        }
        let taken = rename_idents(&lines[a..=b].join("\n"), &map) + "\n";

        let dest = out.entry(module_of(mref).to_string()).or_default();
        match slice_of(dest, name_of(mref))
            .or_else(|| item.base_ref.as_ref().and_then(|b| slice_of(dest, name_of(b))))
        {
            Some((da, db)) => {
                let dl: Vec<&str> = dest.lines().collect();
                let mut merged = dl[..da].join("\n");
                if da > 0 {
                    merged.push('\n');
                }
                merged.push_str(&taken);
                if db + 1 < dl.len() {
                    merged.push_str(&dl[db + 1..].join("\n"));
                    merged.push('\n');
                }
                *dest = merged;
            }
            None => {
                if !dest.ends_with('\n') && !dest.is_empty() {
                    dest.push('\n');
                }
                dest.push_str(&taken);
            }
        }
    }
    if errs.is_empty() {
        Ok(out)
    } else {
        Err(errs)
    }
}

pub fn render(plan: &Plan) -> String {
    let mut o = String::new();
    let mut line = |s: String| {
        o.push_str(&s);
        o.push('\n');
    };
    let n = plan.items.len();
    let conflicts = plan.conflicts();
    let rescued = plan.rescued();

    for item in &plan.items {
        let r = item
            .merged_ref
            .clone()
            .or_else(|| item.base_ref.clone())
            .unwrap_or_default();
        match &item.outcome {
            Outcome::Unchanged | Outcome::Agreed => {}
            Outcome::TakeOne(s) => line(format!("  take {:?}   {}", s, r)),
            Outcome::Added(s) => line(format!("  added {:?}  {}", s, r)),
            Outcome::Removed(_) => line(format!("  removed    {}", r)),
            Outcome::NameFrom(s) => {
                let (namer, editor) = match s {
                    Side::Ours => ("ours", "theirs"),
                    Side::Theirs => ("theirs", "ours"),
                };
                line(format!(
                    "  merged     {}  ({} renamed it, {} changed what it does — kept both)",
                    r, namer, editor
                ));
            }
            Outcome::ConflictName(a, b) => {
                line(format!("  CONFLICT   {}  renamed two ways: {} vs {}", r, a, b))
            }
            Outcome::ConflictBody(..) => line(format!(
                "  CONFLICT   {}  both sides changed what it does, differently",
                r
            )),
            Outcome::ConflictDelete(s) => line(format!(
                "  CONFLICT   {}  {:?} deleted it, the other side changed it",
                r, s
            )),
        }
    }
    line(String::new());
    line(format!("{} definitions considered, {} conflicts.", n, conflicts));
    if rescued > 0 {
        line(format!(
            "{} merged on both coordinates at once — a rename on one side and an\n\
             edit on the other. A line-based merge reports those as conflicts,\n\
             because it cannot see that the two changes are about different things.",
            rescued
        ));
    }
    o
}
