import RigidModel.Syntax

/-!
# Semantics of the surface language

`evalS` is rigid's evaluator for the modelled core, written to match the VM
(`src/eval.rs`): the same values, the same errors, the same messages. The
`rigid-model` executable runs it, and `tests/soundness.rs` checks the real VM
against it on generated programs.

Evaluation takes fuel, decremented on every step, so every function here is
structurally recursive and total. Running out is `timeout`, a result both
semantics agree on; the theorems hold for every amount of fuel.
-/

namespace Rigid

/-- Runtime values. A record's fields are kept sorted by name. -/
inductive Value where
  | int : Int → Value
  | str : String → Value
  | record : List (String × Value) → Value
  deriving Repr, Inhabited

inductive Outcome where
  | ok : Value → Outcome
  | err : Value → Outcome
  | timeout : Outcome
  deriving Repr, Inhabited

/-- Continue with a value; an error or a timeout passes through. -/
def Outcome.bind : Outcome → (Value → Outcome) → Outcome
  | .ok v, k => k v
  | .err e, _ => .err e
  | .timeout, _ => .timeout

/-- Structural equality, as the VM's `==` on these values. -/
partial def Value.beq : Value → Value → Bool
  | .int a, .int b => a == b
  | .str a, .str b => a == b
  | .record xs, .record ys =>
    xs.length == ys.length &&
      (xs.zip ys).all fun (p, q) => p.1 == q.1 && Value.beq p.2 q.2
  | _, _ => false

def hexDigits (n : Nat) : String := String.ofList (Nat.toDigits 16 n)

/-- A string in quotes, escaped by the rule both of rigid's engines use
(`eval::quote`). -/
def quote (s : String) : String :=
  "\"" ++ String.join (s.toList.map esc) ++ "\""
where
  esc (c : Char) : String :=
    if c = '"' then "\\\""
    else if c = '\\' then "\\\\"
    else if c = '\n' then "\\n"
    else if c = '\r' then "\\r"
    else if c = '\t' then "\\t"
    else if c.val = 0 then "\\0"
    else if c.val < 0x20 ∨ (0x7f ≤ c.val ∧ c.val ≤ 0x9f) then "\\u{" ++ hexDigits c.val.toNat ++ "}"
    else c.toString

/-- How the VM displays a value (`eval::show`). -/
partial def Value.show : Value → String
  | .int i => toString i
  | .str s => quote s
  | .record fs => "{" ++ ", ".intercalate (fs.map fun (k, v) => k ++ ": " ++ Value.show v) ++ "}"

def errStr (s : String) : Outcome := .err (.str s)

def intMin : Int := -9223372036854775808
def intMax : Int := 9223372036854775807

/-- A 64-bit result, or the overflow error: never a wrapped number. -/
def checked (r : Int) (what : String) : Outcome :=
  if intMin ≤ r ∧ r ≤ intMax then .ok (.int r) else errStr ("integer overflow: " ++ what)

def flag (b : Bool) : Outcome := .ok (.int (if b then 1 else 0))

/-- The binary operators, case for case as `eval::binop` on these values. -/
def binop (op : Op) (a b : Value) : Outcome :=
  match op, a, b with
  | .eq, a, b => flag (Value.beq a b)
  | .ne, a, b => flag (!Value.beq a b)
  | .add, .str x, .str y => .ok (.str (x ++ y))
  | .lt, .str x, .str y => flag (decide (x < y))
  | .gt, .str x, .str y => flag (decide (y < x))
  | .le, .str x, .str y => flag (decide (x ≤ y))
  | .ge, .str x, .str y => flag (decide (y ≤ x))
  | .add, .int x, .int y => checked (x + y) s!"{x} + {y}"
  | .sub, .int x, .int y => checked (x - y) s!"{x} - {y}"
  | .mul, .int x, .int y => checked (x * y) s!"{x} * {y}"
  | .div, .int x, .int y =>
    if y = 0 then errStr "division by zero" else checked (x.tdiv y) s!"{x} / {y}"
  | .rem, .int x, .int y =>
    if y = 0 then errStr "remainder by zero" else .ok (.int (x.tmod y))
  | .lt, .int x, .int y => flag (decide (x < y))
  | .gt, .int x, .int y => flag (decide (x > y))
  | .le, .int x, .int y => flag (decide (x ≤ y))
  | .ge, .int x, .int y => flag (decide (x ≥ y))
  | _, a, b => errStr s!"operator needs numbers (got {a.show} and {b.show})"

