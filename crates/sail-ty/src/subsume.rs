#![forbid(unsafe_code)]
//! Subsumption (`subtyp`), numeric coercions, and scheme instantiation.
//!
//! Lane 79: this is the Sail `subtyp`/`coerce` layer of the bidirectional
//! checker. Everything solver-dependent goes through the generic
//! [`Solver`](crate::solver::Solver) trait — there is deliberately no Z3
//! import here (Z3 backends implement the trait in Lane 06/80).
//!
//! Rules covered: `S-Refl`, `S-RangeInt`, `S-IntRangeFail`, `S-RangeRange`,
//! `S-BitsEq`, `S-FunContraCo`, `S-TupleArity`, `S-NamedArgs`,
//! `S-BaseMismatch`, `X-LitToRange`, `X-RangeToInt`, `Q-Inst`, `Q-Freshen`
//! (see [`crate::rules::RULE_TABLE`]).

use std::collections::HashMap;

use crate::error::{Span, TypeError, TypeErrorKind, TypeResult};
use crate::nexp::{MiniTyp, NConstraint, Nexp, TypQuant};
use crate::solver::{ProveResult, Solver};

// ---------------------------------------------------------------------------
// Level substitution + freshening
// ---------------------------------------------------------------------------

/// Capture-avoiding substitution on level (`K_int`) variables.
#[derive(Clone, Debug, Default)]
pub struct NexpSubst {
    map: HashMap<String, Nexp>,
}

impl NexpSubst {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(&mut self, var: String, to: Nexp) {
        self.map.insert(var, to);
    }

    #[must_use]
    pub fn apply_nexp(&self, n: &Nexp) -> Nexp {
        match n {
            Nexp::Const(_) => n.clone(),
            Nexp::Var(v) => self.map.get(v).cloned().unwrap_or_else(|| n.clone()),
            Nexp::Neg(a) => Nexp::Neg(Box::new(self.apply_nexp(a))),
            Nexp::Abs(a) => Nexp::Abs(Box::new(self.apply_nexp(a))),
            Nexp::Exp(a) => Nexp::Exp(Box::new(self.apply_nexp(a))),
            Nexp::Add(a, b) => {
                Nexp::Add(Box::new(self.apply_nexp(a)), Box::new(self.apply_nexp(b)))
            }
            Nexp::Sub(a, b) => {
                Nexp::Sub(Box::new(self.apply_nexp(a)), Box::new(self.apply_nexp(b)))
            }
            Nexp::Mul(a, b) => {
                Nexp::Mul(Box::new(self.apply_nexp(a)), Box::new(self.apply_nexp(b)))
            }
            Nexp::Div(a, b) => {
                Nexp::Div(Box::new(self.apply_nexp(a)), Box::new(self.apply_nexp(b)))
            }
            Nexp::Mod(a, b) => {
                Nexp::Mod(Box::new(self.apply_nexp(a)), Box::new(self.apply_nexp(b)))
            }
            Nexp::If(c, a, b) => Nexp::If(
                Box::new(self.apply_constraint(c)),
                Box::new(self.apply_nexp(a)),
                Box::new(self.apply_nexp(b)),
            ),
        }
    }

