//! rigid as a library.
//!
//! `main.rs` is the CLI over this; `wasm.rs` is the browser playground's
//! entry point, compiled only for wasm32. Keeping the compiler in a library
//! is what lets both front ends share one implementation.

pub mod ast;
pub mod attest;
pub mod backend_js;
pub mod canon;
pub mod diag;
pub mod domains;
pub mod eval;
pub mod fixes;
pub mod fmt;
pub mod graph;
pub mod infer;
pub mod lexer;
pub mod lift;
pub mod lsp;
pub mod merge;
pub mod metrics;
pub mod parser;
pub mod property;
pub mod resolve;
pub mod rules;
pub mod semdiff;
pub mod sidecar;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// Resolve, hash, and run every static analysis pass, in order.
///
/// The CLI and the browser playground both go through here. They used to each
/// list the passes themselves, and a pass added to one was missing from the
/// other — so this exists to make that failure impossible rather than
/// unlikely. Sidecar reconciliation is deliberately not part of it: it needs a
/// filesystem, and the playground has none.
pub fn check_workspace(
    ws: &mut resolve::Workspace,
) -> (
    Vec<diag::Diagnostic>,
    std::collections::HashMap<resolve::DefId, canon::Hash>,
) {
    let mut diags = resolve::resolve(ws);
    let hashes = canon::hash_workspace(ws);
    diags.extend(infer::analyze(ws));
    diags.extend(domains::analyze(ws));
    diags.extend(graph::coverage_diags(ws));
    diags.extend(graph::key_shadow_diags(ws));
    (diags, hashes)
}
