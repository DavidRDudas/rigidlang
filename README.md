# rigid v1.8.1

**A text-first, content-addressed language kernel.** Text is authoritative
for content; the derived graph is authoritative for identity; ambiguity is a
compile error.

Every definition's identity is the SHA-256 of its canonical form — local
names erased, references replaced by the hashes of what they reference. Names
are metadata. The compiler therefore maintains an *exact* dependency graph as
a byproduct of compilation, refuses to let a name silently change its
meaning, and recognizes renamed definitions by identity.

## Positioning in one breath

- **Unison** made the graph primary and demoted text — database codebase,
  bespoke tooling, pure FP. The idea is right; the conversion cost is the
  adoption ceiling. rigid keeps files, Git, diffs, and mainstream-looking
  syntax, and derives the graph from canonical text.
- **Zero** (Vercel Labs) made compiler *output* agent-native — structured
  JSON diagnostics, repair hints — on top of C-style name-based identity.
  rigid adopts that output convention and fixes the layer underneath: agents
  don't need better reports about an approximate graph; they need a graph
  that cannot be wrong.
- **CodeQL / Glean / Kythe** reconstruct reference graphs after the fact,
  approximately. Here the graph is exact by construction — it's what the
  compiler resolved, not what an analyzer guessed.

## Quickstart

New here? [docs/TUTORIAL.md](docs/TUTORIAL.md) is the ten-minute path: setup, first
program, and the rename/identity walkthrough, every command verified.
[docs/PAPER.md](docs/PAPER.md) is the workshop-paper draft (thesis, design, guarantees,
honest evaluation), and [docs/STUDY.md](docs/STUDY.md) reports the pre-registered
dispatch-width study over five real Python codebases.

```
cargo build
./demo.sh          # scripted walkthrough of every claim below, on a scratch copy
rigid watch        # re-check and re-run only the affected tests on every save
```

## What the demo proves (all verified, not aspirational)

1. **Pinning.** First `check` pins every imported reference by hash into a
   per-module sidecar (`math.rg.bind.json`) — a resolution lockfile.
2. **Identity survives local renames.** Rename a function's parameters;
   `rigid hash` is byte-identical. The name layer is not the identity layer.
3. **Rename a definition, miss a call site → structured error + typed fix.**
   The compiler finds the pinned identity under its new name and emits a
   mechanical `rename_reference` fix (E0201 + fix). Not a silent break.
4. **Re-use an old name for a different definition → E0202.** The silent
   rebind — the bug every name-bound language permits — is refused, with
   both remedies named.
5. **Blast radius is a query.** `rigid rdeps util.rg#clamp` answers "what
   breaks if I touch this" exactly, because the graph is the compiler's own
   resolution, not an approximation.
6. **Drift is visible.** Editing a dependency's content is a named event
   (W0401, repinned), and `--frozen` turns any drift into a CI failure.
7. **The repair loop closes.** `rigid fix --apply` takes the typed fix from
   (3), edits exactly the broken occurrences, and iterates until `check` is
   green — diagnostic in, converged source out, no human translation.
8. **Mutual recursion hashes.** SCCs hash as a unit ([docs/SPEC.md](docs/SPEC.md) §3.5).
9. **Programs run.** `rigid run math.rg#fib 20` → 6765. Tree-walking
   evaluator; Int/Str; first-class functions rejected pending quarantine.
10. **Content-addressed test caching.** `rigid test` caches by test hash;
    unchanged tests are *provably* identical reruns and skip; editing one
    definition re-executes exactly its downstream tests, nothing else.
11. **One true rendering.** Two arbitrarily-spaced sources of the same
    program: identical `fmt` output, identical hashes.
12. **Quarantined dynamism.** `&name` creates a first-class reference
    (a static edge); only `dyn { declared, set }(f, args)` can invoke one,
    membership-checked at runtime. A ref outside the set is refused by
    name *and hash* — and `rdeps` sees through the dynamism, listing the
    funref creator and every dyn site. Blast radius survives first-class
    functions. ([docs/QUARANTINE.md](docs/QUARANTINE.md))
