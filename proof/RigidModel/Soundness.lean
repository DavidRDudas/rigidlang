import RigidModel.Canon

/-!
# Same hash, same behaviour

The guarantee every one of rigid's tools stands on — a cached test result,
a `memo` entry, an attestation, `dup`, `diff --require-no-semantic-change` —
is that two pieces of code with one hash behave identically. Here it is
proved, for the modelled core:

* `lower_sound`: running a program is running its canonical form. Whatever
  `lower` erases (local names, definition names, the order record fields
  were written in) cannot affect a result.
* `same_canon_same_behaviour`: two expressions with one canonical form give
  one result, in any two workspaces whose definitions share a store.
* `same_identity_same_behaviour`: two definitions with one identity give one
  result on every argument.
* `same_hash_same_behaviour`: the same, for any collision-free hash of the
  canonical form.

The store being a function — one body per identity — is where collision
resistance of the real hash (SHA-256) enters; the model assumes it rather
than proving anything about SHA-256.
-/

namespace Rigid

/-! ## Lemmas -/

theorem lookup_none {x : String} :
    ∀ {ρ : Env}, index x (ρ.map Prod.fst) = none → lookupEnv x ρ = none
  | [], _ => rfl
  | (y, v) :: ρ, h => by
    by_cases hy : y = x
    · simp [index, hy] at h
    · simp only [List.map_cons, index, hy, ite_false, Option.map_eq_none_iff] at h
      simp [lookupEnv, hy, lookup_none h]

theorem lookup_some {x : String} :
    ∀ {ρ : Env} {i : Nat}, index x (ρ.map Prod.fst) = some i →
      ∃ v, lookupEnv x ρ = some v ∧ (ρ.map Prod.snd)[i]? = some v
  | [], _, h => by simp [index] at h
  | (y, v) :: ρ, i, h => by
    by_cases hy : y = x
    · simp only [List.map_cons, index, hy, ite_true, Option.some.injEq] at h
      subst h
      exact ⟨v, by simp [lookupEnv, hy], rfl⟩
    · simp only [List.map_cons, index, hy, ite_false] at h
      cases hj : index x (ρ.map Prod.fst) with
      | none => simp [hj] at h
      | some j =>
        simp only [hj, Option.map_some, Option.some.injEq] at h
        subst h
        obtain ⟨w, hw, hw'⟩ := lookup_some hj
        exact ⟨w, by simp [lookupEnv, hy, hw], by simpa using hw'⟩

theorem lowerFields_eq (ids : Ids) (me : Option String) (Γ : List String) :
    ∀ fs : List (String × Expr),
      lower.lowerFields ids me Γ fs = fs.map fun p => (p.1, lower ids me Γ p.2)
  | [] => rfl
  | (k, e) :: rest => by simp [lower.lowerFields, lowerFields_eq ids me Γ rest]

/-- Sorting looks only at names, so it commutes with anything done to values. -/
theorem insertBy_map {α β : Type} (g : α → β) (k : String) (v : α) :
    ∀ xs : List (String × α),
      insertBy k (g v) (xs.map fun p => (p.1, g p.2)) =
        (insertBy k v xs).map fun p => (p.1, g p.2)
  | [] => rfl
  | (k', v') :: rest => by
    by_cases h : k < k'
    · simp [insertBy, h]
    · simp [insertBy, h, insertBy_map g k v rest]

theorem sortBy_map {α β : Type} (g : α → β) :
    ∀ xs : List (String × α),
      sortBy (xs.map fun p => (p.1, g p.2)) = (sortBy xs).map fun p => (p.1, g p.2)
  | [] => rfl
  | (k, v) :: rest => by
    simp only [List.map_cons, sortBy, sortBy_map g rest, insertBy_map]

/-- `cur` is the canonical body of the definition `me` names, if any. -/
def SelfOK (P : Prog) (ids : Ids) (me : Option String) (cur : CExpr) : Prop :=
  ∀ f, me = some f → ∃ x b, P f = some (x, b) ∧ cur = lower ids (some f) [x] b

theorem selfOK_none (P : Prog) (ids : Ids) (cur : CExpr) : SelfOK P ids none cur := by
  intro f h; cases h

