//! Semantic diff: grade a *text* diff by identity.
//!
//! A text diff cannot tell a rename from a rewrite, so every reviewer reads
//! one line at a time to find out. Here identity is exact, so the question is
//! a set comparison: if the two revisions carry the same set of hashes, then
//! no definition changed meaning, however many lines moved.
//!
//! Two questions get separate answers, because they have different victims:
//!
//!   - **meaning** — did any definition's hash move? Breaks behaviour.
//!   - **surface** — did any definition's name or module move? Breaks callers
//!     outside this workspace, while behaviour is untouched.
//!
//! A pure refactor is `meaning: unchanged, surface: changed`. That pair is
//! not expressible in a language where a rename *is* an edit.

use crate::canon::{self, Hash};
use crate::graph;
use crate::resolve::{DefId, Workspace};
use std::collections::{BTreeMap, HashMap};

pub const FORMAT: &str = "rigid-snapshot/0";

/// `path#name` -> hex identity, for every definition in a workspace.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub format: String,
    pub defs: BTreeMap<String, String>,
}

pub fn snapshot(ws: &Workspace, hashes: &HashMap<DefId, Hash>) -> Snapshot {
    let mut defs = BTreeMap::new();
    for d in 0..ws.defs.len() {
        if ws.defs[d].poisoned {
            continue;
        }
        if let Some(h) = hashes.get(&d) {
            defs.insert(graph::ref_str(ws, d), canon::hex(h));
        }
    }
    Snapshot {
        format: FORMAT.to_string(),
        defs,
    }
}

pub fn snapshot_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".rigid-snapshot.json")
}

pub fn load(path: &std::path::Path) -> Result<Snapshot, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let snap: Snapshot =
        serde_json::from_str(&s).map_err(|e| format!("{}: {}", path.display(), e))?;
    if snap.format != FORMAT {
        return Err(format!(
            "{}: unknown snapshot format '{}' (expected '{}')",
            path.display(),
            snap.format,
            FORMAT
        ));
    }
    Ok(snap)
}

pub fn save(path: &std::path::Path, snap: &Snapshot) -> Result<(), String> {
    let s = serde_json::to_string_pretty(snap).map_err(|e| e.to_string())?;
    std::fs::write(path, s + "\n").map_err(|e| format!("{}: {}", path.display(), e))
}

