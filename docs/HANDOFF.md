# HANDOFF.md — continuation brief for Claude Code

You are picking up **rigid**, a working research language built with David
across one long session. This file is self-contained: state, rules,
in-progress work, exact next steps, and the traps already hit.

## What this is

rigid is a text-canonical, content-addressed language: every definition's
identity is the SHA-256 of its canonical form (local names erased,
references replaced by their targets' hashes). Resolution is pinned in
per-module sidecars (`<file>.rg.bind.json` — a lockfile for *meaning*).
Dynamism is quarantined: indirect calls require declared `dyn` sets;
string-keyed dispatch lives in `table` definitions, which are graph nodes.
Effects (`!io`) propagate over the exact graph. The name is Kripke's
(rigid designator). Author of record: David Dudas (decide "Dudas
Engineering, LLC" in PAPER.md author line before publishing).

## Non-negotiable working rules (established, repeatedly enforced)

1. **Measured, not asserted.** Every performance/behavior claim gets a
   number from a run, recorded in COMPLETENESS.md. Overclaims get
   corrected in writing (it has happened; David catches them).
2. **Everything demo'd ships.** Any feature shown must be a green step in
   `demo.sh`. Currently 34 steps; memo adds 35, temporal adds 36.
3. **Scope by ledger.** COMPLETENESS.md decides "done." v14 (memo +
   temporal) reopened it with its own bar; close the bar, stop.
4. **Search before novelty claims.** Prior-art search before claiming
   anything is new (early sessions went 0-for-4 without it).
5. **David's style:** terse, direct, ranked decisive recommendations,
   push back on over-engineering, correct errors explicitly. He runs
   everything on his own Mac (zsh; `sed -i ''` there vs `sed -i` in this
   Linux sandbox; he uses `--root` instead of cd; PATH export from repo).
6. **Honesty is the product.** Failed predictions, voided experiment
   items, and found defects are *recorded*, not smoothed over. This is
   the project's differentiator and David explicitly values it.

## Repo map

~7,700 lines Rust at repo root (this folder). `src/`: lexer.rs,
parser.rs, ast.rs, resolve.rs (workspace/bindings/effects/arity),
canon.rs (canonical encoding + hashing; tags 0x01–0x2B, append-only),
sidecar.rs (reconcile rules R1–R7), diag.rs (E0101, E0201–E0211, W0401,
W0403, W0404, I0402, I0405, I0406), fixes.rs (`fix --apply` typed-fix
loop), eval.rs (tree-walk + bytecode VM), backend_js.rs (JS target;
`$ffi` value-domain guard on extern returns), fmt.rs (comment-preserving,
hash-neutral), lsp.rs, infer.rs, main.rs (CLI arms incl. `describe`).
`examples/`: hello, data, flow, std, todo, triage, migration,
httpie-migration, mandel, interop (externs), press (4-module site
generator, 22 tests — the maintenance-battery subject). `study/`:
fetch.sh, analyze2.py, raw_results_v2.json. `experiments/`: legacy.js,
pipeline.rg, groundtruth.json (comprehension pilot). `tools/`:
difftest.sh (16 VM-vs-node behaviors, must stay 16/16), lsp_smoke.py.
`demo.sh` (34 green steps). Docs: SPEC.md, README.md, COMPLETENESS.md,
QUARANTINE.md, TUTORIAL.md, PAPER.md, STUDY.md, PREREGISTRATION.md,
DESIGNS.md (v14 specs).

Shipped artifacts in /mnt/user-data/outputs (if present): rigid-v13.zip,
rigid-site.html, PAPER.md, STUDY.md, comprehension-experiment.html,
press-spec.json, press-sample.html.

## Verified state (v13)

- Sidecar reconciliation R1–R7. R2: silent rebind refused (E0202 names
  both hashes). R4: cross-module rename recovered by identity. R7: a
  module with unresolved refs keeps its sidecar untouched (rewriting it
  destroys rename evidence). I0406: same-module rename recovery.
- Test caching is a proof (hash covers the dependency cone). Quarantine:
  E0204 + I0405/W0403 inference with applyable fixes; sidecar pins
  (W0404, `--frozen`).
- Externs: `extern fn name(a,b) [!io] = "host.path";` — arity checked
  (E0209), effects propagate (E0207), declaration content-addressed
  (host-path drift = W0401), VM refuses with directive, JS target binds;
  every extern return passes `$ffi` (ints/strings/lists/records pass,
  bools→1/0, floats/null/objects refused loudly naming the extern).
- `rigid describe`: whole workspace as JSON — per def: hash, kind,
  arity, io, exact deps/rdeps, **precomputed transitive `reach`**, table
  entries, dyn sets, extern host, `covered_by` (transitive test
  coverage); plus module hashes, boundary list, stats.
- Numbers on record: 3,000 modules / 6,000 defs check in 477ms cold /
  353ms warm (~linear). fib(30): VM 433ms vs node 49ms (~9×; JS target
  is the perf answer). Study (pre-registered, David replicated exactly):
  430 dispatch tables over 5 Python projects — median width 1, p90 5,
  88.8% ≤4 targets, 3.0% >10, max 602 (pygments).
- Comprehension pilot (pre-registered, 5 runs, fresh sonnet via
  keyless API *from an artifact page only* — bash curl gets 401):
  A convoluted JS ≈2/5, B rigid source 4/5 every run, C rigid+spec 5/5
  on the repaired instrument; ordering C≥B>A held on every valid
  comparison. Recorded defects: run-1 harness bug (prose-wrapped JSON),
  one authored contradictory item (voided, repaired), one repair
  hypothesis pre-registered and falsified. All documented on the site.

## v14 — COMPLETE (both features verified)

memo: E0211 purity gate; persistent VM store (.rigid-memo.json) keyed by
def-hash + canonical args; bytecode CALL routes memo defs through the
full path; JS in-run Map; fmt prints the keyword; hash unaffected by the
flag. Measured 360ms cold / 2ms cross-process hit / recompute on edit.
Temporal: append-only .rigid-store written on green check; @"prefix"(args)
+ `rigid recall`; effect-checked; JS refuses at runtime. Demo steps 35-36.
Note for future sessions: post-compaction work DUPLICATED half-built
features already in the repo (memo eval scaffolding, entire temporal
syntax path) — inventory `grep -rn <feature>` across src/ BEFORE
implementing anything this file claims is TODO.

Next real work is not code: publish (see below). Candidate v15 items
live in DESIGNS.md (provenance stamps) — only on demand.

## Environment traps (each already cost real time — read this)

- **Python heredoc + Rust `\u{...}`**: `\u` inside a python triple-quoted
  patch string kills the whole script at parse time (nothing runs, files
  untouched). Use ASCII in Rust strings you write via python, or `\\u`.
- **`serde_json::json!` eats blocks**: `{ let ... }` after a `:` inside
  `json!` parses as a JSON object → bizarre errors ("comparison
  operators cannot be chained"). Compute into a local *before* the
  macro; reference the local.
- **Silent `str.replace` no-ops**: python patches that don't match do
  nothing and report nothing. After every patch, grep/assert the anchor
  landed before building.
- **`grep -c` in `&&` chains**: zero matches → exit 1 → chain dies.
- **Stale binaries**: a failed `cargo build` leaves the previous binary;
  a "successful" run may be the old code. Check `errs:` count first.
- **`||` short-circuit illusions**: `cmd >/dev/null || cmd` shows
  nothing when the first succeeds — looked like a silent failure once.
- **macOS vs Linux sed**: David needs `sed -i ''`; sandbox uses `sed -i`.
- **Keyless Anthropic API**: works only from artifact pages
  (fetch in-browser), NOT from sandbox curl (401). Experiments that need
  a fresh model must ship as self-running HTML artifacts.
- **fmt is destructive to unknown syntax**: any new top-level form must
  be added to fmt.rs *in the same pass* or `fmt --write` erases it.

## Standing recommendation to David (restate when natural, don't nag)

Publish: PAPER.md → Zenodo under his ORCID (0009-0000-9348-5438), STUDY
+ PREREGISTRATION as supplements, zip as artifact — an afternoon, and
the only step that produces information the project can't produce for
itself. Decide the Dudas Engineering author line. Host the site (swap
`unzip rigid.zip` for the real repo URL; add localStorage theme
persistence, deliberately omitted in preview). After v14: stop
deliberately; further passes should answer users or reviewers, not
momentum.