theorem selfOK_some {P : Prog} {ids : Ids} {f x : String} {b : Expr}
    (h : P f = some (x, b)) : SelfOK P ids (some f) (lower ids (some f) [x] b) := by
  intro g hg
  cases hg
  exact ⟨x, b, h, rfl⟩

/-! ## Lowering preserves meaning -/

/-- **Running a program is running its canonical form.** For every amount of
fuel, the surface evaluator on an expression (in environment `ρ`) and the
canonical evaluator on its lowering (with the names of `ρ` erased) agree —
values, errors, error messages and timeouts alike. -/
theorem lower_sound {P : Prog} {ids : Ids} {store : Store} (hc : Consistent P ids store) :
    ∀ n : Nat,
      (∀ (me : Option String) (ρ : Env) (e : Expr) (cur : CExpr), SelfOK P ids me cur →
        evalS P n ρ e =
          evalC store n cur (ρ.map Prod.snd) (lower ids me (ρ.map Prod.fst) e)) ∧
      (∀ (me : Option String) (ρ : Env) (fs : List (String × Expr))
          (acc : List (String × Value)) (cur : CExpr), SelfOK P ids me cur →
        evalFieldsS P n ρ fs acc =
          evalFieldsC store n cur (ρ.map Prod.snd)
            (fs.map fun p => (p.1, lower ids me (ρ.map Prod.fst) p.2)) acc) := by
  intro n
  induction n with
  | zero =>
    refine ⟨?_, ?_⟩
    · intro me ρ e cur _
      simp [evalS, evalC]
    · intro me ρ fs acc cur _
      simp [evalFieldsS, evalFieldsC]
  | succ n ih =>
    obtain ⟨ihE, ihF⟩ := ih
    refine ⟨?_, ?_⟩
    · intro me ρ e cur hs
      cases e with
      | int i => simp [evalS, evalC, lower]
      | str s => simp [evalS, evalC, lower]
      | var x =>
        simp only [evalS, lower]
        cases hi : index x (ρ.map Prod.fst) with
        | none => simp [evalC, lookup_none hi]
        | some i =>
          obtain ⟨v, hv, hv'⟩ := lookup_some hi
          simp [evalC, hv, hv']
      | bin op a b =>
        simp only [evalS, evalC, lower, ihE me ρ a cur hs, ihE me ρ b cur hs]
      | ite c t f =>
        simp only [evalS, evalC, lower, ihE me ρ c cur hs, ihE me ρ t cur hs,
          ihE me ρ f cur hs]
      | letE x a b =>
        simp only [evalS, evalC, lower, ihE me ρ a cur hs]
        congr 1
        funext v
        simpa using ihE me ((x, v) :: ρ) b cur hs
      | record fs =>
        simp only [evalS, evalC, lower, lowerFields_eq, sortBy_map]
        exact ihF me ρ (sortBy fs) [] cur hs
      | get a f => simp only [evalS, evalC, lower, ihE me ρ a cur hs]
      | fail a => simp only [evalS, evalC, lower, ihE me ρ a cur hs]
      | «try» a => simp only [evalS, evalC, lower, ihE me ρ a cur hs]
      | call f a =>
        simp only [evalS, lower]
        by_cases hme : me = some f
        · -- a call to the definition being run: `self`
          obtain ⟨x, b, hPf, hcur⟩ := hs f hme
          subst hme
          simp only [↓reduceIte, hPf, evalC]
          rw [← ihE (some f) ρ a cur hs]
          congr 1
          funext v
          have h := ihE (some f) [(x, v)] b cur (by rw [hcur]; exact selfOK_some hPf)
          simp only [List.map_cons, List.map_nil] at h
          rw [h, ← hcur]
        · simp only [hme, ite_false]
          cases hPf : P f with
          | none =>
            simp [hc.named f hPf, evalC]
          | some xb =>
            obtain ⟨x, b⟩ := xb
            obtain ⟨h, hid, hst⟩ := hc.defined f x b hPf
            simp only [hid, evalC, hst, ihE me ρ a cur hs]
            congr 1
            funext v
            simpa using ihE (some f) [(x, v)] b _ (selfOK_some hPf)
    · intro me ρ fs acc cur hs
      cases fs with
      | nil => simp [evalFieldsS, evalFieldsC]
      | cons p rest =>
        obtain ⟨k, e⟩ := p
        simp only [evalFieldsS, evalFieldsC, List.map_cons, ihE me ρ e cur hs]
        congr 1
        funext v
        exact ihF me ρ rest ((k, v) :: acc) cur hs

/-! ## The guarantees -/

/-- **Same canonical form, same behaviour.** Two expressions — in two
workspaces, written with any names, fields in any order — that lower to the
same canonical form give the same result in the same environment values,
for every amount of fuel. The workspaces only have to agree on what each
identity means, which content addressing guarantees. -/
theorem same_canon_same_behaviour
    {P₁ P₂ : Prog} {ids₁ ids₂ : Ids} {store : Store}
    (h₁ : Consistent P₁ ids₁ store) (h₂ : Consistent P₂ ids₂ store)
    {ρ₁ ρ₂ : Env} (hv : ρ₁.map Prod.snd = ρ₂.map Prod.snd)
    {e₁ e₂ : Expr}
    (hcanon : lower ids₁ none (ρ₁.map Prod.fst) e₁ = lower ids₂ none (ρ₂.map Prod.fst) e₂)
    (n : Nat) :
    evalS P₁ n ρ₁ e₁ = evalS P₂ n ρ₂ e₂ := by
  rw [(lower_sound h₁ n).1 none ρ₁ e₁ (.int 0) (selfOK_none _ _ _),
      (lower_sound h₂ n).1 none ρ₂ e₂ (.int 0) (selfOK_none _ _ _), hcanon, hv]

/-- **Same identity, same behaviour.** Two definitions, in two workspaces,
whose names map to one identity return the same result on every argument.
This is why a test result cached by hash is valid after any rename, and why
an attestation about one copy covers the other. -/
theorem same_identity_same_behaviour
    {P₁ P₂ : Prog} {ids₁ ids₂ : Ids} {store : Store}
    (h₁ : Consistent P₁ ids₁ store) (h₂ : Consistent P₂ ids₂ store)
    {f g x₁ x₂ : String} {b₁ b₂ : Expr}
    (hf : P₁ f = some (x₁, b₁)) (hg : P₂ g = some (x₂, b₂))
    (hid : ids₁ f = ids₂ g) (v : Value) (n : Nat) :
    evalS P₁ n [(x₁, v)] b₁ = evalS P₂ n [(x₂, v)] b₂ := by
  obtain ⟨h, hidf, hsf⟩ := h₁.defined f x₁ b₁ hf
  obtain ⟨h', hidg, hsg⟩ := h₂.defined g x₂ b₂ hg
  have hh : h = h' := by rw [hidf, hidg] at hid; exact Option.some.inj hid
  subst hh
  have hbody : lower ids₁ (some f) [x₁] b₁ = lower ids₂ (some g) [x₂] b₂ := by
    rw [hsf] at hsg; exact Option.some.inj hsg
  have e₁ := (lower_sound h₁ n).1 (some f) [(x₁, v)] b₁ _ (selfOK_some hf)
  have e₂ := (lower_sound h₂ n).1 (some g) [(x₂, v)] b₂ _ (selfOK_some hg)
  simp only [List.map_cons, List.map_nil] at e₁ e₂
  rw [e₁, e₂, hbody]

/-- **Same hash, same behaviour**, for any hash of the canonical form that
has no collisions on the forms compared. -/
theorem same_hash_same_behaviour
    (H : CExpr → Hash)
    {P₁ P₂ : Prog} {ids₁ ids₂ : Ids} {store : Store}
    (h₁ : Consistent P₁ ids₁ store) (h₂ : Consistent P₂ ids₂ store)
    {ρ₁ ρ₂ : Env} (hv : ρ₁.map Prod.snd = ρ₂.map Prod.snd) {e₁ e₂ : Expr}
    (hcollision : H (lower ids₁ none (ρ₁.map Prod.fst) e₁) = H (lower ids₂ none (ρ₂.map Prod.fst) e₂) →
      lower ids₁ none (ρ₁.map Prod.fst) e₁ = lower ids₂ none (ρ₂.map Prod.fst) e₂)
    (hhash : H (lower ids₁ none (ρ₁.map Prod.fst) e₁) = H (lower ids₂ none (ρ₂.map Prod.fst) e₂))
    (n : Nat) :
    evalS P₁ n ρ₁ e₁ = evalS P₂ n ρ₂ e₂ :=
  same_canon_same_behaviour h₁ h₂ hv (hcollision hhash) n

end Rigid
