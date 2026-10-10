# `sail-ty` rule coverage (Lane 79)

Source of truth: `src/rules.rs` (`RULE_TABLE`). This file mirrors it for
readers. Status values: `implemented` | `deferred` (with owner lane).

## NExp kinds (`K_int` / `K_nat`)

| Regel | Sail-Ref | Status | Test |
|---|---|---|---|
| K-Const | nexp kind (const) | implemented | `kind_nexp_arith` |
| K-Var | nexp kind (var) | implemented | `kind_nexp_arith` |
| K-Arith | nexp kind (`+`/`-`/`*`) | implemented | `kind_nexp_arith` |
| K-DivMod | nexp kind (`/`/`mod`) | implemented | `kind_nexp_arith` |
| K-Neg | nexp kind (neg) | implemented | `kind_nexp_arith` |
| K-Exp | nexp kind (`2^n`) | implemented | `kind_nexp_arith` |
| K-Abs | nexp kind (`abs`) | implemented | `kind_nexp_arith` |
| K-If | nexp kind (if) | implemented | `kind_nexp_if` |
| K-NatSub | `K_nat <: K_int` | implemented | `kind_nexp_arith` |

## Constraint kinds (`K_bool`)

| Regel | Sail-Ref | Status | Test |
|---|---|---|---|
| CK-TrueFalse | constraint kind | implemented | `constraint_kinds` |
| CK-Cmp | constraint kind (comparisons) | implemented | `constraint_kinds` |
| CK-SetApp | `NC_set`/`NC_app`/`NC_var`/`NC_id` kinds + folding | implemented | `prove_set_app` |
| CK-AndOr | constraint kind (`/\`/`\/`) | implemented | `constraint_kinds` |
| CK-Not | constraint kind (`~`) | implemented | `constraint_kinds` |

## Type kinds (`K_type`)

| Regel | Sail-Ref | Status | Test |
|---|---|---|---|
| T-IntBoolBit | typ kind (base) | implemented | `typ_kinds` |
| T-Bits | typ kind (`bits`) | implemented | `typ_kinds` |
| T-BitsNeg | negative lengths rejected | implemented | `typ_bits_negative` |
| T-Range | typ kind (`range`) | implemented | `typ_kinds` |
| T-Named | typ kind (abbrev) | implemented | `typ_kinds` |
| T-Fun | typ kind (`->`) | implemented | `typ_kinds` |
| T-Tuple | typ kind (tuple) | implemented | `typ_kinds` |

## Quantifiers / schemes

| Regel | Sail-Ref | Status | Test |
|---|---|---|---|
| Q-Forall | telescope well-formedness | implemented | `scheme_closed_and_escape` |
| Q-Escape | no escaping free vars | implemented | `scheme_closed_and_escape` |
| Q-DepthFuel | depth bound ⇒ termination | implemented | `scheme_depth_fuel` |
| Q-Inst | instantiation at use sites | implemented | `quant_freshen_instantiate` |
| Q-Freshen | capture-avoiding freshening | implemented | `quant_freshen_instantiate` |

## Definitions

| Regel | Sail-Ref | Status | Test |
|---|---|---|---|
| D-ValMono | val body checking (mono) | implemented | `val_defs` |
| D-ValPoly | val body checking (poly, abstract) | implemented | `val_defs` |
| D-ValPreRefuted | refuted preconditions error | implemented | `val_defs` |
| D-RegMono | register checking (mono) | implemented | `registers` |
| D-RegPoly | register checking (poly) | implemented | `registers` |
| D-RegBad | ill-kinded registers rejected | implemented | `registers` |
| D-Scattered | scattered clauses under accumulated solver context | implemented | `scattered_clauses` |

## Inference / checking

| Regel | Sail-Ref | Status | Test |
|---|---|---|---|
| I-Int / I-Bool / I-VarMono / I-Annot | infer (lits/vars/annot) | implemented | `infer_lits_vars` |
| I-VarPoly | infer (poly instantiation) | implemented | `infer_poly_var` |
| I-Tuple / I-If / I-Let / I-App / I-AppHeadErr | infer (compound) | implemented | `infer_tuple_if_let_app` |
| C-LitRangeConst / C-LitRangeSolver / X-LitToRange | literal `<= range` | implemented | `check_literal_range` |
| C-BitsLength / C-FallbackSub | bits + infer/subsumption fallback | implemented | `check_bits_and_fallback` |

## Subsumption / coercions

| Regel | Sail-Ref | Status | Test |
|---|---|---|---|
| S-Refl / S-RangeInt / S-IntRangeFail / S-BaseMismatch / X-RangeToInt | subtyp (base) | implemented | `subsumption_base` |
| S-RangeRange / S-BitsEq | subtyp (numeric, solver-backed) | implemented | `subsumption_range_bits` |
| S-FunContraCo / S-TupleArity / S-NamedArgs | subtyp (structural) | implemented | `subsumption_fun_tuple_named` |

## Patterns

| Regel | Sail-Ref | Status | Test |
|---|---|---|---|
| P-Wild / P-Var / P-Lit / P-Tuple / P-Annot / P-ArityErr | pattern typing | implemented | `patterns` |

## Solver policy / environment

| Regel | Sail-Ref | Status | Test |
|---|---|---|---|
| Z-TraitOnly / Z-StubTotal / Z-UnknownSound | solver interface (trait-only, total stub, sound `Unknown`) | implemented | `solver_trait_only_backend` |
| Z-CacheBounded / Z-FuelTerm | bounded cache + fuel termination | implemented | `caching_and_fuel` |
| Z-ProveCtx | kind-checked goals + val/register/scattered assumptions via trait | implemented | `prove_solver_context` |
| E-LocalShadow / E-MonoProject / E-PolyOpaque | environment scoping | implemented | `env_scoping` |

## Deferred (explicit, with owner)

| Regel | Grund | Test |
|---|---|---|
| X-Existential | `Typ_exist` elaboration lives in Lane 09 | `deferred_documented` |
| X-FullExp | full `exp_aux` port lives in Lane 02 | `deferred_documented` |
| X-FullPat | full `pat_aux` port lives in Lane 02 | `deferred_documented` |
| X-NcSetApp | foreign `NC_set`/`NC_app`/`NC_var` shapes only (`NExp_id`, non-`NExp` `typ_arg`s, unparseable `Set` members); local forms are implemented (CK-SetApp) | `deferred_documented` |
