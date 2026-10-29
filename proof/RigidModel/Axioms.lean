import RigidModel.Soundness
import RigidModel.Counterexample

/-!
# What the proofs rest on

Every theorem here depends only on Lean's three standard axioms (propositional
extensionality, choice, quotient soundness) — no `sorry`, nothing assumed
about rigid. If a proof ever comes to rest on anything else, these checks
fail the build.
-/

/-- info: 'Rigid.lower_sound' depends on axioms: [propext, Classical.choice, Quot.sound] -/
#guard_msgs in #print axioms Rigid.lower_sound

/-- info: 'Rigid.same_canon_same_behaviour' depends on axioms: [propext, Classical.choice, Quot.sound] -/
#guard_msgs in #print axioms Rigid.same_canon_same_behaviour

/-- info: 'Rigid.same_identity_same_behaviour' depends on axioms: [propext, Classical.choice, Quot.sound] -/
#guard_msgs in #print axioms Rigid.same_identity_same_behaviour

/-- info: 'Rigid.same_hash_same_behaviour' depends on axioms: [propext, Classical.choice, Quot.sound] -/
#guard_msgs in #print axioms Rigid.same_hash_same_behaviour

/-- info: 'Rigid.source_order_breaks_identity' depends on axioms: [propext, Classical.choice, Quot.sound] -/
#guard_msgs in #print axioms Rigid.source_order_breaks_identity