#[derive(Debug, Default)]
pub struct Report {
    pub base_total: usize,
    pub now_total: usize,
    pub unchanged: usize,
    /// (before, after) — same hash, same module, different name.
    pub renamed: Vec<(String, String)>,
    /// (before, after) — same hash, different module.
    pub moved: Vec<(String, String)>,
    /// (ref, before-hash, after-hash) — same name, different meaning.
    pub changed: Vec<(String, String, String)>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl Report {
    /// True when the two revisions carry exactly the same set of identities.
    /// Nothing gained a new meaning, nothing lost one — whatever the text did.
    pub fn meaning_preserved(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }

    pub fn surface_changes(&self) -> usize {
        self.renamed.len() + self.moved.len()
    }
}

fn module_of(r: &str) -> &str {
    r.split('#').next().unwrap_or(r)
}

pub fn compare(base: &Snapshot, now: &Snapshot) -> Report {
    let mut rep = Report {
        base_total: base.defs.len(),
        now_total: now.defs.len(),
        ..Default::default()
    };

    // 1. Same reference, same identity. The overwhelming majority, and the
    //    only category that costs a reviewer nothing.
    let mut base_left: BTreeMap<&str, &str> = BTreeMap::new();
    let mut now_left: BTreeMap<&str, &str> = BTreeMap::new();
    for (k, h) in &base.defs {
        match now.defs.get(k) {
            Some(h2) if h2 == h => rep.unchanged += 1,
            _ => {
                base_left.insert(k, h);
            }
        }
    }
    for (k, h) in &now.defs {
        if base.defs.get(k).map(|h2| h2 == h) != Some(true) {
            now_left.insert(k, h);
        }
    }

    // 2. An identity that survived under a different reference is a rename or
    //    a move — never a rewrite. Done before (3) on purpose: when a name is
    //    reused for something new, the old meaning is still findable, and
    //    calling that a rename plus an addition is the truthful reading.
    let mut by_hash: HashMap<&str, Vec<&str>> = HashMap::new();
    for (k, h) in &now_left {
        by_hash.entry(h).or_default().push(k);
    }
    for v in by_hash.values_mut() {
        v.sort();
    }
    let mut claimed: Vec<&str> = Vec::new();
    let base_keys: Vec<(&str, &str)> = base_left.iter().map(|(k, h)| (*k, *h)).collect();
    for (bk, bh) in base_keys {
        let Some(cands) = by_hash.get_mut(bh) else {
            continue;
        };
        let Some(nk) = cands.pop() else { continue };
        if module_of(bk) == module_of(nk) {
            rep.renamed.push((bk.to_string(), nk.to_string()));
        } else {
            rep.moved.push((bk.to_string(), nk.to_string()));
        }
        base_left.remove(bk);
        claimed.push(nk);
    }
    for k in claimed {
        now_left.remove(k);
    }

    // 3. A reference that survived with a different identity changed meaning.
    let survivors: Vec<&str> = base_left
        .keys()
        .copied()
        .filter(|k| now_left.contains_key(k))
        .collect();
    for k in survivors {
        let before = base_left.remove(k).unwrap().to_string();
        let after = now_left.remove(k).unwrap().to_string();
        rep.changed.push((k.to_string(), before, after));
    }

    rep.added = now_left.keys().map(|k| k.to_string()).collect();
    rep.removed = base_left.keys().map(|k| k.to_string()).collect();
    rep.renamed.sort();
    rep.moved.sort();
    rep.changed.sort();
    rep.added.sort();
    rep.removed.sort();
    rep
}

fn short(h: &str) -> String {
    h.strip_prefix("sha256:")
        .unwrap_or(h)
        .chars()
        .take(12)
        .collect()
}

pub fn render(rep: &Report) -> String {
    let mut o = String::new();
    let line = |o: &mut String, s: String| {
        o.push_str(&s);
        o.push('\n');
    };

    line(
        &mut o,
        format!(
            "{} definitions before, {} after — {} identical",
            rep.base_total, rep.now_total, rep.unchanged
        ),
    );
    line(&mut o, String::new());

    for (a, b) in &rep.renamed {
        line(&mut o, format!("  renamed   {}  ->  {}", a, b));
    }
    for (a, b) in &rep.moved {
        line(&mut o, format!("  moved     {}  ->  {}", a, b));
    }
    for (k, before, after) in &rep.changed {
        line(
            &mut o,
            format!(
                "  changed   {}  {} -> {}",
                k,
                short(before),
                short(after)
            ),
        );
    }
    for k in &rep.added {
        line(&mut o, format!("  added     {}", k));
    }
    for k in &rep.removed {
        line(&mut o, format!("  removed   {}", k));
    }
    if rep.renamed.is_empty()
        && rep.moved.is_empty()
        && rep.changed.is_empty()
        && rep.added.is_empty()
        && rep.removed.is_empty()
    {
        line(&mut o, "  (nothing)".into());
    }
    line(&mut o, String::new());

    if rep.meaning_preserved() {
        line(&mut o, "meaning   unchanged — provably semantics-preserving".into());
        line(
            &mut o,
            "          every identity on both sides is the same set, so no".into(),
        );
        line(&mut o, "          definition can behave differently.".into());
    } else {
        let n = rep.changed.len() + rep.added.len() + rep.removed.len();
        line(
            &mut o,
            format!(
                "meaning   {} definition(s) changed meaning ({} changed, {} added, {} removed)",
                n,
                rep.changed.len(),
                rep.added.len(),
                rep.removed.len()
            ),
        );
    }
    if rep.surface_changes() == 0 {
        line(&mut o, "surface   unchanged — no caller outside can break".into());
    } else {
        line(
            &mut o,
            format!(
                "surface   {} name(s) moved — behaviour is intact, but callers",
                rep.surface_changes()
            ),
        );
        line(&mut o, "          outside this workspace refer to them by name".into());
    }
    o
}

pub fn to_json(rep: &Report) -> serde_json::Value {
    serde_json::json!({
        "base_total": rep.base_total,
        "now_total": rep.now_total,
        "unchanged": rep.unchanged,
        "meaning_preserved": rep.meaning_preserved(),
        "surface_changes": rep.surface_changes(),
        "renamed": rep.renamed.iter()
            .map(|(a, b)| serde_json::json!({"from": a, "to": b})).collect::<Vec<_>>(),
        "moved": rep.moved.iter()
            .map(|(a, b)| serde_json::json!({"from": a, "to": b})).collect::<Vec<_>>(),
        "changed": rep.changed.iter()
            .map(|(k, a, b)| serde_json::json!({"ref": k, "before": a, "after": b}))
            .collect::<Vec<_>>(),
        "added": rep.added,
        "removed": rep.removed,
    })
}
