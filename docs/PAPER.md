# Gradual Reference Exactness: Quarantined Dynamism in a Text-First, Content-Addressed Language

**David Dudas**
*The system is called **rigid**, after Kripke's rigid designator [7]:
a term that denotes the same object in every possible world, which is what a
content hash does for a definition.*

## Abstract

Large codebases bind by name and dispatch by string, so the question "what
breaks if I change this?" is answered by search, approximation, and
production incidents. Systems that fix this by making the reference graph
primary — content-addressed languages such as Unison — pay for exactness
with a wholesale change of medium: code leaves text files, version control,
and mainstream syntax. We present a different design point: **text remains
the authoritative medium; an exact reference graph is derived from a
canonical form; and every construct that would make the graph inexact is
quarantined behind a declaration of its possible targets.** A per-module
*binding sidecar* — a resolution lockfile — lets the compiler distinguish a
definition being *edited* from a name silently changing its *meaning*, and
refuse the latter. Dynamism enters through two constructs, first-class
references invoked inside declared-set `dyn` blocks and string-keyed
dispatch tables whose value sets are closed, both of which keep blast-radius
queries and effect checking sound as over-approximations. We propose
**gradual reference exactness** — the fraction of call edges that are
singletons — as the migration metric, by analogy to gradual typing. A
7,700-line Rust implementation realizes the design end to end: programs
run; renames are detected by identity and repaired mechanically; test
caching is sound by construction; every claim in this paper is executable as a
script, and programs compile to JavaScript so they run outside the system
that defines them.

## 1. Introduction

Two facts hold simultaneously in most production software. First, nearly
every reference is a late-bound name: an import, a string key into a
handler table, a symbol resolved at load time. Second, the questions that
dominate maintenance — who depends on this, what breaks if I delete it, did
this rename reach every call site — are questions about the reference
graph. The industry's answer is to reconstruct that graph after the fact:
grep, IDE indexes, and fact-extraction systems such as Kythe, Glean, and
CodeQL [4]. Reconstruction is necessarily approximate over dynamic
features, and approximate blast radius fails in exactly the cases that
matter.

The known alternative makes the graph primary. Unison identifies each
definition by a hash of its abstract syntax tree, stores code in a
database, and treats names as metadata; renames cannot break anything and
builds disappear [1]. The cost is the medium: text files, Git, code
review, and mainstream syntax are replaced wholesale. A decade after the
idea became publicly admired, adoption remains niche, and we take the
bundle — not the idea — to be the cause. Scrapscript explores the same
identity model at small scale, also within a pure-functional surface [2].
Most recently, Zero (Vercel Labs, May 2026) redesigned the *toolchain
surface* for machine consumers: structured diagnostics with stable codes
and typed repair plans, a `graph` command emitting modules, imports,
public symbols, capabilities and effects as JSON, and capability-based IO
in which effectful functions take an explicit `World` parameter [3]. Zero
is the nearest neighbour to this work in intent, and its effect discipline
is comparable to the one described here. The difference is the granularity
and the guarantee: Zero's graph facts are module- and import-level over a
conventional name-based identity model, so at the level of individual call
sites through dynamic dispatch the same reconstruction gap remains. This
paper's contribution is at that level — per-site target sets that bound
what may be invoked, over definitions identified by content rather than
name.

This paper occupies the cell those systems leave open: **text-canonical,
content-addressed identity, open-world, with machine consumers as
first-class readers of every diagnostic.** Contributions:

1. **A text-first content-addressing design** in which identity is the
   SHA-256 of a canonical form that erases local names and replaces
   references with the hashes of their targets, while source files, Git,
   and a mainstream surface syntax are retained (§2).
2. **The binding sidecar**, a per-module resolution lockfile, with six
   reconciliation rules that separate content drift from meaning change,
   yielding rename detection by identity, mechanical repair, and a hard
   refusal of silent rebinding (§3).
3. **Quarantined dynamism**: first-class references and string-keyed
   dispatch that preserve an exact-as-over-approximation graph, with four
   guarantees and a soundness sketch; declared sets simultaneously bound
   call targets and effects (§4).
4. **Gradual reference exactness** as a migration metric and research
   framing (§5).
5. **An end-to-end implementation** whose claims are executable, including
   test caching that is sound by construction (§6).

## 2. Text-first content addressing

