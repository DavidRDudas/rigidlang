# Completeness ledger

The agreed bar for "complete language" is **Lox-class** (the *Crafting
Interpreters* teaching-language standard) plus rigid's own reasons to
exist. Checked items are demonstrated in `demo.sh` or `examples/`; the
list is falsifiable and updated every pass.

## Lox-class expressiveness

- [x] Arithmetic, comparison, string concatenation
- [x] Conditionals (`if` expressions)
- [x] Functions, recursion, mutual recursion (SCC-hashed)
- [x] Lists — literals, indexing, `len`, `push`, `+` concat (v5)
- [x] Records — literals, field access, order-insensitive identity (v5)
- [x] String indexing (v5)
- [x] Console IO — `print`, `input` (v5), gated by `!io`
- [x] Modules and imports
- [x] First-class function values — `&name` + `dyn` (deliberate design
      *divergence* from Lox closures; see below)
- [x] Mutation and loops — `x = e;` assignment, `while`, flat function
      scope, stable loop-local slots (v6)
- [x] Recoverable errors — `fail(v)` raises any value; `try(e)` returns
      `{ ok, value }` / `{ ok, error }`; builtin runtime errors are
      catchable too (v6)
- [x] String/number stdlib — `str`, `int` (catchable failure), `substr`,
      `chars`, `join` (v7)
- [ ] Closures — *open design question*, not backlog: a closure captures
      environment, and "what is the content-hash of a captured value" is
      unresolved. rigid's current answer is `&name` + quarantine instead.
- [x] REPL — expression and statement mode against the live workspace,
      module context switching (v7)
- [x] File IO — `read_file`/`write_file`, `!io`-gated, missing-file errors
      catchable (v7)

## Verdict (v7)

Every gating box above is checked. Closures remain the one open item, and
they are documented as a deliberate design divergence (`&name` + `dyn`
instead), not a pending feature — the underlying question, the identity of
a captured environment, is research, and the ledger does not paper over
research by calling it backlog.

**By the bar this project defined, rigid is a complete small language.**
What that sentence does not include is listed at the bottom of this file,
and it is the part that finishes languages: performance, tooling depth,
and users.

## Beyond Lox (rigid's reasons to exist — all done)

- [x] Content-addressed identity; names as metadata
- [x] Binding sidecars, reconciliation R1–R6, `--frozen`
- [x] Mechanical repair (`fix --apply`), explicit `rebind`
- [x] Quarantined dynamism (`dyn`), effects over the exact graph
- [x] Dispatch tables (string-keyed, closed-set, graph-visible)
- [x] Provably-sound test caching
- [x] Comment-preserving canonical formatter
- [x] Arity checked at every call form
- [x] Structured JSON diagnostics with typed fixes
- [x] Reference-exactness metric computed by the compiler (`exactness`),
      with a measured migration case study (v8)

## Beyond Lox-class (not gating "complete", tracked honestly)

- [x] Performance: bytecode VM — each definition compiled once to a stack
      machine; identical semantics (26-step regression green); measured
      2.1x over the tree-walker at fib(36), with the unified frame-stack
      rewrite identified as the next win (v9)
- [x] LSP served from the graph (v11): diagnostics from the real pipeline
      (sidecar rules included, read-only), definition and references from
      the bindings map — exact through dyn sets and table entries — hover
      with content identity, quarantine fixes as code actions; verified by
      a scripted protocol client (tools/lsp_smoke.py, 6 checks)
- [x] Quarantine roadmap: set-inference as typed fixes (I0405/W0403,
      applied by `fix --apply`), dyn-set/table sidecar pins with W0404
      drift under reconciliation and `--frozen` (v10)
- [x] Module identity: a module's hash over its members' sorted hashes —
      renaming and reordering definitions leave it fixed (v10)
## Backends

- [x] JavaScript (v12): `rigid build --target js` lowers a resolved
      workspace to one ES/CommonJS module — tables become frozen objects,
      `dyn` sites keep their membership check at runtime, `fail`/`try`
      become throw/catch of a value-carrying error, integer division
      truncates, `!io` is erased (it is a compile-time discipline).
      Verified by `tools/difftest.sh`: 16 behaviors run on both rigid's VM
      and Node and must agree to the character, error messages included.
- [ ] C, WASM — the same lowering, not yet written.
- [x] Foreign bindings (v13): `extern fn name(args) [!io] = "host.path";`
      — declared boundary nodes with checked arity, propagated effects,
      and content identity over the declaration (host-path drift is
      W0401). VM refuses with a directive; the JS target binds. Verified:
      rigid calling Math.max and Date.now on node, fmt round-trip
      hash-neutral, rdeps showing the boundary as a graph node.

## Measured, not asserted (v13)

- Compile time: 3,000 modules / 6,000 defs check in 477ms cold / 353ms
  warm on this machine — roughly linear. The earlier claim that
  whole-project re-hashing would be "painful at ten thousand files" was
  overstated: extrapolated cost at 10k modules is under two seconds.
  Incremental resolution remains unbuilt because the measurement says it
  is not yet needed.
- Execution: fib(30) — rigid VM 433ms, compiled-to-JS on node 49ms. The
  JS target is the performance answer (~9x here); the VM's job is
  tooling, tests, and the REPL, where it is adequate.

## v14 ledger

- [x] memo: E0211 purity gate; VM persistent store (.rigid-memo.json)
      keyed by def-hash + canonical args; bytecode CALL routes memo defs
      through the full path; JS target in-run Map (persistence VM-only,
      documented); fmt prints the keyword; hash unchanged by the flag.
      Measured: 360ms cold / 2ms cross-process hit / recompute on edit.
- [x] temporal references: append-only content store written on green
      check (self-contained hash-named defs, refs rewritten to
      d_<hash12>); @"prefix"(args) materializes and runs the old cone
      (recursion, tables, cross-def all verified); `rigid recall` CLI;
      !io required; JS target refuses at runtime. Demo step 36.

## Found by writing a larger program (v13)

examples/press — a 4-module, 270-line static site generator (54 defs,
22 tests) — was written as a test subject, then put through the
maintenance battery. It exposed three real defects, all fixed: unresolved
call callees were never recorded as Broken bindings; a broken module's
sidecar was rewritten, destroying rename evidence (now rule R7); and
same-module renames had no recovery path (now I0406 with a typed fix).
Cross-engine check: the generated site is byte-identical from the VM and
from compiled JS on node.

## v14 — the signature features

- [x] memo: persistent provably-sound memoization (E0211 purity gate;
      cross-process verified 389ms -> 2ms; identity-neutral annotation;
      JS target runs memo fns uncached in v14).
- [x] temporal references: content store on green check + `@"hash"(args)`
      (verified live: current and prior version of the same function
      executing side by side; `!io` required; VM-only; store immutable).
      Verified on single-module cones; multi-module cones use the same
      hash-rewritten mechanism but have not been battery-tested yet.

## Deliberately unbuilt (usage-driven)

Network effects, the frame-stack VM optimization, a debugger, and a
package/publish story are correctly built *in response to usage*, not
speculatively. They are classified here, on purpose, as outside
"finished." Users remain what they always were: the one thing no pass
can produce.