    #[must_use]
    pub fn apply_constraint(&self, c: &NConstraint) -> NConstraint {
        match c {
            NConstraint::True | NConstraint::False => c.clone(),
            NConstraint::Eq(a, b) => NConstraint::Eq(
                Box::new(self.apply_nexp(a)),
                Box::new(self.apply_nexp(b)),
            ),
            NConstraint::Neq(a, b) => NConstraint::Neq(
                Box::new(self.apply_nexp(a)),
                Box::new(self.apply_nexp(b)),
            ),
            NConstraint::Ge(a, b) => NConstraint::Ge(
                Box::new(self.apply_nexp(a)),
                Box::new(self.apply_nexp(b)),
            ),
            NConstraint::Gt(a, b) => NConstraint::Gt(
                Box::new(self.apply_nexp(a)),
                Box::new(self.apply_nexp(b)),
            ),
            NConstraint::Le(a, b) => NConstraint::Le(
                Box::new(self.apply_nexp(a)),
                Box::new(self.apply_nexp(b)),
            ),
            NConstraint::Lt(a, b) => NConstraint::Lt(
                Box::new(self.apply_nexp(a)),
                Box::new(self.apply_nexp(b)),
            ),
            NConstraint::And(a, b) => NConstraint::And(
                Box::new(self.apply_constraint(a)),
                Box::new(self.apply_constraint(b)),
            ),
            NConstraint::Or(a, b) => NConstraint::Or(
                Box::new(self.apply_constraint(a)),
                Box::new(self.apply_constraint(b)),
            ),
            NConstraint::Not(a) => NConstraint::Not(Box::new(self.apply_constraint(a))),
            NConstraint::Set(n, vs) => {
                NConstraint::Set(Box::new(self.apply_nexp(n)), vs.clone())
            }
            NConstraint::App(func, args) => NConstraint::App(
                func.clone(),
                args.iter().map(|a| self.apply_nexp(a)).collect(),
            ),
            // Boolean level variable: renaming flows through `Var`-to-`Var`
            // mappings (OCaml `typquant_subst_kid`: freshening a `K_bool`
            // binder must rename its `NC_var` occurrences, otherwise the
            // freshened scheme mentions a stale, unbound name); constant
            // mappings fold to truth values (mirrors
            // `NConstraint::subst_cow`); anything else leaves the variable
            // untouched (an ill-kinded non-var mapping is rejected by the
            // kind check, never silently applied).
            NConstraint::Var(v) => match self.map.get(v) {
                Some(Nexp::Var(w)) => NConstraint::Var(w.clone()),
                Some(Nexp::Const(0)) => NConstraint::False,
                Some(Nexp::Const(_)) => NConstraint::True,
                _ => c.clone(),
            },
            NConstraint::Id(_) => c.clone(),
        }
    }

    #[must_use]
    pub fn apply_typ(&self, t: &MiniTyp) -> MiniTyp {
        match t {
            MiniTyp::Int | MiniTyp::Bool | MiniTyp::Bit => t.clone(),
            MiniTyp::Bits(n) => MiniTyp::Bits(Box::new(self.apply_nexp(n))),
            MiniTyp::Range(lo, hi) => MiniTyp::Range(
                Box::new(self.apply_nexp(lo)),
                Box::new(self.apply_nexp(hi)),
            ),
            MiniTyp::Named { name, args } => MiniTyp::Named {
                name: name.clone(),
                args: args.iter().map(|a| self.apply_nexp(a)).collect(),
            },
            MiniTyp::Fun(a, b) => MiniTyp::Fun(
                Box::new(self.apply_typ(a)),
                Box::new(self.apply_typ(b)),
            ),
            MiniTyp::Tuple(ts) => {
                MiniTyp::Tuple(ts.iter().map(|t| self.apply_typ(t)).collect())
            }
        }
    }
}

/// Freshen all bound variables of `scheme` (rule `Q-Freshen`).
///
/// Capture-avoiding skeleton for later monomorphisation (Lane 09): since
/// [`MiniTyp`] carries no level binders, a global rename of the telescope
/// is sufficient — no shadowing can occur inside constraint/body. Names
/// come from [`crate::nexp::gensym`] (OCaml `Type_env.fresh_kid`): the old
/// predictable `{base}{i}` names could collide with user binders (a scheme
/// may already bind `f0`) and two freshenings with the same base captured
/// each other.
#[must_use]
pub fn freshen_scheme(scheme: &TypQuant, base: &str) -> TypQuant {
    let mut subst = NexpSubst::new();
    let mut quants = Vec::with_capacity(scheme.quants.len());
    for (i, (v, k)) in scheme.quants.iter().enumerate() {
        let fresh = crate::nexp::gensym(&format!("{base}{i}"));
        subst.bind(v.clone(), Nexp::Var(fresh.clone()));
        quants.push((fresh, *k));
    }
    TypQuant {
        quants,
        constraint: subst.apply_constraint(&scheme.constraint),
        typ: subst.apply_typ(&scheme.typ),
    }
}

