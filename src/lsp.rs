//! `rigid lsp` — a Language Server over the exact graph, JSON-RPC on
//! stdio. The thesis cashed out: the queries an editor asks are the
//! primitives the compiler already owns. Definitions and references come
//! from the resolver's bindings map (so references are exact, including
//! through dyn sets and table entries); hover shows content identity;
//! diagnostics are the ordinary pipeline plus read-only sidecar
//! reconciliation (the server never writes sidecars); code actions carry
//! the typed quarantine fixes.
//!
//! Positions: LSP is 0-based; our spans are 1-based columns counted per
//! character — correct for ASCII sources, off-by-N on non-BMP text
//! (documented limitation).

use crate::canon;
use crate::diag::{Diagnostic, Severity};
use crate::eval;
use crate::fixes;
use crate::resolve::{self, Binding, DefId, DefKind, Workspace};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;

pub struct Lsp {
    root: PathBuf,
    overlay: HashMap<String, String>,
}

struct Snapshot {
    ws: Workspace,
    diags: Vec<Diagnostic>,
    hashes: HashMap<DefId, canon::Hash>,
}

impl Lsp {
    pub fn new(root: PathBuf) -> Lsp {
        Lsp {
            root,
            overlay: HashMap::new(),
        }
    }

    fn snapshot(&self) -> Result<Snapshot, Vec<Diagnostic>> {
        let mut ws = resolve::load_workspace_overlay(&self.root, &self.overlay)?;
        let mut diags = resolve::resolve(&mut ws);
        let hashes = canon::hash_workspace(&ws);
        diags.extend(crate::infer::analyze(&ws));
        // read-only sidecar reconciliation: rules fire, nothing is written
        for mi in 0..ws.modules.len() {
            let outcome = crate::sidecar::reconcile(&ws, mi, &hashes, false);
            diags.extend(outcome.diags);
        }
        Ok(Snapshot { ws, diags, hashes })
    }

    fn uri_to_rel(&self, uri: &str) -> Option<String> {
        let p = uri.strip_prefix("file://")?;
        let p = PathBuf::from(p);
        let rel = p.strip_prefix(&self.root).ok()?;
        Some(rel.to_string_lossy().replace('\\', "/"))
    }

    fn rel_to_uri(&self, rel: &str) -> String {
        format!("file://{}", self.root.join(rel).to_string_lossy())
    }

    /// Find the binding occurrence covering (rel, line0, char0).
    fn occurrence_at(
        &self,
        ws: &Workspace,
        rel: &str,
        line0: u64,
        char0: u64,
    ) -> Option<(DefId, u32, u32, String, Binding)> {
        let line = line0 as u32 + 1;
        let ch = char0 as u32 + 1;
        for ((def, l, c, name), b) in &ws.bindings {
            if ws.defs[*def].module_path != rel || *l != line {
                continue;
            }
            let end = *c + name.chars().count() as u32;
            if *c <= ch && ch < end {
                return Some((*def, *l, *c, name.clone(), *b));
            }
        }
        // definition-name sites are not in bindings; probe them
        for (mi, m) in ws.modules.iter().enumerate() {
            if m.path != rel {
                continue;
            }
            for (fi, f) in m.fns.iter().enumerate() {
                let sp = f.name.span;
                let end = sp.col + f.name.name.chars().count() as u32;
                if sp.line == line && sp.col <= ch && ch < end {
                    let d = ws
                        .def_index
                        .get(&(m.path.clone(), f.name.name.clone()))
                        .copied()?;
                    if ws.defs[d].module_idx == mi && ws.defs[d].item_idx == fi {
                        return Some((d, sp.line, sp.col, f.name.name.clone(), Binding::Def(d)));
                    }
                }
            }
            for t in &m.tables {
                let sp = t.name.span;
                let end = sp.col + t.name.name.chars().count() as u32;
                if sp.line == line && sp.col <= ch && ch < end {
                    let d = ws
                        .def_index
                        .get(&(m.path.clone(), t.name.name.clone()))
                        .copied()?;
                    return Some((d, sp.line, sp.col, t.name.name.clone(), Binding::Def(d)));
                }
            }
        }
        None
    }

