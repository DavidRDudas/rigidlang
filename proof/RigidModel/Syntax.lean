/-!
# Syntax

Two languages: `Expr` is what a programmer writes, with names; `CExpr` is the
canonical form the hash is taken over, with no local names at all and every
definition referred to by its identity. `lower` (in `Canon.lean`) turns the
first into the second.

The model covers rigid's expression core: integers, strings, locals, the
binary operators, `if`, `let`, records, field access, `fail`/`try`, and calls
to definitions (one parameter; several arguments travel as a record), with
self-recursion. Loops, mutation, lists, floats, closures, `dyn`, tables and
effects are not modelled yet.
-/

namespace Rigid

inductive Op where
  | add | sub | mul | div | rem | eq | ne | lt | gt | le | ge
  deriving DecidableEq, Repr, Inhabited

/-- Surface syntax. `letE x a b` is `let x = a;` followed by `b`; rigid writes
it as statements, the model as an expression, which is the same thing. -/
inductive Expr where
  | int  : Int → Expr
  | str  : String → Expr
  | var  : String → Expr
  | bin  : Op → Expr → Expr → Expr
  | ite  : Expr → Expr → Expr → Expr
  | letE : String → Expr → Expr → Expr
  | record : List (String × Expr) → Expr
  | get  : Expr → String → Expr
  | fail : Expr → Expr
  | try  : Expr → Expr
  | call : String → Expr → Expr
  deriving Repr, Inhabited

/-- A definition's identity. In rigid it is a SHA-256 of the canonical form;
here it is any value the store is keyed by. -/
abbrev Hash := Nat

/-- Canonical form. A local is its de Bruijn index — which binder, never what
it was called. A call names its target by identity, and a definition calling
itself says `self`, because a hash cannot contain itself. -/
inductive CExpr where
  | int  : Int → CExpr
  | str  : String → CExpr
  | loc  : Nat → CExpr
  | bin  : Op → CExpr → CExpr → CExpr
  | ite  : CExpr → CExpr → CExpr → CExpr
  | letE : CExpr → CExpr → CExpr
  | record : List (String × CExpr) → CExpr
  | get  : CExpr → String → CExpr
  | fail : CExpr → CExpr
  | try  : CExpr → CExpr
  | call : Hash → CExpr → CExpr
  | self : CExpr → CExpr
  /-- An unresolved name. rigid refuses such a program (E0201); the model
  gives it a meaning — an error — so `lower` can be total. -/
  | bad  : String → CExpr
  deriving Repr, Inhabited

/-- Insert by key, keeping the list sorted. -/
def insertBy {α : Type} (k : String) (v : α) : List (String × α) → List (String × α)
  | [] => [(k, v)]
  | (k', v') :: rest =>
    if k < k' then (k, v) :: (k', v') :: rest else (k', v') :: insertBy k v rest

/-- Record fields in canonical order: sorted by name. The canonical form
stores them this way, and — the point of the whole model — the evaluator
runs them in this order too. -/
def sortBy {α : Type} : List (String × α) → List (String × α)
  | [] => []
  | (k, v) :: rest => insertBy k v (sortBy rest)

end Rigid
