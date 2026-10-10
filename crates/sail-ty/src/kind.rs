#![forbid(unsafe_code)]
//! Kind checking: `K_int`, `K_bool`, `K_type`.
//!
//! Mirrors Sail `kind` (`K_int | K_bool | K_type`, plus `K_nat` treated as a
//! `K_int` sub-kind at this layer — see [`Kind::KNat`]).

use crate::error::{Span, TypeError, TypeResult};
use crate::nexp::{MiniTyp, NConstraint, Nexp};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Sail kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    /// `K_int`: level (numeric) kind.
    KInt,
    /// `K_nat`: natural-number sub-kind of `K_int` (for Lean `Nat` mapping).
    /// Kept distinct so Lane 04/10 can map `Nat` vs `Int` precisely;
    /// subsumes to [`Kind::KInt`] wherever a `K_int` is expected.
    KNat,
    /// `K_bool`: bool-level (constraint) kind.
    KBool,
    /// `K_type`: proper type kind.
    KType,
}

impl Kind {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::KInt => "K_int",
            Self::KNat => "K_nat",
            Self::KBool => "K_bool",
            Self::KType => "K_type",
        }
    }

    /// `K_nat <: K_int` subsumption used at bind sites.
    #[must_use]
    pub fn subsumes(&self, other: Kind) -> bool {
        *self == other || (*self == Self::KInt && other == Self::KNat)
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Kind environment: maps level variables to their kinds.
///
/// Retained (MIGRATION.md #23, Lane 72 decision: kept local, see
/// [`crate::ast_env`]): a pure binding map, not an AST node;
/// `sail-ast` has no kind-context type (`Env.t` is checker state, not data).
/// [`Kind`] itself erases onto `sail_ast::Kind` (see [`crate::conv`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindEnv {
    vars: HashMap<String, Kind>,
}

impl KindEnv {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(&mut self, var: impl Into<String>, kind: Kind) {
        self.vars.insert(var.into(), kind);
    }

    /// Iterate `(variable, kind)` bindings (order is hash order; callers
    /// needing determinism must sort — see [`crate::ast_env`]).
    pub fn iter(&self) -> impl Iterator<Item = (&str, Kind)> + '_ {
        self.vars.iter().map(|(var, kind)| (var.as_str(), *kind))
    }

    #[must_use]
    pub fn lookup(&self, var: &str) -> Option<Kind> {
        self.vars.get(var).copied()
    }
}

/// Check that `nexp` has kind `K_int` (or `K_nat`).
pub fn check_nexp_kind(nexp: &Nexp, env: &KindEnv, span: Span) -> TypeResult<Kind> {
    match nexp {
        Nexp::Const(c) => {
            if *c >= 0 {
                Ok(Kind::KNat)
            } else {
                Ok(Kind::KInt)
            }
        }
        Nexp::Var(v) => match env.lookup(v) {
            Some(k @ (Kind::KInt | Kind::KNat)) => Ok(k),
            Some(other) => Err(TypeError::kind_mismatch(span, Kind::KInt, other)),
            None => Err(TypeError::unbound_var(span, v.clone())),
        },
        Nexp::Neg(n) | Nexp::Abs(n) => {
            check_nexp_kind(n, env, span)?;
            Ok(Kind::KInt)
        }
        Nexp::Exp(n) => {
            // `2^n`: argument must be `K_int`; result is `K_nat` if provably
            // non-negative constant, else `K_int`.
            check_nexp_kind(n, env, span)?;
            match n.as_const() {
                Some(v) if v >= 0 => Ok(Kind::KNat),
                _ => Ok(Kind::KInt),
            }
        }
        Nexp::Add(a, b) | Nexp::Sub(a, b) | Nexp::Mul(a, b) => {
            let ka = check_nexp_kind(a, env, span)?;
            let kb = check_nexp_kind(b, env, span)?;
            expect_int_like(span, ka)?;
            expect_int_like(span, kb)?;
            // `nat + nat = nat`; anything with `-`/mixed is `int`.
            match (ka, kb, nexp) {
                (Kind::KNat, Kind::KNat, Nexp::Add(_, _) | Nexp::Mul(_, _)) => Ok(Kind::KNat),
                _ => Ok(Kind::KInt),
            }
        }
        Nexp::Div(a, b) | Nexp::Mod(a, b) => {
            let ka = check_nexp_kind(a, env, span)?;
            let kb = check_nexp_kind(b, env, span)?;
            expect_int_like(span, ka)?;
            expect_int_like(span, kb)?;
            Ok(Kind::KInt)
        }
        Nexp::If(c, a, b) => {
            check_constraint_kind(c, env, span)?;
            let ka = check_nexp_kind(a, env, span)?;
            let kb = check_nexp_kind(b, env, span)?;
            expect_int_like(span, ka)?;
            expect_int_like(span, kb)?;
            if ka == Kind::KNat && kb == Kind::KNat {
                Ok(Kind::KNat)
            } else {
                Ok(Kind::KInt)
            }
        }
    }
}

