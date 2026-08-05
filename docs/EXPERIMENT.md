# Comprehension pilot — does a machine-readable spec help a model answer dependency questions?

`rigid describe` emits the whole workspace as one hash-anchored JSON
document: every definition with its identity, arity, effects, exact
dependencies and reverse dependencies, table topology, declared `dyn` sets,
extern boundary, and test coverage. Nothing in it is reconstructed — it is
the compiler's own resolution, serialised.

The obvious question is whether that is worth anything to a reader that is
not a human. This pilot asks it directly, and it is a *pilot*: one program,
five questions, five runs. It is suggestive, not conclusive.

## Design

One program, three views of it. The program is an event pipeline that
dispatches on a runtime string key — the shape that makes dependency
questions hard.

| condition | what the model was shown |
|---|---|
| A | the same program as convoluted JavaScript, registry built at runtime |
| B | the program in rigid source |
| C | the program in rigid source **plus** its `rigid describe` spec |

Condition A is not a strawman: it is ordinary platform-script style. Here is
the part that matters, from `experiments/legacy.js`:

```
const R = {};
function reg(k, f) { R[k] = f; }

reg("created", audit);
reg("updated", notify);
const h = R;                       // alias used below
h["closed"] = close_out;
if (true) { reg("archived", archive); }
reg("breach", escalate);

function handle(kind, sev) {
  const rec = { kind: kind, sev: sev };
  const f = R[kind];
  if (!f) return "drop:" + kind;
  return f(rec);
}
```

The registry is assembled by five separate statements, one of them through an
alias and one inside a conditional. Nothing in the text enumerates what
`R[kind]` can be, so a question like "which handlers perform IO?" cannot be
answered from the source at all — only by running it and watching.

The same program in rigid, `experiments/pipeline.rg`, declares it:

```
extern fn log_line(s) !io = "console.error";

fn audit(rec) !io { log_line("AUDIT " + fmt(rec)); return "AUDIT " + fmt(rec); }
fn escalate(rec) !io { audit(rec); return "escalated:" + rec.kind; }

table handlers {
  "created"  -> audit,
  "updated"  -> notify,
  "closed"   -> close_out,
  "archived" -> archive,
  "breach"   -> escalate,
}

fn handle(kind, sev) !io {
  let rec = { kind: kind, sev: sev };
  return handlers[kind](rec);
}
```

Condition C adds the `rigid describe` output for that program, in which the
table appears as a node with its five targets named, its hash, and its
reverse dependencies:

```
{
  "name": "handlers",
  "kind": "table",
  "hash": "sha256:e17952f86a72…",
  "entries": {
    "created":  "pipeline.rg#audit",
    "updated":  "pipeline.rg#notify",
    "closed":   "pipeline.rg#close_out",
    "archived": "pipeline.rg#archive",
    "breach":   "pipeline.rg#escalate"
  },
  "rdeps": ["pipeline.rg#handle"]
}
```

Note also `archive_v2` in both versions: a candidate replacement that is
never wired up. In the rigid version it has no reverse dependencies, which is
checkable; in the JavaScript version, deciding whether it is reachable means
reasoning about the whole file.

Five questions were asked, each with a single mechanically-checkable answer
taken from the compiler's graph: dependency cones, reverse dependencies,
the transitive effect surface, and reachability. Answers were scored by
exact comparison against `experiments/groundtruth.json`, not by judgement.

The ground truth is the compiler's own output rather than the author's
opinion, and it can be re-derived at any time. For example
`rigid describe --root experiments` reports `audit` with dependencies
`fmt` and `log_line`, and reverse dependencies `escalate` and `handlers`,
which is exactly what the scored answers for those two questions contain.

The subject was a fresh model over the API — not the model that built the
language, and with no context beyond the condition it was given. Five runs.

## Results

| condition | score |
|---|---|
| A — convoluted JavaScript | ~2/5 |
| B — rigid source | 4/5, identical every run |
| C — rigid source + spec | 5/5 on the repaired instrument |

The ordering C >= B > A held on every valid comparison.

What condition C got right that the others did not, every run, was the
**transitive effect surface** — the set of definitions that perform IO or
reach something that does, including the dispatch table on the path.

That last part is the interesting one. In the spec, `handlers` is a node
whose edges name all five of its targets, two of which are `!io`, and
`handle`'s only outgoing edge is to the table. So the effect reaching
`handle` is traceable hop by hop. In condition A the same registry is
assembled at runtime by `reg()` calls, one of them inside a conditional and
one through an alias, so its contents never appear in the text at all — there
is nothing to trace, only something to guess.