    fn def_site(&self, ws: &Workspace, d: DefId) -> (String, u32, u32, u32) {
        let info = &ws.defs[d];
        let m = &ws.modules[info.module_idx];
        let (sp, len) = match info.kind {
            DefKind::Fn => {
                let f = &m.fns[info.item_idx];
                (f.name.span, f.name.name.chars().count() as u32)
            }
            DefKind::Table => {
                let t = &m.tables[info.item_idx];
                (t.name.span, t.name.name.chars().count() as u32)
            }
            DefKind::Extern => {
                let x = &m.externs[info.item_idx];
                (x.name.span, x.name.name.chars().count() as u32)
            }
        };
        (info.module_path.clone(), sp.line, sp.col, len)
    }

    fn location(&self, rel: &str, line: u32, col: u32, len: u32) -> Value {
        json!({
            "uri": self.rel_to_uri(rel),
            "range": {
                "start": {"line": line - 1, "character": col - 1},
                "end": {"line": line - 1, "character": col - 1 + len}
            }
        })
    }

    fn publish_all(&self, snap: &Snapshot, out: &mut impl Write) {
        let mut by_file: HashMap<String, Vec<Value>> = HashMap::new();
        for m in &snap.ws.modules {
            by_file.entry(m.path.clone()).or_default();
        }
        for d in &snap.diags {
            let sev = match d.severity {
                Severity::Error => 1,
                Severity::Warning => 2,
                Severity::Info => 3,
            };
            let (l, c) = d
                .span
                .map(|s| (s.line - 1, s.col - 1))
                .unwrap_or((0, 0));
            by_file.entry(d.file.clone()).or_default().push(json!({
                "range": {"start": {"line": l, "character": c},
                           "end": {"line": l, "character": c + 1}},
                "severity": sev,
                "code": d.code,
                "source": "rigid",
                "message": d.message,
            }));
        }
        for (file, diags) in by_file {
            notify(
                out,
                "textDocument/publishDiagnostics",
                json!({"uri": self.rel_to_uri(&file), "diagnostics": diags}),
            );
        }
    }

    fn hover(&self, snap: &Snapshot, rel: &str, line0: u64, char0: u64) -> Value {
        let Some((_, _, _, _, b)) = self.occurrence_at(&snap.ws, rel, line0, char0) else {
            return Value::Null;
        };
        match b {
            Binding::Def(d) => {
                let info = &snap.ws.defs[d];
                let h = snap
                    .hashes
                    .get(&d)
                    .map(|h| canon::hex(h))
                    .unwrap_or_else(|| "(unhashed)".to_string());
                let detail = match info.kind {
                    DefKind::Fn => {
                        let f = &snap.ws.modules[info.module_idx].fns[info.item_idx];
                        format!(
                            "fn, arity {}{}",
                            f.params.len(),
                            if f.caps != 0 { format!(", {}", crate::ast::caps_str(f.caps)) } else { String::new() }
                        )
                    }
                    DefKind::Table => {
                        let n = snap
                            .ws
                            .table_entries
                            .get(&d)
                            .map(|e| e.len())
                            .unwrap_or(0);
                        format!("table, {} entries", n)
                    }
                    DefKind::Extern => {
                        let x = &snap.ws.modules[info.module_idx].externs[info.item_idx];
                        format!(
                            "extern, arity {}{}, -> {}",
                            x.params.len(),
                            if x.caps != 0 { format!(", {}", crate::ast::caps_str(x.caps)) } else { String::new() },
                            x.host
                        )
                    }
                };
                let text = format!(
                    "**{}#{}**  \n{}  \n`{}`",
                    info.module_path, info.name, detail, h
                );
                json!({"contents": {"kind": "markdown", "value": text}})
            }
            Binding::Local(slot) => {
                json!({"contents": {"kind": "markdown",
                       "value": format!("local (slot {})", slot)}})
            }
            Binding::Broken => {
                json!({"contents": {"kind": "markdown", "value": "unresolved"}})
            }
        }
    }

