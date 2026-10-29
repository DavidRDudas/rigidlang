# proof/ — rigid's core guarantee, machine-checked

A Lean 4 model of rigid's expression core, and proofs about it that Lean's
kernel checks. `lake build` checks every proof; a `sorry` or an unexpected
axiom fails the build.

```
cd proof
lake build                      # check the proofs, build the model's evaluator
cd .. && cargo test --release --test soundness model   # the VM against the model
```

## What is proved

**Same hash, same behaviour.** Every one of rigid's tools — cached test
results, `memo`, attestations, `dup`, `diff --require-no-semantic-change` —
assumes that two pieces of code with one hash behave identically. For the
modelled core this is now a theorem:

| theorem | says |
|---|---|
| `lower_sound` | running a program is running its canonical form: what canonical form erases (local names, definition names, the order record fields are written in) cannot affect any result, error message, or timeout |
| `same_canon_same_behaviour` | two expressions with one canonical form give one result — in two workspaces, under any names |
| `same_identity_same_behaviour` | two definitions with one identity give one result on every argument; this is why a test result cached by hash survives any rename |
| `same_hash_same_behaviour` | the same, for any collision-free hash of the canonical form |
| `source_order_breaks_identity` | the counterexample: evaluating record fields in *source* order (what rigid did before) gives one canonical form two results |

`RigidModel/Axioms.lean` pins what these rest on: Lean's three standard
axioms, nothing else.

## What is assumed

* **The hash has no collisions** on the forms compared. The model treats the
  content-addressed store as a function — one body per identity — which is
  what SHA-256's collision resistance provides in practice. Nothing is proved
  about SHA-256.
* **The canonical *bytes* are an unambiguous encoding of the canonical
  tree.** The model hashes the tree itself; `src/canon.rs` writes it as
  tagged, length-prefixed bytes.

## How the proof reaches the real compiler

A proof about a model only says something about rigid if rigid *is* the
model. `rigid-model` (built by `lake build`) runs the proven evaluator `evalS`
on programs given as JSON, and `tests/soundness.rs`
(`the_vm_matches_the_proven_model`) generates programs in the modelled subset,
runs each on the real VM and on the model, and requires identical output —
values, error messages, all of it. CI runs it on 8,000 programs.

The comparison is itself tested: reintroducing the record-order bug into the
VM, or wrapping integer arithmetic, makes it fail.

## What is not modelled yet

Loops and mutation, lists, floats, closures, `dyn`, tables, effects and
capabilities, the binding sidecars (R1–R7), and the JavaScript backend. Each
is a next step; the generator-based tests in `tests/soundness.rs` cover them
meanwhile, which finds counterexamples but cannot rule them out.

## Layout

```
RigidModel/Syntax.lean          surface syntax, canonical syntax, field sorting
RigidModel/Semantics.lean       the evaluator (rigid's VM, for the core)
RigidModel/Canon.lean           lowering to canonical form; evaluating it
RigidModel/Soundness.lean       the theorems
RigidModel/Counterexample.lean  what source-order records broke
RigidModel/Axioms.lean          what the proofs rest on, checked
Main.lean                       rigid-model: the proven evaluator as a program
```