Condition A missed the rename-through-registry question on every run.

## The missing control, and what it shows so far

The pilot as run has a hole: it compares rigid + spec against rigid source and
against JavaScript source, but never against **JavaScript plus a
machine-readable graph produced by an existing tool**. Without that condition,
the result supports "a dependency graph helps a model" much better than it
supports "*rigid's* graph helps," and only the second would justify a
language.

Condition D closes it: the same JavaScript, plus a call graph emitted by the
**TypeScript compiler API** — a standard tool, doing its best on the same
program. `bench/tsgraph.mjs` builds it, propagating an IO marker along call
edges from `fs`/`console` the same way rigid propagates `!io`.

Part of that comparison needs no model at all, because the question is whether
the graph is *correct*. It is not:

| | effect surface reported |
|---|---|
| ground truth | `audit`, `escalate`, `handle`, `handlers`, `log_line` |
| TypeScript-API graph over `legacy.js` | `audit`, `escalate`, `log_line` |
| `rigid describe` over `pipeline.rg` | `audit`, `escalate`, `handle`, `log_line` (+ `handlers` via the table's edges) |

The analyzer reports `handle` as performing **no IO**. It does — it dispatches
to `audit` and `escalate`, both of which do. The analyzer cannot know, because
`R[kind]` resolves through a registry assembled at runtime, so the call edge
does not exist in anything it can read. Its answer is not merely thinner than
rigid's; on this question it is wrong.

It is right about other things — it correctly reports `archive_v2` as called
by nothing, which is the reachability question.

**The honest caveat.** This analyzer is simple, and a tool that special-cased
the `reg(key, fn)` idiom could enumerate the registry and recover the edges.
That is exactly the distinction being drawn, though: recovering the set means
pattern-matching one registration idiom, and it stops working the moment the
key or the function is computed. rigid does not infer the set, because the
set is declared.

So the part of condition D that can be checked mechanically has been, and it
favours the spec. The part that needs a model — whether that difference in
the graph changes what a model answers — has not been run.

## Recorded defects

Three things went wrong. They are recorded because an instrument that cannot
report its own failures is not measuring anything.

1. **Run 1 had a harness bug.** Answers wrapped in prose were scored as
   wrong. Fixed, and run 1's numbers are not used.
2. **One question was authored with contradictory instructions**, and it
   penalised exactly the condition that could see dispatch tables. That item
   was voided and repaired — note that the defect ran *against* the
   hypothesis, which is why it was worth finding.
3. **One repair hypothesis was pre-registered and falsified.** The proposed
   explanation for a scoring discrepancy turned out to be wrong, and the real
   cause was found afterwards.

## Honest limits

- **This is a pilot.** One program, five questions, five runs. It is not a
  benchmark and should not be cited as one.
- **One model, one point in time.** No claim is made about models generally,
  or about the same model later.
- **The author wrote the questions.** Ground truth is mechanical — it comes
  from the compiler — but the *choice* of what to ask is not, and questions
  about dependencies are the questions rigid is built to answer well.
- **A is one JavaScript program, not JavaScript.** A different program, or a
  codebase with a statically-visible registry, would narrow the gap.

The result worth taking seriously is the narrow one: on this program, a
model given the spec answered effect-propagation questions that the same
model, given only source, did not.

## Reproducing it

The inputs, the ground truth, and both graphs are in the repo, so everything
that does not need a model is checkable right now:

```
rigid describe --root experiments        # condition C's spec
node bench/tsgraph.mjs experiments/legacy.js   # condition D's graph
```

The *scored runs* are not reproducible from a shell. They were made against a
keyless API reachable only from a browser page — `curl` gets a 401 — so
re-running the pilot means driving it from a page, and adding condition D
means re-running all four.

Treat the scores as a recorded result, at a lower standard of evidence than
anything in `demo.sh` or `bench/run.py`, both of which you can run in a
terminal right now. The condition-D graph comparison above is at the higher
standard: it is mechanical, and it is checked.

## Materials

`experiments/` holds the exact inputs: `legacy.js` (condition A),
`pipeline.rg` (conditions B and C), and `groundtruth.json` (the scored
answers). Condition C's spec is whatever `rigid describe --root experiments`
prints for that program.
