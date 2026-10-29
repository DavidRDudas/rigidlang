use rigid::{
    attest, backend_js, canon, diag, eval, fixes, fmt, graph, lsp, merge, metrics, parser,
    resolve, rules, semdiff, sidecar,
};

use diag::{emit, has_errors, Severity};
use std::path::PathBuf;
use std::process::exit;

macro_rules! sayln {
    ($($t:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}

const USAGE: &str = "\
rigid v1.8.1 — a text-first, content-addressed language

USAGE:
  rigid check   [--root DIR] [--frozen] [--json] [--require KIND]
  rigid run     PATH#NAME [ARGS...] [--root DIR]
  rigid test    [--root DIR] [--no-cache]
  rigid watch   [--root DIR]
  rigid diff    [--against DIR | --save] [--root DIR] [--json]
  rigid caps    [--root DIR] [--json]
  rigid merge   --base DIR --ours DIR --theirs DIR [-o DIR]
  rigid attest  PATH#NAME --kind K [--by WHO] [--note TEXT] | --list
  rigid trust   PATH#NAME [--root DIR] [--json]
  rigid dead    [--entry PATH#NAME]... [--root DIR] [--json]
  rigid dup     [--root DIR] [--json]
  rigid hash    (--all | PATH#NAME) [--root DIR]
  rigid deps    PATH#NAME [--root DIR] [--json]
  rigid rdeps   PATH#NAME [--root DIR] [--json]
  rigid why     PATH#NAME [--root DIR] [--json]
  rigid fix     [--apply] [--root DIR]
  rigid rebind  PATH#NAME [--root DIR]
  rigid fmt     PATH [--write] [--root DIR]
  rigid repl    [--root DIR] [--mod PATH]
  rigid recall  HASH-PREFIX [ARGS...] [--root DIR]
  rigid exactness [--root DIR] [--json]
  rigid lsp     [--root DIR]
  rigid build   --target js [-o FILE] [--root DIR]
  rigid describe [--root DIR]

  check   parse, resolve, hash, reconcile binding sidecars
  run     evaluate a definition (int/string args)
  test    run test_* definitions; results cached by content hash
  watch   re-check and re-run only the affected tests on every save
  diff    grade a text diff by identity: what changed *meaning* vs. what
          only changed name or file. --require-no-semantic-change for CI
  caps    which definitions reach the outside world, and through which
          extern — complete by construction, because effects travel the graph
  merge   three-way merge over identities: a rename on one side and an edit
          on the other are different coordinates, so both survive
  attest  bind a claim — reviewed, audited, generated — to a definition's
          identity. It is live while that hash is, and void the moment it
          is not, because a hash covers the whole dependency cone
  trust   every live and stale claim about a definition and what it reaches
  dead    definitions nothing can reach — provably, from the exact graph
  dup     definitions that mean exactly the same thing (identical hashes)
  hash    print content identities (identity survives renames)
  deps    outgoing references          rdeps   blast radius
  why     what re-executes if this changes, and the path that reaches it
  fix     apply typed rename_reference fixes mechanically
  rebind  explicitly accept a name's changed meaning
  fmt     canonical rendering (stdout; --write rewrites, comments preserved)
  repl    evaluate expressions against the workspace (:mod PATH, :q)
  recall  call a stored earlier version by hash prefix (.rigid-store)
  exactness  reference-exactness metric: singleton call sites / all call sites
  build   compile the workspace to another language (js)
  describe emit the whole architecture as one hash-anchored JSON spec:
          every definition with identity, arity, effects, exact deps and
          rdeps, table topology, dyn sets, extern boundary, test coverage
  lsp     language server (stdio JSON-RPC): diagnostics, definition,
          references, hover-with-identity, quarantine code actions
";

struct Pipeline {
    ws: resolve::Workspace,
    diags: Vec<diag::Diagnostic>,
    hashes: std::collections::HashMap<resolve::DefId, canon::Hash>,
}

fn pipeline(root: &PathBuf, json_mode: bool) -> Pipeline {
    let _t0 = std::time::Instant::now();
    let mut ws = match resolve::load_workspace(root) {
        Ok(ws) => ws,
        Err(diags) => {
            emit(&diags, json_mode);
            exit(1);
        }
    };
    if std::env::var("RIGID_TIMING").is_ok() {
        eprintln!("  parse+load : {:?}", _t0.elapsed());
    }
    let _t1 = std::time::Instant::now();
    let mut d = resolve::resolve(&mut ws);
    if std::env::var("RIGID_TIMING").is_ok() {
        eprintln!("  resolve    : {:?}", _t1.elapsed());
    }
    let _t2 = std::time::Instant::now();
    let hashes = canon::hash_into(&mut ws);
    if std::env::var("RIGID_TIMING").is_ok() {
        eprintln!("  hash       : {:?}", _t2.elapsed());
    }
    let _t3 = std::time::Instant::now();
    d.extend(rigid::infer::analyze(&ws));
    d.extend(rigid::domains::analyze(&ws));
    d.extend(graph::coverage_diags(&ws));
    d.extend(graph::key_shadow_diags(&ws));
    if std::env::var("RIGID_TIMING").is_ok() {
        eprintln!("  analyses   : {:?}", _t3.elapsed());
    }
    let diags = d;
    Pipeline { ws, diags, hashes }
}


/// Per-module fingerprint: this module's own definition hashes plus the
/// hashes of everything it references outside itself.
///
/// Sidecar reconciliation compares pinned hashes against current ones, so if
/// neither input has moved the outcome cannot have changed. That makes this
/// fingerprint a sound key for skipping the work — and skipping it is worth
/// real time, because reconciliation reads a JSON file per module.
fn module_fingerprints(p: &Pipeline) -> std::collections::HashMap<String, String> {
    use sha2::{Digest, Sha256};
    let mut per: Vec<Vec<String>> = vec![Vec::new(); p.ws.modules.len()];
    for d in 0..p.ws.defs.len() {
        let mi = p.ws.defs[d].module_idx;
        if let Some(h) = p.hashes.get(&d) {
            per[mi].push(canon::hex(h));
        }
        for &t in &p.ws.edges[d] {
            if p.ws.defs[t].module_idx != mi {
                if let Some(h) = p.hashes.get(&t) {
                    per[mi].push(canon::hex(h));
                }
            }
        }
    }
    let mut out = std::collections::HashMap::new();
    for (mi, m) in p.ws.modules.iter().enumerate() {
        per[mi].sort();
        per[mi].dedup();
        let mut h = Sha256::new();
        for x in &per[mi] {
            h.update(x.as_bytes());
            h.update(b"\n");
        }
        out.insert(m.path.clone(), format!("{:x}", h.finalize()));
    }
    out
}


fn check_cache_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".rigid-checkcache.json")
}

fn reconcile_all(p: &mut Pipeline, frozen: bool) {
    reconcile_all_opts(p, frozen, true)
}

fn reconcile_all_opts(p: &mut Pipeline, frozen: bool, write: bool) {
    reconcile_all_cached(p, frozen, write, None)
}

/// As above, but able to skip modules whose fingerprint is unchanged since a
/// previous clean run. `root` enables that fast path; `None` disables it.
fn reconcile_all_cached(
    p: &mut Pipeline,
    frozen: bool,
    write: bool,
    root: Option<&std::path::Path>,
) {
    let _t = std::time::Instant::now();
    let mut wrote = 0usize;
    let mut skipped = 0usize;

    let fps = root.map(|_| module_fingerprints(p));
    let prev: std::collections::HashMap<String, String> = root
        .and_then(|r| std::fs::read_to_string(check_cache_path(r)).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    for mi in 0..p.ws.modules.len() {
        // Unchanged inputs cannot produce a different outcome, and the entry
        // is only recorded when the previous outcome was clean.
        if let (Some(fps), false) = (&fps, frozen) {
            let path = &p.ws.modules[mi].path;
            if let (Some(now), Some(before)) = (fps.get(path), prev.get(path)) {
                if now == before {
                    skipped += 1;
                    continue;
                }
            }
        }
        let outcome = sidecar::reconcile(&p.ws, mi, &p.hashes, frozen);
        p.diags.extend(outcome.diags);
        if !write {
            continue;
        }
        if let Some(bf) = outcome.updated {
            let path = p.ws.modules[mi].path.clone();
            // The standard library is not part of the workspace being
            // checked; its pins would land in a directory the user does not
            // have. Importers still pin std definitions by hash, so a
            // changed standard library is still visible drift.
            if path.starts_with("std/") {
                continue;
            }
            wrote += 1;
            if let Err(e) = sidecar::save(&p.ws, &path, &bf) {
                eprintln!("warning: could not write sidecar for {}: {}", path, e);
            }
        }
    }
    if std::env::var("RIGID_TIMING").is_ok() {
        eprintln!(
            "  reconcile  : {:?}  ({} rewritten, {} skipped as unchanged)",
            _t.elapsed(),
            wrote,
            skipped
        );
    }

    // Record fingerprints for the modules that came back clean, so the next
    // run can skip them. A module with any diagnostic is left out.
    if let (Some(root), Some(fps)) = (root, fps) {
        let dirty: std::collections::HashSet<&str> =
            p.diags.iter().map(|d| d.file.as_str()).collect();
        let mut keep: std::collections::HashMap<String, String> = prev;
        for (path, fp) in fps {
            if dirty.contains(path.as_str()) {
                keep.remove(&path);
            } else {
                keep.insert(path, fp);
            }
        }
        if let Ok(s) = serde_json::to_string(&keep) {
            let _ = std::fs::write(check_cache_path(root), s);
        }
    }
}


fn write_store(ws: &resolve::Workspace, hashes: &std::collections::HashMap<usize, canon::Hash>, root: &std::path::Path) {
    let dir = root.join(".rigid-store");
    let _ = std::fs::create_dir_all(&dir);
    let h12 = |d: &usize| -> Option<String> {
        hashes.get(d).map(|h| canon::hex(h)[7..19].to_string())
    };
    let mut index: std::collections::HashMap<String, Vec<String>> =
        std::fs::read_to_string(dir.join("index.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
    let mut srcs: std::collections::HashMap<String, Vec<String>> = Default::default();
    for d in 0..ws.defs.len() {
        let info = &ws.defs[d];
        if info.poisoned {
            continue;
        }
        let Some(own) = h12(&d) else { continue };
        if index.contains_key(&own) {
            continue;
        }
        let lines: &Vec<String> = match srcs.entry(info.module_path.clone()) {
            std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
            std::collections::hash_map::Entry::Vacant(v) => {
                let t = std::fs::read_to_string(root.join(&info.module_path)).unwrap_or_default();
                v.insert(t.lines().map(|s| s.to_string()).collect())
            }
        };
        let (nspan, is_ext) = match info.kind {
            resolve::DefKind::Fn => (ws.modules[info.module_idx].fns[info.item_idx].name.span, false),
            resolve::DefKind::Table => (ws.modules[info.module_idx].tables[info.item_idx].name.span, false),
            resolve::DefKind::Extern => (ws.modules[info.module_idx].externs[info.item_idx].name.span, true),
        };
        let start = (nspan.line as usize).saturating_sub(1);
        if start >= lines.len() {
            continue;
        }
        let mut end = start;
        if is_ext {
            while end < lines.len() && !lines[end].contains(';') { end += 1; }
        } else {
            let mut depth = 0i32;
            let mut seen = false;
            'scan: for (li, line) in lines.iter().enumerate().skip(start) {
                let mut in_str = false;
                let mut esc = false;
                for c in line.chars() {
                    if esc { esc = false; continue; }
                    match c {
                        '\\' if in_str => esc = true,
                        '"' => in_str = !in_str,
                        '{' if !in_str => { depth += 1; seen = true; }
                        '}' if !in_str => {
                            depth -= 1;
                            if seen && depth == 0 { end = li; break 'scan; }
                        }
                        _ => {}
                    }
                }
            }
        }
        let mut slice: Vec<String> = lines[start..=end.min(lines.len() - 1)].to_vec();
        // rewrite: own name + every resolved reference inside the slice
        let mut edits: Vec<(usize, usize, String, String)> = Vec::new();
        edits.push((nspan.line as usize, nspan.col as usize, info.name.clone(), format!("d_{}", own)));
        for ((bd, line, col, name), b) in &ws.bindings {
            if *bd != d { continue; }
            if let resolve::Binding::Def(t) = b {
                if let Some(th) = h12(t) {
                    edits.push((*line as usize, *col as usize, name.clone(), format!("d_{}", th)));
                }
            }
        }
        edits.sort_by(|a, b| b.cmp(a));
        for (line, col, name, rep) in edits {
            let li = line.saturating_sub(1);
            if li < start || li > end { continue; }
            let rel = li - start;
            let c0 = col.saturating_sub(1);
            let l = &slice[rel];
            let bytes: Vec<char> = l.chars().collect();
            if c0 + name.chars().count() <= bytes.len()
                && bytes[c0..c0 + name.chars().count()].iter().collect::<String>() == name
            {
                let mut nl: String = bytes[..c0].iter().collect();
                nl.push_str(&rep);
                nl.push_str(&bytes[c0 + name.chars().count()..].iter().collect::<String>());
                slice[rel] = nl;
            }
        }
        let deps: Vec<String> = ws.edges[d].iter().filter_map(|t| h12(t)).collect();
        let _ = std::fs::write(dir.join(format!("d_{}.rg", own)), slice.join("\n") + "\n");
        index.insert(own, deps);
    }
    let _ = std::fs::write(dir.join("index.json"), serde_json::to_string_pretty(&index).unwrap());
}

/// Stack for the thread every command runs on. The parser bounds how deep a
/// syntax tree can be (parser::MAX_NESTING) and the VM bounds how deep calls
/// can go (eval::MAX_CALL_DEPTH); this is sized so both bounds are reached
/// long before the stack is, in debug builds too. It is address space, not
/// memory: pages are only committed when a program actually recurses.
const STACK_BYTES: usize = 1 << 30;

fn main() {
    let worker = std::thread::Builder::new()
        .name("rigid".into())
        .stack_size(STACK_BYTES)
        .spawn(cli_main)
        .expect("could not start the main thread");
    if worker.join().is_err() {
        // the panic message has already been printed
        std::process::exit(101);
    }
}

fn cli_main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json_mode = args.iter().any(|a| a == "--json");
    let frozen = args.iter().any(|a| a == "--frozen");
    let no_cache = args.iter().any(|a| a == "--no-cache");
    let apply = args.iter().any(|a| a == "--apply");
    let write = args.iter().any(|a| a == "--write");
    let root = args
        .iter()
        .position(|a| a == "--root")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let pos: Vec<&String> = {
        let mut skip_next = false;
        let mut v = Vec::new();
        for a in args.iter() {
            if skip_next {
                skip_next = false;
                continue;
            }
            if matches!(a.as_str(), "--root" | "--against" | "--target" | "-o" | "--base" | "--ours" | "--theirs" | "--kind" | "--by" | "--note" | "--require" | "--entry") {
                skip_next = true;
                continue;
            }
            if a.starts_with("--") {
                continue;
            }
            v.push(a);
        }
        v
    };

    let Some(cmd) = pos.first() else {
        eprint!("{}", USAGE);
        exit(2);
    };

    match cmd.as_str() {
        "check" => {
            let mut p = pipeline(&root, json_mode);
            // `check` is the hot path, so it gets the incremental fast path.
            // Other commands reconcile in full: they run rarely and some of
            // them deliberately want every diagnostic re-emitted.
            reconcile_all_cached(&mut p, frozen, true, Some(root.as_path()));
            emit(&p.diags, json_mode);
            if has_errors(&p.diags) {
                exit(1);
            }
            write_store(&p.ws, &p.hashes, &root);

            // Declared relationships are checked here, after the graph
            // exists and before anything is reported as ok.
            match rules::load(&root) {
                Ok(rs) if !rs.is_empty() => {
                    let ledger = attest::load(&root).unwrap_or_default();
                    let live = attest::live_index(&ledger, &p.ws, &p.hashes);
                    let mut attested: std::collections::HashMap<
                        String,
                        std::collections::HashSet<usize>,
                    > = Default::default();
                    for (d, aa) in &live {
                        for a in aa {
                            attested.entry(a.kind.clone()).or_default().insert(*d);
                        }
                    }
                    let cx = rules::Ctx {
                        ws: &p.ws,
                        hashes: &p.hashes,
                        attested,
                    };
                    let rd = rules::check(&rs, &cx);
                    if !rd.is_empty() {
                        emit(&rd, json_mode);
                        exit(1);
                    }
                    if !json_mode {
                        sayln!("{} declared rule(s) hold.", rs.len());
                    }
                }
                Ok(_) => {}
                Err(ds) => {
                    emit(&ds, json_mode);
                    exit(1);
                }
            }

            // `--require KIND` is the gate: nothing ships without live
            // evidence of the kind you named. Because a claim is bound to a
            // hash and a hash covers its cone, "live" here means the exact
            // code being released was the code that was reviewed.
            let required: Vec<&String> = args
                .iter()
                .enumerate()
                .filter(|(i, a)| *a == "--require" && args.len() > i + 1)
                .map(|(i, _)| &args[i + 1])
                .collect();
            if !required.is_empty() {
                let ledger = attest::load(&root).unwrap_or_default();
                let live = attest::live_index(&ledger, &p.ws, &p.hashes);
                let mut missing: Vec<(String, String)> = Vec::new();
                for &kind in &required {
                    for d in attest::gated(&p.ws) {
                        let ok = live
                            .get(&d)
                            .map(|aa| aa.iter().any(|a| &a.kind == kind))
                            .unwrap_or(false);
                        if !ok {
                            missing.push((kind.clone(), graph::ref_str(&p.ws, d)));
                        }
                    }
                }
                if !missing.is_empty() {
                    if json_mode {
                        sayln!("{}", serde_json::to_string_pretty(&serde_json::json!({
                            "missing": missing.iter().map(|(k, r)| serde_json::json!({
                                "kind": k, "ref": r
                            })).collect::<Vec<_>>(),
                        })).unwrap());
                    } else {
                        sayln!(
                            "{} definition(s) lack a live claim of the required kind:",
                            missing.len()
                        );
                        for (k, r) in missing.iter().take(20) {
                            sayln!("  {:<10} {}", k, r);
                        }
                        if missing.len() > 20 {
                            sayln!("  … and {} more", missing.len() - 20);
                        }
                    }
                    exit(1);
                }
                if !json_mode {
                    let n = attest::gated(&p.ws).len();
                    sayln!(
                        "all {} definition(s) carry live {} evidence.",
                        n,
                        required.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" + ")
                    );
                }
            }
            if !json_mode {
                sayln!(
                    "ok: {} module(s), {} definition(s), {} hashed",
                    p.ws.modules.len(),
                    p.ws.defs.len(),
                    p.hashes.len()
                );
            }
        }
        "run" => {
            let Some(r) = pos.get(1) else {
                eprint!("{}", USAGE);
                exit(2);
            };
            let p = pipeline(&root, json_mode);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let Some(d) = graph::find_ref(&p.ws, r) else {
                eprintln!("no such definition: {}", r);
                exit(1);
            };
            let vals: Vec<eval::Value> = pos[2..]
                .iter()
                .map(|s| eval::cli_value(s))
                .collect();
            let eng = eval::Engine::new(&p.ws).with_memo(&p.hashes, &root);
            match eng.call(d, vals) {
                Ok(v) => sayln!("{}", eval::show(&p.ws, &v)),
                Err(e) => {
                    eprintln!("runtime error: {}", eval::annotate(&p.ws, &eval::show(&p.ws, &e)));
                    exit(1);
                }
            }
        }
        "test" => {
            let p = pipeline(&root, json_mode);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let rep = eval::run_tests(&p.ws, &p.hashes, no_cache);
            sayln!(
                "{} executed, {} skipped via content-address cache, {} failed",
                rep.executed, rep.skipped, rep.failed
            );
            if rep.failed > 0 {
                exit(1);
            }
        }
        "hash" => {
            // bare module path (no '#') -> module identity
            if let Some(path) = args.get(1).filter(|a| !a.contains('#') && !a.starts_with("--")) {
                let p = pipeline(&root, json_mode);
                if has_errors(&p.diags) {
                    emit(&p.diags, json_mode);
                    exit(1);
                }
                let h = canon::module_hash_hex(&p.ws, &p.hashes, path);
                if h.is_empty() {
                    eprintln!("no such module (or unhashed members): {}", path);
                    exit(1);
                }
                sayln!("{}  module:{}", path, h);
                return;
            }
            let p = pipeline(&root, json_mode);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let all = args.iter().any(|a| a == "--all");
            if all {
                let mut lines: Vec<(String, String)> = p
                    .hashes
                    .iter()
                    .map(|(d, h)| (graph::ref_str(&p.ws, *d), canon::hex(h)))
                    .collect();
                lines.sort();
                for (r, h) in lines {
                    sayln!("{}  {}", r, h);
                }
            } else if let Some(r) = pos.get(1) {
                match graph::find_ref(&p.ws, r) {
                    Some(d) => match p.hashes.get(&d) {
                        Some(h) => sayln!("{}  {}", r, canon::hex(h)),
                        None => {
                            eprintln!("{}: not hashed (upstream errors)", r);
                            exit(1);
                        }
                    },
                    None => {
                        eprintln!("no such definition: {}", r);
                        exit(1);
                    }
                }
            } else {
                eprint!("{}", USAGE);
                exit(2);
            }
        }
        "why" => {
            let p = pipeline(&root, json_mode);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let Some(r) = pos.get(1) else {
                eprint!("{}", USAGE);
                exit(2);
            };
            let Some(d) = graph::find_ref(&p.ws, r) else {
                eprintln!("no such definition: {}", r);
                exit(1);
            };
            let (affected, tests) = graph::blast(&p.ws, d);
            let hash = p.hashes.get(&d).map(canon::hex).unwrap_or_default();

            if json_mode {
                let paths: Vec<serde_json::Value> = tests
                    .iter()
                    .map(|&t| {
                        let path = graph::path_between(&p.ws, t, d).unwrap_or_default();
                        serde_json::json!({
                            "test": graph::ref_str(&p.ws, t),
                            "path": path.iter().map(|&x| graph::ref_str(&p.ws, x))
                                .collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                sayln!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "of": graph::ref_str(&p.ws, d),
                    "hash": hash,
                    "definitions_affected": affected.iter()
                        .map(|&x| graph::ref_str(&p.ws, x)).collect::<Vec<_>>(),
                    "tests": paths,
                })).unwrap());
            } else {
                sayln!("{}  {}", graph::ref_str(&p.ws, d), hash);
                sayln!("");
                sayln!("if this changes, {} definition(s) get a new hash:", affected.len());
                for x in &affected {
                    sayln!("  {}", graph::ref_str(&p.ws, *x));
                }
                sayln!("");
                if tests.is_empty() {
                    sayln!("no test reaches it, so nothing would re-execute.");
                } else {
                    sayln!("and {} test(s) re-execute, because:", tests.len());
                    for &t in &tests {
                        let path = graph::path_between(&p.ws, t, d).unwrap_or_default();
                        let names: Vec<String> = path
                            .iter()
                            .map(|&x| p.ws.defs[x].name.clone())
                            .collect();
                        sayln!("  {}", names.join(" -> "));
                    }
                }
            }
        }
        "attest" => {
            let mut p = pipeline(&root, json_mode);
            reconcile_all_opts(&mut p, false, false);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let mut ledger = match attest::load(&root) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("error: {}", e);
                    exit(1);
                }
            };

            if args.iter().any(|a| a == "--list") {
                let live = attest::live_index(&ledger, &p.ws, &p.hashes);
                let stale = attest::stale(&ledger, &p.ws, &p.hashes);
                if json_mode {
                    sayln!("{}", serde_json::to_string_pretty(&serde_json::json!({
                        "live": live.iter().map(|(d, aa)| serde_json::json!({
                            "ref": graph::ref_str(&p.ws, *d),
                            "claims": aa,
                        })).collect::<Vec<_>>(),
                        "stale": stale,
                    })).unwrap());
                } else {
                    let mut rows: Vec<(String, &Vec<&attest::Attestation>)> = live
                        .iter()
                        .map(|(d, aa)| (graph::ref_str(&p.ws, *d), aa))
                        .collect();
                    rows.sort_by(|a, b| a.0.cmp(&b.0));
                    sayln!("{} live claim(s):", rows.iter().map(|(_, a)| a.len()).sum::<usize>());
                    for (r, aa) in &rows {
                        for a in aa.iter() {
                            sayln!("  {:<10} {:<28} {} on {}", a.kind, r, a.by, a.at);
                        }
                    }
                    if !stale.is_empty() {
                        sayln!("");
                        sayln!("{} stale claim(s) — the code they covered has changed:", stale.len());
                        for a in &stale {
                            sayln!("  {:<10} {:<28} {} on {}", a.kind, a.reference, a.by, a.at);
                        }
                    }
                }
                return;
            }

            let Some(r) = pos.get(1) else {
                eprintln!("usage: rigid attest PATH#NAME --kind KIND [--by WHO] [--note TEXT]");
                exit(2);
            };
            let Some(d) = graph::find_ref(&p.ws, r) else {
                eprintln!("no such definition: {}", r);
                exit(1);
            };
            let opt = |k: &str| -> Option<String> {
                args.iter()
                    .position(|a| a == k)
                    .and_then(|i| args.get(i + 1))
                    .cloned()
            };
            let Some(kind) = opt("--kind") else {
                eprintln!("error: --kind is required (e.g. reviewed, audited, generated)");
                exit(2);
            };
            let by = opt("--by")
                .or_else(|| std::env::var("RIGID_USER").ok())
                .or_else(|| std::env::var("USER").ok())
                .unwrap_or_else(|| "unknown".to_string());
            let hash = canon::hex(p.hashes.get(&d).expect("checked workspace"));
            let a = attest::Attestation {
                reference: graph::ref_str(&p.ws, d),
                hash: hash.clone(),
                kind: kind.clone(),
                by: by.clone(),
                at: attest::today(),
                note: opt("--note"),
            };
            if ledger.attestations.iter().any(|x| {
                x.hash == a.hash && x.kind == a.kind && x.by == a.by
            }) {
                sayln!("already attested: {} {} by {}", kind, graph::ref_str(&p.ws, d), by);
                return;
            }
            ledger.attestations.push(a);
            if let Err(e) = attest::save(&root, &ledger) {
                eprintln!("error: {}", e);
                exit(1);
            }
            let cone = attest::cone(&p.ws, d);
            sayln!("attested {} {}", kind, graph::ref_str(&p.ws, d));
            sayln!("  identity {}", hash);
            sayln!(
                "  covering {} definition(s) in its cone — an edit to any of them",
                cone.len()
            );
            sayln!("  moves this hash and voids the claim.");
        }
        "trust" => {
            let mut p = pipeline(&root, json_mode);
            reconcile_all_opts(&mut p, false, false);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let Some(r) = pos.get(1) else {
                eprint!("{}", USAGE);
                exit(2);
            };
            let Some(d) = graph::find_ref(&p.ws, r) else {
                eprintln!("no such definition: {}", r);
                exit(1);
            };
            let ledger = attest::load(&root).unwrap_or_default();
            let live = attest::live_index(&ledger, &p.ws, &p.hashes);
            let cone = attest::cone(&p.ws, d);
            let mine: Vec<&attest::Attestation> =
                live.get(&d).map(|v| v.to_vec()).unwrap_or_default();
            let unattested: Vec<usize> = cone
                .iter()
                .copied()
                .filter(|x| !live.contains_key(x))
                .collect();
            let stale: Vec<&attest::Attestation> = attest::stale(&ledger, &p.ws, &p.hashes)
                .into_iter()
                .filter(|a| a.reference == graph::ref_str(&p.ws, d))
                .collect();

            if json_mode {
                sayln!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "ref": graph::ref_str(&p.ws, d),
                    "hash": canon::hex(p.hashes.get(&d).unwrap()),
                    "live": mine,
                    "stale": stale,
                    "cone": cone.len(),
                    "cone_unattested": unattested.iter()
                        .map(|&x| graph::ref_str(&p.ws, x)).collect::<Vec<_>>(),
                })).unwrap());
            } else {
                sayln!("{}  {}", graph::ref_str(&p.ws, d), canon::hex(p.hashes.get(&d).unwrap()));
                sayln!("");
                if mine.is_empty() {
                    sayln!("no live claim about this definition.");
                } else {
                    sayln!("live claims:");
                    for a in &mine {
                        sayln!(
                            "  {:<10} by {} on {}{}",
                            a.kind,
                            a.by,
                            a.at,
                            a.note.as_ref().map(|n| format!("  — {}", n)).unwrap_or_default()
                        );
                    }
                    sayln!("");
                    sayln!("These cover all {} definition(s) this reaches, because a hash", cone.len());
                    sayln!("covers its cone: edit anything below and the claims above go void.");
                }
                if !stale.is_empty() {
                    sayln!("");
                    sayln!("stale — made about a version that no longer exists:");
                    for a in &stale {
                        sayln!("  {:<10} by {} on {}", a.kind, a.by, a.at);
                    }
                }
                if !unattested.is_empty() && unattested.len() < cone.len() {
                    sayln!("");
                    sayln!("{} of {} in the cone carry no claim of their own:", unattested.len(), cone.len());
                    for x in unattested.iter().take(8) {
                        sayln!("  {}", graph::ref_str(&p.ws, *x));
                    }
                    if unattested.len() > 8 {
                        sayln!("  … and {} more", unattested.len() - 8);
                    }
                }
            }
        }
        "dup" => {
            let mut p = pipeline(&root, json_mode);
            reconcile_all_opts(&mut p, false, false);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let dups = rules::duplicates(&p.ws, &p.hashes);
            if json_mode {
                sayln!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "groups": dups.iter().map(|(h, ds)| serde_json::json!({
                        "hash": h,
                        "definitions": ds.iter().map(|&d| graph::ref_str(&p.ws, d))
                            .collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                })).unwrap());
            } else if dups.is_empty() {
                sayln!("no two definitions share an identity.");
            } else {
                let n: usize = dups.iter().map(|(_, d)| d.len()).sum();
                sayln!("{} definition(s) in {} group(s) mean exactly the same thing:", n, dups.len());
                for (h, ds) in &dups {
                    sayln!("");
                    sayln!("  {}", &h[..19.min(h.len())]);
                    for &d in ds {
                        sayln!("    {}", graph::ref_str(&p.ws, d));
                    }
                }
                sayln!("");
                sayln!("These are not similar — they are identical. The canonical form erased");
                sayln!("the names and what remained was the same bytes, so one of them can");
                sayln!("import the other and nothing else changes.");
            }
        }
        "dead" => {
            let mut p = pipeline(&root, json_mode);
            reconcile_all_opts(&mut p, false, false);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            // Roots: every test, anything named `main`, and whatever else was
            // named explicitly. Everything else is reachable or it is not —
            // and because the graph is the compiler's own resolution rather
            // than a search, "is not" is a proof, not a guess.
            let mut roots: Vec<usize> = Vec::new();
            for (i, a) in args.iter().enumerate() {
                if a == "--entry" {
                    if let Some(r) = args.get(i + 1) {
                        match graph::find_ref(&p.ws, r) {
                            Some(d) => roots.push(d),
                            None => {
                                eprintln!("no such definition: {}", r);
                                exit(1);
                            }
                        }
                    }
                }
            }
            let named = !roots.is_empty();
            for d in 0..p.ws.defs.len() {
                let i = &p.ws.defs[d];
                if i.module_path.starts_with("std/") {
                    roots.push(d);
                } else if i.name.starts_with("test_") || i.name == "main" {
                    roots.push(d);
                }
            }
            let mut seen = vec![false; p.ws.defs.len()];
            let mut stack = roots.clone();
            while let Some(x) = stack.pop() {
                if seen[x] {
                    continue;
                }
                seen[x] = true;
                for &n in &p.ws.edges[x] {
                    stack.push(n);
                }
            }
            let dead: Vec<usize> = (0..p.ws.defs.len())
                .filter(|&d| !seen[d] && !p.ws.defs[d].module_path.starts_with("std/"))
                .collect();

            if json_mode {
                sayln!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "roots": roots.len(),
                    "total": p.ws.defs.len(),
                    "unreachable": dead.iter()
                        .map(|&d| serde_json::json!({
                            "ref": graph::ref_str(&p.ws, d),
                            "hash": p.hashes.get(&d).map(canon::hex),
                        })).collect::<Vec<_>>(),
                })).unwrap());
            } else if dead.is_empty() {
                sayln!("every definition is reachable from a test, a `main`, or a named entry.");
            } else {
                // Split the tops — nothing in the workspace references them
                // at all — from what merely hangs beneath one. Only the tops
                // are decisions; the rest follow.
                let mut referenced = vec![false; p.ws.defs.len()];
                for d in 0..p.ws.defs.len() {
                    for &n in &p.ws.edges[d] {
                        referenced[n] = true;
                    }
                }
                let (tops, beneath): (Vec<usize>, Vec<usize>) =
                    dead.iter().partition(|&&d| !referenced[d]);
                sayln!(
                    "{} of {} definition(s) unreachable from any test, any `main`{}.",
                    dead.len(),
                    p.ws.defs.len(),
                    if named { ", or the named entries" } else { "" }
                );
                sayln!("");
                sayln!("{} nothing in this workspace references at all:", tops.len());
                for &d in &tops {
                    sayln!("  {}", graph::ref_str(&p.ws, d));
                }
                if !beneath.is_empty() {
                    sayln!("");
                    sayln!("{} reachable only from those:", beneath.len());
                    for &d in beneath.iter().take(20) {
                        sayln!("  {}", graph::ref_str(&p.ws, d));
                    }
                    if beneath.len() > 20 {
                        sayln!("  … and {} more", beneath.len() - 20);
                    }
                }
                sayln!("");
                sayln!("This is the compiler's own resolution, not a search: nothing here can");
                sayln!("reach them. Each name in the first list is a decision — an entry point");
                sayln!("to declare with --entry, or code to delete. The second list follows.");
            }
        }
        "caps" => {
            let p = pipeline(&root, json_mode);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let caps_of = |d: usize| -> rigid::ast::Caps {
                let info = &p.ws.defs[d];
                match info.kind {
                    resolve::DefKind::Fn => p.ws.modules[info.module_idx].fns[info.item_idx].caps,
                    resolve::DefKind::Extern => {
                        p.ws.modules[info.module_idx].externs[info.item_idx].caps
                    }
                    resolve::DefKind::Table => 0,
                }
            };
            // The externs are the supply: every capability in the workspace
            // enters through one, and the graph says exactly who reaches it.
            let mut rows: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
            for (bit, cname) in rigid::ast::CAP_NAMES {
                let holders: Vec<usize> =
                    (0..p.ws.defs.len()).filter(|&d| caps_of(d) & bit != 0).collect();
                if holders.is_empty() {
                    continue;
                }
                let sources: Vec<String> = holders
                    .iter()
                    .filter(|&&d| p.ws.defs[d].kind == resolve::DefKind::Extern)
                    .map(|&d| {
                        let info = &p.ws.defs[d];
                        format!(
                            "{} = {:?}",
                            graph::ref_str(&p.ws, d),
                            p.ws.modules[info.module_idx].externs[info.item_idx].host
                        )
                    })
                    .collect();
                rows.push((
                    format!("!{}", cname),
                    holders.iter().map(|&d| graph::ref_str(&p.ws, d)).collect(),
                    sources,
                ));
            }
            let pure = (0..p.ws.defs.len()).filter(|&d| caps_of(d) == 0).count();

            if json_mode {
                sayln!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "total": p.ws.defs.len(),
                    "pure": pure,
                    "capabilities": rows.iter().map(|(c, hs, ss)| serde_json::json!({
                        "capability": c, "definitions": hs, "hosts": ss
                    })).collect::<Vec<_>>(),
                })).unwrap());
            } else if rows.is_empty() {
                sayln!("{} definitions, all pure — this workspace reaches nothing outside itself.", p.ws.defs.len());
            } else {
                sayln!("{} definitions, {} pure.", p.ws.defs.len(), pure);
                sayln!("");
                for (c, holders, sources) in &rows {
                    sayln!("{:<7} {} definition(s)", c, holders.len());
                    for s in sources {
                        sayln!("        via extern {}", s);
                    }
                    for h in holders.iter().take(12) {
                        sayln!("          {}", h);
                    }
                    if holders.len() > 12 {
                        sayln!("          … and {} more", holders.len() - 12);
                    }
                    sayln!("");
                }
                sayln!("Every capability enters through an extern and travels the exact");
                sayln!("graph, so this list is complete by construction, not by search.");
            }
        }
        "merge" => {
            let opt = |k: &str| -> Option<PathBuf> {
                args.iter()
                    .position(|a| a == k)
                    .and_then(|i| args.get(i + 1))
                    .map(PathBuf::from)
            };
            let (Some(base), Some(ours), Some(theirs)) =
                (opt("--base"), opt("--ours"), opt("--theirs"))
            else {
                eprintln!("usage: rigid merge --base DIR --ours DIR --theirs DIR [-o DIR]");
                exit(2);
            };
            let snap = |d: &PathBuf, label: &str| {
                let p = pipeline(d, json_mode);
                if has_errors(&p.diags) {
                    eprintln!("error: the {} revision does not check clean", label);
                    emit(&p.diags, json_mode);
                    exit(1);
                }
                (semdiff::snapshot(&p.ws, &p.hashes), p)
            };
            let (bs, _) = snap(&base, "base");
            let (os, op) = snap(&ours, "ours");
            let (ts, tp) = snap(&theirs, "theirs");

            let plan = merge::plan(&bs, &os, &ts);
            let read_all = |p: &Pipeline, root: &PathBuf| -> merge::Files {
                p.ws.modules
                    .iter()
                    .map(|m| {
                        (
                            m.path.clone(),
                            std::fs::read_to_string(root.join(&m.path)).unwrap_or_default(),
                        )
                    })
                    .collect()
            };

            if json_mode {
                sayln!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "conflicts": plan.conflicts(),
                    "rescued": plan.rescued(),
                    "items": plan.items.iter().map(|i| serde_json::json!({
                        "ref": i.merged_ref.clone().or_else(|| i.base_ref.clone()),
                        "outcome": format!("{:?}", i.outcome),
                    })).collect::<Vec<_>>(),
                })).unwrap());
            } else {
                print!("{}", merge::render(&plan));
            }

            let out_dir = args
                .iter()
                .position(|a| a == "-o")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from);
            if plan.conflicts() > 0 {
                if out_dir.is_some() {
                    eprintln!("\nnot writing: resolve the conflicts above first.");
                }
                exit(1);
            }
            if let Some(dir) = out_dir {
                let files = match merge::assemble(
                    &plan,
                    &read_all(&op, &ours),
                    &read_all(&tp, &theirs),
                ) {
                    Ok(f) => f,
                    Err(es) => {
                        for e in es {
                            eprintln!("error: {}", e);
                        }
                        exit(1);
                    }
                };
                let _ = std::fs::create_dir_all(&dir);
                for (path, src) in &files {
                    let p = dir.join(path);
                    if let Some(parent) = p.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(e) = std::fs::write(&p, src) {
                        eprintln!("error: {}: {}", p.display(), e);
                        exit(1);
                    }
                }
                sayln!("");
                sayln!("wrote {} module(s) to {}", files.len(), dir.display());
                sayln!("Bodies moved with their comments; references were rewritten to the");
                sayln!("merged names. `rigid check --root {}` is the proof.", dir.display());
            }
        }
        "diff" => {
            let p = pipeline(&root, json_mode);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let now = semdiff::snapshot(&p.ws, &p.hashes);

            if args.iter().any(|a| a == "--save") {
                let path = semdiff::snapshot_path(&root);
                if let Err(e) = semdiff::save(&path, &now) {
                    eprintln!("error: {}", e);
                    exit(1);
                }
                sayln!("snapshot saved: {} definitions -> {}", now.defs.len(), path.display());
                return;
            }

            // Either against another checkout of the workspace, or against the
            // snapshot this workspace saved earlier. Both are just two sets of
            // identities; the comparison does not care where they came from.
            let against = args
                .iter()
                .position(|a| a == "--against")
                .and_then(|i| args.get(i + 1));
            let base = match against {
                Some(dir) => {
                    let other = PathBuf::from(dir);
                    let bp = pipeline(&other, json_mode);
                    if has_errors(&bp.diags) {
                        eprintln!("error: the workspace at {} does not check clean", dir);
                        emit(&bp.diags, json_mode);
                        exit(1);
                    }
                    semdiff::snapshot(&bp.ws, &bp.hashes)
                }
                None => {
                    let path = semdiff::snapshot_path(&root);
                    if !path.exists() {
                        eprintln!(
                            "no baseline: run `rigid diff --save` first, or pass \
                             `--against DIR` to compare against another checkout"
                        );
                        exit(2);
                    }
                    match semdiff::load(&path) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("error: {}", e);
                            exit(1);
                        }
                    }
                }
            };

            let rep = semdiff::compare(&base, &now);
            if json_mode {
                sayln!("{}", serde_json::to_string_pretty(&semdiff::to_json(&rep)).unwrap());
            } else {
                print!("{}", semdiff::render(&rep));
            }
            // `--require-no-semantic-change` is the CI knob: a refactor that
            // claims to be a refactor can be made to prove it.
            if args.iter().any(|a| a == "--require-no-semantic-change") && !rep.meaning_preserved()
            {
                exit(1);
            }
        }
        "watch" => {
            // Poll rather than pull in a notify crate: the dependency list is
            // three pure-Rust crates and it stays that way.
            fn snapshot(root: &std::path::Path) -> Vec<(String, u64, u64)> {
                fn walk(dir: &std::path::Path, out: &mut Vec<(String, u64, u64)>) {
                    let Ok(rd) = std::fs::read_dir(dir) else { return };
                    for e in rd.flatten() {
                        let p = e.path();
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.starts_with('.') || name == "target" {
                            continue;
                        }
                        if p.is_dir() {
                            walk(&p, out);
                        } else if p.extension().map(|x| x == "rg").unwrap_or(false) {
                            let (len, ms) = e
                                .metadata()
                                .map(|m| {
                                    let ms = m
                                        .modified()
                                        .ok()
                                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                        .map(|d| d.as_millis() as u64)
                                        .unwrap_or(0);
                                    (m.len(), ms)
                                })
                                .unwrap_or((0, 0));
                            out.push((p.to_string_lossy().to_string(), len, ms));
                        }
                    }
                }
                let mut v = Vec::new();
                walk(root, &mut v);
                v.sort();
                v
            }

            sayln!("watching {} — ctrl-c to stop", root.display());
            let mut last = Vec::new();
            loop {
                let now = snapshot(&root);
                if now != last {
                    if !last.is_empty() {
                        sayln!("");
                    }
                    last = now;
                    let mut p = pipeline(&root, false);
                    reconcile_all_cached(&mut p, false, true, Some(root.as_path()));
                    let errs = has_errors(&p.diags);
                    for d in p.diags.iter().filter(|d| d.severity != Severity::Info) {
                        sayln!("{}", d.render_human());
                    }
                    if errs {
                        sayln!("check failed — not running tests");
                    } else {
                        let rep = eval::run_tests_opts(&p.ws, &p.hashes, false, true);
                        sayln!(
                            "{} executed, {} skipped via content-address cache, {} failed",
                            rep.executed, rep.skipped, rep.failed
                        );
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
        "deps" | "rdeps" => {
            let p = pipeline(&root, json_mode);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let Some(r) = pos.get(1) else {
                eprint!("{}", USAGE);
                exit(2);
            };
            let Some(d) = graph::find_ref(&p.ws, r) else {
                eprintln!("no such definition: {}", r);
                exit(1);
            };
            let v = if cmd.as_str() == "deps" {
                graph::deps(&p.ws, &p.hashes, d)
            } else {
                graph::rdeps(&p.ws, &p.hashes, d)
            };
            if json_mode {
                sayln!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else {
                let results = v["results"].as_array().unwrap();
                sayln!("{} of {} ({} result{}):", cmd, r, results.len(),
                    if results.len() == 1 { "" } else { "s" });
                for e in results {
                    sayln!("  {}  {}",
                        e["ref"].as_str().unwrap_or("?"),
                        e["hash"].as_str().unwrap_or("?"));
                }
            }
        }
        "fix" => {
            // iterate: collect diagnostics, apply rename fixes, converge
            let mut round = 0;
            loop {
                round += 1;
                let mut p = pipeline(&root, json_mode);
                reconcile_all(&mut p, true); // frozen inside fix: never repin silently
                let fixes = fixes::rename_fixes(&p.diags);
                let qfixes = fixes::quarantine_fixes(&p.diags);
                if fixes.is_empty() && qfixes.is_empty() {
                    sayln!("no applicable fixes (round {})", round);
                    break;
                }
                if !apply {
                    sayln!(
                        "{} applicable fix(es) — dry run (use --apply):",
                        fixes.len() + qfixes.len()
                    );
                    for (m, from, to) in &fixes {
                        sayln!("  {}: rename {} -> {}", m, from, to);
                    }
                    for q in &qfixes {
                        match q {
                            fixes::QFix::QuarantineCall { module, callee, targets, .. } => sayln!(
                                "  {}: quarantine call '{}' with {{ {} }}",
                                module, callee, targets.join(", ")
                            ),
                            fixes::QFix::AddDynTargets { module, targets, .. } => sayln!(
                                "  {}: widen dyn set with {{ {} }}",
                                module, targets.join(", ")
                            ),
                        }
                    }
                    break;
                }
                for q in &qfixes {
                    match fixes::apply_quarantine(&p.ws, q) {
                        Ok(true) => match q {
                            fixes::QFix::QuarantineCall { module, callee, targets, .. } => sayln!(
                                "applied: quarantined '{}' in {} with {{ {} }}",
                                callee, module, targets.join(", ")
                            ),
                            fixes::QFix::AddDynTargets { module, targets, .. } => sayln!(
                                "applied: widened dyn set in {} with {{ {} }}",
                                module, targets.join(", ")
                            ),
                        },
                        Ok(false) => sayln!("skipped a quarantine fix (source changed?)"),
                        Err(e) => sayln!("fix failed: {}", e),
                    }
                }
                for (m, from, to) in &fixes {
                    match fixes::apply_rename(&p.ws, m, from, to) {
                        Ok(n) => sayln!("applied: {} occurrences of '{}' -> '{}' in {}", n, from, to, m),
                        Err(e) => {
                            eprintln!("failed to apply fix in {}: {}", m, e);
                            exit(1);
                        }
                    }
                }
                if round >= 5 {
                    eprintln!("fix did not converge in 5 rounds");
                    exit(1);
                }
            }
            if apply {
                let mut p = pipeline(&root, json_mode);
                reconcile_all(&mut p, false);
                emit(&p.diags, json_mode);
                if has_errors(&p.diags) {
                    exit(1);
                }
                sayln!("converged: check is green");
            }
        }
        "rebind" => {
            let Some(r) = pos.get(1) else {
                eprint!("{}", USAGE);
                exit(2);
            };
            let Some((module, name)) = r.split_once('#') else {
                eprintln!("expected MODULE#NAME");
                exit(2);
            };
            let p = pipeline(&root, json_mode);
            match fixes::rebind(&p.ws, &p.hashes, module, name) {
                Ok(msg) => sayln!("{}", msg),
                Err(e) => {
                    eprintln!("rebind failed: {}", e);
                    exit(1);
                }
            }
        }
        "recall" => {
            let target = args.get(1).cloned().unwrap_or_default();
            if target.is_empty() {
                eprintln!("usage: rigid recall <hash-prefix> [args...] [--root DIR]");
                exit(2);
            }
            let p = pipeline(&root, false);
            if has_errors(&p.diags) {
                emit(&p.diags, false);
                exit(1);
            }
            let vals: Vec<eval::Value> = args[2..]
                .iter()
                .take_while(|s| !s.starts_with("--"))
                .map(|s| eval::cli_value(s))
                .collect();
            let eng = eval::Engine::new(&p.ws).with_memo(&p.hashes, &root);
            match eng.recall(&target, vals) {
                Ok(v) => sayln!("{}", eval::show(&p.ws, &v)),
                Err(e) => {
                    eprintln!("runtime error: {}", eval::annotate(&p.ws, &eval::show(&p.ws, &e)));
                    exit(1);
                }
            }
        }
        "describe" => {
            let p = pipeline(&root, json_mode);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let ws = &p.ws;
            let hex = |d: &usize| {
                p.hashes
                    .get(d)
                    .map(|h| canon::hex(h))
                    .unwrap_or_default()
            };
            let refname =
                |d: &usize| format!("{}#{}", ws.defs[*d].module_path, ws.defs[*d].name);
            // reverse edges once
            let mut rdeps: Vec<Vec<usize>> = vec![Vec::new(); ws.defs.len()];
            for (from, outs) in ws.edges.iter().enumerate() {
                for to in outs {
                    rdeps[*to].push(from);
                }
            }
            // transitive test coverage: which test_* reach this def
            let mut covered: Vec<Vec<String>> = vec![Vec::new(); ws.defs.len()];
            for t in 0..ws.defs.len() {
                if !ws.defs[t].name.starts_with("test_") {
                    continue;
                }
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
                        covered[d].push(refname(&t));
                    }
                }
            }
            let mut mods = Vec::new();
            for mm in &ws.modules {
                let mut defs_json = Vec::new();
                for (d, info) in ws.defs.iter().enumerate() {
                    if info.module_path != mm.path {
                        continue;
                    }
                    let mut seen = vec![false; ws.defs.len()];
                    let mut st = vec![d];
                    while let Some(x) = st.pop() {
                        if seen[x] { continue; }
                        seen[x] = true;
                        for n in &ws.edges[x] { st.push(*n); }
                    }
                    let mut reach_v: Vec<String> = seen.iter().enumerate()
                        .filter(|&(x, s)| *s && x != d)
                        .map(|(x, _)| refname(&x)).collect();
                    reach_v.sort();
                    let mut o = serde_json::json!({
                        "name": info.name,
                        "reach": reach_v,
                        "ref": refname(&d),
                        "kind": match info.kind {
                            resolve::DefKind::Fn => "fn",
                            resolve::DefKind::Table => "table",
                            resolve::DefKind::Extern => "extern",
                        },
                        "hash": hex(&d),
                        "deps": ws.edges[d].iter().map(|x| refname(x)).collect::<Vec<_>>(),
                        "rdeps": rdeps[d].iter().map(|x| refname(x)).collect::<Vec<_>>(),
                        "covered_by": covered[d],
                    });
                    match info.kind {
                        resolve::DefKind::Fn => {
                            let f = &mm.fns[info.item_idx];
                            o["arity"] = serde_json::json!(f.params.len());
                            o["io"] = serde_json::json!(f.caps & rigid::ast::CAP_IO != 0);
                            o["effects"] = serde_json::json!(rigid::ast::caps_list(f.caps));
                            if !ws.dyn_sites[d].is_empty() {
                                o["dyn_sites"] = serde_json::json!(ws.dyn_sites[d]
                                    .iter()
                                    .map(|(_, set)| set
                                        .iter()
                                        .map(|x| refname(x))
                                        .collect::<Vec<_>>())
                                    .collect::<Vec<_>>());
                            }
                        }
                        resolve::DefKind::Table => {
                            if let Some(es) = ws.table_entries.get(&d) {
                                let map: serde_json::Map<String, serde_json::Value> = es
                                    .iter()
                                    .map(|(k, t)| {
                                        (k.clone(), serde_json::json!(refname(t)))
                                    })
                                    .collect();
                                o["entries"] = serde_json::Value::Object(map);
                            }
                        }
                        resolve::DefKind::Extern => {
                            let x = &mm.externs[info.item_idx];
                            o["arity"] = serde_json::json!(x.params.len());
                            o["io"] = serde_json::json!(x.caps & rigid::ast::CAP_IO != 0);
                            o["effects"] = serde_json::json!(rigid::ast::caps_list(x.caps));
                            o["host"] = serde_json::json!(x.host);
                        }
                    }
                    defs_json.push(o);
                }
                mods.push(serde_json::json!({
                    "path": mm.path,
                    "module_hash": canon::module_hash_hex(ws, &p.hashes, &mm.path),
                    "definitions": defs_json,
                }));
            }
            let total_sites: usize = ws.call_sites.iter().map(|v| v.len()).sum();
            let spec = serde_json::json!({
                "format": "rigid-spec/1",
                "modules": mods,
                "boundary": ws.defs.iter().enumerate()
                    .filter(|(_, i)| i.kind == resolve::DefKind::Extern)
                    .map(|(d, _)| refname(&d)).collect::<Vec<_>>(),
                "stats": {
                    "modules": ws.modules.len(),
                    "definitions": ws.defs.len(),
                    "call_sites": total_sites,
                },
            });
            println!("{}", serde_json::to_string_pretty(&spec).unwrap());
        }
        "build" => {
            let target = args
                .iter()
                .position(|a| a == "--target")
                .and_then(|i| args.get(i + 1))
                .cloned()
                .unwrap_or_else(|| "js".to_string());
            if target != "js" {
                eprintln!("unknown target '{}' (supported: js)", target);
                exit(1);
            }
            let p = pipeline(&root, json_mode);
            if has_errors(&p.diags) {
                emit(&p.diags, json_mode);
                exit(1);
            }
            let unrepresentable = backend_js::check(&p.ws);
            if !unrepresentable.is_empty() {
                emit(&unrepresentable, json_mode);
                exit(1);
            }
            let code = backend_js::Js::emit(&p.ws);
            match args.iter().position(|a| a == "-o").and_then(|i| args.get(i + 1)) {
                Some(path) => match std::fs::write(path, &code) {
                    Ok(()) => sayln!("wrote {} ({} bytes)", path, code.len()),
                    Err(e) => {
                        eprintln!("could not write {}: {}", path, e);
                        exit(1);
                    }
                },
                None => print!("{}", code),
            }
        }
        "lsp" => {
            lsp::Lsp::new(root.clone()).serve();
        }
        "exactness" => {
            let json = args.iter().any(|a| a == "--json");
            let p = pipeline(&root, false);
            if has_errors(&p.diags) {
                emit(&p.diags, json);
                exit(1);
            }
            let ex = metrics::measure(&p.ws);
            let fmtw = |v: &Vec<usize>| {
                v.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(", ")
            };
            if json {
                sayln!(
                    "{{\"sites\":{},\"singleton\":{},\"dyn_widths\":[{}],\"table_widths\":[{}],\"exactness\":{:.4}}}",
                    ex.sites,
                    ex.singleton,
                    ex.dyn_widths.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(","),
                    ex.table_widths.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(","),
                    ex.score()
                );
            } else {
                sayln!("reference exactness (see the paper, §5)");
                sayln!("  call sites:      {}", ex.sites);
                sayln!("  singleton sites: {}", ex.singleton);
                sayln!(
                    "  dyn sites:       {}{}",
                    ex.dyn_widths.len(),
                    if ex.dyn_widths.is_empty() { String::new() } else { format!("  (widths: {})", fmtw(&ex.dyn_widths)) }
                );
                sayln!(
                    "  table sites:     {}{}",
                    ex.table_widths.len(),
                    if ex.table_widths.is_empty() { String::new() } else { format!("  (widths: {})", fmtw(&ex.table_widths)) }
                );
                sayln!("  exactness:       {:.3}", ex.score());
            }
        }
        "repl" => {
            use std::io::{BufRead, Write};
            let mut ctx: Option<String> = args
                .iter()
                .position(|a| a == "--mod")
                .and_then(|i| args.get(i + 1))
                .cloned();
            {
                let p0 = pipeline(&root, false);
                if ctx.is_none() {
                    ctx = p0.ws.modules.first().map(|m| m.path.clone());
                }
                let errs: Vec<_> = p0
                    .diags
                    .iter()
                    .filter(|d| d.severity == Severity::Error)
                    .cloned()
                    .collect();
                if !errs.is_empty() {
                    emit(&errs, false);
                    eprintln!("(workspace has errors; repl may misbehave)");
                }
            }
            let Some(mut ctx) = ctx else {
                eprintln!("no modules found under {}", root.display());
                exit(1);
            };
            sayln!(
                "rigid repl — context: {}  (:mod PATH to switch, :q to quit; a line with ';' is statements, otherwise an expression)",
                ctx
            );
            let stdin = std::io::stdin();
            loop {
                print!("rigid> ");
                std::io::stdout().flush().ok();
                let mut line = String::new();
                if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if line == ":q" || line == ":quit" {
                    break;
                }
                if let Some(rest) = line.strip_prefix(":mod ") {
                    ctx = rest.trim().to_string();
                    sayln!("context: {}", ctx);
                    continue;
                }
                let body = if line.contains(';') {
                    line.clone()
                } else {
                    format!("return ({});", line)
                };
                let src = format!("fn __repl__() !io {{ {} }}", body);
                let wrapper = match parser::parse_module(&src, &ctx) {
                    Ok(m) => m,
                    Err(d) => {
                        sayln!("{}", d.render_human());
                        continue;
                    }
                };
                let Some(f) = wrapper.fns.into_iter().next() else {
                    continue;
                };
                let mut ws = match resolve::load_workspace(&root) {
                    Ok(ws) => ws,
                    Err(diags) => {
                        emit(&diags, false);
                        continue;
                    }
                };
                let Some(id) = resolve::inject_fn(&mut ws, &ctx, f) else {
                    sayln!("no such module (or name collision): {}", ctx);
                    continue;
                };
                let diags = resolve::resolve(&mut ws);
                let errs: Vec<_> = diags
                    .into_iter()
                    .filter(|d| d.severity == Severity::Error)
                    .collect();
                if !errs.is_empty() {
                    emit(&errs, false);
                    continue;
                }
                canon::hash_into(&mut ws);
                let eng = eval::Engine::new(&ws);
                match eng.call(id, vec![]) {
                    Ok(v) => sayln!("{}", eval::show(&ws, &v)),
                    Err(e) => sayln!("error: {}", eval::annotate(&ws, &eval::show(&ws, &e))),
                }
            }
        }
        "fmt" => {
            let Some(path) = pos.get(1) else {
                eprint!("{}", USAGE);
                exit(2);
            };
            let p = pipeline(&root, json_mode);
            // suppress info-level pinning chatter for fmt
            let hard: Vec<_> = p.diags.iter().filter(|d| d.severity == Severity::Error).cloned().collect();
            if !hard.is_empty() {
                emit(&hard, json_mode);
                exit(1);
            }
            match p.ws.modules.iter().find(|m| &m.path == *path) {
                Some(m) => {
                    let rendered = fmt::render(m);
                    if write {
                        let disk = root.join(&m.path);
                        if let Err(e) = std::fs::write(&disk, &rendered) {
                            eprintln!("could not write {}: {}", m.path, e);
                            exit(1);
                        }
                        sayln!("formatted {}", m.path);
                    } else {
                        print!("{}", rendered);
                    }
                }
                None => {
                    eprintln!("no such module: {}", path);
                    exit(1);
                }
            }
        }
        other => {
            eprintln!("unknown command: {}\n", other);
            eprint!("{}", USAGE);
            exit(2);
        }
    }
}
