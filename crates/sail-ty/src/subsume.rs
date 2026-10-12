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
//!
//! Lane 10 adds, on the same trait-only path: numeric/boolean synonym
//! expansion (`implicit('n)` ≡ `atom('n)`, `bitvector('n)` ≡ `bits('n)`,
//! `atom_bool` ≡ `bool`; OCaml `Env.expand_synonyms`), atom singleton
//! subsumption (`atom('n)` is the exact integer `'n`), and
//! simple-equation instantiation (OCaml `instantiate_simple_equations`:
//! omitted implicits solved by `Var == nexp` conjuncts).

use std::collections::HashMap;
use std::collections::HashSet;

use crate::error::{Span, TypeError, TypeErrorKind, TypeResult};
use crate::nexp::{MiniTyp, NConstraint, Nexp, TypQuant, linear_form};
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

/// Expand numeric/boolean type synonyms (OCaml `Env.expand_synonyms`,
/// local skeleton): `implicit('n)` ≡ `atom('n)` (argument-vs-return
/// normalization stays the caller's job, as in `implicit_to_int`),
/// `bitvector('n)` ≡ `bits('n)`, `atom_bool` ≡ `bool` (the local
/// [`MiniTyp::Named`] carries `Nexp`-only args, so any `atom_bool`
/// abbreviates `bool`). Unknown heads pass through untouched.
/// Pure and total.
#[must_use]
pub fn expand_synonyms_typ(t: &MiniTyp) -> MiniTyp {
    match t {
        MiniTyp::Int | MiniTyp::Bool | MiniTyp::Bit => t.clone(),
        MiniTyp::Bits(n) => MiniTyp::Bits(n.clone()),
        MiniTyp::Range(lo, hi) => MiniTyp::Range(lo.clone(), hi.clone()),
        MiniTyp::Named { name, args } if name == "implicit" => MiniTyp::Named {
            name: "atom".to_string(),
            args: args.clone(),
        },
        MiniTyp::Named { name, args } if name == "bitvector" && args.len() == 1 => {
            MiniTyp::Bits(Box::new(args[0].clone()))
        }
        MiniTyp::Named { name, .. } if name == "atom_bool" => MiniTyp::Bool,
        MiniTyp::Named { name, args } => MiniTyp::Named {
            name: name.clone(),
            args: args.clone(),
        },
        MiniTyp::Fun(a, b) => MiniTyp::Fun(
            Box::new(expand_synonyms_typ(a)),
            Box::new(expand_synonyms_typ(b)),
        ),
        MiniTyp::Tuple(ts) => {
            MiniTyp::Tuple(ts.iter().map(expand_synonyms_typ).collect())
        }
    }
}

/// Apply simple equations to a scheme telescope (OCaml `inst_from_eq`):
/// each quantifier solved by a top-level `Var == nexp` conjunct of the
/// scheme constraint (see [`crate::quant::simple_equations`]) is
/// substituted into the constraint/body and dropped from the telescope.
/// Only telescope-bound variables are solved, in telescope order with
/// chaining (`'a` solved first is substituted into `'b`'s equation;
/// right-hand sides are normalized so chained solutions fold).
/// Pure and total; unsolved schemes are returned unchanged.
#[must_use]
pub fn apply_simple_equations(scheme: &TypQuant) -> TypQuant {
    use std::collections::HashSet;
    let eqs = crate::quant::simple_equations(&scheme.constraint);
    if eqs.is_empty() || scheme.quants.is_empty() {
        return scheme.clone();
    }
    let bound: HashSet<&str> = scheme.quants.iter().map(|(v, _)| v.as_str()).collect();
    let mut subst = NexpSubst::new();
    let mut solved: Vec<String> = Vec::new();
    for (var, rhs) in eqs {
        if !bound.contains(var.as_str()) || solved.iter().any(|s| s == &var) {
            continue;
        }
        let rhs = crate::nexp::normalize_nexp(&subst.apply_nexp(&rhs));
        let mut check = Vec::new();
        rhs.free_vars(&mut check);
        if check.iter().any(|v| v == &var) {
            continue;
        }
        subst.bind(var.clone(), rhs);
        solved.push(var);
    }
    if solved.is_empty() {
        return scheme.clone();
    }
    TypQuant {
        quants: scheme
            .quants
            .iter()
            .filter(|(v, _)| !solved.iter().any(|s| s == v))
            .cloned()
            .collect(),
        constraint: subst.apply_constraint(&scheme.constraint),
        typ: normalize_typ(&subst.apply_typ(&scheme.typ)),
    }
}

