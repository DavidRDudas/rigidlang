//! The binding sidecar: `<module>.rg.bind.json`, a resolution lockfile.
//!
//! Text is authoritative for CONTENT. The sidecar is authoritative for
//! IDENTITY: which definition each imported name means, pinned by hash.
//! Reconciliation rules (docs/SPEC.md §4.3):
//!
//!   R1  name present, hash matches pin            -> OK
//!   R2  name present, hash differs, pinned hash
//!       still exists under another name           -> E0202 resolution-changed
//!       (someone re-used the name for a different definition; refusing to
//!        silently rebind is the entire point of this language)
//!   R3  name present, hash differs, pinned hash
//!       gone from target module                   -> W0401 drift; repin
//!       (normal editing of the target definition; error under --frozen)
//!   R4  name absent, pinned hash exists under
//!       another name                              -> E0201 + typed rename fix
//!       (the definition was renamed; a machine can apply the fix)
//!   R5  name absent, pinned hash gone             -> E0201 plain
//!   R6  no pin yet                                -> pin now; I0402

use crate::canon::{hex, Hash};
use crate::diag::*;
use crate::resolve::{DefId, Workspace};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;

pub const FORMAT: &str = "rigid-bind/0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RefPin {
    pub target: String, // "path/to/module.rg#name" as of last pin
    pub hash: String,   // "sha256:..." as of last pin
    /// The capability set this definition declared when it was pinned, in
    /// canonical order. Capabilities are part of the hash, so a widening
    /// already shows up as drift — recording them is what lets the drift say
    /// *what kind* of drift, which is the difference between "this changed"
    /// and "this can now reach the network". Absent in sidecars written
    /// before capabilities existed, and an absent set is not a claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BindFile {
    pub format: String,
    /// this module's own definitions, name -> hash (informational; feeds rdeps
    /// tooling and makes within-module drift visible in diffs)
    pub definitions: BTreeMap<String, String>,
    /// quarantine pins: "table:<name>" or "dyn:<def>#<ordinal>" ->
    /// (entry key or target name -> target hash). Drift here is W0404.
    #[serde(default)]
    pub quarantine: BTreeMap<String, BTreeMap<String, String>>,
    /// this module's own content identity: hash over its members'
    /// hashes, sorted, names excluded (informational).
    #[serde(default)]
    pub module_hash: String,
    /// imported name -> pin
    pub references: BTreeMap<String, RefPin>,
}

impl BindFile {
    fn empty() -> Self {
        BindFile {
            format: FORMAT.to_string(),
            definitions: BTreeMap::new(),
            quarantine: BTreeMap::new(),
            module_hash: String::new(),
            references: BTreeMap::new(),
        }
    }
}

pub fn sidecar_path(ws: &Workspace, module_path: &str) -> PathBuf {
    ws.root.join(format!("{}.bind.json", module_path))
}