/// Instantiate a scheme at a use site (rule `Q-Inst`): freshen, then split
/// into the proof obligation and the monomorphic body type.
///
/// The caller discharges the obligation with its [`Solver`]; a `Refuted`
/// obligation is a hard error, `Proved`/`Unknown` mirror the `check_val_def`
/// precondition policy (sound: unknown preconditions keep the abstract
/// body, they never fabricate a proof).
#[must_use]
pub fn instantiate_scheme(scheme: &TypQuant) -> (NConstraint, MiniTyp) {
    let fresh = freshen_scheme(scheme, "inst");
    (fresh.constraint, fresh.typ)
}

// ---------------------------------------------------------------------------
// Subsumption + coercions
// ---------------------------------------------------------------------------

/// The coercion witnessing a successful subsumption step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Coercion {
    /// Reflexivity: no runtime/codegen effect.
    Identity,
    /// `range(lo,hi) <: int`: forget the bounds.
    RangeToInt,
    /// `range(lo1,hi1) <: range(lo2,hi2)`: narrow via proved inclusion.
    RangeNarrow,
    /// `bits(n) <: bits(m)` with `n == m` proved.
    BitsCongruence,
    /// Function subsumption (contravariant domain, covariant codomain).
    Fun,
    /// Tuple subsumption of arity `n` (pointwise).
    Tuple(usize),
    /// Same-head type abbreviation with provably equal level arguments.
    NamedUnfold(String),
}

impl std::fmt::Display for Coercion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identity => write!(f, "identity"),
            Self::RangeToInt => write!(f, "range-to-int"),
            Self::RangeNarrow => write!(f, "range-narrow"),
            Self::BitsCongruence => write!(f, "bits-congruence"),
            Self::Fun => write!(f, "fun"),
            Self::Tuple(n) => write!(f, "tuple({n})"),
            Self::NamedUnfold(name) => write!(f, "unfold({name})"),
        }
    }
}

/// Outcome of the pure (solver-free) subsumption analysis.
#[derive(Clone, Debug)]
pub enum Subsumption {
    /// Holds syntactically with this coercion.
    Holds(Coercion),
    /// Fails syntactically with a human-readable reason.
    Fails(String),
    /// Holds iff the solver proves the obligation (checked soundly:
    /// only [`ProveResult::Proved`] accepts).
    NeedsProof(NConstraint, Coercion),
}

impl Subsumption {
    fn obligation(&self) -> Option<NConstraint> {
        match self {
            Self::NeedsProof(nc, _) => Some(nc.clone()),
            _ => None,
        }
    }
}

fn and_all(mut obligations: Vec<NConstraint>) -> Option<NConstraint> {
    let mut acc: Option<NConstraint> = None;
    // Reverse so the conjunction reads left-to-right in source order.
    obligations.reverse();
    while let Some(nc) = obligations.pop() {
        acc = Some(match acc {
            None => nc,
            Some(prev) => NConstraint::And(Box::new(prev), Box::new(nc)),
        });
    }
    acc
}