A program is a directory of source files; there is no manifest and no
build step. Each top-level definition's identity is the SHA-256 of its
*canonical form*: a tagged byte encoding of its syntax tree in which (i)
local names are erased and replaced by binding-order slot indices, (ii)
every reference to another definition is replaced by that definition's
32-byte hash, (iii) direct self-reference is a fixed tag, and (iv)
mutually recursive components are hashed as a unit with name-free member
ordering, in the manner of Unison [1]. Record literals are encoded with
fields sorted by name, so two differently-ordered literals of the same
record are one identity — field order, like naming, is presentation.

Consequences, all mechanically demonstrated in the implementation:
renaming a parameter, a definition, or anything a definition references
does not change its hash; editing structure, literals, or the content of
anything it transitively references does. A hash therefore pins an exact
implementation together with its entire dependency cone, which is the
property that later makes caching and drift detection sound.

Whitespace and comments never reach identity. A canonical formatter emits
exactly one rendering per syntax tree — comments preserved by
line-association — so formatting a file is provably hash-neutral.

## 3. The binding sidecar

Content addressing answers "what is this definition"; it does not by
itself answer "what did this *name* mean here yesterday." That is the job
of the sidecar: a per-module JSON lockfile, written by the compiler and
committed like `Cargo.lock`, mapping each imported name to the hash of the
definition it was bound to when last accepted.

Reconciliation compares each pin against the current target module:

| rule | condition | outcome |
|---|---|---|
| R1 | name present, hash matches pin | accepted |
| R2 | name present, hash differs, **pinned hash exists under another name** | **error: resolution changed** — the name was re-used for a different definition; the compiler refuses to silently rebind and offers two typed fixes (rename the reference, or explicitly re-pin) |
| R3 | name present, hash differs, pinned hash gone | warning: content drift; re-pin (an error under a frozen CI mode) |
| R4 | name absent, **pinned hash exists under another name** | error with a mechanical `rename_reference` fix: the definition was renamed, recognized *by identity* |
| R5 | name absent, hash gone | plain unresolved |
| R6 | no pin yet | pin now |

R2 and R4 are the design's center. R4 turns the classic missed-call-site
rename from a silent break into a structured error carrying a
machine-applicable edit; the implementation's `fix --apply` command
iterates such fixes to convergence, closing a repair loop in which the
diagnostic — JSON, stable code, typed fix, following the convention Zero
established [3] — is itself the patch instruction. R2 addresses the dual
failure that name-bound languages cannot even express: re-introducing an
old name for a *different* definition quietly changes the meaning of every
existing reference; here it is a hard error naming both identities.

## 4. Quarantined dynamism

Indirect calls destroy exact graphs, and prior systems choose a pole:
total dynamism with post-hoc approximate analysis, or no first-class
functions at all. We claim the middle point is not "some approximation"
but **declared over-approximation**.

Two constructs. `&name` creates a first-class reference; the creation site
is an ordinary graph edge, so a reference cannot be manufactured without
the compiler seeing its target. References flow freely but are inert:
invocation happens only inside `dyn { t1, …, tn }(callee, args…)`, whose
declared names resolve at compile time (each becoming an edge at the site)
and against which the callee's identity is membership-checked at runtime,
with violations reported by hash. The data-keyed form is the **dispatch
table**: `table routes { "created" -> on_created, … }`, itself a
definition with its own hash and sidecar pins, dispatched as
`routes[key](args…)`. Keys are runtime data; the value set is closed. The
dispatch site's edge is the table; the table's edges are its value set, so
blast radius flows through in two exact hops — matching the topology of
the enterprise systems (string-keyed rules, handlers, and policies) that
motivated the design.

Guarantees: **G1 (graph totality)** every definition a site can invoke
appears as a static edge there; **G2 (blast-radius soundness)** reverse
dependencies are a superset of true dynamic callers; **G3 (effect bound)**
a site's effect is the union over its declared set — the same declaration
bounds targets and effects, which is how IO enters the language (`print`,
`input`, `read_file`, `write_file` require an `!io` annotation whose
propagation is checked in one pass over the exact graph); **G4 (no ambient
escape)** an escaped reference cannot smuggle a call. Soundness is an
induction on evaluation: only direct calls (single static target) and
membership-checked dyn/table dispatch transfer control, so every runtime
call edge lies in the static edge set. Arity is checked at compile time at
every call form — direct calls, each dyn target, each table entry — as a
deliberate first step of typing chosen because it narrows the quarantine.

## 5. Gradual reference exactness