13. **Effects travel the exact graph.** `print` requires `!io`; a pure
    function calling an `!io` function is a compile error (E0207); a dyn
    site's effect is the union of its declared set. IO enters through the
    quarantine door, not around it.
14. **Comment-preserving canonical formatting.** `fmt --write` rewrites to
    the one canonical form, comments reattached, hash byte-identical.
15. **Data-driven dispatch, graph intact.** `table handlers { "created" ->
    on_created, ... }`; `handlers[ev](p)` dispatches by runtime string. A
    missing key fails at the boundary listing the set; `rdeps` flows
    through the table in two exact hops (handler ← table ← dispatcher);
    a pure dispatcher of an io-reaching table is E0207. The string stays
    a string — the graph stays exact anyway.
16. **Tables are definitions.** Content-addressed (key-sorted entries over
    target hashes), sidecar-pinned when imported; editing one handler body
    changes the table's hash and every dispatcher's — the cone is real.
16a. **Arity, checked everywhere** — direct calls, every dyn target, every
    table entry (E0209). The first type in the language, admitted because
    it narrows the quarantine.
16b. **Dogfood.** `examples/triage/` is an incident-triage pipeline written
    *in* rigid — string-keyed routing, quarantined escalation, declared
    effects, a pure tested core — the language's origin story as its first
    program.
17. **Data structures** (v0.5): lists, records, indexing, field access,
    `len`/`push`/`input`, list concat — general programs, not just routing
    (examples/data). Record identity ignores field order: two
    differently-ordered literals hash identically.
18. **Mutation + `while`** (v0.6): assignment to locals, iteration, stable
    loop-local slots; assigning to a definition is E0210 — definitions
    stay immutable, which is what makes their hashes meaningful.
19. **Errors are values** (v0.6): `fail(v)` raises anything, `try(e)`
    catches everything — including builtin runtime errors — as an
    ordinary record. No exception machinery; the data machinery is the
    error machinery.
20. **Stdlib + file IO + REPL** (v0.7): `str`/`int`/`substr`/`chars`/`join`
    with catchable parse failures; `read_file`/`write_file` behind `!io`
    with catchable missing-file errors; `rigid repl` evaluates expressions
    and statements against the live workspace.
21. **The metric is measurable** (v0.8): `rigid exactness` computes the
    paper's reference-exactness score from the compiler's own resolution;
    `examples/migration/` is a legacy string-registry app and its migrated
    form side by side — unmeasurable before, E = 0.833 after, with all
    dynamism localized in two declared widths.
22. **Compiled execution** (v0.9): a bytecode VM with identical semantics —
    the entire demo is the regression suite — measured 2.1x over the
    tree-walker at fib(36).
23. **The compiler proposes the quarantine** (v1.0): may-reach inference
    turns an illegal indirect call into an inferred minimal `dyn` set
    (I0405) and widens provably-narrow sets (W0403) — both as typed fixes
    `fix --apply` performs. Sidecars now pin dyn/table targets (W0404
    drift, frozen-enforced), and modules have content identity: renaming
    or reordering definitions leaves a module's hash fixed.
24. **LSP over the graph** (v1.1): `rigid lsp` speaks JSON-RPC on stdio.
    Diagnostics are the real pipeline (sidecar rules included, read-only);
    references come from the resolver's bindings map, so they are exact —
    through dyn sets and table entries; hover shows a definition's content
    hash; quarantine fixes surface as code actions; an unsaved rename in
    one buffer produces a live identity-hint diagnostic in its importer.
    Verified by a scripted protocol client (six checks).
25. **A real utility ships with it**: `examples/todo/` is a persistent
    todo CLI — file storage, line parsing, table-dispatched commands,
    caught bad input — written in the language, tested, and the reason
    string escapes and pipe-safe output exist.