/// Pure subsumption analysis: `inferred <: expected`?
///
/// Covers `S-Refl`, `S-RangeInt`, `S-IntRangeFail`, `S-RangeRange`,
/// `S-BitsEq`, `S-FunContraCo`, `S-TupleArity`, `S-NamedArgs`,
/// `S-BaseMismatch`. Never calls the solver; use [`check_subtype`] to
/// discharge [`Subsumption::NeedsProof`].
#[must_use]
pub fn subtype_obligation(inferred: &MiniTyp, expected: &MiniTyp) -> Subsumption {
    if inferred == expected {
        return Subsumption::Holds(Coercion::Identity);
    }
    match (inferred, expected) {
        // `range <: int` always holds: bounds are forgotten (X-RangeToInt).
        (MiniTyp::Range(_, _), MiniTyp::Int) => {
            Subsumption::Holds(Coercion::RangeToInt)
        }
        // `int <: range` never holds without a value: literals are handled
        // at value sites (`check` special case, X-LitToRange). (S-IntRangeFail)
        (MiniTyp::Int, MiniTyp::Range(_, _)) => Subsumption::Fails(
            "int is not a subtype of range(..) outside literal sites".to_string(),
        ),
        // Range inclusion: lo2 <= lo1 /\ hi1 <= hi2. (S-RangeRange)
        (MiniTyp::Range(lo1, hi1), MiniTyp::Range(lo2, hi2)) => {
            let nc = NConstraint::And(
                Box::new(NConstraint::Le(lo2.clone(), lo1.clone())),
                Box::new(NConstraint::Le(hi1.clone(), hi2.clone())),
            );
            Subsumption::NeedsProof(nc, Coercion::RangeNarrow)
        }
        // `bits` lengths must be provably equal. (S-BitsEq)
        (MiniTyp::Bits(n), MiniTyp::Bits(m)) => Subsumption::NeedsProof(
            NConstraint::Eq(n.clone(), m.clone()),
            Coercion::BitsCongruence,
        ),
        // Functions: contravariant domain, covariant codomain. (S-FunContraCo)
        (MiniTyp::Fun(a1, b1), MiniTyp::Fun(a2, b2)) => {
            let dom = subtype_obligation(a2, a1);
            let cod = subtype_obligation(b1, b2);
            match (&dom, &cod) {
                (Subsumption::Holds(_), Subsumption::Holds(_)) => {
                    Subsumption::Holds(Coercion::Fun)
                }
                (Subsumption::Fails(e), _) | (_, Subsumption::Fails(e)) => {
                    Subsumption::Fails(format!("fun: {e}"))
                }
                _ => {
                    let mut obs = Vec::new();
                    if let Some(o) = dom.obligation() {
                        obs.push(o);
                    }
                    if let Some(o) = cod.obligation() {
                        obs.push(o);
                    }
                    match and_all(obs) {
                        Some(nc) => Subsumption::NeedsProof(nc, Coercion::Fun),
                        None => Subsumption::Holds(Coercion::Fun),
                    }
                }
            }
        }
        // Tuples: pointwise, arity must match. (S-TupleArity)
        (MiniTyp::Tuple(ts1), MiniTyp::Tuple(ts2)) => {
            if ts1.len() != ts2.len() {
                return Subsumption::Fails(format!(
                    "tuple arity: inferred {} vs expected {}",
                    ts1.len(),
                    ts2.len()
                ));
            }
            let mut obs = Vec::new();
            for (a, b) in ts1.iter().zip(ts2.iter()) {
                match subtype_obligation(a, b) {
                    Subsumption::Holds(_) => {}
                    Subsumption::Fails(e) => {
                        return Subsumption::Fails(format!("tuple: {e}"));
                    }
                    Subsumption::NeedsProof(nc, _) => obs.push(nc),
                }
            }
            match and_all(obs) {
                Some(nc) => Subsumption::NeedsProof(nc, Coercion::Tuple(ts1.len())),
                None => Subsumption::Holds(Coercion::Tuple(ts1.len())),
            }
        }
        // Same-head abbreviations: level arguments must be provably equal.
        // (S-NamedArgs)
        (
            MiniTyp::Named { name: n1, args: a1 },
            MiniTyp::Named { name: n2, args: a2 },
        ) => {
            if n1 != n2 || a1.len() != a2.len() {
                return Subsumption::Fails(format!(
                    "named type mismatch: {inferred} vs {expected}"
                ));
            }
            let mut obs = Vec::new();
            for (x, y) in a1.iter().zip(a2.iter()) {
                if x != y {
                    obs.push(NConstraint::Eq(
                        Box::new(x.clone()),
                        Box::new(y.clone()),
                    ));
                }
            }
            match and_all(obs) {
                Some(nc) => {
                    Subsumption::NeedsProof(nc, Coercion::NamedUnfold(n1.clone()))
                }
                None => Subsumption::Holds(Coercion::NamedUnfold(n1.clone())),
            }
        }
        _ => Subsumption::Fails(format!("{inferred} is not a subtype of {expected}")),
    }
}