Gradual typing gave migration a unit: the typed fraction of a program, and
a boundary discipline between typed and untyped regions [5]. We propose
the transposition. Define a program's **reference exactness** as the
fraction of call edges that are singletons; `dyn` sets and table value
sets are the declared boundary regions; migration means shrinking sets,
and the limit is a fully singleton graph. The metric is computed by the
compiler itself (`exactness`, human and JSON output): each direct call
contributes a singleton site; each `dyn` site contributes its declared-set
width; each table dispatch contributes its table's value-set width. The
framing yields concrete questions we consider open and tractable: how set widths distribute in
real migrated codebases; whether review pressure keeps declared sets
honest or they decay toward "declared everything"; what lint and
inference-as-a-fix support (the compiler proposing minimal sets as typed
fixes, never applying them silently) does to that decay; and whether
data-driven tables in enterprise systems, once made closed-set, cluster
into a small number of reusable shapes.

## 6. Implementation and evaluation

The implementation is ~7,700 lines of Rust: lexer, recursive-descent
parser, resolver, canonicalizer/hasher, sidecar engine, a bytecode VM
(each definition compiled once to a stack machine), test runner,
formatter, REPL, a language server that answers definition and reference
queries directly from the resolver's bindings — so references are exact
through dispatch tables and declared sets — a JavaScript backend, and a
CLI whose diagnostics are stable-coded JSON with typed fixes. A funref
may-reach analysis proposes minimal quarantine sets as applyable fixes;
the declaration in source remains authoritative. The language surface includes
integers, strings, lists, records, first-class references, tables,
mutation and `while`, errors-as-values (`fail` raises any value; `try`
returns an ordinary `{ok, value|error}` record and catches builtin
runtime errors uniformly), a small string/number stdlib, console and file
IO behind `!io`, and a REPL that evaluates against a live workspace by
injecting a synthetic definition through the ordinary pipeline.

Evaluation is by executable demonstration: a script of twenty-five steps
exercises every claim above, including (a) byte-identical hashes across
definition and parameter renames; (b) rename detection with mechanical
repair converging to a green check; (c) the silent-rebind refusal; (d)
two-hop blast radius through a dispatch table; (e) effect errors through
tables and dyn sets; (f) **sound test caching** — a test's hash pins its
dependency cone, so an unchanged hash *proves* a rerun identical; editing
one definition re-executes exactly its downstream tests, demonstrated
live; and (g) hash-neutral, comment-preserving formatting. Eight programs are written in the language, including an
incident-triage pipeline, a persistent todo CLI, and a fixed-point
Mandelbrot renderer with an animated zoom. Programs compile to a single
JavaScript module; a differential suite runs sixteen behaviours on both
the native VM and Node and requires character-identical output, error
messages included, so the quarantine's runtime refusal survives
compilation.

As a first measurement, we migrated a miniature platform application
written in the registry style that motivated the work — a mutable
string-keyed handler map populated at load time and dispatched on
`record.class + "." + action` (the legacy source ships alongside the
migrated code). In the original, dispatch width is *unmeasurable*: the
registry is runtime state. The migrated program measures **E = 0.833**
(12 call sites, 10 singleton), with all non-exactness localized in two
declared sites of known width — one `dyn` set of 2 and one lifecycle
table of 4 — while its pure core, like the pure iteration example
(**E = 1.000**, 8/8), is fully singleton; the dogfooded triage pipeline
measures **E = 0.882** (17 sites, 15 singleton; widths 2 and 3). The
absolute numbers are small; their value is that the quantity exists, is
computed from the compiler's own resolution rather than reconstruction,
and localizes a program's dynamism to a handful of reviewable widths.