26. **And one for the demoscene**: `examples/mandel/` renders the
    Mandelbrot set in pure integer fixed-point — recursive escape-time
    core (tested), palettes dispatched through a closed table — and
    `zoom.rg` animates a terminal zoom into the seahorse valley, frames
    driven by recursion, targets dispatched through a table. Writing it
    is why `\e` escapes and raw string `print` exist.
27. **It compiles to JavaScript** (v1.2): `rigid build --target js` emits a
    single module that runs on Node or in a browser and can be `require`d
    like any other JS. The quarantine survives the trip — a dyn violation
    throws the same message in JS that rigid's VM prints. Verified by
    `tools/difftest.sh`, which runs 25 behaviors on both engines and
    requires character-identical output.
28. **Foreign functions are declared, not smuggled** (v1.3):
    `extern fn jmax(a, b) = "Math.max";` puts a host function in the graph
    as a boundary node — arity checked, effects propagated, the
    declaration itself content-addressed so host-path drift is caught by
    the sidecar. The VM refuses externs with a directive; the JS target
    binds them. Measured while we were at it: 3,000 modules check in
    under half a second, and compiled output runs ~9x faster on node than
    the VM.
29. **The architecture is a compile target too** (v1.3):
    `rigid describe` emits the entire program as one hash-anchored JSON
    spec — every definition with its identity, arity, effects, exact deps
    and rdeps, table topology, dyn sets, the extern boundary, and which
    tests cover it. An agent reads the spec instead of the source: smaller,
    unambiguous, and citable by hash, because it is the compiler's own
    graph serialized rather than a reconstruction.
30. **`memo`: incremental computation as a keyword** (v1.4): pure functions
    opt into a persistent cache keyed by their content hash plus canonical
    arguments — cross-process, cross-machine, stale-proof by construction.
    Delete the word `memo` from `fib(30)` and it takes 230ms; keep it and
    the first run is 4ms, because the recursion memoizes as it goes. Edit
    the body and it provably recomputes. Purity is checked (E0211), not
    assumed. Python decorators hope; Bazel approximates; rigid proves.
31. **Temporal references: the past is callable** (v1.4): every green
    `check` archives each definition by hash; `@"f2aeacb386b8"(x)` runs the
    version that hash names, next to today's — differential tests and
    canaries in one line. Git checks out files; rigid *names a prior
    meaning and calls it*. VM-only, `!io`-gated.
32. **A text diff, graded by identity** (v1.5): `rigid diff` compares two
    revisions as sets of hashes and answers two questions separately —
    did any definition change *meaning*, and did any name move. Rename a
    function, rename every parameter, reflow the body: *"3 definitions,
    0 changed meaning — provably semantics-preserving."* `tsc` cannot say
    that, because to it a rename **is** a change; Unison cannot, because it
    has no text diff to grade. `--require-no-semantic-change` makes a PR
    that claims to be a refactor prove it.
33. **Capabilities, pinned** (v1.5): `!io` split into a set —
    `!fs !net !env !time !rand !proc` — declared on `extern`, propagated
    along the exact graph, and *part of the hash*. The sidecar records the
    capability set at the moment of pinning, so a dependency that starts
    reaching the network is `W0405` and, under `--frozen`, a build failure.
    Supply-chain compromise as a compile error rather than an audit
    finding. `rigid caps` lists the whole surface, complete by
    construction rather than by search.
34. **Floats** (v1.5): `3.14`, `float(n)`, mixed arithmetic that promotes,
    exact comparison across `1 == 1.0`. Two integers are never widened
    silently. Canonical form merges every NaN into one quiet NaN, which no
    program can tell apart, and keeps `-0.0` apart from `+0.0`, which
    `1.0 / x` can — the rule that keeps a `memo` key from answering for
    the wrong zero.
35. **Merging renames** (v1.5): `rigid merge` pairs definitions across
    three revisions by identity first and name second, then resolves name
    and body *independently*. You renamed `clamp` to `clip`; they changed
    what it does. Git calls that a conflict — same lines, both sides.
    rigid takes both, rewrites the references, and the merged workspace
    checks clean. No language whose identity is its name can do this,
    because there the two edits really are the same edit.