    fn definition(&self, snap: &Snapshot, rel: &str, line0: u64, char0: u64) -> Value {
        match self.occurrence_at(&snap.ws, rel, line0, char0) {
            Some((_, _, _, _, Binding::Def(d))) => {
                let (m, l, c, len) = self.def_site(&snap.ws, d);
                self.location(&m, l, c, len)
            }
            _ => Value::Null,
        }
    }

    fn references(&self, snap: &Snapshot, rel: &str, line0: u64, char0: u64) -> Value {
        let Some((_, _, _, _, Binding::Def(target))) =
            self.occurrence_at(&snap.ws, rel, line0, char0)
        else {
            return json!([]);
        };
        let mut locs = Vec::new();
        let (m, l, c, len) = self.def_site(&snap.ws, target);
        locs.push(self.location(&m, l, c, len));
        for ((def, line, col, name), b) in &snap.ws.bindings {
            if *b == Binding::Def(target) {
                let file = snap.ws.defs[*def].module_path.clone();
                locs.push(self.location(&file, *line, *col, name.chars().count() as u32));
            }
        }
        Value::Array(locs)
    }

    fn code_actions(&self, snap: &Snapshot, rel: &str, start_line0: u64, end_line0: u64) -> Value {
        let qfixes = fixes::quarantine_fixes(&snap.diags);
        let mut actions = Vec::new();
        for q in &qfixes {
            let (module, line, col) = match q {
                fixes::QFix::QuarantineCall { module, line, col, .. }
                | fixes::QFix::AddDynTargets { module, line, col, .. } => (module, *line, *col),
            };
            if module != rel {
                continue;
            }
            let l0 = (line - 1) as u64;
            if l0 < start_line0 || l0 > end_line0 {
                continue;
            }
            let (title, edit) = match q {
                fixes::QFix::AddDynTargets { after, targets, .. } => {
                    let at = col - 1 + after.chars().count() as u32;
                    (
                        format!("Widen dyn set with {{ {} }}", targets.join(", ")),
                        json!([{
                            "range": {"start": {"line": l0, "character": at},
                                       "end": {"line": l0, "character": at}},
                            "newText": format!(", {}", targets.join(", "))
                        }]),
                    )
                }
                fixes::QFix::QuarantineCall {
                    callee,
                    argc,
                    targets,
                    ..
                } => {
                    let s = (col - 1) as u64;
                    let clen = callee.chars().count() as u64;
                    if *argc == 0 {
                        (
                            format!("Quarantine call with {{ {} }}", targets.join(", ")),
                            json!([{
                                "range": {"start": {"line": l0, "character": s},
                                           "end": {"line": l0, "character": s + clen + 2}},
                                "newText": format!("dyn {{ {} }}({})", targets.join(", "), callee)
                            }]),
                        )
                    } else {
                        (
                            format!("Quarantine call with {{ {} }}", targets.join(", ")),
                            json!([{
                                "range": {"start": {"line": l0, "character": s},
                                           "end": {"line": l0, "character": s + clen + 1}},
                                "newText": format!("dyn {{ {} }}({}, ", targets.join(", "), callee)
                            }]),
                        )
                    }
                }
            };
            actions.push(json!({
                "title": title,
                "kind": "quickfix",
                "edit": {"changes": {self.rel_to_uri(rel): edit}}
            }));
        }
        Value::Array(actions)
    }

