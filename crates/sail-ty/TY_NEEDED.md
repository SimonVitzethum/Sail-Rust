# `TY_NEEDED.md` — what `sail-ty` needs from `sail-ast` / `sail-nexp`

> Lane 61 status (MIGRATION.md #17–#25): **SPAN CLOSED, BRIDGES WIRED**.
> `error::Span` is `sail_ast::Span` (#25). `src/conv.rs` lowers
> `Nexp`/`NConstraint`/`MiniTyp`/`TypQuant`/`MiniExp` onto canonical
> nodes and back (#17–#21; Div/Mod/Abs as `Nexp_app`, Eq/Neq via
> `Typ_arg`, Range as reversible `range`-app, currying/existentials as
> `None` pending Lanes 02/09). `Kind` erases `KNat→Int`, never
> silently recovered (#22). `KindEnv`/`TypeEnv` retained locally by Lane-72
> decision: no `Env.t` target in `sail-ast` (checker state, not data —
> see `sail-ty/src/ast_env.rs`); canonical views (`KOpt` telescope,
> `TypSchm` schemes) wired there. Driver file table landed centrally as
> `sail_ast::FileTable` (#3/#25 rest). `Solver`/`CachingSolver`
> untouched. Checklist items 2–5 become Lane 02/04/09 follow-ups;
> item 1 done, item 6 holds.
Lane 05 defines local minimal types (`sail_ty::nexp::{Nexp, NConstraint, MiniTyp, MiniExp, TypQuant}`)
because Lane 01 (`sail-ast`) and Lane 04 (`sail-nexp`) are not merged yet.
This file is the swap contract: when those crates land, replace the minimals field-by-field.

## From `sail-ast` (Lane 01)

| `sail-ty` minimal | Real `sail-ast` counterpart (`ast.ml`) | Notes |
|---|---|---|
| `Span { file, start, end }` | `Span` / `Lexing.position` pair | File table interning comes from Lane 01; keep `DUMMY` |
| `Nexp::{Const,Var,Add,Sub,Mul,Neg,Exp,Div,Mod,Abs,If}` | `Nexp_aux::{Nexp_constant,Nexp_var,Nexp_times,Nexp_sum,Nexp_minus,Nexp_exp,Nexp_neg}` (+ div/mod/abs extensions) | Real AST may add `Nexp_id`, `Nexp_app`; map `If` to Sail `n_constraint`-guarded nexp |
| `NConstraint::{True,False,Eq,Neq,Ge,Gt,Le,Lt,And,Or,Not}` | `NC_aux::{NC_true,NC_false,NC_equal,NC_not_equal,NC_ge,NC_gt,NC_le,NC_lt,NC_and,NC_or,NC_not,NC_var,NC_app,NC_set,NC_leq}` | `NC_set`/`NC_app` have no minimal yet — add on demand |
| `MiniTyp::{Int,Bool,Bit,Bits,Range,Named,Fun,Tuple}` | `Typ_aux::{Typ_int,Typ_bool,Typ_bit,Typ_bits,Typ_atom(?),Typ_app,Typ_fn,Typ_tuple,Typ_exist,…}` | `Range` ≈ `Typ_atom`-bounded int; `Named` ≈ `Typ_app`/`Typ_id`; existentials come with Lane 09 |
| `TypQuant { quants, constraint, typ }` | `TypQ_aux::{TypQ_tq,TypQ_no_forall}` + `kind_aux::{K_int,K_bool,K_type}` | `quants: Vec<(String, Kind)>` ≈ `quant_item` telescope |
| `MiniExp::{IntLit,BoolLit,Var,Let,App,Tuple,Annot,If}` | `Exp_aux` (tiny subset) | Full `Exp_aux` port is Lane 02/03; checker matches on subsets |
| `Kind::{KInt,KNat,KBool,KType}` | `kind_aux::{K_int,K_bool,K_type}` | `KNat` is a `sail-ty`-local refinement sub-kind for the Lean `Nat`/`Int` mapping (Lane 04/10); erase to `K_int` when converting |
| `TypeEnv { shared(vals, registers via Arc), locals }` | RETAINED locally (Lane 72: `Env.t` is checker state, not AST data) — canonical view `val_scheme_ast`/`register_scheme_ast -> TypSchm` in `sail-ty/src/ast_env.rs` | Real env carries kinds + constraints + locations; `TypeEnv` is the closed test skeleton; Lane 68 shares program tables via `Arc` (cheap `Clone` snapshots for workers) |

## From `sail-nexp` (Lane 04)

| `sail-ty` minimal | Real `sail-nexp` counterpart | Notes |
|---|---|---|
| `Nexp::as_const` | constant folding / `nexp_simp` | Lane 04 owns canonical simp; keep `as_const` as the no-solver fast path |
| `NConstraint::as_bool` | constraint simp + `provably_nneg` equivalent | Lane 04 decides `Nat`-vs-`Int`; `sail-ty` only needs bool folding + `depth` |
| `Nexp::depth` / fuel (`CheckLimits`) | normal-form size metric | Keep a size metric in Lane 04 so the checker fuel survives the swap |
| `Kind::KNat` subsumption | `Nat`-vs-`Int` decision for Lean printer | Lane 04/10 consume `KNat`; do not drop the distinction silently |

## Swap checklist (when Lanes 01+04 merge)

1. Replace `Span` with `sail_ast::Span` (keep `DUMMY` constructor).
2. Replace `Nexp`/`NConstraint` with `sail_nexp` types; re-implement `depth`/`free_vars`/`as_const`/`as_bool` as thin wrappers.
3. Replace `MiniTyp`/`TypQuant` with `sail_ast::{typ, typquant}`; keep `KNat` as wrapper refinement or move it into `sail-nexp`.
4. Replace `MiniExp` fragments with `sail_ast::exp`; extend `infer`/`check` match arms.
5. Replace `TypeEnv` with (a view over) `sail_ast::Env`; keep the `Clone + Send + Sync` snapshots for workers.
6. Keep `Solver` trait + `CachingSolver` untouched — Lane 06 implements the trait, no checker changes needed.
