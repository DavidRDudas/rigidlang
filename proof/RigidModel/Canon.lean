import RigidModel.Semantics

/-!
# Canonical form

`lower` is the model of `src/canon.rs`: it erases every local name (a local
becomes the index of its binder), refers to other definitions by identity, says
`self` for direct self-recursion, and sorts record fields by name. A hash is a
function of the canonical form, so whatever `lower` erases a hash cannot see.

`evalC` runs canonical form directly, against a content-addressed store that
maps an identity to the canonical body it stands for. It is what a program
*means*, once names are gone.
-/

namespace Rigid

/-- Position of the innermost binder named `x`. -/
def index (x : String) : List String → Option Nat
  | [] => none
  | y :: ys => if y = x then some 0 else (index x ys).map (· + 1)

/-- A workspace's name layer: which identity a definition name stands for. -/
abbrev Ids := String → Option Hash

/-- Lower to canonical form. `me` is the definition being lowered, so that a
call to itself becomes `self`; `Γ` holds the local names in scope. -/
def lower (ids : Ids) (me : Option String) : List String → Expr → CExpr
  | _, .int i => .int i
  | _, .str s => .str s
  | Γ, .var x =>
    match index x Γ with
    | some i => .loc i
    | none => .bad ("unbound " ++ x)
  | Γ, .bin op a b => .bin op (lower ids me Γ a) (lower ids me Γ b)
  | Γ, .ite c t f => .ite (lower ids me Γ c) (lower ids me Γ t) (lower ids me Γ f)
  | Γ, .letE x a b => .letE (lower ids me Γ a) (lower ids me (x :: Γ) b)
  | Γ, .record fs => .record (sortBy (lowerFields ids me Γ fs))
  | Γ, .get a f => .get (lower ids me Γ a) f
  | Γ, .fail a => .fail (lower ids me Γ a)
  | Γ, .try a => .try (lower ids me Γ a)
  | Γ, .call f a =>
    if me = some f then .self (lower ids me Γ a)
    else
      match ids f with
      | some h => .call h (lower ids me Γ a)
      | none => .bad ("unbound " ++ f)
where
  lowerFields (ids : Ids) (me : Option String) (Γ : List String) :
      List (String × Expr) → List (String × CExpr)
    | [] => []
    | (k, e) :: rest => (k, lower ids me Γ e) :: lowerFields ids me Γ rest

/-- Content-addressed storage of canonical definitions: the meaning behind
every identity (rigid's `.rigid-store`, and the graph `check` builds). -/
abbrev Store := Hash → Option CExpr

/-- A workspace is consistent with a store when every definition's identity
names its own canonical body there, and every identity the name layer hands
out belongs to a definition. This is what content addressing provides: the
store is keyed by the hash of what it stores. -/
structure Consistent (P : Prog) (ids : Ids) (store : Store) : Prop where
  defined : ∀ f x b, P f = some (x, b) →
    ∃ h, ids f = some h ∧ store h = some (lower ids (some f) [x] b)
  named : ∀ f, P f = none → ids f = none

mutual
/-- Evaluate canonical form. `cur` is the body of the definition running now,
which `self` re-enters; `σ` holds the locals, innermost first. -/
def evalC (store : Store) : Nat → CExpr → List Value → CExpr → Outcome
  | 0, _, _, _ => .timeout
  | n + 1, cur, σ, e =>
    match e with
    | .int i => .ok (.int i)
    | .str s => .ok (.str s)
    | .loc i =>
      match σ[i]? with
      | some v => .ok v
      | none => errStr "unbound local"
    | .bad msg => errStr msg
    | .bin op a b =>
      (evalC store n cur σ a).bind fun va => (evalC store n cur σ b).bind fun vb => binop op va vb
    | .ite c t f =>
      (evalC store n cur σ c).bind fun vc =>
        branch vc (fun _ => evalC store n cur σ t) (fun _ => evalC store n cur σ f)
    | .letE a b => (evalC store n cur σ a).bind fun v => evalC store n cur (v :: σ) b
    | .record fs => evalFieldsC store n cur σ fs []
    | .get a f => (evalC store n cur σ a).bind fun v => field v f
    | .fail a => (evalC store n cur σ a).bind fun v => .err v
    | .try a => caught (evalC store n cur σ a)
    | .self a => (evalC store n cur σ a).bind fun v => evalC store n cur [v] cur
    | .call h a =>
      match store h with
      | none => errStr "dangling reference"
      | some body => (evalC store n cur σ a).bind fun v => evalC store n body [v] body

def evalFieldsC (store : Store) : Nat → CExpr → List Value → List (String × CExpr) →
    List (String × Value) → Outcome
  | 0, _, _, _, _ => .timeout
  | _ + 1, _, _, [], acc => .ok (.record acc.reverse)
  | n + 1, cur, σ, (k, e) :: rest, acc =>
    (evalC store n cur σ e).bind fun v => evalFieldsC store n cur σ rest ((k, v) :: acc)
end

end Rigid