fn expect_int_like(span: Span, got: Kind) -> TypeResult<()> {
    if got == Kind::KInt || got == Kind::KNat {
        Ok(())
    } else {
        Err(TypeError::kind_mismatch(span, Kind::KInt, got))
    }
}

/// Check that `nc` has kind `K_bool`.
pub fn check_constraint_kind(
    nc: &NConstraint,
    env: &KindEnv,
    span: Span,
) -> TypeResult<Kind> {
    match nc {
        NConstraint::True | NConstraint::False => Ok(Kind::KBool),
        NConstraint::Eq(a, b)
        | NConstraint::Neq(a, b)
        | NConstraint::Ge(a, b)
        | NConstraint::Gt(a, b)
        | NConstraint::Le(a, b)
        | NConstraint::Lt(a, b) => {
            check_nexp_kind(a, env, span)?;
            check_nexp_kind(b, env, span)?;
            Ok(Kind::KBool)
        }
        NConstraint::And(a, b) | NConstraint::Or(a, b) => {
            check_constraint_kind(a, env, span)?;
            check_constraint_kind(b, env, span)?;
            Ok(Kind::KBool)
        }
        NConstraint::Not(a) => {
            check_constraint_kind(a, env, span)?;
            Ok(Kind::KBool)
        }
    }
}

/// Check that `typ` has kind `K_type`.
pub fn check_typ_kind(typ: &MiniTyp, env: &KindEnv, span: Span) -> TypeResult<Kind> {
    match typ {
        MiniTyp::Int | MiniTyp::Bool | MiniTyp::Bit => Ok(Kind::KType),
        MiniTyp::Bits(n) => {
            let k = check_nexp_kind(n, env, span)?;
            expect_int_like(span, k)?;
            // Reject provably negative lengths syntactically.
            if let Some(c) = n.as_const() {
                if c < 0 {
                    return Err(TypeError::negative_bits(span, c));
                }
            }
            Ok(Kind::KType)
        }
        MiniTyp::Range(lo, hi) => {
            let klo = check_nexp_kind(lo, env, span)?;
            let khi = check_nexp_kind(hi, env, span)?;
            expect_int_like(span, klo)?;
            expect_int_like(span, khi)?;
            Ok(Kind::KType)
        }
        MiniTyp::Named { args, .. } => {
            for a in args {
                let k = check_nexp_kind(a, env, span)?;
                expect_int_like(span, k)?;
            }
            Ok(Kind::KType)
        }
        MiniTyp::Fun(a, b) => {
            check_typ_kind(a, env, span)?;
            check_typ_kind(b, env, span)?;
            Ok(Kind::KType)
        }
        MiniTyp::Tuple(ts) => {
            for t in ts {
                check_typ_kind(t, env, span)?;
            }
            Ok(Kind::KType)
        }
    }
}
