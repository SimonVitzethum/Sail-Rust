#![forbid(unsafe_code)]
//! Pure NExp / nconstraint algebra for the sail-rs compiler.
//!
//! Models the numeric type-level expressions of Sail
//! (`Nexp_*` in `sail/src/lib/extraction/Ast.ml`) as plain data plus
//! total, side-effect-free functions:
//!
//! - `expr`: AST (`NExp`, `NConstraint`), constructors, substitution,
//!   total-valuation evaluation.
//! - `simplify`: constant folding, identities, canonical normal forms.
//! - `pred`: syntactic provability predicates (`provably_nneg` and
//!   friends), the pure equivalent of the Lean backend's
//!   `provably_nneg ctx x = prove env (x >= 0)`.
//! - `lean`: `Nat`-vs-`Int` mapping for Lean emission.
//!
//! There is deliberately no solver backend here: every predicate is
//! conservative (it only answers `true` when the fact follows
//! syntactically from the expression and the assumed [`pred::Facts`]).

pub mod expr;
pub mod lean;
pub mod pred;
pub mod simplify;

pub use expr::{Kid, NConstraint, NExp};
pub use lean::{int_kind, int_kind_in, lean_type_name, IntKind};
pub use pred::{
    provably_neg, provably_neg_in, provably_nneg, provably_nneg_in, provably_nonpos,
    provably_nonpos_in, provably_nonzero, provably_nonzero_in, provably_pos, provably_pos_in,
    provably_zero, provably_zero_in, Facts,
};
pub use simplify::{normalize, normalize_constraint, simplify, simplify_constraint};
