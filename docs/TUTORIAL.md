# Writing your first rigid program

Every command and output below was executed as written. Ten minutes, start
to finish.

## 0. Setup (once)

You need Rust (`rustup.rs`). Then:

```
git clone https://github.com/DavidRDudas/rigidlang && cd rigid-lang
cargo build --release
export PATH="$PWD/target/release:$PATH"
```

A browsable version of this walkthrough, plus a full language tutorial, is
in `site/learn.html`.

## 1. The mental model

A rigid program is a folder of `.rg` files. There is no project file, no
manifest, no build step. Three commands do everything:

- `rigid check` — parse, resolve every name, hash every definition,
  reconcile the sidecars. Your "compile."
- `rigid run PATH#NAME args...` — evaluate one definition.
- `rigid test` — run every `test_*` function; results cached by content
  hash.

The first `check` creates `*.rg.bind.json` files next to your sources.
Those are **resolution lockfiles** — they record which definition each
imported name means, pinned by hash. Commit them; they are how the
compiler notices a name's *meaning* changed, as opposed to its target
being edited. You never write them by hand.

## 2. Hello

Make a folder, create `main.rg`:

```
fn main() !io {
  return print("hello, rigid");
}
```

```
$ rigid check
ok: 1 module(s), 1 definition(s), 1 hashed
$ rigid run "main.rg#main"
"hello, rigid"
"hello, rigid"
```

(You see it twice: `print` prints, and `run` prints the returned value —
`print` returns its operand.)

`!io` is required because `print` is an effect. Delete `!io` and `check`
gives you `E0207`. Effects are declared, and they propagate: anything that
*calls* an `!io` function must be `!io` too.

## 3. Three files, and the point of the language

`words.rg`:
```
fn shout(s) {
  return s + "!";
}
```

`main.rg`:
```
import { shout } from "./words.rg";

fn main() !io {
  return print(shout("hello"));
}
```

`tests.rg`:
```
import { shout } from "./words.rg";

fn test_shout() {
  return shout("hi") == "hi!";
}
```

```
$ rigid check
info[I0402] main.rg:1:10: pinned 'shout' -> words.rg#shout (sha256:f2aeacb386b8…)
info[I0402] tests.rg:1:10: pinned 'shout' -> words.rg#shout (sha256:f2aeacb386b8…)
ok: 3 module(s), 3 definition(s), 3 hashed
$ rigid run "main.rg#main"
"hello!"
"hello!"
$ rigid test
PASS  tests.rg#test_shout
1 executed, 0 skipped via content-address cache, 0 failed
$ rigid test
PASS  tests.rg#test_shout  [cached — content unchanged, rerun provably identical]
0 executed, 1 skipped via content-address cache, 0 failed
```

The cached rerun is not a heuristic: the test's hash pins its whole
dependency cone, so "unchanged hash" *proves* the rerun would be identical.

**Now break it on purpose.** Rename `shout` to `yell` in `words.rg` only —
"forget" both callers:

```
$ rigid check
error[E0201] main.rg:1:10: unresolved import 'shout' from "words.rg" —
  the pinned definition (sha256:f2aeacb386b8…) now exists under the name 'yell'
error[E0201] tests.rg:1:10: ...same...
```

The compiler recognized the renamed definition **by identity** — the pinned
hash — and found it under its new name. And the fix is mechanical:

```
$ rigid fix --apply
applied: 2 occurrences of 'shout' -> 'yell' in main.rg
applied: 2 occurrences of 'shout' -> 'yell' in tests.rg
converged: check is green
```

Both callers, including the import lines, rewritten. No grep, no missed
call site, no silent break.

**The identity is real.** Hash it, rename its parameter, hash again:

```
$ rigid hash "words.rg#yell"
words.rg#yell  sha256:f2aeacb386b85feb…
# rename parameter s -> word, in both places
$ rigid hash "words.rg#yell"
words.rg#yell  sha256:f2aeacb386b85feb…
```

Identical — and it's *still the hash from step one*, before the definition
was ever renamed. Names (of definitions, of parameters) are metadata;
structure is identity.

**Blast radius is a query:**

```
$ rigid rdeps "words.rg#yell"
rdeps of words.rg#yell (2 results):
  main.rg#main   sha256:a17a93feacb2…
  tests.rg#test_shout  sha256:c2064b0a4043…
```

## 4. The whole language on a card

**Values:** integers, strings (escapes: `\n \t \r \e \" \\`), lists `[1, 2, 3]`, records
`{ x: 1, y: 2 }`, function references. `+ - * / %` on ints;
`+` also concatenates strings and lists; `xs[i]` indexes lists and
strings; `r.field` reads records; `len(x)`, `push(xs, v)`, `input()`
(io), `fail(v)`/`try(e)` (errors as values), `str`/`int`/`substr`/
`chars`/`join`, and `read_file`/`write_file` (io) are builtins;
`rigid repl --root DIR` gives you a prompt against your workspace; `== != < > <= >=` (comparisons return 1/0);
`if (cond) a else b` is an expression; `0` is false, any other int is true.

**Statements:** `let x = expr;` · `x = expr;` (locals only) ·
`while (cond) { ... }` · `return expr;` · `expr;`

**Top-level items:**

```
import { name, name } from "./other.rg";

fn name(a, b) { ... }            // pure
fn name(a, b) !io { ... }        // may print / reach printers

table routes {                   // string keys, closed value set
  "key" -> handler,
}
```

**Calls:** `f(x)` direct. `&f` makes a reference (a value). References are
invoked *only* through a quarantine that declares its possible targets:

```
dyn { f, g }(ref_expr, args...)      // membership-checked at runtime
routes[key_expr](args...)            // dispatch through a declared table
routes[key_expr]                     // bare lookup -> a reference
```

Arity is checked at compile time everywhere — direct calls, every dyn
target, every table entry (`E0209`). Comments are `//` and survive
`rigid fmt PATH --write`, which rewrites to the one canonical form without
changing any hash.

**Test functions:** zero-argument, pure, named `test_*`, return nonzero to
pass.

## 5. What you can't write yet (honest list)

No `break` or `continue`; `while`, `for (x in xs)`, and recursion are the
iteration forms. No `&&` or `||` — nest the conditions. Calling a closure is
quarantined: `fn(x) { ... }` exists and captures, but the call site names its
possible targets with `dyn`. No user-defined types, generics, or pattern
matching; records are the aggregate. No async or concurrency — the outside
world is reached through declared capabilities (`!io !fs !net !env !time
!rand !proc`), file and console IO being the ones the language supplies
itself. So: think *logic, scoring, routing, dispatch* —
the triage pipeline in `examples/triage/` is the intended shape of a rigid
program today — not interactive apps.

## 6. When things fight you

- `E0202 resolution changed` — a name you'd pinned now means a *different*
  definition. That refusal is the language working. Rename the reference,
  or accept the new meaning explicitly: `rigid rebind MODULE#NAME`.
- `W0401` — a dependency's content changed; it was repinned. In CI, run
  `rigid check --frozen` and drift becomes a failure instead.
- Machine-readable everything: add `--json` to `check` for structured
  diagnostics with typed fixes.
- Cache lives in `.rigid-testcache.json`; delete it to force reruns.

Full grammar and semantics: SPEC.md. Why `dyn` exists: QUARANTINE.md.
Sixteen more worked demonstrations: `./demo.sh`.