36. **Two number domains survive the trip to JavaScript** (v1.5). Adding
    floats exposed an older bug: JS has one number type, so the backend was
    using a float64 for rigid's 64-bit integers, and node answered
    `9007199254740993 + 1` with `9007199254740992` — wrong, and unnoticed
    because no differential case used a large number. `Int` now compiles to
    **BigInt** and `Float` to a plain number. `tools/difftest.sh` grew from
    16 cases to **25** (33 today), covering both domains, and all 25 match. Crossing
    out to a host function narrows an `Int` only when it fits; above 2^53
    the call is refused by name rather than truncated. Compiled output is
    still ~9x faster than the VM (fib(34): 1568ms vs 163ms).

37. **Control flow that reads like code** (v1.6): `if` statements with
    `else if` chains, and `for (x in xs)` over a list or a string. Both are
    real nodes with their own canonical tags rather than sugar, so `fmt`
    gives back what you wrote. Renaming a loop variable moves no hash.
38. **Content-addressed closures** (v1.6): `fn(x) { ... }` is lifted to a
    real definition — hash, graph edges, `rdeps`, quarantine and all — whose
    leading parameters are the locals it captures. Anonymous functions are
    the standard place a reference graph stops being exact; here `rdeps` on
    a lambda still lists every dyn site that can invoke it, and renaming a
    captured local still moves no hash. Unison gets this by construction
    from a graph-primary codebase; this gets it from text.
39. **A standard library, written in rigid** (v1.6): `std/list.rg`,
    `std/str.rg`, `std/math.rg` — 77 definitions, 31 tests, imported by
    path and loaded only when reached. Writing it found two real gaps in
    the language, both now fixed: strings could not be compared with `<`,
    and standard-library modules were writing sidecars into the importing
    workspace.
40. **Numbers from other people's code** (v1.6): `docs/EVIDENCE.md`, two
    studies on artifacts nobody here controls.
    *Capabilities:* across 299 real npm packages and 3,533 consecutive
    version upgrades, a **patch release widens its capability set 0.5% of
    the time** — about one in 190. That is the property `W0405` needs to be
    worth having: quiet through ordinary churn, so a widening carries
    information.
    *Renames:* across 22,583 commits in ten Python projects, **8 of 458
    renames shipped a dangling reference (1.7%)** — including black's
    commit literally titled "Rename FileMode into just Mode", after which
    `blackd.py` still called `black.FileMode(`. A low rate, in ten of the
    best-maintained projects in the ecosystem, and every one of the eight
    is a case this compiler turns into an error at the moment of the
    rename. Every previous number in this repository came from a corpus
    written by its own author.

41. **Evidence bound to identity** (v1.7): `rigid attest api.rg#handler
    --kind reviewed`. A claim — reviewed, audited, benchmarked, generated —
    binds to a definition's hash, and because a hash covers the whole
    dependency cone, that review covers everything the definition reaches.
    Edit something four calls away and the review is *void*, automatically.
    Rename anything in the cone and it is *not*, because a rename moves no
    hash. Every code-review system in existence binds approval to a diff, so
    it survives a rebase that changed the logic and dies on a rename that
    changed nothing; this is the inverse, and it is the guarantee review has
    always claimed. `rigid trust` shows live and stale claims;
    `rigid check --require reviewed` is the CI gate. Stale claims are kept,
    because *unreviewed* and *stale* are different facts.
42. **Provable dead code** (v1.7): `rigid dead` lists what nothing can
    reach — a proof from the compiler's own resolution, not a search.
    It separates the definitions nothing references at all, each a real
    decision, from everything that merely hangs beneath them.

43. **Declared relationships, enforced exactly** (v1.8.1): `rigid.rules`
    states what the graph may look like — `deny domain/** -> http/**`,
    `deny parse.rg#untrusted -> !fs, !net`, `require test handlers.rg#**`,
    `require attest reviewed api.rg#**`. Every large codebase has rules like
    these and enforces them with import linters that approximate the call
    graph. Here the graph *is* the compiler's resolution, so the answer is a
    proof and the error names the path: `total -> ship`. An agent that wires
    the domain layer to the transport layer fails `check` on the spot.
