# Breakage detection — TypeScript's compiler vs `rigid check`

Does rigid actually catch broken references that an existing toolchain
misses? This measures it instead of asserting it.

The same program is written twice — once in strict TypeScript, once in rigid —
and the same logical breakage is applied to both. Each checker then gets to
report it, in its strongest CI configuration: `tsc --strict --noEmit` and
`rigid check`. A scenario counts as caught if the checker exits non-zero.
Nothing is scored by hand.

Run it yourself:

```
cd bench && npm install typescript@5
python3 bench/run.py
```

## Result

Twelve scenarios. **rigid catches eleven, TypeScript ten.**

| scenario | tsc | rigid | what changed |
|---|---|---|---|
| rename-miss | caught | caught | Rename a definition, leave its callers behind |
| rename-reuse | **MISSED** | caught | Rename a definition, then give a different function its old name |
| arity-change | caught | caught | Add a parameter, miss one call site |
| delete-referenced | caught | caught | Delete a function the dispatch set still points at |
| type-mismatch | caught | caught | Pass a string literal where a number belongs |
| type-via-local | caught | caught | Bind a string to a local, then pass it where a number belongs |
| type-via-return | caught | caught | Pass a function's string result where a number belongs |
| arith-on-string | caught | caught | Multiply a string |
| field-on-int | caught | caught | Read a field off an integer |
| table-target-missing | caught | caught | Point a dispatch key at a name that does not exist |
| handler-wrong-arity | caught | caught | Give a dispatch target an extra parameter |
| swap-dispatch-targets | [missed by design](#what-can-and-cannot-be-checked-about-a-dispatch-table) | [missed by design](#what-can-and-cannot-be-checked-about-a-dispatch-table) | Swap which handler two keys map to |

That last row is not a gap. No error exists to report, and rigid's hash for
the table *does* move so the tests downstream of it re-execute — this column
counts `check` diagnostics only. The next section is the whole explanation.

This has been made harder three times rather than easier. It began at seven
scenarios and 5–5; four type scenarios were added *because* rigid failed them,
and value-domain inference was extended until it did not; then the last three
rows were grouped to show where checking a dispatch table stops.

A score is only worth what its scenarios are worth. This set is small and
self-authored, so read the shape of the disagreement, not the tally.

## What can and cannot be checked about a dispatch table

The last three rows are one program edited three ways, and together they draw
the line exactly.

**The target must exist.** Point a key at a name nothing defines and it is a
plain unresolved reference:

```
error[E0201] m.rg:5:14: unresolved name 'light'
```

**The target must fit.** Change a handler's shape and the dispatch that
reaches it no longer matches:

```
error[E0209] m.rg:6:23: table entry "light" -> 'slash' has arity 2,
but this dispatch passes 1 argument(s)
```

**The pairing is yours.** Swap which handler two keys select and there is
nothing to report, because keys are data and bear no required relationship to
the names of the functions they select. This is ordinary, correct code:

```
table moves {
  "light"    -> slash,
  "heavy"    -> pierce,
  "cleave"   -> slash,
  "critical" -> pierce,
}
```

No key there matches a function name, and two keys share a handler. The only
rule that would catch a swapped pairing — *a key must equal its target's name*
— would reject all of that.

So a checker can verify everything about the target and nothing about the
choice. That choice is what a test is for, and the point of the hashing is
that the right test reruns the moment you touch it.

There is one shape of wrong pairing that *can* be recognised without being
told what you meant. When a key is exactly the name of another function the
same table dispatches to, and points somewhere else, the entries have most
likely been transposed:

```
info[I0408] m.rg:3:7: in table 'moves', the key "slash" is also the name of
another target of this table, but selects 'pierce'. If that is deliberate,
nothing is wrong; if the entries were transposed, no other check will notice.
```

It is information, never an error, because the code is valid either way. It
stays silent when a key merely fails to match its target, which is the common
case — and it stays silent on this benchmark, whose keys are `"network"` and
`"database"` rather than handler names. So it covers one convention, not the
problem. Across all eleven bundled examples it reports nothing.

## Where TypeScript cannot help

`rename-reuse` is the case rigid exists for. A function is renamed, and a
*different* function is later given its old name — the kind of thing that
happens across a merge, or when two people work on one module in a week.

TypeScript compiles it clean, and correctly so: the name resolves, the
signature matches, nothing is broken by its rules. Every existing caller now
means the new function. There is no diagnostic to emit because TypeScript has
no notion of what a name meant *before*.

rigid has one, because the sidecar pinned it:

```
error[E0202] router.rg:1:10: 'notify_oncall' from "core.rg" no longer means
what it meant: the pinned definition (sha256:079abb19fb05…) is now named
'page_oncall', and 'notify_oncall' now names a different definition
(sha256:935bcc07c541…). Refusing to silently rebind. Run `rigid rebind
router.rg#notify_oncall` to accept the new meaning, or rename the reference
to 'page_oncall'.
```

This is not a gap TypeScript could close by trying harder. Detecting it
requires a record of the previous resolution, which is what the binding
sidecar is.

## The type gap, and how far it closed

`type-mismatch` was rigid's miss on the first run, and the three that followed
it — a string bound to a local, a function's string result, a field read off an
integer — were added afterwards precisely because rigid failed them too.

All four now report, for example:

```
error[E0212] router.rg:14:10: 'severity' uses parameter 'impact' as integer
(arithmetic or ordering), but this call passes string

error[E0212] router.rg:20:10: this operator needs integers, but one side is
a string

error[E0212] router.rg:21:12: '.kind' needs a record, but this is a integer
```

This is **value-domain inference**, not a type system, and the distinction is
worth keeping. It knows three things: a parameter's domain, from uses that
admit exactly one (`-`, `*`, `/`, `%` and ordering are integer-only;
`.field` needs a record; `substr`/`chars` need a string; `push`/`join` need a
list); a local's domain, from its initializer; and a function's return domain,
when every `return` in it agrees. It then rejects a call argument, an operator
operand, or a field access whose domain is *known* and wrong.

Everything else infers nothing and reports nothing. A parameter with no
domain-forcing use stays polymorphic, and a conflict silences the slot rather
than becoming an error:

```
fn id(x) { return x; }      // no domain inferred
fn a() { return id("s"); }  // fine
fn b() { return id(1); }    // also fine
```

It remains far weaker than a type system. There are no record shapes, no
element domains for lists, no domains across a `dyn` set or a table dispatch,
and nothing at all is declared. `tsc` still knows much more about a program
than this does; the benchmark says only that on these eleven breakages the
gap no longer shows.

Two properties were held fixed while extending it, and both are checked on
every run: **zero E0212 on all eleven bundled examples**, and the 36-step
`demo.sh` still green. An analysis that fired on working code would be worse
than no analysis.

## What neither catches, and why one of them still finds it

`swap-dispatch-targets` is missed by both, and should be. No reference is
broken, both handlers exist with the right arity, and the program is valid.
Asking a checker to flag it is asking it which mapping you *meant* — but the
mapping is the specification, so there is nothing to compare against.

It is in the set so the benchmark is not composed only of scenarios somebody
wins.

It is worth being precise about what "missed" means here, because the
machinery does react. Swapping two entries moves the table's hash, and moves
the hash of every definition that reaches it:

```
BEFORE   table moves : sha256:1563586a1c62…
AFTER    table moves : sha256:4f0d1e545eec…
```

A moved hash is not an error, though. It is the signal to re-check what
depended on the definition, and every edit moves one. The column above counts
`check` diagnostics, and there is no diagnostic to emit.

What a checker *can* say is that nothing is asserting the mapping, so a swap
would go unnoticed. That is now `I0407`:

```
info[I0407] router.rg:3:7: no test reaches table 'registry', so nothing
asserts which handler each key selects (3 entries: network, database,
breach). Swapping two of them would still check clean.
```

It is informational, it only appears in workspaces that already have tests,
and it goes quiet as soon as one test reaches the table. Turning it on found a
real gap in this project's own showcase example: `examples/triage` has four
tests, all covering the scoring core, and **none** reaching the `routes`
table — so swapping two of its handlers would have passed the suite.

rigid does catch the swap itself, with `test` rather than `check`, and this is
reproducible:

```
$ rigid check                       # after swapping two table entries
ok: 2 module(s), 10 definition(s), 10 hashed

$ rigid test
FAIL  router.rg#test_network_pages  (returned 0)
PASS  router.rg#test_severity_only  [cached — content unchanged, rerun provably identical]
1 executed, 1 skipped via content-address cache, 1 failed
```

The table's hash moved, so every test downstream of it re-executed and the
one asserting the mapping failed. The test that does not depend on the table
stayed cached. That is the intended division of labour: `check` answers
whether references are intact, `test` answers whether behaviour is what you
said it was, and the content-addressing is what guarantees the right tests
run. This benchmark measures the first of those only.

## A claim this corrects

Written up elsewhere, rigid's pitch has included the idea that conventional
tooling is blind to string-keyed dispatch. On this evidence that is too
strong.

The TypeScript version registers handlers into a `Record<string, Handler>`
at load time, through a `register()` helper — and `tsc` still caught
`handler-wrong-arity` and `delete-referenced` through it, because the handlers
are referenced *by identifier* even though the keys are strings. A type
checker follows those references fine.

What defeats a type checker is not string keys as such; it is when the target
is named by a computed string, or the registry is assembled somewhere the
checker cannot follow. That is a narrower and more honest claim than "dispatch
tables are opaque," and the benchmark is the reason to prefer it.

## Honest limits

- **Twelve scenarios, one small two-module program, one baseline.** This is a
  probe, not a survey. A larger program, or JavaScript without types, or a
  language with no checker at all, would all move the numbers.
- **The scenarios are ours.** They were chosen to span the space including
  cases rigid loses, but choosing scenarios is not a mechanical act.
- **`tsc` is a strong baseline and was given its best configuration.** A repo
  without strict mode, or with `any` at the boundaries, would do worse — but
  that is a fact about that repo, not about TypeScript.
- **Detection is not value.** Catching `rename-reuse` matters only as often as
  `rename-reuse` happens, and nobody has measured that. rigid still has no
  users, so its practical benefit remains untested.

## What it does establish

On this program, with a mature type checker as the baseline, rigid caught
eleven of twelve breakages to TypeScript's ten — including one class
TypeScript structurally cannot catch.

The honest reading is not "rigid wins." It is that the two find different
things for different reasons. TypeScript's type system is far stronger than
E0212 and always will be; what rigid adds is **identity** — noticing that a
name's meaning changed, as distinct from a name failing to resolve — and that
is not something a type checker can be improved into. It needs a record of the
previous resolution, which is what the sidecar is.

And detection is still not value. Catching `rename-reuse` matters as often as
`rename-reuse` happens, which nobody has measured.