    pub fn serve(&mut self) {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let mut input = stdin.lock();
        let mut out = stdout.lock();
        // no snapshot until initialize
        let mut snap: Option<Snapshot> = None;
        loop {
            let Some(msg) = read_message(&mut input) else {
                break;
            };
            let method = msg["method"].as_str().unwrap_or("").to_string();
            let id = msg.get("id").cloned();
            match method.as_str() {
                "initialize" => {
                    respond(
                        &mut out,
                        id,
                        json!({
                            "capabilities": {
                                "textDocumentSync": 1,
                                "hoverProvider": true,
                                "definitionProvider": true,
                                "referencesProvider": true,
                                "codeActionProvider": true
                            },
                            "serverInfo": {"name": "rigid-lsp", "version": "v11"}
                        }),
                    );
                }
                "initialized" => {
                    if let Ok(s) = self.snapshot() {
                        self.publish_all(&s, &mut out);
                        snap = Some(s);
                    }
                }
                "textDocument/didOpen" => {
                    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                    let text = msg["params"]["textDocument"]["text"].as_str().unwrap_or("");
                    if let Some(rel) = self.uri_to_rel(uri) {
                        self.overlay.insert(rel, text.to_string());
                    }
                    if let Ok(s) = self.snapshot() {
                        self.publish_all(&s, &mut out);
                        snap = Some(s);
                    }
                }
                "textDocument/didChange" => {
                    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                    if let (Some(rel), Some(changes)) = (
                        self.uri_to_rel(uri),
                        msg["params"]["contentChanges"].as_array(),
                    ) {
                        if let Some(full) = changes.last().and_then(|c| c["text"].as_str()) {
                            self.overlay.insert(rel, full.to_string());
                        }
                    }
                    if let Ok(s) = self.snapshot() {
                        self.publish_all(&s, &mut out);
                        snap = Some(s);
                    }
                }
                "textDocument/hover" => {
                    let v = self.positioned(&snap, &msg, |s, rel, l, c| self.hover(s, &rel, l, c));
                    respond(&mut out, id, v);
                }
                "textDocument/definition" => {
                    let v =
                        self.positioned(&snap, &msg, |s, rel, l, c| self.definition(s, &rel, l, c));
                    respond(&mut out, id, v);
                }
                "textDocument/references" => {
                    let v =
                        self.positioned(&snap, &msg, |s, rel, l, c| self.references(s, &rel, l, c));
                    respond(&mut out, id, v);
                }
                "textDocument/codeAction" => {
                    let v = (|| {
                        let s = snap.as_ref()?;
                        let uri = msg["params"]["textDocument"]["uri"].as_str()?;
                        let rel = self.uri_to_rel(uri)?;
                        let sl = msg["params"]["range"]["start"]["line"].as_u64()?;
                        let el = msg["params"]["range"]["end"]["line"].as_u64()?;
                        Some(self.code_actions(s, &rel, sl, el))
                    })()
                    .unwrap_or(json!([]));
                    respond(&mut out, id, v);
                }
                "shutdown" => respond(&mut out, id, Value::Null),
                "exit" => break,
                _ => {
                    if id.is_some() {
                        respond(&mut out, id, Value::Null);
                    }
                }
            }
        }
    }

    fn positioned<F: Fn(&Snapshot, String, u64, u64) -> Value>(
        &self,
        snap: &Option<Snapshot>,
        msg: &Value,
        f: F,
    ) -> Value {
        (|| {
            let s = snap.as_ref()?;
            let uri = msg["params"]["textDocument"]["uri"].as_str()?;
            let rel = self.uri_to_rel(uri)?;
            let l = msg["params"]["position"]["line"].as_u64()?;
            let c = msg["params"]["position"]["character"].as_u64()?;
            Some(f(s, rel, l, c))
        })()
        .unwrap_or(Value::Null)
    }
}

fn read_message(input: &mut impl Read) -> Option<Value> {
    let mut header = Vec::new();
    let mut last4 = [0u8; 4];
    loop {
        let mut b = [0u8; 1];
        if input.read_exact(&mut b).is_err() {
            return None;
        }
        header.push(b[0]);
        last4.rotate_left(1);
        last4[3] = b[0];
        if &last4 == b"\r\n\r\n" {
            break;
        }
    }
    let text = String::from_utf8_lossy(&header);
    let len: usize = text
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length:"))
        .and_then(|v| v.trim().parse().ok())?;
    let mut body = vec![0u8; len];
    input.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn write_message(out: &mut impl Write, v: &Value) {
    let body = v.to_string();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = out.flush();
}

fn respond(out: &mut impl Write, id: Option<Value>, result: Value) {
    write_message(
        out,
        &json!({"jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result}),
    );
}

fn notify(out: &mut impl Write, method: &str, params: Value) {
    write_message(out, &json!({"jsonrpc": "2.0", "method": method, "params": params}));
}

// keep eval linked for future evaluate-on-hover work without a warning
#[allow(dead_code)]
fn _touch(_: &eval::Value) {}
