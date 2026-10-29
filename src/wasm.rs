//! Browser entry point for the playground.
//!
//! Deliberately dependency-free: raw `extern "C"` exports over a tiny
//! length-prefixed byte protocol rather than wasm-bindgen, so the crate keeps
//! its three pure-Rust dependencies and the wasm build needs no extra
//! toolchain beyond `rustup target add wasm32-unknown-unknown`.
//!
//! Protocol. The host calls `rigid_alloc(n)` for a buffer, writes UTF-8 JSON
//! into it, then calls `rigid_eval(ptr, n)`. The return value points at
//! `[u32 little-endian length][UTF-8 JSON]`, which the host reads and then
//! releases with `rigid_free(ptr)`.
//!
//! Input:  {"source": "...", "action": "check"|"run"|"test", "entry": "m.rg#f",
//!          "args": ["..."]}
//! Output: {"ok": bool, "output": "...", "result": "...", "diagnostics": [...],
//!          "stats": {...}}

use crate::{diag, eval, graph, resolve};
use serde_json::{json, Value as J};

const MODULE: &str = "play.rg";

// ── memory the host can drive ────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn rigid_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Frees a buffer produced by `rigid_eval` (length-prefixed).
#[no_mangle]
pub unsafe extern "C" fn rigid_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    let len = u32::from_le_bytes([*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)]) as usize;
    drop(Vec::from_raw_parts(ptr, len + 4, len + 4));
}

fn to_host(s: String) -> *mut u8 {
    let bytes = s.into_bytes();
    let mut out = Vec::with_capacity(bytes.len() + 4);
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&bytes);
    let ptr = out.as_mut_ptr();
    std::mem::forget(out);
    ptr
}

// ── the one entry point ──────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn rigid_eval(ptr: *const u8, len: usize) -> *mut u8 {
    let input = std::slice::from_raw_parts(ptr, len);
    let text = String::from_utf8_lossy(input).to_string();
    // The host owns the input buffer it allocated; take it back and drop it.
    drop(Vec::from_raw_parts(ptr as *mut u8, len, len));
    to_host(dispatch(&text))
}

fn dispatch(input: &str) -> String {
    let req: J = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => return err_response(&format!("bad request: {}", e)),
    };
    let source = req["source"].as_str().unwrap_or("");
    let action = req["action"].as_str().unwrap_or("check");
    let entry = req["entry"].as_str().unwrap_or("");
    let args: Vec<String> = req["args"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| x.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();

    let _ = eval::host::take_output(); // start each run with a clean buffer

    // Parse and resolve. This is the same path the CLI's `pipeline` takes.
    let mut ws = match resolve::load_workspace_from_sources(&[(MODULE.to_string(),
                                                               source.to_string())]) {
        Ok(ws) => ws,
        Err(diags) => return diag_response(&diags, ""),
    };
    let (diags, hashes) = crate::check_workspace(&mut ws);

    if diag::has_errors(&diags) {
        return diag_response(&diags, &eval::host::take_output());
    }

    let stats = json!({
        "modules": ws.modules.len(),
        "definitions": ws.defs.len(),
    });

    let result: String;
    match action {
        "check" => {
            let hashed = hashes.len();
            result = format!(
                "ok: {} module(s), {} definition(s), {} hashed",
                ws.modules.len(),
                ws.defs.len(),
                hashed
            );
        }
        "test" => {
            let rep = eval::run_tests(&ws, &hashes, false);
            result = format!(
                "{} executed, {} skipped via content-address cache, {} failed",
                rep.executed, rep.skipped, rep.failed
            );
        }
        "run" => {
            let Some(d) = graph::find_ref(&ws, entry) else {
                return diag_response(&diags, &format!("no such definition: {}", entry));
            };
            let vals: Vec<eval::Value> = args
                .iter()
                .map(|s| eval::cli_value(s))
                .collect();
            // Memoization is on, backed by a store that survives between
            // runs. Temporal calls stay off: they need a workspace on disk.
            let eng = eval::Engine::new(&ws).with_memo_host(&hashes);
            let called = eng.call(d, vals);
            eng.publish_memo();
            match called {
                Ok(v) => result = eval::show(&ws, &v),
                Err(e) => {
                    return response(false, &eval::host::take_output(),
                                    &format!("runtime error: {}", eval::annotate(&ws, &eval::show(&ws, &e))),
                                    &diags, stats);
                }
            }
        }
        other => return err_response(&format!("unknown action: {}", other)),
    }

    response(true, &eval::host::take_output(), &result, &diags, stats)
}

// ── responses ────────────────────────────────────────────────────────────

fn diags_json(diags: &[diag::Diagnostic]) -> J {
    J::Array(diags.iter().map(|d| d.to_json()).collect())
}

fn response(ok: bool, output: &str, result: &str, diags: &[diag::Diagnostic], stats: J) -> String {
    json!({
        "ok": ok,
        "output": output,
        "result": result,
        "diagnostics": diags_json(diags),
        "stats": stats,
    })
    .to_string()
}

fn diag_response(diags: &[diag::Diagnostic], output: &str) -> String {
    json!({
        "ok": false,
        "output": output,
        "result": "",
        "diagnostics": diags_json(diags),
        "stats": json!({}),
    })
    .to_string()
}

fn err_response(msg: &str) -> String {
    json!({ "ok": false, "output": "", "result": msg, "diagnostics": [], "stats": {} }).to_string()
}
