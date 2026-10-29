import RigidModel.Soundness

/-!
# Why record fields run in canonical order

Canonical form sorts record fields by name, so `{a: …, b: …}` and
`{b: …, a: …}` have one hash. Until this was fixed, rigid's VM evaluated the
fields in *source* order (`evalOld`). Here is the consequence, checked by the
kernel: two expressions with one canonical form, one hash, and two different
results. Test caching, attestations and `diff --require-no-semantic-change`
all vouched for the swap.

With fields evaluated in canonical order (`evalS`), the same two expressions
agree — as `same_canon_same_behaviour` guarantees they must.
-/

namespace Rigid

private def noDefs : Prog := fun _ => none
private def noIds : Ids := fun _ => none

def recAB : Expr := .record [("a", .fail (.str "A")), ("b", .fail (.str "B"))]
def recBA : Expr := .record [("b", .fail (.str "B")), ("a", .fail (.str "A"))]

theorem recAB_recBA_same_canonical_form :
    lower noIds none [] recAB = lower noIds none [] recBA := by
  rfl

/-- In source order, one canonical form produced two results. -/
theorem source_order_breaks_identity :
    evalOld noDefs 10 [] recAB = .err (.str "A") ∧
    evalOld noDefs 10 [] recBA = .err (.str "B") := by
  constructor <;> rfl

/-- In canonical order, they agree. -/
theorem canonical_order_agrees :
    evalS noDefs 10 [] recAB = .err (.str "A") ∧
    evalS noDefs 10 [] recBA = .err (.str "A") := by
  constructor <;> rfl

end Rigid
