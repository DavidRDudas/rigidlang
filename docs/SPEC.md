# rigid v1.8 — specification

Working name only ("rigid designator," Kripke: a reference that picks out the
same thing in every possible world — which is what a hash-pinned reference is).
Rename at will.

## 1. Thesis

**Text is authoritative for content. The derived graph is authoritative for
identity. Ambiguity is a compile error.**

A definition's identity is the SHA-256 of its canonical form, in which local
names are erased and every reference to another definition is that
definition's hash. Names are a human-facing metadata layer. The compiler
maintains an exact reference graph as a byproduct of compilation — never
reconstructed, never approximate — and refuses to let a name silently change
its meaning.

Design ordering, used to settle every dispute: **exactness > interop >
ergonomics > expressiveness.** Interop is why text, files, Git, and
mainstream syntax survive (contrast Unison, which made the graph primary and
demoted text). Exactness is why silent rebinding does not.

## 2. Surface grammar (v0 core)

Deliberately tiny: definitions, calls, imports, arithmetic, `if`-expressions.
Enough to exercise the identity model; nothing more.

```
module   := item* EOF
item     := import | fndef | tabledef
tabledef := "table" ident "{" (STRING "->" ident ("," STRING "->" ident)* ","?)? "}"
import   := "import" "{" ident ("," ident)* "}" "from" STRING ";"
fndef    := "fn" ident "(" (ident ("," ident)*)? ")" ("!" "io")? "{" stmt* "}"
stmt     := "let" ident "=" expr ";"
          | ident "=" expr ";"                                -- assignment (locals only)
          | "while" "(" expr ")" "{" stmt* "}"
          | "return" expr ";"
          | expr ";"
expr     := cmp
cmp      := add (("=="|"!="|"<"|">"|"<="|">=") add)*
add      := mul (("+"|"-") mul)*
mul      := unary (("*"|"/"|"%") unary)*
unary    := "-" unary | "&" ident | dyncall | "print" "(" expr ")" | call
call     also admits: ident "[" expr "]" ("(" args ")")?     -- table dispatch
dyncall  := "dyn" "{" ident ("," ident)* "}" "(" expr ("," expr)* ")"
call     := ident "(" (expr ("," expr)*)? ")"     -- direct calls only in v0
          | ident
          | primary
primary  := INT | STRING | "(" expr ")"      -- STRING supports \n \t \r \e \" \\
          | "[" (expr ("," expr)* ","?)? "]"                 -- list literal
          | "{" (ident ":" expr ("," ident ":" expr)* ","?)? "}" -- record literal
          | "len" "(" expr ")" | "push" "(" expr "," expr ")" | "input" "(" ")"
          | "fail" "(" expr ")" | "try" "(" expr ")"
          | "str" "(" expr ")" | "int" "(" expr ")" | "chars" "(" expr ")"
          | "substr" "(" expr "," expr "," expr ")" | "join" "(" expr "," expr ")"
          | "read_file" "(" expr ")" | "write_file" "(" expr "," expr ")"
          | "if" "(" expr ")" expr "else" expr
postfix  := atom ("." ident | "[" expr "]")*                 -- field access, value index
```

Comments: `//` to end of line. Files: `*.rg`, one module per file, module id =
path relative to workspace root with forward slashes.

## 3. Canonical form and hashing

### 3.1 Resolution precedes hashing

Every name occurrence is resolved to exactly one `Binding`:

- `Local(slot)` — parameters then `let`s, numbered in binding order per
  function; innermost match wins. A `let` name is bound *after* its
  initializer.
- `Def(id)` — a top-level definition: same module first, then imports.
- Anything else is `E0201` (unresolved) or `E0203` (ambiguous — an imported
  name colliding with a module-local definition, or a duplicate definition).

### 3.2 Encoding

`canon(def)` is a tagged byte string:

| tag  | node       | payload                                     |
|------|------------|---------------------------------------------|
| 0x01 | Int        | i64 big-endian                              |
| 0x02 | Str        | u32 length + UTF-8 bytes                    |
| 0x03 | Local      | u32 slot                                    |
| 0x04 | Ref        | 32-byte hash of the target definition       |
| 0x05 | SelfRef    | — (direct self-recursion)                   |
| 0x06 | Call       | target (0x04/0x05 form) + u32 argc + args   |
| 0x07 | Binop      | u8 op tag + lhs + rhs                       |
| 0x08 | If         | cond + then + else                          |
| 0x09 | Neg        | operand                                     |
| 0x0B | FunRef     | target reference (0x04/0x05/0x0A form)      |
| 0x0C | DynCall    | u32 set size + set (sorted by encoded bytes) + callee + u32 argc + args |
| 0x0D | Print      | operand (io builtin)                        |
| 0x0E | TDispatch  | table ref + key expr + u8 has-args (+ u32 argc + args) |
| 0x0F | LocalIndex | local slot + key expr (value indexing on a local)     |
| 0x13 | List       | u32 count + elements                                  |
| 0x14 | Len        | operand                                               |
| 0x15 | Push       | list + element                                        |
| 0x16 | Record     | u32 count + name-sorted (u32 len + name + value)      |
| 0x17 | Get        | base + u32 len + field name                           |
| 0x18 | Index      | base + key (generic value indexing)                   |
| 0x19 | Input      | — (io builtin)                                        |
| 0x1A | Assign     | u32 slot + value                                      |
| 0x1B | While      | cond + u32 stmt count + stmts                         |
| 0x1C | Fail       | payload expression                                    |
| 0x1D | Try        | operand                                               |
| 0x1E | Str        | operand (int -> decimal string)                       |
| 0x1F | Int        | operand (parse; failure recoverable)                  |
| 0x25 | Substr     | string + start + count                                |
| 0x26 | Chars      | operand                                               |
| 0x27 | Join       | list + separator                                      |
| 0x28 | ReadFile   | path (io; missing file recoverable)                   |
| 0x29 | WriteFile  | path + contents (io; returns contents)                |
| 0x2A | Module     | identity: u32 count + sorted member hashes (names excluded) |
| 0x2B | Extern     | u32 arity + u8 io + u32 hostlen + host path bytes     |
| 0x10 | Let        | initializer (bound name erased; slot is positional) |
| 0x11 | Return     | operand                                     |
| 0x12 | ExprStmt   | operand                                     |
| 0x20 | Fn header  | u32 param count + u32 stmt count + stmts (pure) |
| 0x23 | FnIo header| as 0x20, for `!io` definitions              |
| 0x2D | FnEff header| as 0x20, preceded by a u32 capability mask, for any set other than `{}` or `{io}` |
| 0x2E | Float      | 8 bytes, canonical IEEE-754 (see below)     |
| 0x2F | FloatConv  | operand — `float(e)`                        |
| 0x30 | IfStmt     | cond + u32 then-count + then + u32 else-count + else |
| 0x31 | For        | u32 loop slot + iterable + u32 stmt count + stmts |
| 0x32 | Closure    | reference to the lifted definition + u32 capture count + slots |
| 0x24 | Table def  | u32 count + key-sorted entries (u32 keylen + key + target ref) |

`hash(def) = SHA-256(canon(def))`, computed in dependency post-order so every
`Ref` payload is already available. Tags are frozen; extend by appending.

### 3.3 Invariants (what identity means)

- Renaming a parameter, a `let`, a definition, or anything a definition
  references does **not** change its hash.
- Changing structure, literals, operators, or the *content* of anything it
  transitively references **does** change its hash — a hash pins the exact
  implementation and its whole dependency cone (the Unison property).