pub fn load(ws: &Workspace, module_path: &str) -> Option<BindFile> {
    let p = sidecar_path(ws, module_path);
    let s = fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

pub struct ReconcileOutcome {
    pub diags: Vec<Diagnostic>,
    pub updated: Option<BindFile>, // Some(..) if the sidecar should be rewritten
}

pub fn reconcile(
    ws: &Workspace,
    module_idx: usize,
    hashes: &HashMap<DefId, Hash>,
    frozen: bool,
) -> ReconcileOutcome {
    let m = &ws.modules[module_idx];
    let old = load(ws, &m.path).unwrap_or_else(BindFile::empty);
    let mut new = BindFile::empty();
    let mut diags = Vec::new();

    // own definitions (functions and tables alike — both are defs)
    for (d, info) in ws.defs.iter().enumerate() {
        if info.module_path == m.path {
            if let Some(h) = hashes.get(&d) {
                new.definitions.insert(info.name.clone(), hex(h));
            }
        }
    }
    new.module_hash = crate::canon::module_hash_hex(ws, hashes, &m.path);

    // quarantine pins: tables and dyn sites of this module
    for (d, info) in ws.defs.iter().enumerate() {
        if info.module_path != m.path {
            continue;
        }
        if let Some(entries) = ws.table_entries.get(&d) {
            let mut inner = BTreeMap::new();
            for (k, t) in entries {
                if let Some(h) = hashes.get(t) {
                    inner.insert(k.clone(), hex(h));
                }
            }
            new.quarantine.insert(format!("table:{}", info.name), inner);
        }
        for (ord, (_, set)) in ws.dyn_sites.get(d).map(|v| v.as_slice()).unwrap_or(&[]).iter().enumerate() {
            let mut inner = BTreeMap::new();
            for t in set {
                if let Some(h) = hashes.get(t) {
                    inner.insert(ws.defs[*t].name.clone(), hex(h));
                }
            }
            new.quarantine
                .insert(format!("dyn:{}#{}", info.name, ord), inner);
        }
    }

    // A module with unresolved references keeps its sidecar untouched:
    // rewriting it would destroy the very evidence (old name -> hash)
    // that makes rename recovery possible. R7, learned the hard way.
    let broken_count = ws
        .bindings
        .iter()
        .filter(|((def, _, _, _), b)| {
            **b == crate::resolve::Binding::Broken && ws.defs[*def].module_path == m.path
        })
        .count();
    let module_broken = broken_count > 0;

    // I0406: a definition of THIS module was renamed (same hash, new name)
    // and broken local references to the old name exist. Cross-module refs
    // ride the import machinery (R4); this closes the same-module gap.
    {
        let old_sc = &old;
        for (oldname, oldhash) in &old_sc.definitions {
            if new.definitions.contains_key(oldname) {
                continue;
            }
            let Some((newname, _)) = new
                .definitions
                .iter()
                .find(|(n, h)| *h == oldhash && *n != oldname)
            else {
                continue;
            };
            let has_broken = ws.bindings.iter().any(|((def, _, _, n), b)| {
                *b == crate::resolve::Binding::Broken
                    && n == oldname
                    && ws.defs[*def].module_path == m.path
            });
            if has_broken {
                let mut diag = Diagnostic::new(
                    I0406_LOCAL_RENAME,
                    Severity::Info,
                    &m.path,
                    None,
                    format!(
                        "'{}' was renamed to '{}' in this module ({}...); local references are repairable",
                        oldname,
                        newname,
                        &oldhash[..12.min(oldhash.len())]
                    ),
                );
                diag = diag.with_fix(serde_json::json!({
                    "kind": "rename_reference",
                    "module": m.path,
                    "from": oldname,
                    "to": newname,
                    "scope": "all occurrences in this module",
                }));
                diags.push(diag);
            }
        }
    }

    // W0404: pinned quarantine target content changed
    {
        let old_sc = &old;
        for (site, old_inner) in &old_sc.quarantine {
            if let Some(new_inner) = new.quarantine.get(site) {
                for (k, oldh) in old_inner {
                    if let Some(newh) = new_inner.get(k) {
                        if oldh != newh {
                            let sev = if frozen { Severity::Error } else { Severity::Warning };
                            diags.push(Diagnostic::new(
                                W0404_QUARANTINE_DRIFT,
                                sev,
                                &m.path,
                                None,
                                format!(
                                    "quarantine drift: {} [{}] now resolves to different content ({}… -> {}…){}",
                                    site,
                                    k,
                                    &oldh[..12.min(oldh.len())],
                                    &newh[..12.min(newh.len())],
                                    if frozen { " (--frozen: refusing to repin)" } else { "" }
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }

    // helper: current name->hash table for a target module
    let target_table = |target: &str| -> BTreeMap<String, String> {
        let mut t = BTreeMap::new();
        for (d, info) in ws.defs.iter().enumerate() {
            if info.module_path == *target {
                if let Some(h) = hashes.get(&d) {
                    t.insert(info.name.clone(), hex(h));
                }
            }
        }
        t
    };

    // helper: current name->capability-set table for a target module
    let caps_table = |target: &str| -> BTreeMap<String, Vec<String>> {
        let mut t = BTreeMap::new();
        for info in ws.defs.iter() {
            if info.module_path != *target {
                continue;
            }
            let caps = match info.kind {
                crate::resolve::DefKind::Fn => {
                    ws.modules[info.module_idx].fns[info.item_idx].caps
                }
                crate::resolve::DefKind::Extern => {
                    ws.modules[info.module_idx].externs[info.item_idx].caps
                }
                crate::resolve::DefKind::Table => 0,
            };
            t.insert(
                info.name.clone(),
                crate::ast::caps_list(caps)
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
        }
        t
    };

    let imp = &ws.imports[module_idx];

    // broken imports first (R4 / R5)
    for b in &imp.broken {
        if !b.target_exists {
            diags.push(Diagnostic::new(
                E0201_UNRESOLVED,
                Severity::Error,
                &m.path,
                Some(b.span),
                format!("imported module \"{}\" not found", b.target_module),
            ));
            continue;
        }
        let table = target_table(&b.target_module);
        let pin = old.references.get(&b.name);
        let renamed_to = pin.and_then(|p| {
            table
                .iter()
                .find(|(_, h)| **h == p.hash)
                .map(|(n, _)| n.clone())
        });
        match (pin, renamed_to) {
            (Some(p), Some(n2)) => {
                diags.push(
                    Diagnostic::new(
                        E0201_UNRESOLVED,
                        Severity::Error,
                        &m.path,
                        Some(b.span),
                        format!(
                            "unresolved import '{}' from \"{}\" — the pinned definition \
                             ({}) now exists under the name '{}'",
                            b.name,
                            b.target_module,
                            short(&p.hash),
                            n2
                        ),
                    )
                    .with_data(serde_json::json!({
                        "name": b.name, "target": b.target_module,
                        "pinned_hash": p.hash, "renamed_to": n2
                    }))
                    .with_fix(serde_json::json!({
                        "kind": "rename_reference",
                        "module": m.path,
                        "from": b.name,
                        "to": n2,
                        "scope": "all occurrences in this module, including the import"
                    })),
                );
                // keep the old pin so the fix can still be applied later
                new.references
                    .insert(b.name.clone(), p.clone());
            }
            (Some(p), None) => {
                diags.push(
                    Diagnostic::new(
                        E0201_UNRESOLVED,
                        Severity::Error,
                        &m.path,
                        Some(b.span),
                        format!(
                            "unresolved import '{}' from \"{}\" — pinned definition ({}) \
                             no longer exists there",
                            b.name,
                            b.target_module,
                            short(&p.hash)
                        ),
                    )
                    .with_data(serde_json::json!({
                        "name": b.name, "target": b.target_module, "pinned_hash": p.hash
                    })),
                );
            }
            (None, _) => {
                diags.push(Diagnostic::new(
                    E0201_UNRESOLVED,
                    Severity::Error,
                    &m.path,
                    Some(b.span),
                    format!(
                        "unresolved import '{}' from \"{}\"",
                        b.name, b.target_module
                    ),
                ));
            }
        }
    }

    // resolving imports (R1 / R2 / R3 / R6)
    for (name, (target, span)) in &imp.map {
        if imp.broken.iter().any(|b| &b.name == name) {
            continue;
        }
        let table = target_table(target);
        let Some(current_hash) = table.get(name) else {
            continue; // poisoned target def: no hash this run; leave pin as-is
        };
        let pin = old.references.get(name);
        match pin {
            None => {
                // R6: first sight — pin it
                new.references.insert(
                    name.clone(),
                    RefPin {
                        target: format!("{}#{}", target, name),
                        hash: current_hash.clone(),
                        caps: caps_table(target).get(name).cloned(),
                    },
                );
                diags.push(
                    Diagnostic::new(
                        I0402_PINNED,
                        Severity::Info,
                        &m.path,
                        Some(*span),
                        format!("pinned '{}' -> {}#{} ({})", name, target, name, short(current_hash)),
                    )
                    .with_data(serde_json::json!({
                        "name": name, "target": format!("{}#{}", target, name),
                        "hash": current_hash
                    })),
                );
            }
            Some(p) if p.hash == *current_hash => {
                // R1: exact match
                new.references.insert(name.clone(), p.clone());
            }
            Some(p) => {
                // hash differs — is the pinned definition still around under
                // another name? (R2) or was it edited in place? (R3)
                let elsewhere = table
                    .iter()
                    .find(|(n, h)| *n != name && **h == p.hash)
                    .map(|(n, _)| n.clone());
                if let Some(other) = elsewhere {
                    // R2: the name now means a DIFFERENT definition
                    diags.push(
                        Diagnostic::new(
                            E0202_RESOLUTION_CHANGED,
                            Severity::Error,
                            &m.path,
                            Some(*span),
                            format!(
                                "'{}' from \"{}\" no longer means what it meant: the pinned \
                                 definition ({}) is now named '{}', and '{}' now names a \
                                 different definition ({}). Refusing to silently rebind. \
                                 Run `rigid rebind {}#{}` to accept the new meaning, or \
                                 rename the reference to '{}'.",
                                name,
                                target,
                                short(&p.hash),
                                other,
                                name,
                                short(current_hash),
                                m.path,
                                name,
                                other
                            ),
                        )
                        .with_data(serde_json::json!({
                            "name": name, "target": target,
                            "pinned_hash": p.hash, "pinned_now_named": other,
                            "name_now_hash": current_hash
                        }))
                        .with_fix(serde_json::json!({
                            "kind": "rename_reference",
                            "module": m.path, "from": name, "to": other
                        }))
                        .with_fix(serde_json::json!({
                            "kind": "rebind",
                            "module": m.path, "name": name, "to_hash": current_hash
                        })),
                    );
                    new.references.insert(name.clone(), p.clone());
                } else {
                    // R3: content drift of the same definition lineage.
                    //
                    // A widened capability set is drift too, but it is the one
                    // kind worth its own name: an upstream definition that can
                    // suddenly reach the network is a different risk from one
                    // that fixed a typo, and the graph knows which is which.
                    let now_caps = caps_table(target).get(name).cloned();
                    if let (Some(before), Some(after)) = (&p.caps, &now_caps) {
                        let gained: Vec<String> = after
                            .iter()
                            .filter(|c| !before.contains(c))
                            .cloned()
                            .collect();
                        if !gained.is_empty() {
                            diags.push(
                                Diagnostic::new(
                                    W0405_CAP_WIDENED,
                                    if frozen { Severity::Error } else { Severity::Warning },
                                    &m.path,
                                    Some(*span),
                                    format!(
                                        "'{}' from \"{}\" gained {} — it was pinned as {}{}",
                                        name,
                                        target,
                                        gained
                                            .iter()
                                            .map(|c| format!("!{}", c))
                                            .collect::<Vec<_>>()
                                            .join(" "),
                                        if before.is_empty() {
                                            "pure".to_string()
                                        } else {
                                            before
                                                .iter()
                                                .map(|c| format!("!{}", c))
                                                .collect::<Vec<_>>()
                                                .join(" ")
                                        },
                                        if frozen { " (--frozen: refusing to accept)" } else { "" },
                                    ),
                                )
                                .with_data(serde_json::json!({
                                    "name": name, "target": target,
                                    "pinned_caps": before, "current_caps": after,
                                    "gained": gained
                                })),
                            );
                        }
                    }
                    let sev = if frozen { Severity::Error } else { Severity::Warning };
                    diags.push(
                        Diagnostic::new(
                            W0401_DRIFT,
                            sev,
                            &m.path,
                            Some(*span),
                            format!(
                                "'{}' from \"{}\" changed: {} -> {}{}",
                                name,
                                target,
                                short(&p.hash),
                                short(current_hash),
                                if frozen {
                                    " (--frozen: refusing to repin)"
                                } else {
                                    " (repinned)"
                                }
                            ),
                        )
                        .with_data(serde_json::json!({
                            "name": name, "target": target,
                            "old_hash": p.hash, "new_hash": current_hash
                        })),
                    );
                    if frozen {
                        new.references.insert(name.clone(), p.clone());
                    } else {
                        new.references.insert(
                            name.clone(),
                            RefPin {
                                target: format!("{}#{}", target, name),
                                hash: current_hash.clone(),
                                caps: caps_table(target).get(name).cloned(),
                            },
                        );
                    }
                }
            }
        }
    }

    let changed = new != old;
    let has_err = diags.iter().any(|d| d.severity == Severity::Error);
    ReconcileOutcome {
        diags,
        updated: if changed && !has_err && !frozen && !module_broken {
            Some(new)
        } else {
            None
        },
    }
}

pub fn save(ws: &Workspace, module_path: &str, bf: &BindFile) -> std::io::Result<()> {
    let p = sidecar_path(ws, module_path);
    let s = serde_json::to_string_pretty(bf).unwrap();
    fs::write(p, s + "\n")
}

fn short(h: &str) -> String {
    if h.len() > 19 {
        format!("{}…", &h[..19])
    } else {
        h.to_string()
    }
}
