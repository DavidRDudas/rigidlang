# DESIGNS.md — post-v13 signature-feature candidates (specced, not built)

## 1. `memo` — provably-sound persistent memoization  [recommended v14]
Syntax: `memo fn f(a, b) { ... }`. Constraint: f must be pure (no `!io`
anywhere in its dependency cone — already checkable). Semantics: results
cached under key (def-hash, canonical-encoding(args)) in a content store
(.rigid-memo/), valid forever by construction: any change to f or
anything it transitively references moves the hash. Generalizes the
existing test cache (tests = memo of arity-0). Needs: canonical value
encoding at runtime (exists for all value kinds), store with eviction
policy, `dyn` inside memo fns keyed by resolved-target hash (sound: the
set is declared). Payoff: incremental computation as a language default.

## 2. Temporal references — `f@<hash-prefix>(args)`
Retain canonical bodies by hash (.rigid-store/); a reference names a
prior meaning and calls it live beside the current one. Differential
tests in one line; canaries as `dyn { f, f@old }` — the quarantine
dispatching across time. Needs: body retention on every successful
check, parser syntax, eval/JS-backend lookup, GC story.

## 3. Provenance stamps
Any output value optionally carries the hash-cone that produced it;
`rigid attest out.json` verifies. Auditable computation; niche.

Rule: whichever is chosen reopens COMPLETENESS.md with its own bar,
after v13 is published. Specs precede code; the ledger decides "done."