44. **`rigid dup`** (v1.8.1): two definitions with the same canonical form
    have the same hash — not similar, *identical*. Catches the most common
    machine-authored regression, reimplementing something that already
    exists, and it composes: a review attested about one covers the other,
    because it is the same identity.
45. **Properties** (v1.8.1): `property fn sort_preserves_length(xs: list)` is
    a claim about every input, checked over 200 generated cases, with any
    failure shrunk to its simplest counterexample. Generation is seeded from
    the property's own hash, so results cache like tests and failures
    reproduce exactly. **On its first run it found a real bug in this
    repository's own standard library** — `abs(-2^63)` returned a negative
    number, because -(-2^63) is -2^63 — now refused, with the property that
    caught it checked in. 18 properties ship in `std/`.

## Performance and caches

Three caches, all keyed by content hash and therefore machine-independent:
`.rigid-testcache.json` (test results), `.rigid-memo.json` (`memo` results),
and `.rigid-checkcache.json` (per-module fingerprints, letting `check` skip
sidecar reconciliation for modules whose own and imported hashes are
unchanged). `--frozen` never reads the check cache.

```
rigid watch                  # re-check on save, re-run only the affected tests
rigid why math.rg#clamp      # what gets a new hash, which tests re-execute, and the path
rigid diff --against ../old  # what changed meaning, versus what only changed name
rigid caps                   # what reaches the outside world, and through which extern
RIGID_CACHE_DIR=/shared      # share the test and memo caches between CI and a working copy
RIGID_TIMING=1               # report where `check` spends its time
```

Because the cache keys cover the whole dependency cone, a result computed
anywhere is valid everywhere: a second checkout pointed at the same
`RIGID_CACHE_DIR` skips all 22 tests of `examples/press` without running one.
Incremental `check` on a 2,000-module workspace cuts reconciliation from
142ms to 6.8ms, and the whole `check` from 230ms to 89ms.

## Layout

```
src/lexer.rs      tokens with spans
src/parser.rs     recursive descent over the tiny core grammar
src/resolve.rs    workspace load, name resolution, collisions, cycles, topo
src/canon.rs      canonical bytes + SHA-256 identity
src/sidecar.rs    the resolution lockfile and reconciliation rules R1–R7
src/graph.rs      deps / rdeps, test coverage, blast radius, `why` paths
src/semdiff.rs    two revisions as sets of identities: meaning vs. surface
src/merge.rs      three-way merge over identities, not lines
src/lift.rs       lambda lifting: every closure becomes a real definition
src/attest.rs     claims bound to identity: live while the hash is
std/              the standard library, written in rigid
evidence/         measurements on code this project does not control
src/infer.rs      may-reach inference for the quarantine
src/domains.rs    value-domain inference (E0212)
src/eval.rs       evaluator, VM, content-addressed test and memo caches
src/backend_js.rs the JavaScript target
src/lsp.rs        JSON-RPC language server over the graph
src/wasm.rs       dependency-free browser entry point
src/lib.rs        the shared front-end pass list (CLI and wasm agree by construction)
src/fixes.rs      fix --apply / rebind
src/fmt.rs        canonical rendering (comment-preserving)
src/diag.rs       stable-coded, JSON-emitting diagnostics with typed fixes
tests/soundness.rs the guarantees, checked over generated programs
docs/SPEC.md           grammar, canonical form, sidecar semantics, roadmap
docs/QUARANTINE.md     design doc: dyn, guarantees G1–G4, gradual reference exactness
demo.sh           the walkthrough
bench/run.py      the breakage benchmark (rigid vs tsc)
site/             the website (see below)
```

## Site

