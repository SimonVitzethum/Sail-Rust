#![forbid(unsafe_code)]
//! Minimal value patterns for the bidirectional checker (Lane 79).
//!
//! Covers the pattern rules `P-Wild`, `P-Var`, `P-Lit`, `P-Tuple`, `P-Annot`
//! and `P-ArityErr`. Richer Sail patterns (`as`, `or`, vector/struct/list
//! patterns) stay deferred to the Lane 02 full-`Exp` port (rule `X-FullPat`)
//! — inside this subset there are no check gaps.

use crate::nexp::MiniTyp;

/// Minimal pattern (subset of Sail `pat_aux`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiniPat {
    /// `_`
    Wild,
    /// `x` — binds the scrutinee type.
    Var(String),
    /// Integer literal pattern.
    IntLit(i128),
    /// Boolean literal pattern.
    BoolLit(bool),
    /// `(p1, p2, ...)` — scrutinee must be a tuple of the same arity.
    Tuple(Vec<MiniPat>),
    /// `(p : t)` — check `p` against `t`, then require `t <: expected`.
    Annot(Box<MiniPat>, MiniTyp),
}
