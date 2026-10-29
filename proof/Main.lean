import RigidModel
import Lean.Data.Json

/-!
# rigid-model

Runs programs with the proven evaluator `Rigid.evalS`, so the real VM can be
checked against it (`tests/soundness.rs`, `the_vm_matches_the_proven_model`).

    rigid-model PROGRAMS.json

The input is a JSON array of programs, `{"defs": [[name, param, E], …],
"main": E}`, with expressions as arrays:

    ["int", "123"]  ["str", s]  ["var", x]  ["bin", op, E, E]  ["ite", E, E, E]
    ["let", x, E, E]  ["rec", [[field, E], …]]  ["get", E, field]
    ["fail", E]  ["try", E]  ["call", f, E]

The output is a JSON array with one line per program, in `rigid run`'s
format: the value, or `error: ` and the error.
-/

open Lean (Json)
open Rigid

def opOf : String → Except String Op
  | "+" => pure .add | "-" => pure .sub | "*" => pure .mul | "/" => pure .div
  | "%" => pure .rem | "==" => pure .eq | "!=" => pure .ne | "<" => pure .lt
  | ">" => pure .gt | "<=" => pure .le | ">=" => pure .ge
  | s => throw s!"unknown operator {s}"

partial def toExpr (j : Json) : Except String Expr := do
  let a ← j.getArr?
  let at_ (i : Nat) : Json := a.getD i Json.null
  match ← (at_ 0).getStr? with
  | "int" =>
    match (← (at_ 1).getStr?).toInt? with
    | some i => pure (.int i)
    | none => throw "bad integer"
  | "str" => return .str (← (at_ 1).getStr?)
  | "var" => return .var (← (at_ 1).getStr?)
  | "bin" => return .bin (← opOf (← (at_ 1).getStr?)) (← toExpr (at_ 2)) (← toExpr (at_ 3))
  | "ite" => return .ite (← toExpr (at_ 1)) (← toExpr (at_ 2)) (← toExpr (at_ 3))
  | "let" => return .letE (← (at_ 1).getStr?) (← toExpr (at_ 2)) (← toExpr (at_ 3))
  | "rec" =>
    let fs ← (at_ 1).getArr?
    let fs ← fs.toList.mapM fun f => do
      let p ← f.getArr?
      return ((← (p.getD 0 Json.null).getStr?), (← toExpr (p.getD 1 Json.null)))
    return .record fs
  | "get" => return .get (← toExpr (at_ 1)) (← (at_ 2).getStr?)
  | "fail" => return .fail (← toExpr (at_ 1))
  | "try" => return .try (← toExpr (at_ 1))
  | "call" => return .call (← (at_ 1).getStr?) (← toExpr (at_ 2))
  | t => throw s!"unknown expression {t}"

def display : Outcome → String
  | .ok v => v.show
  | .err e => "error: " ++ e.show
  | .timeout => "timeout"

/-- Enough for any program the tests generate; the theorems hold for all. -/
def fuel : Nat := 100000

def runProgram (j : Json) : Except String String := do
  let defs ← (← j.getObjVal? "defs").getArr?
  let defs ← defs.toList.mapM fun d => do
    let p ← d.getArr?
    return ((← (p.getD 0 Json.null).getStr?), (← (p.getD 1 Json.null).getStr?),
      (← toExpr (p.getD 2 Json.null)))
  let P : Prog := fun f => (defs.find? (·.1 = f)).map fun d => (d.2.1, d.2.2)
  return display (evalS P fuel [] (← toExpr (← j.getObjVal? "main")))

def main (args : List String) : IO UInt32 := do
  let [path] := args | IO.eprintln "usage: rigid-model PROGRAMS.json"; return 2
  match Json.parse (← IO.FS.readFile path) >>= Json.getArr? with
  | .error e => IO.eprintln s!"rigid-model: {e}"; return 1
  | .ok progs =>
    let mut out : Array Json := #[]
    for p in progs do
      match runProgram p with
      | .ok line => out := out.push (Json.str line)
      | .error e => IO.eprintln s!"rigid-model: {e}"; return 1
    IO.println (Json.arr out).compress
    return 0