/// Normalize the level terms inside a type (canonical-path identity
/// folding; see [`crate::nexp::normalize_nexp`]). Pure and total.
fn normalize_typ(t: &MiniTyp) -> MiniTyp {
    match t {
        MiniTyp::Int | MiniTyp::Bool | MiniTyp::Bit => t.clone(),
        MiniTyp::Bits(n) => MiniTyp::Bits(Box::new(crate::nexp::normalize_nexp(n))),
        MiniTyp::Range(lo, hi) => MiniTyp::Range(
            Box::new(crate::nexp::normalize_nexp(lo)),
            Box::new(crate::nexp::normalize_nexp(hi)),
        ),
        MiniTyp::Named { name, args } => MiniTyp::Named {
            name: name.clone(),
            args: args.iter().map(crate::nexp::normalize_nexp).collect(),
        },
        MiniTyp::Fun(a, b) => MiniTyp::Fun(
            Box::new(normalize_typ(a)),
            Box::new(normalize_typ(b)),
        ),
        MiniTyp::Tuple(ts) => MiniTyp::Tuple(ts.iter().map(normalize_typ).collect()),
    }
}

/// Instantiate with omitted-implicit solving: freshen (rule `Q-Freshen`),
/// then discharge telescope variables already determined by
/// `Var == nexp` constraint conjuncts (OCaml `instantiate_simple_equations`
/// after freshening). Returns the residual obligation and body type.
#[must_use]
pub fn instantiate_with_equations(scheme: &TypQuant) -> (NConstraint, MiniTyp) {
    let fresh = freshen_scheme(scheme, "inst");
    let solved = apply_simple_equations(&fresh);
    (solved.constraint, solved.typ)
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
    /// `atom('n) <: int`: forget the singleton value (synonym-expanded
    /// `implicit('n)` shares this coercion).
    AtomToInt,
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
            Self::AtomToInt => write!(f, "atom-to-int"),
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
/// `S-BaseMismatch`, plus the Lane-10 synonym/atom layer
/// (`implicit('n)` ≡ `atom('n)`, `bitvector('n)` ≡ `bits('n)`,
/// `atom_bool` ≡ `bool`; `atom('n)` is the singleton integer `'n`;
/// OCaml `Env.expand_synonyms` at the head of `subtyp`).
/// Synonyms are matched inline, arm by arm — never via a separate
/// pre-traversal, so deep types keep the baseline recursion profile
/// (small-stack safe). See [`expand_synonyms_typ`] for the standalone
/// expansion. Never calls the solver; use [`check_subtype`] to
/// discharge [`Subsumption::NeedsProof`].
#[must_use]
pub fn subtype_obligation(inferred: &MiniTyp, expected: &MiniTyp) -> Subsumption {
    if inferred == expected {
        return Subsumption::Holds(Coercion::Identity);
    }
    match (inferred, expected) {
        // Boolean synonym: `atom_bool` ≡ `bool` (either orientation).
        (
            MiniTyp::Named { name: n1, .. },
            MiniTyp::Bool,
        )
        | (
            MiniTyp::Bool,
            MiniTyp::Named { name: n1, .. },
        ) if n1 == "atom_bool" => Subsumption::Holds(Coercion::Identity),
        // Length synonym: `bitvector('n)` ≡ `bits('n)` (either orientation).
        (
            MiniTyp::Named { name: n1, args: a1 },
            MiniTyp::Bits(m),
        )
        | (
            MiniTyp::Bits(m),
            MiniTyp::Named { name: n1, args: a1 },
        ) if n1 == "bitvector" && a1.len() == 1 => {
            Subsumption::NeedsProof(
                NConstraint::Eq(Box::new(a1[0].clone()), m.clone()),
                Coercion::BitsCongruence,
            )
        }
        // `range <: int` always holds: bounds are forgotten (X-RangeToInt).
        (MiniTyp::Range(_, _), MiniTyp::Int) => {
            Subsumption::Holds(Coercion::RangeToInt)
        }
        // `atom('n)` / `implicit('n)` <: int: the singleton value is
        // forgotten (OCaml `implicit_to_int` normalizes arguments likewise).
        (MiniTyp::Named { name, args }, MiniTyp::Int)
            if is_atom_head(name) && args.len() == 1 =>
        {
            Subsumption::Holds(Coercion::AtomToInt)
        }
        // `int <: range` never holds without a value: literals are handled
        // at value sites (`check` special case, X-LitToRange). (S-IntRangeFail)
        (MiniTyp::Int, MiniTyp::Range(_, _)) => Subsumption::Fails(
            "int is not a subtype of range(..) outside literal sites".to_string(),
        ),
        // `int <: atom('n)` likewise needs a value site.
        (MiniTyp::Int, MiniTyp::Named { name, .. }) if is_atom_head(name) => {
            Subsumption::Fails(
                "int is not a subtype of atom(..) outside literal sites".to_string(),
            )
        }
        // Range inclusion: lo2 <= lo1 /\ hi1 <= hi2. (S-RangeRange)
        (MiniTyp::Range(lo1, hi1), MiniTyp::Range(lo2, hi2)) => {
            let nc = NConstraint::And(
                Box::new(NConstraint::Le(lo2.clone(), lo1.clone())),
                Box::new(NConstraint::Le(hi1.clone(), hi2.clone())),
            );
            Subsumption::NeedsProof(nc, Coercion::RangeNarrow)
        }
        // `range(lo,hi) <: atom('n)` iff both bounds equal the singleton.
        (MiniTyp::Range(lo, hi), MiniTyp::Named { name, args })
            if is_atom_head(name) && args.len() == 1 =>
        {
            let n = &args[0];
            let nc = NConstraint::And(
                Box::new(NConstraint::Eq(lo.clone(), Box::new(n.clone()))),
                Box::new(NConstraint::Eq(hi.clone(), Box::new(n.clone()))),
            );
            Subsumption::NeedsProof(nc, Coercion::NamedUnfold(canon_head(name)))
        }
        // `atom('n) <: range(lo,hi)` iff the singleton sits inside.
        (MiniTyp::Named { name, args }, MiniTyp::Range(lo, hi))
            if is_atom_head(name) && args.len() == 1 =>
        {
            let n = &args[0];
            let nc = NConstraint::And(
                Box::new(NConstraint::Le(lo.clone(), Box::new(n.clone()))),
                Box::new(NConstraint::Le(Box::new(n.clone()), hi.clone())),
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
        // Same-head abbreviations (with `implicit` ≡ `atom` canonicalized):
        // level arguments must be provably equal. (S-NamedArgs)
        (
            MiniTyp::Named { name: n1, args: a1 },
            MiniTyp::Named { name: n2, args: a2 },
        ) => {
            let (h1, h2) = (canon_head(n1), canon_head(n2));
            if h1 != h2 || a1.len() != a2.len() {
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
                    Subsumption::NeedsProof(nc, Coercion::NamedUnfold(h1))
                }
                None => Subsumption::Holds(Coercion::NamedUnfold(h1)),
            }
        }
        _ => Subsumption::Fails(format!("{inferred} is not a subtype of {expected}")),
    }
}

/// Whether `name` heads a numeric singleton (`atom` or its `implicit`
/// spelling, OCaml `implicit_to_int`).
fn is_atom_head(name: &str) -> bool {
    name == "atom" || name == "implicit"
}

/// Canonical head for synonym comparison: `implicit` ≡ `atom`.
fn canon_head(name: &str) -> String {
    if name == "implicit" {
        "atom".to_string()
    } else {
        name.to_string()
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

// ---------------------------------------------------------------------------
// Dependent call-argument matching (existential opening / arithmetic solving)
// ---------------------------------------------------------------------------

/// Outcome of matching one formal scalar argument against its actual.
///
/// This pins the sound rule behind dependent-call instantiation: a formal
/// that *is* the quantified variable binds whatever the actual denotes
/// (the bare-variable core the canonical `instantiate` implements); a
/// formal that is *exactly* invertible in one quantified variable solves
/// to the unique integer preimage; anything else stays open. There is no
/// guessing — [`ScalarMatch::Uninferred`] is the local analogue of the
/// canonical `cannot infer dependent call parameter` error (rule
/// `Z-UnknownSound`): the caller must reject the call, never fabricate a
/// binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScalarMatch {
    /// Bind `var := to` (exact: bare-variable formal, or unique linear
    /// preimage with coefficient `+1`/`-1`, or exact constant division).
    Bind {
        var: String,
        to: Nexp,
    },
    /// The equation holds identically; nothing needs binding.
    Holds,
    /// No exact binding exists locally (nonlinear formal, ambiguous or
    /// non-invertible arithmetic, rigid mismatch). `detail` names the
    /// reason; the caller reports a sound error.
    Uninferred {
        detail: String,
    },
}

/// Boolean-level counterpart of [`ScalarMatch`]: a formal that is a
/// quantified boolean variable (optionally negated) binds exactly;
/// anything else stays open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoolMatch {
    /// Bind `var := to` (exact: bare or negated-variable formal).
    Bind {
        var: String,
        to: NConstraint,
    },
    /// The equation holds identically; nothing needs binding.
    Holds,
    /// No exact binding exists locally. The caller reports a sound error.
    Uninferred {
        detail: String,
    },
}

/// Match one formal integer-level argument against its actual.
///
/// `quantified` holds the caller's quantified `K_int`-like variables (the
/// caller sorts kinds, as [`crate::check`] does for its opaque kind
/// environment — a `K_bool` name never belongs here).
///
/// Rules (all exact over the integers, checked `i128` arithmetic):
/// - formal `Var(v)`, `v` quantified → [`ScalarMatch::Bind`](`v`,
///   simplified actual), whatever the actual denotes (existential
///   opening needs no solving);
/// - otherwise the difference `formal - actual` must be linear in exactly
///   one quantified variable `v` with coefficient `+1`/`-1`: then
///   `v` is bound to the unique preimage (arithmetic solving, e.g. formal
///   `'n + 1` against actual `5` binds `'n := 4`);
/// - a non-unit coefficient binds only a constant preimage under exact
///   division (formal `2 * 'n` against `8` binds `'n := 4`; against `7`
///   or a symbolic term it stays open);
/// - a difference with no quantified variable that is identically zero is
///   [`ScalarMatch::Holds`]; everything else (nonlinear formals such as
///   `2 ^ 'n` or `'n * 'm`, several quantified variables, rigid
///   mismatches, overflow) is [`ScalarMatch::Uninferred`].
#[must_use]
pub fn match_scalar_arg(
    formal: &Nexp,
    actual: &Nexp,
    quantified: &HashSet<String>,
) -> ScalarMatch {
    let formal = formal.simplify();
    let actual = actual.simplify();
    // Bare-variable rule: needs no solving, binds any actual.
    if let Nexp::Var(v) = &formal {
        if quantified.contains(v) {
            return ScalarMatch::Bind { var: v.clone(), to: actual };
        }
    }
    // Exact linear inversion over `formal - actual`.
    let diff = Nexp::Sub(Box::new(formal.clone()), Box::new(actual));
    let form = match linear_form(&diff) {
        Some(form) => form,
        None => {
            return ScalarMatch::Uninferred {
                detail: "non-linear dependent call parameter (arithmetic solving required)"
                    .to_string(),
            };
        }
    };
    let mut solvable: Vec<(String, i128)> = Vec::new();
    let mut rigid = crate::nexp::LinearForm::constant(form.constant);
    for (v, c) in &form.coeffs {
        if quantified.contains(v) {
            solvable.push((v.clone(), *c));
        } else {
            let mut single = std::collections::BTreeMap::new();
            single.insert(v.clone(), *c);
            rigid = match rigid.add(&crate::nexp::LinearForm { constant: 0, coeffs: single }) {
                Some(sum) => sum,
                None => {
                    return ScalarMatch::Uninferred {
                        detail: "dependent call parameter overflows i128".to_string(),
                    };
                }
            };
        }
    }
    match solvable.as_slice() {
        [] => {
            if form.coeffs.is_empty() && form.constant == 0 {
                ScalarMatch::Holds
            } else {
                ScalarMatch::Uninferred {
                    detail: "no quantified variable to solve for (proof required)".to_string(),
                }
            }
        }
        [(v, 1)] => match rigid.neg() {
            Some(rhs) => ScalarMatch::Bind { var: v.clone(), to: rhs.into_nexp().simplify() },
            None => ScalarMatch::Uninferred {
                detail: "dependent call parameter overflows i128".to_string(),
            },
        },
        [(v, -1)] => ScalarMatch::Bind { var: v.clone(), to: rigid.into_nexp().simplify() },
        [(v, c)] => {
            // Non-unit coefficient: exact constant preimages only.
            if rigid.coeffs.is_empty() {
                if let Some(neg) = rigid.constant.checked_neg() {
                    if neg % *c == 0 {
                        if let Some(q) = neg.checked_div(*c) {
                            return ScalarMatch::Bind {
                                var: v.clone(),
                                to: Nexp::Const(q),
                            };
                        }
                    }
                }
            }
            ScalarMatch::Uninferred {
                detail: format!(
                    "non-invertible coefficient {c} for '{v} (arithmetic solving required)"
                ),
            }
        }
        _ => ScalarMatch::Uninferred {
            detail: "ambiguous dependent call parameter (several quantified variables)"
                .to_string(),
        },
    }
}

/// Match one formal boolean-level argument against its actual.
///
/// `quantified` holds the caller's quantified `K_bool` variables. A bare
/// (optionally negated) quantified variable binds exactly; an identical
/// formal/actual pair holds; everything else stays open.
#[must_use]
pub fn match_bool_arg(
    formal: &NConstraint,
    actual: &NConstraint,
    quantified: &HashSet<String>,
) -> BoolMatch {
    let formal = formal.simplify();
    let actual = actual.simplify();
    match &formal {
        NConstraint::Var(v) if quantified.contains(v) => {
            BoolMatch::Bind { var: v.clone(), to: actual }
        }
        NConstraint::Not(inner) => match inner.as_ref() {
            NConstraint::Var(v) if quantified.contains(v) => BoolMatch::Bind {
                var: v.clone(),
                to: NConstraint::Not(Box::new(actual)).simplify(),
            },
            _ => BoolMatch::Uninferred {
                detail: "non-variable boolean call parameter (proof required)".to_string(),
            },
        },
        _ if formal == actual => BoolMatch::Holds,
        _ => BoolMatch::Uninferred {
            detail: "non-variable boolean call parameter (proof required)".to_string(),
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

    fn quants(vars: &[&str]) -> HashSet<String> {
        vars.iter().map(|v| (*v).to_string()).collect()
    }

    fn var(v: &str) -> Nexp {
        Nexp::Var(v.to_string())
    }

    fn const_(c: i128) -> Nexp {
        Nexp::Const(c)
    }

    #[test]
    fn scalar_match_binds_bare_variables_and_holds_tautologies() {
        let q = quants(&["n"]);
        // Bare quantified formal binds any actual (existential opening,
        // no solving needed) — even a nonlinear one.
        assert_eq!(
            match_scalar_arg(&var("n"), &Nexp::Exp(Box::new(var("m"))), &q),
            ScalarMatch::Bind { var: "n".to_string(), to: Nexp::Exp(Box::new(var("m"))) }
        );
        // A rigid formal against itself holds.
        assert_eq!(match_scalar_arg(&var("m"), &var("m"), &q), ScalarMatch::Holds);
        // Constants agree identically.
        assert_eq!(match_scalar_arg(&const_(5), &const_(5), &q), ScalarMatch::Holds);
    }

    #[test]
    fn scalar_match_solves_exact_linear_preimages() {
        let q = quants(&["n"]);
        let plus_one = Nexp::Add(Box::new(var("n")), Box::new(const_(1)));
        // `'n + 1 = 5` binds `'n := 4`.
        assert_eq!(
            match_scalar_arg(&plus_one, &const_(5), &q),
            ScalarMatch::Bind { var: "n".to_string(), to: const_(4) }
        );
        // Negated coefficient: `3 - 'n = 1` binds `'n := 2`.
        let three_minus_n = Nexp::Sub(Box::new(const_(3)), Box::new(var("n")));
        assert_eq!(
            match_scalar_arg(&three_minus_n, &const_(1), &q),
            ScalarMatch::Bind { var: "n".to_string(), to: const_(2) }
        );
        // Symbolic actuals stay exact: `'n + 1 = 'm + 1` binds `'n := 'm`.
        let m_plus_one = Nexp::Add(Box::new(var("m")), Box::new(const_(1)));
        assert_eq!(
            match_scalar_arg(&plus_one, &m_plus_one, &q),
            ScalarMatch::Bind { var: "n".to_string(), to: var("m") }
        );
        // Exact constant division: `2 * 'n = 8` binds `'n := 4` …
        let twice_n = Nexp::Mul(Box::new(const_(2)), Box::new(var("n")));
        assert_eq!(
            match_scalar_arg(&twice_n, &const_(8), &q),
            ScalarMatch::Bind { var: "n".to_string(), to: const_(4) }
        );
        // … but `2 * 'n = 7` has no integer preimage: soundly open.
        assert!(matches!(
            match_scalar_arg(&twice_n, &const_(7), &q),
            ScalarMatch::Uninferred { .. }
        ));
    }

    #[test]
    fn scalar_match_stays_open_instead_of_guessing() {
        let q = quants(&["n"]);
        // Nonlinear formal (`2 ^ 'n`): no linear inversion exists.
        assert!(matches!(
            match_scalar_arg(&Nexp::Exp(Box::new(var("n"))), &const_(8), &q),
            ScalarMatch::Uninferred { .. }
        ));
        // Several quantified variables: ambiguous.
        let q2 = quants(&["n", "m"]);
        let sum = Nexp::Add(Box::new(var("n")), Box::new(var("m")));
        assert!(matches!(
            match_scalar_arg(&sum, &const_(3), &q2),
            ScalarMatch::Uninferred { .. }
        ));
        // Rigid mismatch with no quantified variable: needs a proof.
        assert!(matches!(
            match_scalar_arg(&var("m"), &const_(3), &q),
            ScalarMatch::Uninferred { .. }
        ));
        // Non-unit coefficient against a symbolic term: not exact.
        let twice_n = Nexp::Mul(Box::new(const_(2)), Box::new(var("n")));
        assert!(matches!(
            match_scalar_arg(&twice_n, &var("m"), &q),
            ScalarMatch::Uninferred { .. }
        ));
        // Unquantified names never bind (kind confusion is not resolved
        // by matching: the caller sorts `K_int` binders first).
        assert!(matches!(
            match_scalar_arg(&var("b"), &const_(1), &quants(&[])),
            ScalarMatch::Uninferred { .. }
        ));
    }

    #[test]
    fn bool_match_binds_bare_and_negated_variables() {
        let q = quants(&["p"]);
        assert_eq!(
            match_bool_arg(
                &NConstraint::Var("p".to_string()),
                &NConstraint::True,
                &q
            ),
            BoolMatch::Bind { var: "p".to_string(), to: NConstraint::True }
        );
        assert_eq!(
            match_bool_arg(
                &NConstraint::Not(Box::new(NConstraint::Var("p".to_string()))),
                &NConstraint::True,
                &q
            ),
            BoolMatch::Bind {
                var: "p".to_string(),
                to: NConstraint::False
            }
        );
        assert_eq!(
            match_bool_arg(&NConstraint::True, &NConstraint::True, &q),
            BoolMatch::Holds
        );
        assert!(matches!(
            match_bool_arg(
                &NConstraint::App("p".to_string(), vec![var("n")]),
                &NConstraint::True,
                &q
            ),
            BoolMatch::Uninferred { .. }
        ));
    }
}