- Whitespace and comments never matter (they don't survive parsing).

### 3.4 Cycles (v1: SCC hashing)

Direct self-recursion in a singleton component uses `SelfRef`. Mutual
recursion is legal: the reference graph is condensed into strongly-connected
components (Kosaraju), hashed in dependency order.

For a multi-member component: every intra-component reference is encoded as
`CREF` (0x0A) + u32 ordinal. Member ordering must itself be name-free, so
members are first encoded with placeholder ordinals (0xFFFFFFFF) and sorted
by those bytes; component bytes are then `0x21 + u32 count + members in
order` with real ordinals; `component_hash = SHA-256(component bytes)`; each
member's identity is `SHA-256(0x22 + component_hash + u32 ordinal)`.

Known pragmatic leak: two *structurally identical* members of one cycle have
identical placeholder encodings, so their ordering falls back to
(module, name) — the name layer touches identity for exactly that
pathological case. Documented, revisit in v2. `E0301` is retired; the code
stays reserved.

## 4. The binding sidecar

### 4.1 What it is

Per module: `<module>.rg.bind.json`, a **resolution lockfile**. It does not
pin content (content evolves in text, where it belongs). It pins **which
definition each imported name means**, by hash. Diffable, mergeable,
reviewable — deliberately shaped like `Cargo.lock`, deliberately per-module
so parallel edits (human or agent) don't collide in one giant file.

```json
{
  "format": "rigid-bind/0",
  "definitions": { "norm": "sha256:…", "fib": "sha256:…" },
  "references": {
    "clamp": { "target": "util.rg#clamp", "hash": "sha256:…" }
  }
}
```

`definitions` (the module's own defs) is informational: it makes
within-module drift visible in ordinary diffs and feeds tooling.
`references` is normative.

### 4.2 The contract

The import statement names the *target module*; the sidecar pins the
*identity* of what the name bound to there. Text can change freely; the
sidecar is how the compiler notices that a name's **meaning** moved, as
distinct from its target being **edited**.

### 4.3 Reconciliation rules

For each imported name `n` pinned as hash `h`, against the target module's
current name→hash table:

| rule | condition | outcome |
|------|-----------|---------|
| R1 | `n` present, hash == `h` | OK |
| R2 | `n` present, hash != `h`, **`h` still exists under another name** | **E0202 resolution-changed.** The name was re-used for a different definition. Hard error; never silently rebind. Typed fixes: rename the reference to the pin's new name, or explicitly `rebind`. |
| R3 | `n` present, hash != `h`, `h` gone from target | **W0401 drift.** The definition was edited in place. Repin (dev mode) / error (`--frozen`). |
| R4 | `n` absent, **`h` exists under another name** | **E0201 + typed `rename_reference` fix.** The definition was renamed; a machine can apply the fix. A missed call site is a structured error, not a silent break. |
| R5 | `n` absent, `h` gone | E0201 plain. |
| R6 | no pin yet | Pin now; I0402 info. |

### 4.4 Repair

`rigid fix --apply` mechanically applies `rename_reference` fixes — only
occurrences the resolver marked broken, so an unrelated local sharing the
name is untouched — and iterates until `check` converges. `rebind` fixes are
never auto-applied: accepting a changed meaning is a decision, so it is its
own explicit command (`rigid rebind MODULE#NAME`).

R2 and R4 are the point of the language. R2 is the *silent-rebind refusal*:
in every name-bound language, re-using a name quietly changes the meaning of
every existing reference; here it cannot. R4 is *rename detection by
identity*: the compiler recognizes the renamed definition by hash and hands
back a mechanical fix.

`--frozen` (CI mode): any sidecar write required → error. The build is
reproducible or it fails.

## 5. Diagnostics

Stable codes, never renumbered. `--json` emits an array of
`{code, severity, file, span, message, data, fixes}`; `fixes` are typed,
mechanically applicable hints (`rename_reference`, `rebind`). Machine
consumers are first-class readers of compiler output.

| code  | meaning |
|-------|---------|
| E0101 | syntax error |
| E0201 | unresolved name / import (with rename hint when identity is found under another name) |
| E0202 | resolution would change — silent-rebind refusal |
| E0203 | ambiguous binding (import/local collision, duplicate definition) |
| E0204 | calling a value outside a `dyn` block (quarantine required) |
| E0205 | `&` or dyn target is not a definition |
| E0206 | bare definition in value position; use `&name` |
| E0207 | an effect used or reached without declaring it |
| E0208 | table misuse (direct call of a table, dispatch of a non-table, non-function target) |
| E0209 | argument count does not match the target's arity (direct calls, dyn targets, table entries) |
| E0210 | assignment target is not a local (definitions are immutable) |
| E0211 | `memo` fn's dependency cone reaches `!io`; memoization requires purity |
| E0212 | literal argument outside a parameter's inferred value domain |
| E0213 | the selected compile target cannot represent this construct (reserved; no target currently reports it) |
| E0214 | a relationship declared in `rigid.rules` is violated |
| W0405 | an imported definition gained a capability it did not have when pinned (error under `--frozen`) |
| W0403 | a funref may reach a dyn site outside its declared set (fix: add_dyn_targets) |
| W0404 | pinned quarantine target content changed (error under --frozen) |
| I0405 | inferred quarantine set for an unquarantined call (fix: quarantine_call) |
| I0406 | a same-module definition was renamed (identity match); typed rename fix |
| I0407 | a dispatch table no test reaches; nothing asserts its key-to-handler mapping |
| I0408 | a table key that names another target of the same table but selects a different one |
| E0301 | retired (v1 SCC hashing); code reserved |
| W0401 | pinned target's content drifted; repinned (error under `--frozen`) |
| I0402 | new reference pinned |

## Value domains (E0212)

rigid has no type system. It does have *value domains* — the same four-way
distinction the extern boundary already enforces on returns (integer, string,
list, record) — and it applies them at call sites.

Three things are inferred: a parameter's domain, from uses admitting exactly
one (`-`, `*`, `/`, `%` and the ordering comparisons are integer-only;
`.field` requires a record; `substr` and `chars` require a string; `push` and
`join` require a list; `+` against a known operand pins the other, since both
sides must agree); a local's domain, from its initializer; and a definition's
return domain, when every `return` in it agrees. A definition outranks a use,
so `let n = 5; n.kind` is a bad field access rather than an ambiguous slot.

Three things are then checked, and only when the offending domain is *known*:
an argument at a direct call against the parameter's domain, an operand of an
integer-only operator, and the base of a field access. Conflicting inferences
silence a slot instead of failing it.

The analysis is one-sided by construction: it never rejects a program that
would have run. It says nothing about record shapes, list element domains, or
values crossing a `dyn` set or table dispatch, and nothing is ever declared.

Domains are inferred, never declared, and never enter canonical form — adding
or removing a domain-forcing use changes a definition's hash only because the
body changed, and the domain itself is not encoded.

## 6. CLI

```
rigid check   [--root DIR] [--frozen] [--json]   parse, resolve, hash, reconcile
rigid run     PATH#NAME [ARGS...]                evaluate a definition
rigid test    [--no-cache]                       test_* defs; results cached by hash
rigid watch   [--root DIR]                       re-check and re-run affected tests on save
rigid diff    [--against DIR | --save]           grade a diff by identity: meaning vs. surface
rigid caps    [--root DIR] [--json]              what reaches the outside world, and through what
rigid attest  PATH#NAME --kind K [--by W] | --list  bind a claim to an identity
rigid trust   PATH#NAME [--json]                 live and stale claims about a definition
rigid dead    [--entry PATH#NAME]...             definitions nothing can reach
rigid dup     [--root DIR] [--json]              definitions that share an identity
rigid merge   --base D --ours D --theirs D [-o D] three-way merge over identities
rigid why     PATH#NAME [--json]                 what re-executes if this changes, and why
rigid hash    (--all | PATH#NAME)                content identities
rigid deps    PATH#NAME [--json]                 outgoing references
rigid rdeps   PATH#NAME [--json]                 blast radius
rigid fix     [--apply]                          apply typed rename fixes, converge
rigid rebind  PATH#NAME                          accept a changed meaning explicitly
rigid fmt     PATH                               canonical rendering to stdout
```

Execution (v9): each definition compiles once to a stack bytecode
(engine-internal; canonical form and hashes are untouched); `try` operands
compile to sub-chunks sharing the caller's locals; semantics are identical
to the reference tree-walker, verified by the full demo regression.
Evaluator (v6): the error channel carries *values* — `fail(v)` raises any
value, `try(e)` yields `{ ok: 1, value: v }` or `{ ok: 0, error: payload }`,
and every builtin runtime error (division by zero, bounds, missing keys,
quarantine violations) is catchable the same way. `while` and assignment
give iteration; scope is flat per function, and a `let` inside a loop body
keeps one stable slot across iterations (reading it after a loop that never
ran yields 0).
Evaluator (v5): Int, Str, Fun, List, and Record values; `+` concatenates
strings and lists; records are immutable maps whose *identity ignores
field order* (canonical form sorts by name — two differently-ordered
literals hash identically); `input()` reads a line and, like `print`,
requires `!io`. `&name` creates a reference
(static edge); only `dyn { targets }(callee, args...)` may invoke one, with
a runtime membership check against the declared set (§8, QUARANTINE.md).
`print(e)` writes a string operand as raw text (any other value in its
display form), returns the operand; it and any path to it require
`!io`, checked statically over the exact graph (E0207). Arity is checked at
compile time at every call form — direct calls, each declared `dyn` target,
and every entry of a dispatched table (E0209). Arity is the first type in
the language, admitted because it narrows the quarantine: violations that
were runtime membership failures become compile errors. `test` caches pass/fail keyed by a test's
content hash; since a hash pins the whole dependency cone, a cache hit is a
proof the rerun would be identical, and editing one definition re-executes
exactly the tests downstream of it.

`fmt PATH` prints the one canonical rendering; `--write` rewrites the file.
Comments survive parsing and are reattached by original-line association
(leading comments precede the next construct; same-line comments trail it).
Anchoring is heuristic but lossless — every comment is emitted exactly once
— and formatting never changes a hash.

## Control flow

`if` is both an expression and a statement. Which one is parsed depends on
what follows the condition: a `{` opens a statement block, anything else is
the value-yielding form, whose `else` is mandatory. A statement `if` may
chain with `else if` and its `else` is optional.

`for (x in xs) { ... }` iterates a list, or the characters of a string. The
loop variable is an ordinary local slot, so renaming it moves no hash, and it
follows the same flat-scope rule `while` does — one slot, reused.

Neither is sugar. Both have their own canonical tags, so `fmt` gives back
what was written rather than a desugared equivalent.

## Closures

`fn(a, b) { ... }` is an expression. Its body is **lifted** to a real
top-level definition named `<enclosing>$<n>`, whose leading parameters are
the enclosing locals the body reads; what remains at the site is a reference
to that definition plus the values to close over.

This is what keeps the graph exact through anonymous functions, which is
normally where a reference graph stops being exact:

  * the lambda has a content hash like anything else;
  * `rdeps` reaches it, and reaches through it to the dyn sites that can
    invoke it;
  * renaming a captured local moves no hash — identity survives renames
    *inside* a closure too;
  * the quarantine still applies: a closure is invoked through `dyn`, and a
    target outside the declared set is refused by hash at runtime.

`$` cannot start an identifier but is legal inside one, so a synthetic name
is expressible in a `dyn` set — which it has to be — and can never collide
with a name an author wrote. `fmt` skips synthetic definitions: the `fn(...)
{ ... }` at the site is what the author wrote and what gets rendered back.

A lambda inherits the capability set of the function it appears in.

**Known limit.** A generic higher-order function cannot live in a shared
library yet, because its `dyn` set would have to name lambdas that live in
the caller's workspace. `examples/closures/` shows the per-workspace form.
Inferring the set and pinning it in the sidecar — the same treatment tables
and dyn targets already get — is the next step.

## Standard library

`std/` ships three modules, written in rigid and tested in rigid:
`std/list.rg`, `std/str.rg`, `std/math.rg`. They are imported by path —
`import { sort } from "std/list.rg"` — and resolved against `RIGID_STD`, or a
`std/` directory near the executable.

Only the modules a workspace actually imports are loaded, transitively, so a
workspace that uses none pays nothing. Standard-library modules never have
sidecars written into the importing workspace; importers still pin std
definitions by hash, so a changed standard library is ordinary visible drift.

Strings order lexicographically under `<`, `>`, `<=`, `>=` — which is what
`sort` needs. Arithmetic operators still refuse them.

## Capabilities

A definition declares the parts of the outside world it may touch:

```
!io   !fs   !net   !env   !time   !rand   !proc
```

Order is free; the set is canonical, and `fmt` renders it in the order
above. `!io` is rigid's own console and file builtins (`print`, `input`,
`read_file`, `write_file`) and keeps exactly the meaning it has always had.
The rest describe the *host* boundary and are declared on `extern`.

The rule is one line: **a definition must declare every capability it can
reach.** A call to something carrying `!net` from a definition that does not
declare `!net` is E0207, naming the missing capability. A table's capability
set is the union of its entries', because dispatching through it can reach
any of them — the quarantine bounds effects exactly as it bounds targets.
Since the check is local and every callee had to satisfy it too, transitivity
holds by induction.

Capabilities are part of canonical form, so they are part of identity: a
definition that gains `!net` gets a new hash, and an importer's sidecar sees
drift. The sidecar also records the capability set at the moment of pinning,
which is what lets the drift say *which* kind it is — W0405 reports the
widening by name, and `--frozen` refuses it. A dependency that starts
reaching the network is a build failure, not an audit finding.

`rigid caps` lists the whole set with the externs that supply it. It is
complete by construction rather than by search, because effects travel the
same exact graph everything else does.

## Floats

`3.14` is a float; `3` is an integer. A digit sequence continues into a
float only when a `.` is followed by another digit, so `1.` and `x.field`
are unaffected. `float(e)` widens an integer or parses a string (a failure
is catchable); `int(e)` truncates toward zero.

Arithmetic on two integers stays integral — widening it would lose
precision above 2^53, which is exactly the quiet wrongness this language
refuses. A float on either side promotes the other. Equality compares
numerically across the two (`1 == 1.0`), and integer division by zero is
still an error while float division by zero is an infinity, as IEEE-754
says.

**Canonical form.** IEEE-754 has two encodings that compare equal (`-0.0`
and `+0.0`) and a value that does not equal itself (NaN). Content addressing
tolerates neither, so before hashing, `-0.0` normalizes to `+0.0` and every
NaN to one quiet NaN. Source cannot spell either — `-0.0` parses as a
negation and there is no NaN literal — so in the hash this is a rule waiting
for a case; it earns its place in the `memo` key, where both values arise
from ordinary arithmetic and a cache that missed on `0.0 * -1.0` would be
silently wrong.

`rigid fmt` renders a float in the shortest form that round-trips, always
with a decimal point, so `2.0` never renders as `2` and reads back as an
integer.

**The JS target keeps both domains.** JavaScript has one number type, a
float64, which is why the backend maps a rigid `Int` to **BigInt** and a
rigid `Float` to a plain number. Using one JS number for both was not merely
imprecise about floats — it silently rounded 64-bit integers, so
`9007199254740993 + 1` came back unchanged from node while the VM answered
correctly. `tools/difftest.sh` now covers both domains.

Crossing out to a host function narrows an `Int` to a JS number, but only
when it survives the trip: above 2^53 the call is refused by name rather
than truncated. Crossing back in, a whole host number becomes an `Int` and a
fractional one a `Float`.

## Declared relationships

`rigid.rules` at the workspace root states what the graph is allowed to look
like. Every rule is checked during `check`, and a violation is `E0214`.

```text
deny    domain/**  ->  http/**
deny    parse.rg#parse_untrusted -> !fs, !net
require test    handlers.rg#**
require attest reviewed  api.rg#**
deny    duplicates
```

A selector is a glob over `module/path.rg#name`; `*` stops at `/` and `#`,
`**` does not. Elsewhere these rules are enforced by import linters that
approximate the call graph, or by reviewers who remember; here the graph is
the compiler's own resolution, so the question is decidable. The report names
the exact path — `total -> ship` — not merely the fact of a violation.

This matters most when the author has no memory of the last conversation. An
agent that wires the domain layer to the transport layer fails `check`
immediately, rather than being caught later by whoever reads the diff.

`deny duplicates` is the rule form of `rigid dup`: two definitions with the
same canonical form have the same hash, so they are not *similar*, they are
the same code under two names. A claim attested about one covers the other,
because it is the same identity.

## Properties

`property fn f(xs) { ... }` is a claim that must hold for every input rather
than for one example. It is checked over 200 generated cases; a failure is
shrunk to the smallest input that still fails and reported as a
counterexample.

Generation is seeded from the property's own content hash. That is not a
detail: the same code always draws the same inputs, so the result is
cacheable exactly like a test, and a failure reproduces for anyone with the
same source. Which properties a change puts back in question is decided the
same way test selection is — by the graph.

Inputs come from the parameter domains inferred for `E0212`, carried
backwards across calls. Where that is genuinely ambiguous — `len(x)` and
`for (c in x)` both accept a list and a string — a property may hint:

```
property fn trim_is_idempotent(s: str) { return trim(trim(s)) == trim(s); }
```

`int`, `float`, `str`, `list`. This is **not** a type: nothing is checked
against it, nothing is coerced, it never enters canonical form, and it is
refused anywhere but a `property` parameter. It exists solely so the
generator draws the right shape. Records cannot be generated — their shape is
not inferable from use — and such a property is reported rather than guessed
at.

Like `memo`, `property` is an execution annotation: adding or removing it
does not move the definition's hash.

## Attestation

An attestation is a claim about a definition — `reviewed`, `audited`,
`benchmarked`, `generated`, or any word a team chooses — bound to that
definition's content hash and recorded in `.rigid-attest.json`.

It is **live** while that hash is current and **void** the moment it is not.
Nobody decides whether it still applies; the identity model decides.

The consequential property costs nothing extra. A definition's hash covers
its whole dependency cone, because references are encoded as the hashes of
what they reference. So a review of `handler` attests to everything `handler`
can reach, and an edit four calls away voids that review automatically —
while a rename anywhere in the cone does not, because a rename moves no hash.
That is the guarantee code review has always claimed and never had: every
approval elsewhere is bound to a diff, so it survives a rebase that changed
the logic and dies on a rename that changed nothing.

Void attestations are never deleted. "Reviewed at a hash that is no longer
current" distinguishes *stale* from *unreviewed*, and both are worth knowing.

`rigid check --require KIND` is the gate: it fails unless every definition in
the workspace carries a live claim of that kind. Tests are excluded — a test
attesting to itself is circular — as are standard-library modules, which are
not the workspace's to review.

Attestations are metadata about identity and never enter canonical form.

## Reachability

`rigid dead` lists definitions nothing can reach. Roots are every `test_*`
definition, anything named `main`, and any `--entry PATH#NAME` given
explicitly. The answer is a proof rather than a guess, because the graph is
what the compiler resolved.

Output separates definitions nothing references *at all* — each one a
decision, an entry point to declare or code to delete — from those reachable
only through them, which follow automatically.

## Semantic diff

`rigid diff` compares two revisions as sets of identities and answers two
questions separately, because they have different victims:

  * **meaning** — did any definition's hash move? Equal hash sets prove that
    none did, whatever the text shows.
  * **surface** — did any name or module move? Behaviour is intact, but
    callers outside the workspace refer to definitions by name.

Every definition lands in exactly one bucket: *renamed* (same hash, same
module, new name), *moved* (same hash, new module), *changed* (same name,
new hash), *added*, or *removed*. Identity is matched before name, so a name
re-used for something new reads as a rename plus an addition — the truthful
reading, and the same event E0202 refuses to let happen silently.

`--against DIR` compares two checkouts; `--save` records a baseline in
`.rigid-snapshot.json` for a later bare `rigid diff`.
`--require-no-semantic-change` exits non-zero when meaning moved, so a
refactor that claims to be a refactor can be made to prove it.

## Merge

`rigid merge --base D --ours D --theirs D` merges three revisions by
definition rather than by line. Definitions are paired across revisions by
identity first and name second: a surviving hash is the same definition
whatever it is called, and a surviving name is the same definition whatever
it now does.

Name and body then resolve **independently**. Two sides that renamed
differently conflict on the name; two sides that edited differently conflict
on the body; one of each does not conflict at all, and both changes survive.
That last case is the one a line-based merge reports as a conflict, because
it cannot see that the two edits are about different coordinates.

`-o DIR` writes the merged workspace. Bodies move as text, comments
included — text is authoritative for content, so a merge relocates it rather
than regenerating it from the graph. References inside a moved body are
rewritten to the merged names, skipping string literals. Conflicts are
refused rather than marked: nothing is written until they are resolved.

## Caches

Three caches, all keyed by content hash and therefore machine-independent:
`.rigid-testcache.json` (test results), `.rigid-memo.json` (`memo` results),
and `.rigid-checkcache.json` (per-module fingerprints letting `check` skip
sidecar reconciliation for modules whose own and imported hashes are
unchanged; `--frozen` never uses it).

`RIGID_CACHE_DIR` relocates the test and memo caches, so CI and a working
copy can share one. A result computed anywhere is valid everywhere, because
the key covers the whole dependency cone. `RIGID_TIMING=1` reports where
`check` spends its time.

## 7. Remaining, in priority order

The ledger bar is reached (COMPLETENESS.md §Verdict); the design roadmap
(QUARANTINE.md §6) is closed (v10); the LSP is served directly from the
graph (v11). Remaining items are classified as usage-driven in
COMPLETENESS.md. **R7 (learned from testing a 4-module program):** a module with unresolved
references keeps its sidecar untouched — rewriting it would destroy the
old-name-to-hash evidence that makes rename recovery possible. With R7,
same-module renames are recovered via I0406 the same way imports are via
R4.

## Temporal references (v1.4)

On every green `check`, each definition is written to `.rigid-store/` under
its hash: the source slice with its own name and every resolved reference
rewritten to `d_<hash12>`, plus an index of dependency hashes, so a stored
cone is a self-contained module. `@"<hash-prefix>"(args)` calls that version
live — the cone is materialized, checked, and executed against its *old*
meanings. Temporal calls require `!io` at the caller (a past version's
effects are unknown) and are VM-only; the JS target refuses at runtime. The
store is immutable: a hash is never rewritten, only added. `rigid recall
<prefix> [args]` is the CLI form.

## memo (v1.4)

`memo fn f(a, b) { ... }` — results cached under (definition hash, canonical
argument encoding). Sound by construction: purity is required (E0211 if the
dependency cone reaches any effect) and the hash covers the entire cone, so
any change that could alter the result changes the key. Staleness is
impossible rather than detected. The cache persists across processes and
machines in `.rigid-memo.json`; `memo` is an execution annotation and does
not move the definition's identity. `dyn` inside a memo fn is sound — the
declared set is inside the hashed cone. The JS target runs memo fns as plain
fns.

## Externs (v1.3)

`extern fn name(params) [!io] = "host.path";` declares a foreign function
as a boundary node in the graph: it has arity (checked at every call form,
E0209), an effect (propagated exactly, E0207), and a content identity over
the declaration itself — so a changed host path is W0401 drift like any
other meaning change. The rigid VM refuses extern calls with a directive
error; `build --target js` emits an arity-shaped wrapper over the dotted
global path, which must be a valid identifier path (anything else is
refused at build time, never emitted as executable text). Declarations are
trusted for arity and effect; *values* are not. Every extern return is
checked at the boundary in emitted code: integers, strings, lists and
records of the same pass through; booleans coerce to 1/0 (documented);
non-integer numbers, null, undefined, and host objects are refused with a
RigidError naming the extern — a float cannot enter rigid arithmetic
silently. Integer-valued host numbers (e.g. Math.sqrt(4)) pass, because
they are rigid integers.

## 8. Quarantined dynamism (normative summary)

`&name` creates a first-class reference; creation is a static edge.
`dyn { t1, ..., tn }(callee, args...)` is the only invocation form for
reference values: each `ti` resolves at compile time and is an edge at the
site; at runtime the callee must be a member or evaluation stops with an
error naming the value and the set. Effects: a dyn site is `!io` iff any
declared target is. Canonical form: §3.2 (set sorted by encoded bytes).
Full design, guarantees G1–G4, soundness sketch, costs, and roadmap:
QUARANTINE.md.

**Dispatch tables** are the named, data-keyed form of the same idea. A
`table` is a *definition*: it hashes (key-sorted entries over target
hashes), it appears in `deps`/`rdeps`, it rides the sidecar rules R1–R6
when imported. `name[key](args...)` dispatches through it: the site's
static edge is the table; the table's edges are its whole value set, so
blast radius and effects flow through in two exact hops. Keys are data;
the value set is closed. A missing key fails at the boundary, listing the
set. Targets must be functions; direct calls of tables, dispatch through
non-tables, and non-function targets are E0208.