`site/` is a static site with no build dependencies. `index.html`,
`learn.html` (a full language tutorial), `play.html`, and `spec.html` are
written by hand; `paper.html`, `study.html`, `quarantine.html`,
`benchmark.html`, `experiment.html`, and `prereg.html` are generated from the
Markdown at the repo root. After editing docs/PAPER.md, docs/STUDY.md, docs/QUARANTINE.md,
docs/BENCHMARK.md, docs/EXPERIMENT.md, or docs/PREREGISTRATION.md, run:

```
python3 site/build.py
```

Standard library only — nothing to install. `style.css` and `site.js` are
shared by every page; bump `ASSET_V` in `site/build.py` and the `?v=` query
in the hand-written pages together when you change them. `build.py` also owns
the nav and footer for *every* page, so edit `NAV`/`FOOTER_LINKS` there rather
than in any individual page.

### Playground

`site/play.html` runs the real compiler and VM in the browser: the crate is
built to WebAssembly (`src/wasm.rs`, a dependency-free `extern "C"` entry
point) and driven from a Web Worker, so a runaway `while` loop can be
terminated without freezing the page. Rebuild the module after changing any
Rust:

```
rustup target add wasm32-unknown-unknown   # once
./build-wasm.sh
```

That writes `site/rigid.wasm` (~712 KB, ~220 KB gzipped), which is committed
so the static site works with no build step. The page must be served over
http — opening it from disk blocks loading the module.

## Guarantees, and how they are checked

rigid's tools stand on three claims, and `tests/soundness.rs` checks all
three on every CI run over thousands of generated programs:

1. **Same hash, same behaviour.** Renaming every definition, parameter and
   local, shuffling record fields and table entries, reordering definitions
   and reformatting must leave every hash *and* every result unchanged —
   because test caching, `memo`, `attest`, `dup` and
   `diff --require-no-semantic-change` all assume it.
   ([docs/SPEC.md](docs/SPEC.md) §3.4 lists the runtime rules that make it
   true.)
2. **One meaning per program.** The VM and the JavaScript backend produce
   the same values and the same errors, messages included.
3. **No crash.** Every input ends in a value or an error: integers are
   checked, nesting and call depth are bounded, and nothing panics.

Each bug that once broke one of these has its own regression test there:
record fields that hashed in one order and ran in another; a `memo` cache
that answered `+inf` for `-0.0`; integers that wrapped in the VM, grew without
bound in JavaScript, and panicked on `min / -1`; names leaking into runtime
values; recursion that aborted the process; `isqrt` returning a negative
number at the largest integer.

These are tests, not proofs: they can find a counterexample, not rule every
one out. Machine-checked proofs of the core (a Lean model of evaluation and
canonical form) are the next step.

## Status

v1.8.1: 48 verified demo steps (diff, capabilities, floats and merge are steps 37–40), and 33 differential cases proving the VM and the JavaScript target agree character-for-character.
Programs run outside rigid, and can now call the world they land in.
**Engineering status: finished.** The completeness bar is reached, the design
roadmap is closed, execution is compiled, the paper's metric is measurable,
and the editor tier is served directly from the graph. What remains is
classified — deliberately — as usage-driven
([docs/COMPLETENESS.md](docs/COMPLETENESS.md)). **The completeness ledger's gating bar
is reached** ([docs/COMPLETENESS.md](docs/COMPLETENESS.md) §Verdict): every Lox-class
box checked, plus everything the language exists for. Post-bar work —
performance, LSP, richer effects, users — is listed and labeled as what it is.

## Name

"rigid", after Kripke's *rigid designator* — a reference that picks out the
same thing in every possible world. Which is what a hash-pinned reference is.

## License

Mozilla Public License 2.0 — see [LICENSE](LICENSE).

File-level copyleft: modifications to rigid's own files must be published,
and you may combine it with proprietary code freely. Programs compiled *by*
rigid carry no obligation at all — see [LICENSE-EXCEPTION](LICENSE-EXCEPTION),
which says so explicitly so nobody has to reason about it.

Copyright (c) 2026 David Dudas.
