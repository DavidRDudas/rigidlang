# Quarantined Dynamism

Design document for `dyn` — how first-class functions and effects enter a
language whose reference graph must stay exact. This is the research core of
the project: every other feature is engineering; this one is a position.

## 1. The problem

Indirect calls destroy exact reference graphs. The moment `f(x)` can mean
"call whatever `f` holds," the compiler no longer knows the call site's
targets, `rdeps` becomes an approximation, blast radius becomes a guess, and
every downstream guarantee (test-cache soundness, effect checking, safe
rename) quietly degrades to heuristics. Existing systems pick a pole:

- **Total dynamism** (JS, Python, enterprise platforms): everything is a
  late-bound name; the graph is reconstructed after the fact by analyzers
  (CHA, points-to, CodeQL/Glean/Kythe) and is *approximate by construction*.
- **No dynamism** (rigid v1): reject first-class functions entirely. Sound,
  and unusable past a certain program size.

The claim: this is a false dichotomy, and the middle point is not "some
approximation" but **declared over-approximation** — dynamism whose possible
targets are stated in the source and enforced at the boundary.

## 2. The primitive

Two constructs, two duties:

**Creation is static.** `&name` produces a function reference value, and
`fn(x) { ... }` produces a closure over one. Both are ordinary graph edges:
a lambda's body is lifted to a top-level definition named `<enclosing>$<n>`,
so even an anonymous function is a definition the compiler can point at, with
a hash of its own. You cannot manufacture a callable without the compiler
seeing exactly which definition it is. These values flow freely (lets,
arguments, returns); flowing is harmless because…

**Invocation is quarantined.** A reference can only be *called* inside a
`dyn` block that declares its possible targets:

```
dyn { double, triple }(f, x)
```

Each declared name resolves at compile time and becomes a graph edge at this
site. At runtime, the callee's identity is checked for membership in the
declared set; a reference from outside the set is refused with an error
naming both the offender and the set. Membership is decided on identity —
the resolved definition, not the spelling — so a rename cannot smuggle a
target in or out.

Direct calls (`g(x)` where `g` is a value) are a compile error (E0204)
pointing at `dyn`. Bare definitions in value position are a compile error
(E0206) pointing at `&`. There is no third door.

## 3. Guarantees

- **G1 — Graph totality.** Every definition a call site can ever invoke
  appears as a static edge at that site. Direct calls contribute their one
  target; `dyn` sites contribute their declared set. Nothing else invokes.
- **G2 — Blast-radius soundness.** `rdeps(d)` is a superset of d's true
  dynamic callers, computed from the compiler's own resolution. The demo
  shows `rdeps triple` finding the funref creator *and* both dyn sites.
- **G3 — Effect bound.** A `dyn` site's effect is the union over its
  declared set. The same declaration that bounds call targets bounds
  effects — one construct, two guarantees. This is why IO enters through
  this door: `print` requires `!io`, and `!io`-ness propagates along call
  edges (including dyn-set edges), checked statically in one pass because
  the graph is exact (E0207).
- **G4 — No ambient escape.** A `Fun` value is inert outside `dyn`: it can
  be compared, stored, passed, returned — never invoked. Escaped references
  cannot smuggle calls.

Soundness sketch (induction on evaluation): the only evaluation rules that
transfer control to a definition are direct Call (single static target,
resolved at compile time) and DynCall (membership-checked against a set
whose members were resolved at compile time). Hence every runtime call edge
is contained in the static edge set; G1 follows, G2–G3 are corollaries over
that set; G4 holds because no other rule eliminates a `Fun`.

## 4. Identity

`&name` canonicalizes as FUNREF over the target's hash. A lambda
canonicalizes as CLOSURE over the *lifted definition's* hash plus the slots
it captures — the body itself is not encoded at the site, so editing a
lambda moves exactly one hash and renaming a captured local moves none. A
`dyn` site canonicalizes as DYN over its target set *sorted by encoded
reference bytes* — name-free, so the set's identity is order-insensitive in
source and rename-insensitive in meaning. Consequences: renaming any target
changes nothing; editing any target's content changes the dyn site's hash
(the dependency cone includes the whole declared set). Test caching and
drift detection therefore remain sound in the presence of dynamism.

## 5. Costs, honestly

- **Verbosity.** Sets are written out at every site. Deliberate for v2 —
  the declaration *is* the documentation — but it won't scale past a few
  targets.
- **Over-approximation.** rdeps through a wide set names callers that never
  fire at runtime. That is the correct direction to be wrong in, and the
  width is chosen by the author, visible in review.
- **Runtime check.** One membership test per dyn call. Typed references
  would move much of it to compile time (below).

## 6. Roadmap

1. ~~Named sets~~ — **delivered as dispatch tables** (v3): a `table` is a
   definition with its own hash and rdeps; sites reference it once.
2. ~~Set inference as a *fix*~~ — **delivered** (v10): a may-reach
   analysis with one-level return summaries powers I0405 (an
   unquarantined call gets an inferred minimal set as a typed
   `quarantine_call` fix) and W0403 (a provably-narrow dyn set gets an
   `add_dyn_targets` fix). `fix --apply` performs both textually and
   converges. Inference proposes; the declaration in source remains the
   truth.
3. **Typed references** — *arity delivered* (v4, E0209): every call form —
   direct, each dyn target, each table entry — is arity-checked at compile
   time, so a whole class of quarantine violations moved left. Richer types
   on refs remain future work.
4. ~~Data-driven dispatch~~ — **delivered** (v3): `table name { "k" ->
   target }` with `name[key](args)` dispatch. Keys are data; the value set
   is closed and graph-visible; blast radius and effects flow through the
   table node in two exact hops. This is the original grievance
   (string-bound platform code) meeting the mechanism: the string stays a
   string, and the graph stays exact anyway.
5. **More effects through the same door** — read/files/net as declared
   capabilities; `!io` splits when there's a reason to split it.

## 7. Relation to prior art

Rust's `unsafe` is the ergonomic model: a scoped, greppable admission with
an audit story — but `unsafe` suspends checking inside, while `dyn` *adds* a
declared bound; nothing is suspended. Class-hierarchy and points-to
analyses recover approximate target sets externally and after the fact;
here the set is source, reviewed and hashed. Effect systems and Unison's
abilities type the *kind* of effect; G3 additionally bounds *which
definitions* can perform it at each site. The closest analogy is gradual
typing, transposed: call it **gradual reference exactness** — a codebase's
exactness is the fraction of call edges that are singletons, `dyn` sets are
the typed-untyped boundary, and migration means shrinking sets. That metric,
and what programs look like as it approaches 1, is the open question worth a
paper.

## 8. Open questions

- Do wide sets decay into "declared everything," and what review pressure
  or lints keep them honest?
- ~~Sidecar pins for dyn sets and tables~~ — **answered yes and delivered**
  (v10): each module's sidecar records per-site target hashes
  (`dyn:<def>#<n>`, `table:<name>`); a pinned target whose content changes
  is W0404 quarantine drift, an error under `--frozen`.
- Composition: higher-order code that assembles sets from sets — is there a
  closed algebra of dyn sets, and is it worth its complexity?
