//! Workspace loading and name resolution.
//!
//! Produces, for every name occurrence in every definition, exactly one
//! `Binding`. Canonicalization (canon.rs) and the sidecar layer (sidecar.rs)
//! both consume this — resolution logic lives only here. docs/SPEC.md §3–4.

use crate::ast::*;
use crate::diag::*;
use crate::parser::parse_module;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

pub type DefId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Local(u32),
    Def(DefId),
    /// Name came from an import that no longer resolves; def is poisoned.
    Broken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    Fn,
    Table,
    Extern,
}

#[derive(Debug)]
pub struct DefInfo {
    pub module_idx: usize,
    pub item_idx: usize,
    pub kind: DefKind,
    pub module_path: String,
    pub name: String,
    pub poisoned: bool,
}

#[derive(Debug)]
pub struct ModuleImports {
    /// local name -> (target module path, import span). Present even if broken.
    pub map: BTreeMap<String, (String, Span)>,
    /// imported names whose target module exists but lacks the name (or the
    /// target module itself is missing -> target_exists = false).
    pub broken: Vec<BrokenImport>,
}

#[derive(Debug)]
pub struct BrokenImport {
    pub name: String,
    pub target_module: String,
    pub target_exists: bool,
    pub span: Span,
}

pub struct Workspace {
    pub root: PathBuf,
    pub modules: Vec<Module>,
    pub defs: Vec<DefInfo>,
    pub def_index: HashMap<(String, String), DefId>,
    pub imports: Vec<ModuleImports>, // parallel to modules
    /// (def, occurrence span, name) -> binding
    pub bindings: HashMap<(DefId, u32, u32, String), Binding>,
    /// def -> outgoing referenced defs (deduped, order of first occurrence);
    /// includes funref creations and dyn-set members — the graph is total
    pub edges: Vec<Vec<DefId>>,
    /// def -> call sites (direct calls + dyn-set members): (span, name, target)
    pub call_sites: Vec<Vec<(Span, String, DefId)>>,
    /// def -> spans of `print` uses
    pub print_sites: Vec<Vec<Span>>,
    /// def -> table-dispatch sites: (span, table def)
    pub dispatch_sites: Vec<Vec<(Span, DefId)>>,
    /// def -> dyn sites: (first-target span, resolved declared set)
    pub dyn_sites: Vec<Vec<(Span, Vec<DefId>)>>,
    /// table def -> resolved (key, target) entries
    pub table_entries: HashMap<DefId, Vec<(String, DefId)>>,
    /// strongly-connected components of the reference graph, in dependency
    /// order (every component's non-member deps appear in earlier components)
    pub sccs: Vec<Vec<DefId>>,
    /// def -> content hash, recorded by `canon::hash_into`. At runtime a
    /// definition *is* its hash: function values compare and display by it,
    /// so nothing a program can observe depends on a name. Empty until the
    /// workspace is hashed; None for poisoned definitions.
    pub ids: Vec<Option<crate::canon::Hash>>,
}