/// Discharge `inferred <: expected`, proving numeric obligations with
/// `solver` (rule `S-*` family + `Z-UnknownSound`).
///
/// Only [`ProveResult::Proved`] accepts a [`Subsumption::NeedsProof`]
/// obligation; `Refuted` and `Unknown` are sound errors. Tuple arity
/// mismatches surface as [`TypeErrorKind::ArityMismatch`](crate::error::TypeErrorKind).
pub fn check_subtype<S: Solver>(
    solver: &S,
    inferred: &MiniTyp,
    expected: &MiniTyp,
    span: Span,
) -> TypeResult<Coercion> {
    // Preserve the precise arity error of the legacy `unify`.
    if let (MiniTyp::Tuple(ts1), MiniTyp::Tuple(ts2)) = (inferred, expected) {
        if ts1.len() != ts2.len() {
            return Err(TypeError {
                span,
                kind: TypeErrorKind::ArityMismatch {
                    expected: ts2.len(),
                    found: ts1.len(),
                },
            });
        }
    }
    match subtype_obligation(inferred, expected) {
        Subsumption::Holds(c) => Ok(c),
        Subsumption::Fails(msg) => Err(TypeError::mismatch(
            span,
            expected.to_string(),
            format!("{inferred} ({msg})"),
        )),
        Subsumption::NeedsProof(nc, c) => match solver.prove(&nc) {
            ProveResult::Proved => Ok(c),
            ProveResult::Refuted | ProveResult::Unknown => {
                Err(TypeError::constraint_not_proved(span, &nc))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_var_substitution_renames_through_freshening() {
        // `NC_var` renaming (OCaml `typquant_subst_kid`): freshening a
        // `K_bool`-quantified scheme must rename its `Var` occurrences —
        // the old fold-only substitution left the stale name behind,
        // producing obligations over unbound variables.
        let scheme = TypQuant {
            quants: vec![("b".to_string(), crate::kind::Kind::KBool)],
            constraint: NConstraint::Var("b".to_string()),
            typ: MiniTyp::Bool,
        };
        let fresh = freshen_scheme(&scheme, "sk");
        assert_eq!(fresh.quants.len(), 1);
        assert_eq!(fresh.quants[0].1, crate::kind::Kind::KBool);
        assert_ne!(fresh.quants[0].0, "b");
        assert_eq!(
            fresh.constraint,
            NConstraint::Var(fresh.quants[0].0.clone())
        );
    }

    #[test]
    fn gensym_freshening_never_collides_or_repeats() {
        // Predictable `{base}{i}` names captured user binders (`f0`) and
        // collided across freshenings; gensym names carry `#fv` (illegal in
        // Sail source, like OCaml `'fv<N>#` kids) and never repeat.
        let scheme = TypQuant {
            quants: vec![("n".to_string(), crate::kind::Kind::KInt)],
            constraint: NConstraint::True,
            typ: MiniTyp::Bits(Box::new(Nexp::Var("n".to_string()))),
        };
        let a = freshen_scheme(&scheme, "f");
        let b = freshen_scheme(&scheme, "f");
        for name in [&a.quants[0].0, &b.quants[0].0] {
            assert!(name.contains("#fv"), "gensym shape, got {name}");
            assert_ne!(name, "n");
        }
        assert_ne!(a.quants[0].0, b.quants[0].0);
    }
}