**Dispatch widths in real code.** The metric's central empirical question
— whether declared sets stay narrow — was tested against five widely-used
Python projects with plugin/registry architectures (flask, scrapy, celery,
pygments, httpie; 983 source files), selection and method pre-registered
before any measurement. Pooling 430 dispatch tables and registries: median
width 1, p90 5, p95 9, max 602; **88.8% have at most four targets and 3.0%
exceed ten**. The tail is concentrated in catalogue-shaped registries
(pygments' 602 lexers) where a wide set is honest rather than a modelling
failure. Separately, 2.05% of non-method call sites resolve their target
from a runtime string (`import_string`, `load_object`, computed
`getattr`) and are unbounded in the original source — few in number, and
sitting exactly at the architectural seams where blast-radius questions
matter most. As a ground-truth check, httpie's formatter and auth
registries were migrated by hand: the analyzer predicted widths 4 and 3,
and `rigid exactness` on the migrated module reports 8 call sites, 6
singleton, two tables of width 4 and 3, E = 0.750 — agreement between the
external analysis and the language's own measurement. Full protocol,
deviations (including one discarded run), and threats to validity:
STUDY.md.

**Honest limits.** This is single-author dogfooding on small programs;
there is no user study, no measurement beyond the three small programs
above, and no performance evaluation (the evaluator is a tree-walker).
The evaluation shows the design is *realizable and internally sound*, not
that it is adoptable; §5's empirical questions are precisely what a real
evaluation would measure.

## 7. Related work

**Content-addressed code.** Unison is the reference point: definitions
identified by AST hash, code in a database, names as metadata, no builds,
non-breaking renames [1]; Scrapscript is a smaller exploration of the
same identity idea [2]. Both make the graph the medium; we derive the
graph from canonical text and add the open-world machinery (sidecars,
quarantine) that a closed world does not need. **Agent-native toolchains.**
Zero restructures the whole compiler surface for machine consumers: stable
diagnostic codes, typed repair plans, JSON graph facts (modules, imports,
public symbols, capabilities, effects, ownership), and capability-based IO
[3]. We adopt its diagnostics-as-data convention wholesale, and differ
beneath it: identity by content rather than name, and bounded target sets
at each dynamic call site rather than module-level import edges. Zero's
capability parameters and this system's `!io` annotation are convergent
solutions to the same effect problem; the claim here is not novelty in
effects but that declared dispatch sets bound *which definitions* may run
at a site, which is what makes the effect check trivial over an exact
graph.
**Reconstructed graphs.** Kythe, Glean, and CodeQL extract reference facts
from name-based code post hoc [4]; their graphs are approximate over
dynamism by construction, which is the property we remove. **Gradual
typing** supplies the migration framing we transpose [5]. **Effect and
capability systems** type what code may do [6]; G3 additionally bounds
*which definitions* may do it at each site, and Rust's `unsafe` supplies
the ergonomic model of a scoped, auditable admission — though `dyn`
suspends nothing, it adds a bound. The philosophical naming is Kripke's:
a hash-pinned reference is a rigid designator [7].

## 8. Limitations and future work

Closures exist, and the identity question this section once called
unresolved was answered by lifting: each `fn(x) { ... }` becomes a
top-level definition named `<enclosing>$<n>` whose leading parameters are
the variables it captures, so a lambda has an ordinary content hash and
`rdeps` reaches through it. What remains is ergonomic rather than
foundational — invoking a closure still goes through a declared `dyn`
set, so a generic higher-order function cannot yet live in a shared
library, because its set would have to name lambdas in the caller's
workspace. Inferring that set and pinning it, the treatment tables and
dyn targets already get, is the open item.

Declared sets may decay wide; named tables mitigate, and
inference-as-a-fix plus lints are the pressure. Module-level content
addressing, the bytecode VM, and an LSP served from the graph have all
shipped since this section was first written, as has sidecar pinning of
dyn-set and table membership. What has not is the actual test:
measurement on a migrated real codebase in the enterprise domain that
motivated the work. Two smaller measurements on foreign code now exist
and are reported in EVIDENCE.md.

## 9. Conclusion

The dichotomy between late-bound names and knowable dependency graphs is
false. Keeping text as the medium, deriving identity from a canonical
form, pinning meaning in a lockfile, and quarantining dynamism behind
declared sets yields a language in which renames are recognized by
identity, silent rebinding is a refused compile-time event, blast radius
and effects are exact over-approximations even through first-class
functions and string dispatch, and test caching is a proof rather than a
heuristic. The open problem worth the community's attention is the metric:
what programs look like as reference exactness approaches one, and what it
costs to get there.

## References

[1] Unison Computing. *The Unison language: the big idea.*
unison-lang.org/docs/the-big-idea; Unison 1.0 announcement, Nov 2025.
[2] T. Troesh et al. *Scrapscript: a functional, content-addressable
programming language.* scrapscript.org; github.com/tekknolagi/scrapscript.
[3] Vercel Labs. *zerolang: the programming language for agents.*
github.com/vercel-labs/zerolang; zerolang.ai. Released May 2026, v0.1.3
(19 May 2026), Apache-2.0. Structured diagnostics with stable codes,
`zero fix --plan --json` typed repair plans, `zero graph --json` module
and import facts, capability-based IO via an explicit `World` parameter.
Repository description and command contracts verified 4 August 2026.
[4] Google Kythe (kythe.io); Meta Glean; GitHub CodeQL — post-hoc code
fact extraction and querying.
[5] J. Siek and W. Taha. *Gradual typing for functional languages.*
Scheme and Functional Programming Workshop, 2006.
[6] D. Leijen. *Koka: programming with row-polymorphic effect types*;
Unison abilities documentation.
[7] S. Kripke. *Naming and Necessity.* Harvard University Press, 1980.