fn norm_join(base_dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if !base_dir.is_empty() {
        for p in base_dir.split('/') {
            if !p.is_empty() {
                parts.push(p);
            }
        }
    }
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn collect_rg(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(rd) = fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name == "target" {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().map(|e| e == "rg").unwrap_or(false) {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

pub fn load_workspace(root: &Path) -> Result<Workspace, Vec<Diagnostic>> {
    load_workspace_overlay(root, &std::collections::HashMap::new())
}

/// Build a workspace from `(module path, source)` pairs held in memory.
/// The browser playground has no filesystem, so it enters here.
pub fn load_workspace_from_sources(
    sources: &[(String, String)],
) -> Result<Workspace, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let mut modules = Vec::new();
    for (path, src) in sources {
        match parse_module(src, path).map(|mut m| {
            crate::lift::lift_module(&mut m);
            m
        }) {
            Ok(m) => modules.push(m),
            Err(d) => diags.push(d),
        }
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    build_workspace(Path::new("."), modules)
}

/// As load_workspace, but any module whose relative path appears in
/// `overlay` is parsed from the given text instead of disk — the LSP's
/// unsaved-buffer view.
pub fn load_workspace_overlay(
    root: &Path,
    overlay: &std::collections::HashMap<String, String>,
) -> Result<Workspace, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let mut modules = Vec::new();
    for file in collect_rg(root) {
        let rel = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let read = match overlay.get(&rel) {
            Some(text) => Ok(text.clone()),
            None => fs::read_to_string(&file),
        };
        match read {
            Ok(src) => match parse_module(&src, &rel).map(|mut m| {
                crate::lift::lift_module(&mut m);
                m
            }) {
                Ok(m) => modules.push(m),
                Err(d) => diags.push(d),
            },
            Err(e) => diags.push(Diagnostic::new(
                E0101_PARSE,
                Severity::Error,
                &rel,
                None,
                format!("cannot read file: {}", e),
            )),
        }
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    load_std_into(&mut modules, &mut diags);
    if !diags.is_empty() {
        return Err(diags);
    }
    build_workspace(root, modules)
}

/// Where the standard library lives on disk.
///
/// `RIGID_STD` wins, then a `std/` beside the executable or one or two
/// directories up from it — which covers both an installed binary and a
/// `target/release/rigid` run from a checkout.
pub fn std_root() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("RIGID_STD") {
        let p = std::path::PathBuf::from(p);
        if p.is_dir() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    for _ in 0..4 {
        let cand = dir.join("std");
        if cand.join("list.rg").is_file() {
            return Some(cand);
        }
        dir = dir.parent()?.to_path_buf();
    }
    None
}

/// Pull in the `std/` modules a workspace actually imports, transitively.
///
/// Only what is reached is loaded: a workspace that imports nothing from the
/// standard library pays nothing for it, in check time or in `hash --all`.
fn load_std_into(modules: &mut Vec<Module>, diags: &mut Vec<Diagnostic>) {
    let Some(root) = std_root() else { return };
    loop {
        let have: std::collections::HashSet<String> =
            modules.iter().map(|m| m.path.clone()).collect();
        let mut want: Vec<String> = Vec::new();
        for m in modules.iter() {
            for imp in &m.imports {
                if imp.from.starts_with("std/")
                    && !have.contains(&imp.from)
                    && !want.contains(&imp.from)
                {
                    want.push(imp.from.clone());
                }
            }
        }
        if want.is_empty() {
            return;
        }
        for path in want {
            let file = root.join(path.trim_start_matches("std/"));
            match fs::read_to_string(&file) {
                Ok(src) => match parse_module(&src, &path).map(|mut m| {
                    crate::lift::lift_module(&mut m);
                    m
                }) {
                    Ok(m) => modules.push(m),
                    Err(d) => diags.push(d),
                },
                Err(e) => diags.push(Diagnostic::new(
                    E0101_PARSE,
                    Severity::Error,
                    &path,
                    None,
                    format!("cannot read standard library module: {}", e),
                )),
            }
        }
        if !diags.is_empty() {
            return;
        }
    }
}

/// Build a workspace from modules already parsed, whatever their origin.
/// The disk loaders above and the browser playground both end up here, so
/// there is exactly one definition-collection path.
pub fn build_workspace(root: &Path, modules: Vec<Module>) -> Result<Workspace, Vec<Diagnostic>> {
    let mut defs: Vec<DefInfo> = Vec::new();
    let mut def_index: HashMap<(String, String), DefId> = HashMap::new();
    for (mi, m) in modules.iter().enumerate() {
        for (fi, f) in m.fns.iter().enumerate() {
            let key = (m.path.clone(), f.name.name.clone());
            let id = defs.len();
            if !def_index.contains_key(&key) {
                def_index.insert(key, id);
            }
            defs.push(DefInfo {
                module_idx: mi,
                item_idx: fi,
                kind: DefKind::Fn,
                module_path: m.path.clone(),
                name: f.name.name.clone(),
                poisoned: false,
            });
        }
        for (ti, t) in m.tables.iter().enumerate() {
            let key = (m.path.clone(), t.name.name.clone());
            let id = defs.len();
            if !def_index.contains_key(&key) {
                def_index.insert(key, id);
            }
            defs.push(DefInfo {
                module_idx: mi,
                item_idx: ti,
                kind: DefKind::Table,
                module_path: m.path.clone(),
                name: t.name.name.clone(),
                poisoned: false,
            });
        }
        for (xi, x) in m.externs.iter().enumerate() {
            let key = (m.path.clone(), x.name.name.clone());
            let id = defs.len();
            if !def_index.contains_key(&key) {
                def_index.insert(key, id);
            }
            defs.push(DefInfo {
                module_idx: mi,
                item_idx: xi,
                kind: DefKind::Extern,
                module_path: m.path.clone(),
                name: x.name.name.clone(),
                poisoned: false,
            });
        }
    }

    Ok(Workspace {
        root: root.to_path_buf(),
        modules,
        defs,
        def_index,
        imports: Vec::new(),
        bindings: HashMap::new(),
        edges: Vec::new(),
        call_sites: Vec::new(),
        print_sites: Vec::new(),
        dispatch_sites: Vec::new(),
        dyn_sites: Vec::new(),
        table_entries: HashMap::new(),
        sccs: Vec::new(),
        ids: Vec::new(),
    })
}

fn bind_name(
    name: &str,
    span: Span,
    locals: &Vec<String>,
    module_path: &str,
    imp: &ModuleImports,
    broken_names: &Vec<String>,
    ws_def_index: &HashMap<(String, String), DefId>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Binding> {
    // innermost local wins
    if let Some(pos) = locals.iter().rposition(|l| l == name) {
        return Some(Binding::Local(pos as u32));
    }
    if let Some(&d) = ws_def_index.get(&(module_path.to_string(), name.to_string())) {
        return Some(Binding::Def(d));
    }
    if let Some((target, _)) = imp.map.get(name) {
        if broken_names.iter().any(|b| b == name) {
            return Some(Binding::Broken);
        }
        if let Some(&d) = ws_def_index.get(&(target.clone(), name.to_string())) {
            return Some(Binding::Def(d));
        }
        return Some(Binding::Broken);
    }
    diags.push(Diagnostic::new(
        E0201_UNRESOLVED,
        Severity::Error,
        module_path,
        Some(span),
        format!("unresolved name '{}'", name),
    ));
    None
}

/// Inject a synthetic definition into an already-loaded workspace (REPL).
pub fn inject_fn(ws: &mut Workspace, module_path: &str, f: crate::ast::FnDef) -> Option<DefId> {
    let mi = ws.modules.iter().position(|m| m.path == module_path)?;
    let name = f.name.name.clone();
    let key = (module_path.to_string(), name.clone());
    if ws.def_index.contains_key(&key) {
        return None;
    }
    let item_idx = ws.modules[mi].fns.len();
    ws.modules[mi].fns.push(f);
    let id = ws.defs.len();
    ws.def_index.insert(key, id);
    ws.defs.push(DefInfo {
        module_idx: mi,
        item_idx,
        kind: DefKind::Fn,
        module_path: module_path.to_string(),
        name,
        poisoned: false,
    });
    Some(id)
}

pub fn resolve(ws: &mut Workspace) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    // duplicate definitions within a module
    {
        let mut seen: HashMap<(String, String), Span> = HashMap::new();
        for m in &ws.modules {
            let names = m
                .fns
                .iter()
                .map(|f| f.name.clone())
                .chain(m.tables.iter().map(|t| t.name.clone()));
            for nm in names {
                let key = (m.path.clone(), nm.name.clone());
                if seen.contains_key(&key) {
                    diags.push(Diagnostic::new(
                        E0203_AMBIGUOUS,
                        Severity::Error,
                        &m.path,
                        Some(nm.span),
                        format!("duplicate definition of '{}'", nm.name),
                    ));
                } else {
                    seen.insert(key, nm.span);
                }
            }
        }
    }

    // imports: resolve target modules, detect collisions with local defs
    let mut imports: Vec<ModuleImports> = Vec::new();
    for m in &ws.modules {
        let dir = match m.path.rfind('/') {
            Some(i) => &m.path[..i],
            None => "",
        };
        let mut map: BTreeMap<String, (String, Span)> = BTreeMap::new();
        let mut broken: Vec<BrokenImport> = Vec::new();
        for imp in &m.imports {
            let target = norm_join(dir, &imp.from);
            let target_exists = ws.modules.iter().any(|mm| mm.path == target);
            for n in &imp.names {
                if m.fns.iter().any(|f| f.name.name == n.name)
                    || m.tables.iter().any(|t| t.name.name == n.name)
                {
                    diags.push(
                        Diagnostic::new(
                            E0203_AMBIGUOUS,
                            Severity::Error,
                            &m.path,
                            Some(n.span),
                            format!(
                                "'{}' is both imported from \"{}\" and defined in this module; \
                                 the binding is ambiguous",
                                n.name, target
                            ),
                        )
                        .with_data(serde_json::json!({
                            "name": n.name, "import_target": target
                        })),
                    );
                    continue;
                }
                map.insert(n.name.clone(), (target.clone(), n.span));
                let resolves = target_exists
                    && ws.def_index.contains_key(&(target.clone(), n.name.clone()));
                if !resolves {
                    broken.push(BrokenImport {
                        name: n.name.clone(),
                        target_module: target.clone(),
                        target_exists,
                        span: n.span,
                    });
                }
            }
        }
        imports.push(ModuleImports { map, broken });
    }

    // resolve every occurrence in every def
    let mut bindings: HashMap<(DefId, u32, u32, String), Binding> = HashMap::new();
    let mut edges: Vec<Vec<DefId>> = vec![Vec::new(); ws.defs.len()];
    let mut table_entries: HashMap<DefId, Vec<(String, DefId)>> = HashMap::new();
    let mut call_sites: Vec<Vec<(Span, String, DefId)>> = vec![Vec::new(); ws.defs.len()];
    let mut print_sites: Vec<Vec<Span>> = vec![Vec::new(); ws.defs.len()];
    let mut dispatch_sites: Vec<Vec<(Span, DefId)>> = vec![Vec::new(); ws.defs.len()];
    let mut dyn_sites: Vec<Vec<(Span, Vec<DefId>)>> = vec![Vec::new(); ws.defs.len()];
    let mut poisoned: Vec<bool> = vec![false; ws.defs.len()];

    // ---- Phase A: table entries (no locals; names resolve via module then
    // imports). Targets must be functions. Duplicate keys are errors. ----
    for id in 0..ws.defs.len() {
        if ws.defs[id].kind != DefKind::Table {
            continue;
        }
        let mi = ws.defs[id].module_idx;
        let module_path = ws.modules[mi].path.clone();
        let t = ws.modules[mi].tables[ws.defs[id].item_idx].clone();
        let imp = &imports[mi];
        let broken_names: Vec<String> = imp.broken.iter().map(|b| b.name.clone()).collect();
        let mut seen_keys: HashMap<String, Span> = HashMap::new();
        let mut resolved: Vec<(String, DefId)> = Vec::new();
        let mut p = false;
        for e in &t.entries {
            if let Some(_) = seen_keys.get(&e.key) {
                diags.push(Diagnostic::new(
                    E0203_AMBIGUOUS,
                    Severity::Error,
                    &module_path,
                    Some(e.key_span),
                    format!("duplicate key {:?} in table '{}'", e.key, t.name.name),
                ));
                p = true;
                continue;
            }
            seen_keys.insert(e.key.clone(), e.key_span);
            let empty_locals: Vec<String> = Vec::new();
            let b = bind_name(
                &e.target.name,
                e.target.span,
                &empty_locals,
                &module_path,
                imp,
                &broken_names,
                &ws.def_index,
                &mut diags,
            );
            match b {
                Some(Binding::Def(d)) => {
                    if ws.defs[d].kind != DefKind::Fn {
                        diags.push(Diagnostic::new(
                            E0208_TABLE,
                            Severity::Error,
                            &module_path,
                            Some(e.target.span),
                            format!(
                                "table targets must be functions; '{}' is a table",
                                e.target.name
                            ),
                        ));
                        p = true;
                    } else {
                        if !edges[id].contains(&d) {
                            edges[id].push(d);
                        }
                        resolved.push((e.key.clone(), d));
                    }
                    bindings.insert(
                        (id, e.target.span.line, e.target.span.col, e.target.name.clone()),
                        Binding::Def(d),
                    );
                }
                Some(Binding::Broken) => {
                    bindings.insert(
                        (id, e.target.span.line, e.target.span.col, e.target.name.clone()),
                        Binding::Broken,
                    );
                    p = true;
                }
                Some(Binding::Local(_)) | None => {
                    p = true;
                }
            }
        }
        poisoned[id] = p;
        table_entries.insert(id, resolved);
    }

    let def_kinds: Vec<DefKind> = ws.defs.iter().map(|d| d.kind).collect();
    let arities: Vec<Option<u32>> = ws
        .defs
        .iter()
        .map(|d| match d.kind {
            DefKind::Fn => {
                let f = &ws.modules[d.module_idx].fns[d.item_idx];
                // A lifted lambda's captures are supplied by the closure, not
                // by the caller, so they do not count toward its arity here.
                Some((f.params.len() - f.captured) as u32)
            }
            DefKind::Extern => {
                Some(ws.modules[d.module_idx].externs[d.item_idx].params.len() as u32)
            }
            DefKind::Table => None,
        })
        .collect();
    for id in 0..ws.defs.len() {
        if ws.defs[id].kind != DefKind::Fn {
            continue;
        }
        let (mi, fi) = (ws.defs[id].module_idx, ws.defs[id].item_idx);
        let module_path = ws.modules[mi].path.clone();
        let f = ws.modules[mi].fns[fi].clone();
        let imp = &imports[mi];
        let broken_names: Vec<String> = imp.broken.iter().map(|b| b.name.clone()).collect();

        let mut locals: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        let mut out_edges: Vec<DefId> = Vec::new();

        // closure-free walker (v0: locals never leave scope; lets accumulate)
        #[allow(clippy::too_many_arguments)]
        fn walk_expr(
            e: &Expr,
            id: DefId,
            module_path: &str,
            locals: &Vec<String>,
            imp: &ModuleImports,
            broken_names: &Vec<String>,
            ws_def_index: &HashMap<(String, String), DefId>,
            bindings: &mut HashMap<(DefId, u32, u32, String), Binding>,
            out_edges: &mut Vec<DefId>,
            calls: &mut Vec<(Span, String, DefId)>,
            prints: &mut Vec<Span>,
            dispatches: &mut Vec<(Span, DefId)>,
            dynsites: &mut Vec<(Span, Vec<DefId>)>,
            def_kinds: &Vec<DefKind>,
            arities: &Vec<Option<u32>>,
            tentries: &HashMap<DefId, Vec<(String, DefId)>>,
            poisoned: &mut bool,
            diags: &mut Vec<Diagnostic>,
        ) {

            match e {
                Expr::Int(_) | Expr::Float(_) | Expr::Str(_) => {}
                Expr::Var(idn) => {
                    if let Some(b) = bind_name(&idn.name, idn.span, locals, module_path, imp, broken_names, ws_def_index, diags) {
                        if let Binding::Def(_) = b {
                            diags.push(Diagnostic::new(
                                E0206_DEF_AS_VALUE,
                                Severity::Error,
                                module_path,
                                Some(idn.span),
                                format!(
                                    "'{}' is a definition, not a value; take an explicit reference with `&{}`",
                                    idn.name, idn.name
                                ),
                            ));
                            *poisoned = true;
                        }
                        if b == Binding::Broken {
                            *poisoned = true;
                        }
                        bindings.insert(
                            (id, idn.span.line, idn.span.col, idn.name.clone()),
                            b,
                        );
                    } else {
                        *poisoned = true;
                    }
                }
                Expr::Call { callee, args } => {
                    if let Some(b) = bind_name(&callee.name, callee.span, locals, module_path, imp, broken_names, ws_def_index, diags) {
                        match b {
                            Binding::Def(d) => {
                                if def_kinds[d] == DefKind::Table {
                                    diags.push(Diagnostic::new(
                                        E0208_TABLE,
                                        Severity::Error,
                                        module_path,
                                        Some(callee.span),
                                        format!(
                                            "'{}' is a table; dispatch it with {}[key](...)",
                                            callee.name, callee.name
                                        ),
                                    ));
                                    *poisoned = true;
                                }
                                if let Some(a) = arities[d] {
                                    if args.len() as u32 != a {
                                        diags.push(Diagnostic::new(
                                            E0209_ARITY,
                                            Severity::Error,
                                            module_path,
                                            Some(callee.span),
                                            format!(
                                                "'{}' has arity {}, called with {} argument(s)",
                                                callee.name, a, args.len()
                                            ),
                                        ));
                                    }
                                }
                                if !out_edges.contains(&d) {
                                    out_edges.push(d);
                                }
                                calls.push((callee.span, callee.name.clone(), d));
                            }
                            Binding::Local(_) => {
                                diags.push(Diagnostic::new(
                                    E0204_DYN_REQUIRED,
                                    Severity::Error,
                                    module_path,
                                    Some(callee.span),
                                    format!(
                                        "cannot call the value '{}' directly; indirect calls are quarantined — write `dyn {{ possible, targets }}({}, ...)` (docs/SPEC.md §8)",
                                        callee.name, callee.name
                                    ),
                                ));
                                *poisoned = true;
                            }
                            Binding::Broken => {
                                *poisoned = true;
                            }
                        }
                        bindings.insert(
                            (id, callee.span.line, callee.span.col, callee.name.clone()),
                            b,
                        );
                    } else {
                        // an unresolved callee is still an occurrence: record it
                        // Broken so rename recovery and R7 can see it
                        bindings.insert(
                            (id, callee.span.line, callee.span.col, callee.name.clone()),
                            Binding::Broken,
                        );
                        *poisoned = true;
                    }
                    for a in args {
                        walk_expr(
                            a, id, module_path, locals, imp, broken_names, ws_def_index,
                            bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Expr::Lambda { lifted, captures, .. } => {
                    // The body lives in the lifted definition and is walked
                    // there. What this site contributes is a static edge to
                    // that definition — which is exactly why an anonymous
                    // function does not blur the graph — plus a local read
                    // per captured name.
                    if let Some(l) = lifted {
                        if let Some(Binding::Def(d)) = bind_name(
                            &l.name, l.span, locals, module_path, imp, broken_names,
                            ws_def_index, diags,
                        ) {
                            if !out_edges.contains(&d) {
                                out_edges.push(d);
                            }
                            bindings.insert(
                                (id, l.span.line, l.span.col, l.name.clone()),
                                Binding::Def(d),
                            );
                        }
                    }
                    for c in captures {
                        if let Some(b) = bind_name(
                            &c.name, c.span, locals, module_path, imp, broken_names,
                            ws_def_index, diags,
                        ) {
                            bindings.insert(
                                (id, c.span.line, c.span.col, c.name.clone()),
                                b,
                            );
                        }
                    }
                }
                Expr::FunRef(idn) => {
                    if let Some(b) = bind_name(&idn.name, idn.span, locals, module_path, imp, broken_names, ws_def_index, diags) {
                        match b {
                            Binding::Def(d) => {
                                if def_kinds[d] == DefKind::Table {
                                    diags.push(Diagnostic::new(
                                        E0205_BAD_FUNREF,
                                        Severity::Error,
                                        module_path,
                                        Some(idn.span),
                                        format!("`&` takes a function; '{}' is a table", idn.name),
                                    ));
                                    *poisoned = true;
                                }
                                if !out_edges.contains(&d) {
                                    out_edges.push(d);
                                }
                            }
                            Binding::Local(_) => {
                                diags.push(Diagnostic::new(
                                    E0205_BAD_FUNREF,
                                    Severity::Error,
                                    module_path,
                                    Some(idn.span),
                                    format!("`&` takes a definition; '{}' is a local", idn.name),
                                ));
                                *poisoned = true;
                            }
                            Binding::Broken => {
                                *poisoned = true;
                            }
                        }
                        bindings.insert(
                            (id, idn.span.line, idn.span.col, idn.name.clone()),
                            b,
                        );
                    } else {
                        *poisoned = true;
                    }
                }
                Expr::DynCall {
                    targets,
                    callee,
                    args,
                } => {
                    let argc = args.len() as u32;
                    let mut resolved_set: Vec<DefId> = Vec::new();
                    for t in targets {
                        if let Some(b) = bind_name(&t.name, t.span, locals, module_path, imp, broken_names, ws_def_index, diags) {
                            match b {
                                Binding::Def(d) => {
                                    if let Some(a) = arities[d] {
                                        if a != argc {
                                            diags.push(Diagnostic::new(
                                                E0209_ARITY,
                                                Severity::Error,
                                                module_path,
                                                Some(t.span),
                                                format!(
                                                    "dyn target '{}' has arity {}, but this site passes {} argument(s)",
                                                    t.name, a, argc
                                                ),
                                            ));
                                        }
                                    }
                                    if !out_edges.contains(&d) {
                                        out_edges.push(d);
                                    }
                                    calls.push((t.span, t.name.clone(), d));
                                    resolved_set.push(d);
                                }
                                Binding::Local(_) => {
                                    diags.push(Diagnostic::new(
                                        E0205_BAD_FUNREF,
                                        Severity::Error,
                                        module_path,
                                        Some(t.span),
                                        format!(
                                            "dyn targets must be definitions; '{}' is a local",
                                            t.name
                                        ),
                                    ));
                                    *poisoned = true;
                                }
                                Binding::Broken => {
                                    *poisoned = true;
                                }
                            }
                            bindings.insert((id, t.span.line, t.span.col, t.name.clone()), b);
                        } else {
                            *poisoned = true;
                        }
                    }
                    if let Some(t0) = targets.first() {
                        dynsites.push((t0.span, resolved_set));
                    }
                    walk_expr(
                        callee, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                    for a in args {
                        walk_expr(
                            a, id, module_path, locals, imp, broken_names, ws_def_index,
                            bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Expr::Print { span, arg } => {
                    prints.push(*span);
                    walk_expr(
                        arg, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                }
                Expr::Input { span } => {
                    prints.push(*span);
                }
                Expr::StrConv(x) | Expr::IntConv(x) | Expr::FloatConv(x) | Expr::Chars(x) => {
                    walk_expr(
                        x, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                }
                Expr::Substr(a, b, c) => {
                    for x in [a, b, c] {
                        walk_expr(
                            x, id, module_path, locals, imp, broken_names, ws_def_index,
                            bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Expr::Join(a, b) => {
                    for x in [a, b] {
                        walk_expr(
                            x, id, module_path, locals, imp, broken_names, ws_def_index,
                            bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Expr::ReadFile { span, path } => {
                    prints.push(*span);
                    walk_expr(
                        path, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                }
                Expr::WriteFile { span, path, contents } => {
                    prints.push(*span);
                    for x in [path, contents] {
                        walk_expr(
                            x, id, module_path, locals, imp, broken_names, ws_def_index,
                            bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Expr::Fail(x) | Expr::Try(x) => {
                    walk_expr(
                        x, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                }
                Expr::ListLit(elems) => {
                    for x in elems {
                        walk_expr(
                        x, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Expr::TimeCall { args, span, .. } => {
                    // past code's effects are unknown: require !io at the caller
                    prints.push(*span);
                    for x in args {
                        walk_expr(
                        x, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Expr::RecordLit(fields) => {
                    for (_, x) in fields {
                        walk_expr(
                        x, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Expr::Get { base: x, .. } => {
                    walk_expr(
                        x, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                }
                Expr::IndexE { base, key } => {
                    for x in [base, key] {
                        walk_expr(
                        x, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Expr::Len(x) => {
                    walk_expr(
                        x, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                }
                Expr::Push(a, b) => {
                    for x in [a, b] {
                        walk_expr(
                        x, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Expr::TableDispatch { table, key, args } => {
                    if let Some(b) = bind_name(&table.name, table.span, locals, module_path, imp, broken_names, ws_def_index, diags) {
                        match b {
                            Binding::Def(d) if def_kinds[d] == DefKind::Table => {
                                if let Some(a) = args {
                                    let argc = a.len() as u32;
                                    if let Some(es) = tentries.get(&d) {
                                        for (k, tgt) in es {
                                            if let Some(ta) = arities[*tgt] {
                                                if ta != argc {
                                                    diags.push(Diagnostic::new(
                                                        E0209_ARITY,
                                                        Severity::Error,
                                                        module_path,
                                                        Some(table.span),
                                                        format!(
                                                            "table entry {:?} -> '{}' has arity {}, but this dispatch passes {} argument(s)",
                                                            k, ws_def_index.iter().find(|(_, id)| **id == *tgt).map(|((_, n), _)| n.clone()).unwrap_or_default(), ta, argc
                                                        ),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                                if !out_edges.contains(&d) {
                                    out_edges.push(d);
                                }
                                dispatches.push((table.span, d));
                            }
                            Binding::Def(_) => {
                                diags.push(Diagnostic::new(
                                    E0208_TABLE,
                                    Severity::Error,
                                    module_path,
                                    Some(table.span),
                                    format!("'{}' is a function, not a table; call it directly", table.name),
                                ));
                                *poisoned = true;
                            }
                            Binding::Local(_) => {
                                // value indexing on a local (list/string) —
                                // legal for bare lookup; calling the result
                                // directly is quarantined.
                                if args.is_some() {
                                    diags.push(Diagnostic::new(
                                        E0204_DYN_REQUIRED,
                                        Severity::Error,
                                        module_path,
                                        Some(table.span),
                                        format!(
                                            "cannot call an indexed value directly; bind it and use `dyn {{ ... }}({}[...], ...)`",
                                            table.name
                                        ),
                                    ));
                                    *poisoned = true;
                                }
                            }
                            Binding::Broken => {
                                *poisoned = true;
                            }
                        }
                        bindings.insert((id, table.span.line, table.span.col, table.name.clone()), b);
                    } else {
                        *poisoned = true;
                    }
                    walk_expr(
                        key, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                    if let Some(args) = args {
                        for a in args {
                            walk_expr(
                                a, id, module_path, locals, imp, broken_names, ws_def_index,
                                bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                            );
                        }
                    }
                }
                Expr::Binop { l, r, .. } => {
                    walk_expr(
                        l, id, module_path, locals, imp, broken_names, ws_def_index, bindings,
                        out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                    walk_expr(
                        r, id, module_path, locals, imp, broken_names, ws_def_index, bindings,
                        out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                }
                Expr::If { c, t, e } => {
                    for x in [c, t, e] {
                        walk_expr(
                            x, id, module_path, locals, imp, broken_names, ws_def_index,
                            bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Expr::Neg(x) => {
                    walk_expr(
                        x, id, module_path, locals, imp, broken_names, ws_def_index, bindings,
                        out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                }
            }
        }

        let mut p = false;
        let mut calls: Vec<(Span, String, DefId)> = Vec::new();
        let mut prints: Vec<Span> = Vec::new();
        let mut disp: Vec<(Span, DefId)> = Vec::new();
        let mut dsites: Vec<(Span, Vec<DefId>)> = Vec::new();
        #[allow(clippy::too_many_arguments)]
        fn walk_stmt(
            s: &Stmt,
            id: DefId,
            module_path: &str,
            locals: &mut Vec<String>,
            imp: &ModuleImports,
            broken_names: &Vec<String>,
            ws_def_index: &HashMap<(String, String), DefId>,
            bindings: &mut HashMap<(DefId, u32, u32, String), Binding>,
            out_edges: &mut Vec<DefId>,
            calls: &mut Vec<(Span, String, DefId)>,
            prints: &mut Vec<Span>,
            dispatches: &mut Vec<(Span, DefId)>,
            dynsites: &mut Vec<(Span, Vec<DefId>)>,
            def_kinds: &Vec<DefKind>,
            arities: &Vec<Option<u32>>,
            tentries: &HashMap<DefId, Vec<(String, DefId)>>,
            poisoned: &mut bool,
            diags: &mut Vec<Diagnostic>,
        ) {
            match s {
                Stmt::Let { name, value } => {
                    walk_expr(
                        value, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                    locals.push(name.name.clone()); // bound after its initializer
                    let slot = (locals.len() - 1) as u32;
                    bindings.insert(
                        (id, name.span.line, name.span.col, name.name.clone()),
                        Binding::Local(slot),
                    );
                }
                Stmt::Assign { name, value } => {
                    walk_expr(
                        value, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                    if let Some(pos) = locals.iter().rposition(|l| l == &name.name) {
                        bindings.insert(
                            (id, name.span.line, name.span.col, name.name.clone()),
                            Binding::Local(pos as u32),
                        );
                    } else if ws_def_index
                        .get(&(module_path.to_string(), name.name.clone()))
                        .is_some()
                        || imp.map.contains_key(&name.name)
                    {
                        diags.push(Diagnostic::new(
                            E0210_ASSIGN,
                            Severity::Error,
                            module_path,
                            Some(name.span),
                            format!("cannot assign to '{}': definitions are immutable", name.name),
                        ));
                        *poisoned = true;
                    } else {
                        diags.push(Diagnostic::new(
                            E0201_UNRESOLVED,
                            Severity::Error,
                            module_path,
                            Some(name.span),
                            format!("cannot assign to unknown local '{}'; declare it with `let` first", name.name),
                        ));
                        *poisoned = true;
                    }
                }
                Stmt::Return(e) | Stmt::Expr(e) => {
                    walk_expr(
                        e, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                }
                Stmt::IfStmt { cond, then, els } => {
                    walk_expr(
                        cond, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                    for bs in then.iter().chain(els.iter()) {
                        walk_stmt(
                            bs, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Stmt::For { var, iter, body } => {
                    walk_expr(
                        iter, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                    // The loop variable is an ordinary local, bound before
                    // the body so the body can see it — and reused across
                    // iterations, the same flat-scope rule `while` follows.
                    locals.push(var.name.clone());
                    let slot = (locals.len() - 1) as u32;
                    bindings.insert(
                        (id, var.span.line, var.span.col, var.name.clone()),
                        Binding::Local(slot),
                    );
                    for bs in body {
                        walk_stmt(
                            bs, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
                Stmt::While { cond, body } => {
                    walk_expr(
                        cond, id, module_path, locals, imp, broken_names, ws_def_index,
                        bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                    );
                    for bs in body {
                        walk_stmt(
                            bs, id, module_path, locals, imp, broken_names, ws_def_index,
                            bindings, out_edges, calls, prints, dispatches, dynsites, def_kinds, arities, tentries, poisoned, diags,
                        );
                    }
                }
            }
        }

        for s in &f.body {
            walk_stmt(
                s,
                id,
                &module_path,
                &mut locals,
                imp,
                &broken_names,
                &ws.def_index,
                &mut bindings,
                &mut out_edges,
                &mut calls,
                &mut prints,
                &mut disp,
                &mut dsites,
                &def_kinds,
                &arities,
                &table_entries,
                &mut p,
                &mut diags,
            );
        }
        poisoned[id] = p;
        edges[id] = out_edges;
        call_sites[id] = calls;
        print_sites[id] = prints;
        dispatch_sites[id] = disp;
        dyn_sites[id] = dsites;
    }

    for (i, p) in poisoned.iter().enumerate() {
        ws.defs[i].poisoned = *p;
    }

    // ---- effect check: io is declared, and the exact graph makes the
    // check trivial. A dyn site's effect bound is the union of its declared
    // set — the quarantine bounds effects as well as call targets. ----
    ws.table_entries = table_entries;

    // A definition's *declared* capability set. A table's is the union of its
    // entries', because dispatching through it can reach any of them — the
    // quarantine bounds effects exactly as it bounds call targets.
    let caps_of = |d: DefId| -> crate::ast::Caps {
        let info = &ws.defs[d];
        match info.kind {
            DefKind::Fn => ws.modules[info.module_idx].fns[info.item_idx].caps,
            DefKind::Extern => ws.modules[info.module_idx].externs[info.item_idx].caps,
            DefKind::Table => ws
                .table_entries
                .get(&d)
                .map(|es| {
                    es.iter().fold(0, |acc, (_, t)| {
                        let ti = &ws.defs[*t];
                        acc | match ti.kind {
                            DefKind::Extern => ws.modules[ti.module_idx].externs[ti.item_idx].caps,
                            DefKind::Table => 0,
                            _ => ws.modules[ti.module_idx].fns[ti.item_idx].caps,
                        }
                    })
                })
                .unwrap_or(0),
        }
    };
    for u in 0..ws.defs.len() {
        if ws.defs[u].kind != DefKind::Fn {
            continue;
        }
        let declared = caps_of(u);
        let name_u = ws.defs[u].name.clone();
        let mpath = ws.defs[u].module_path.clone();

        // The builtins rigid supplies itself are `!io` and stay `!io`; the
        // finer capabilities describe the host boundary reached via `extern`.
        if declared & crate::ast::CAP_IO == 0 {
            for sp in &print_sites[u] {
                diags.push(Diagnostic::new(
                    E0207_EFFECT,
                    Severity::Error,
                    &mpath,
                    Some(*sp),
                    format!("io builtin requires `!io`; declare `fn {}(...) !io`", name_u),
                ));
            }
        }
        for (sp, td) in &dispatch_sites[u] {
            let missing = caps_of(*td) & !declared;
            if missing != 0 {
                diags.push(Diagnostic::new(
                    E0207_EFFECT,
                    Severity::Error,
                    &mpath,
                    Some(*sp),
                    format!(
                        "table '{}' reaches {}; '{}' must declare {}",
                        ws.defs[*td].name,
                        crate::ast::caps_str(caps_of(*td)),
                        name_u,
                        crate::ast::caps_str(missing)
                    ),
                ));
            }
        }
        for (sp, cname, t) in &call_sites[u] {
            let missing = caps_of(*t) & !declared;
            if missing != 0 {
                diags.push(Diagnostic::new(
                    E0207_EFFECT,
                    Severity::Error,
                    &mpath,
                    Some(*sp),
                    format!(
                        "'{}' is {}; '{}' must declare {} to reach it (effects travel the exact graph)",
                        cname,
                        crate::ast::caps_str(caps_of(*t)),
                        name_u,
                        crate::ast::caps_str(missing)
                    ),
                ));
            }
        }
    }

    // Strongly-connected components (Kosaraju), then condensation topo order.
    // Mutual recursion is legal as of v1: each SCC hashes as a unit (canon.rs).
    let n = ws.defs.len();
    let mut order: Vec<DefId> = Vec::with_capacity(n);
    {
        let mut seen = vec![false; n];
        for s in 0..n {
            if seen[s] {
                continue;
            }
            let mut stack: Vec<(DefId, usize)> = vec![(s, 0)];
            seen[s] = true;
            while let Some(&mut (u, ref mut ei)) = stack.last_mut() {
                if *ei < edges[u].len() {
                    let v = edges[u][*ei];
                    *ei += 1;
                    if !seen[v] {
                        seen[v] = true;
                        stack.push((v, 0));
                    }
                } else {
                    order.push(u);
                    stack.pop();
                }
            }
        }
    }
    let mut redges: Vec<Vec<DefId>> = vec![Vec::new(); n];
    for (u, es) in edges.iter().enumerate() {
        for &v in es {
            redges[v].push(u);
        }
    }
    let mut comp = vec![usize::MAX; n];
    let mut comps: Vec<Vec<DefId>> = Vec::new();
    for &s in order.iter().rev() {
        if comp[s] != usize::MAX {
            continue;
        }
        let cid = comps.len();
        let mut members = Vec::new();
        let mut stack = vec![s];
        comp[s] = cid;
        while let Some(u) = stack.pop() {
            members.push(u);
            for &v in &redges[u] {
                if comp[v] == usize::MAX {
                    comp[v] = cid;
                    stack.push(v);
                }
            }
        }
        comps.push(members);
    }
    // Kahn over the condensation so dependencies hash first
    let c = comps.len();
    let mut indeg = vec![0usize; c];
    let mut cedges: Vec<Vec<usize>> = vec![Vec::new(); c];
    for (u, es) in edges.iter().enumerate() {
        for &v in es {
            let (cu, cv) = (comp[u], comp[v]);
            if cu != cv && !cedges[cu].contains(&cv) {
                cedges[cu].push(cv);
                indeg[cv] += 1;
            }
        }
    }
    // note reversed sense: an edge u->v means u DEPENDS ON v, so v hashes first
    let mut ready: Vec<usize> = (0..c).filter(|&i| cedges[i].is_empty()).collect();
    let mut placed = vec![false; c];
    let mut sccs: Vec<Vec<DefId>> = Vec::new();
    // repeatedly take components with no unplaced dependencies
    while sccs.len() < c {
        ready.sort();
        let mut progressed = false;
        for &cid in ready.iter() {
            if !placed[cid] {
                placed[cid] = true;
                sccs.push(comps[cid].clone());
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
        ready = (0..c)
            .filter(|&i| !placed[i] && cedges[i].iter().all(|&d| placed[d]))
            .collect();
    }

    ws.imports = imports;
    ws.bindings = bindings;
    ws.edges = edges;
    // E0211: memo requires purity over the whole cone
    {
        let io_flag: Vec<bool> = (0..ws.defs.len())
            .map(|d| match ws.defs[d].kind {
                DefKind::Fn => ws.modules[ws.defs[d].module_idx].fns[ws.defs[d].item_idx].caps != 0,
                DefKind::Extern => {
                    ws.modules[ws.defs[d].module_idx].externs[ws.defs[d].item_idx].caps != 0
                }
                DefKind::Table => false,
            })
            .collect();
        for d in 0..ws.defs.len() {
            if ws.defs[d].kind != DefKind::Fn {
                continue;
            }
            let f = &ws.modules[ws.defs[d].module_idx].fns[ws.defs[d].item_idx];
            if !f.memo {
                continue;
            }
            let mut seen = vec![false; ws.defs.len()];
            let mut st = vec![d];
            let mut dirty = false;
            while let Some(x) = st.pop() {
                if seen[x] {
                    continue;
                }
                seen[x] = true;
                if io_flag[x] {
                    dirty = true;
                    break;
                }
                for n in &ws.edges[x] {
                    st.push(*n);
                }
            }
            if dirty {
                diags.push(Diagnostic::new(
                    E0211_MEMO_IO,
                    Severity::Error,
                    &ws.defs[d].module_path,
                    Some(f.name.span),
                    format!(
                        "'{}' is `memo` but its dependency cone reaches an effect — memoization requires purity",
                        ws.defs[d].name
                    ),
                ));
            }
        }
    }
    ws.call_sites = call_sites;
    ws.print_sites = print_sites;
    ws.dispatch_sites = dispatch_sites;
    ws.dyn_sites = dyn_sites;
    ws.sccs = sccs;
    diags
}
