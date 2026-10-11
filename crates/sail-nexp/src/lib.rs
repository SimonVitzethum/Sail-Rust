#![forbid(unsafe_code)]
//! Pure NExp / nconstraint algebra for the sail-rs compiler.
//!
//! Models the numeric type-level expressions of Sail
//! (`Nexp_*` in `sail/src/lib/extraction/Ast.ml`) as plain data plus
//! total, side-effect-free functions:
//!
//! - `expr`: AST (`NExp`, `NConstraint`, `TypArg`/`NTyp` type arguments),
//!   constructors, substitution, total-valuation evaluation. Covers all
//!   `Ast.ml` constructors: `Nexp_id/var/constant/app/if/times/sum/minus/
//!   exp/neg`, `A_nexp/A_typ/A_bool` (`typ_arg`), `Typ_id/var/fn/bidir/
//!   tuple/app/exist/unknown`, and `NC_equal/not_equal/ge/gt/le/lt/set/
//!   and/or/app/id/var/true/false` (`NC_equal`/`NC_not_equal`/`NC_app`
//!   carry `typ_arg` sorts, see [`TypArg`]).
//! - `conv`: total `*_to_ast` lowerings plus fallible `*_from_ast`
//!   liftings between canonical `sail-ast` nodes and this algebra.
//! - `simplify`: constant folding, identities, canonical normal forms
//!   (including sorted `And`/`Or` in [`normalize_constraint`], plus
//!   [`normalize_typarg`] for `typ_arg` payloads).
//! - `pred`: syntactic provability predicates (the full `provably_*`
//!   family: `nneg`, `pos`, `nonpos`, `neg`, `zero`, `nonzero`),
//!   the pure equivalent of the Lean backend's
//!   `provably_nneg ctx x = prove env (x >= 0)`.
//! - `lean`: `Nat`-vs-`Int` mapping for Lean emission.
//!
//! Operator coverage for every backend (`eval`, `simplify`,
//! sign analysis, Lean kinds, Z3 mapping) is tabulated in
//! `ARCHITECTURE.md` (Lane 80).
//!
//! There is deliberately no solver backend here: every predicate is
//! conservative (it only answers `true` when the fact follows
//! syntactically from the expression and the assumed [`pred::Facts`]).

pub mod conv;
pub mod expr;
pub mod lean;
pub mod pred;
pub mod simplify;

pub use conv::{
    constraint_from_ast, constraint_to_ast, nexp_from_ast, nexp_to_ast, nkind_erase,
    nkind_from_ast, ntyp_from_ast, ntyp_to_ast, typarg_from_ast, typarg_to_ast,
};
pub use expr::{Kid, NConstraint, NExp, NKind, NQuant, NTyp, TypArg};
pub use lean::{int_kind, int_kind_in, lean_type_name, IntKind};
pub use pred::{
    provably_neg, provably_neg_in, provably_nneg, provably_nneg_in, provably_nonpos,
    provably_nonpos_in, provably_nonzero, provably_nonzero_in, provably_pos, provably_pos_in,
    provably_zero, provably_zero_in, Facts,
};
pub use simplify::{
    normalize, normalize_constraint, normalize_typarg, simplify, simplify_constraint,
    simplify_constraint_cow, simplify_cow, simplify_typarg, simplify_typarg_cow,
};