/-- `if`: an integer condition, nonzero meaning true. -/
def branch (v : Value) (t f : Unit → Outcome) : Outcome :=
  match v with
  | .int 0 => f ()
  | .int _ => t ()
  | v => errStr ("condition must be an integer, got " ++ v.show)

def field (v : Value) (f : String) : Outcome :=
  match v with
  | .record vs =>
    match vs.find? (·.1 = f) with
    | some (_, x) => .ok x
    | none => errStr s!"no field '{f}' (fields: {", ".intercalate (vs.map (·.1))})"
  | v => errStr ("cannot access field on " ++ v.show)

/-- `try`: always a value. -/
def caught : Outcome → Outcome
  | .ok v => .ok (.record [("ok", .int 1), ("value", v)])
  | .err e => .ok (.record [("error", e), ("ok", .int 0)])
  | .timeout => .timeout

/-- Local environments, innermost binding first. -/
abbrev Env := List (String × Value)

def lookupEnv (x : String) : Env → Option Value
  | [] => none
  | (y, v) :: rest => if y = x then some v else lookupEnv x rest

/-- A program: each definition's parameter and body, by name. -/
abbrev Prog := String → Option (String × Expr)

mutual
/-- rigid's evaluator on the modelled core. Record fields run in canonical
(name) order. -/
def evalS (P : Prog) : Nat → Env → Expr → Outcome
  | 0, _, _ => .timeout
  | n + 1, ρ, e =>
    match e with
    | .int i => .ok (.int i)
    | .str s => .ok (.str s)
    | .var x =>
      match lookupEnv x ρ with
      | some v => .ok v
      | none => errStr ("unbound " ++ x)
    | .bin op a b =>
      (evalS P n ρ a).bind fun va => (evalS P n ρ b).bind fun vb => binop op va vb
    | .ite c t f =>
      (evalS P n ρ c).bind fun vc => branch vc (fun _ => evalS P n ρ t) (fun _ => evalS P n ρ f)
    | .letE x a b => (evalS P n ρ a).bind fun v => evalS P n ((x, v) :: ρ) b
    | .record fs => evalFieldsS P n ρ (sortBy fs) []
    | .get a f => (evalS P n ρ a).bind fun v => field v f
    | .fail a => (evalS P n ρ a).bind fun v => .err v
    | .try a => caught (evalS P n ρ a)
    | .call f a =>
      match P f with
      | none => errStr ("unbound " ++ f)
      | some (x, body) => (evalS P n ρ a).bind fun v => evalS P n [(x, v)] body

/-- Evaluate fields in the order given, collecting values (reversed). -/
def evalFieldsS (P : Prog) : Nat → Env → List (String × Expr) → List (String × Value) → Outcome
  | 0, _, _, _ => .timeout
  | _ + 1, _, [], acc => .ok (.record acc.reverse)
  | n + 1, ρ, (k, e) :: rest, acc =>
    (evalS P n ρ e).bind fun v => evalFieldsS P n ρ rest ((k, v) :: acc)
end

/-- The evaluator as it was before the fix: record fields run in *source*
order. Everything else is `evalS`. `Counterexample.lean` shows this breaks
"same hash, same behaviour". -/
def evalOld (P : Prog) : Nat → Env → Expr → Outcome
  | 0, _, _ => .timeout
  | n + 1, ρ, e =>
    match e with
    | .record fs => go n ρ fs []
    | e => evalS P (n + 1) ρ e
where
  go : Nat → Env → List (String × Expr) → List (String × Value) → Outcome
    | 0, _, _, _ => .timeout
    | _ + 1, _, [], acc => .ok (.record acc.reverse)
    | m + 1, ρ, (k, e) :: rest, acc =>
      (evalOld P m ρ e).bind fun v => go m ρ rest ((k, v) :: acc)

end Rigid
