#![forbid(unsafe_code)]
//! Local minimal AST/nexp/typ types.
//!
//! These mirror the *shape* of `sail/src/libsail/ast.ml`
//! (`typ`, `nexp`, `n_constraint`, `typquant`) without copying any logic.
//! Once `sail-ast` (Lane 01) and `sail-nexp` (Lane 04) exist, replace these
//! with the real types — see `ARCHITECTURE.md` for the field-by-field mapping.
//!
//! Lane 127 bestaetigt: kanonische `NExp`-Wahrheit ist `sail-nexp`
//! (`NExp`/`NConstraint`/`TypArg`/`NTyp`, internierte `Kid`s); die
//! Vollmigration (`Nexp`/`NConstraint`/`MiniTyp`/Checker) bleibt
//! Owner-Folgearbeit nach Lane 47 — hier keine Logik-Aenderung.
//!
//! Design notes (low-RAM + multithreaded):
//! - Everything is `Clone + Send + Sync`, no interior `Rc`/raw pointers.
//! - `Nexp` is a boxed tree; [`Nexp::depth`] + [`CheckLimits`] bound recursion
//!   so checking without a solver always terminates.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::pat::MiniPat;

/// Monotonic gensym counter for capture-avoiding freshening.
///
/// Mirrors OCaml `Type_env.fresh_kid` (`type_env.ml`: a global `counter`
/// producing `'fv<N>#…` kids): every call yields a name no user-written
/// binder can collide with, because `#` is not a legal Sail identifier
/// character (OCaml uses it for the same reason in `builtin_typs`
/// `k#<n>` kids). Predictable `{base}{i}` names are UNSOUND here — a
/// scheme may already bind `inst0`/`f0`, and two freshenings with the
/// same base in one scope would capture each other.
static FRESH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Yield a globally fresh level-variable name derived from `base`.
///
/// The result always contains `#fv<N>` (`N` strictly increasing), so it
/// is distinct from every source-parsable name and from every name
/// returned by previous calls, regardless of `base`.
#[must_use]
pub fn gensym(base: &str) -> String {
    let n = FRESH_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{base}#fv{n}")
}

/// Numeric type-level expression (subset of Sail `nexp`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Nexp {
    /// Integer constant.
    Const(i128),
    /// Quantified variable, e.g. `'n`.
    Var(String),
    /// `n + m`
    Add(Box<Nexp>, Box<Nexp>),
    /// `n - m`
    Sub(Box<Nexp>, Box<Nexp>),
    /// `n * m`
    Mul(Box<Nexp>, Box<Nexp>),
    /// `-n`
    Neg(Box<Nexp>),
    /// `2 ^ n` (Sail `Nexp_exp`)
    Exp(Box<Nexp>),
    /// `n / m` (integer division; divisor must be nonzero)
    Div(Box<Nexp>, Box<Nexp>),
    /// `n mod m`
    Mod(Box<Nexp>, Box<Nexp>),
    /// `|n|`
    Abs(Box<Nexp>),
    /// `if c then n else m` (condition is a bool-level nexp encoded via Nc)
    If(Box<NConstraint>, Box<Nexp>, Box<Nexp>),
}

impl Nexp {
    /// Syntactic tree depth. Used for fuel accounting.
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::Const(_) | Self::Var(_) => 1,
            Self::Neg(n) | Self::Exp(n) | Self::Abs(n) => 1 + n.depth(),
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Mod(a, b) => 1 + a.depth().max(b.depth()),
            Self::If(c, a, b) => 1 + c.depth().max(a.depth()).max(b.depth()),
        }
    }

    /// Collect free variables into `out` (dedup left to caller).
    pub fn free_vars(&self, out: &mut Vec<String>) {
        match self {
            Self::Const(_) => {}
            Self::Var(v) => out.push(v.clone()),
            Self::Neg(n) | Self::Exp(n) | Self::Abs(n) => n.free_vars(out),
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Mod(a, b) => {
                a.free_vars(out);
                b.free_vars(out);
            }
            Self::If(c, a, b) => {
                c.free_vars(out);
                a.free_vars(out);
                b.free_vars(out);
            }
        }
    }

    /// Try to fold to a constant. Returns `None` if not constant-foldable.
    /// Pure and total; bounded by tree depth (no solver needed).
    #[must_use]
    pub fn as_const(&self) -> Option<i128> {
        match self {
            Self::Const(c) => Some(*c),
            Self::Var(_) => None,
            Self::Neg(n) => n.as_const().and_then(|v| v.checked_neg()),
            Self::Abs(n) => n.as_const().and_then(|v| v.checked_abs()),
            Self::Exp(n) => {
                let v = n.as_const()?;
                if !(0..=126).contains(&v) {
                    return None;
                }
                2_i128.checked_pow(v as u32)
            }
            Self::Add(a, b) => Some(a.as_const()?.checked_add(b.as_const()?)?),
            Self::Sub(a, b) => Some(a.as_const()?.checked_sub(b.as_const()?)?),
            Self::Mul(a, b) => Some(a.as_const()?.checked_mul(b.as_const()?)?),
            Self::Div(a, b) => {
                let d = b.as_const()?;
                if d == 0 {
                    return None;
                }
                a.as_const()?.checked_div(d)
            }
            Self::Mod(a, b) => {
                let d = b.as_const()?;
                if d == 0 {
                    return None;
                }
                a.as_const()?.checked_rem(d)
            }
            Self::If(c, a, b) => match c.as_bool() {
                Some(true) => a.as_const(),
                Some(false) => b.as_const(),
                None => None,
            },
        }
    }

    /// Sound bottom-up simplifier: constant folding ([`Nexp::as_const`],
    /// including `2 ^ n` for `0 <= n <= 126` and decided `if`) plus
    /// unconditional integer identities:
    /// `x + 0` / `x - 0` / `x * 1` / `x / 1` (identity element),
    /// `x mod 1 = 0` (nonzero divisor), `x - x = 0` (syntactic),
    /// double negation, decided `if`, and equal-branch `if`.
    ///
    /// Total and terminating (one structural pass; every rewrite strictly
    /// shrinks the tree, depth never grows). Every rewrite is a
    /// mathematical-integer identity, so `simplify` preserves exactly the
    /// totality assumption the syntactic reflexivity paths already make
    /// (a proved `Eq(a, a)` assumes level terms denote).
    ///
    /// Deliberately *absent*: the annihilator (`0 * x` — needs totality
    /// of `x`), power unfolding (`2 ^ n` — needs bounds on `n`),
    /// division cancellation (`(x * k) / k` — needs nonzeroness), and any
    /// associative reassociation. Those need side conditions the local
    /// layer cannot discharge — they stay solver business (`Unknown`
    /// until proved, rule `Z-UnknownSound`).
    #[must_use]
    pub fn simplify(&self) -> Nexp {
        // Constants (including decided `if` and small powers) collapse here.
        if let Some(c) = self.as_const() {
            return Nexp::Const(c);
        }
        match self {
            Nexp::Const(_) | Nexp::Var(_) => self.clone(),
            Nexp::Neg(a) => match a.simplify() {
                Nexp::Neg(x) => *x,
                x => Nexp::Neg(Box::new(x)),
            },
            Nexp::Abs(a) => Nexp::Abs(Box::new(a.simplify())),
            Nexp::Exp(a) => Nexp::Exp(Box::new(a.simplify())),
            Nexp::Add(a, b) => {
                let (x, y) = (a.simplify(), b.simplify());
                if is_zero(&x) {
                    y
                } else if is_zero(&y) {
                    x
                } else {
                    Nexp::Add(Box::new(x), Box::new(y))
                }
            }
            Nexp::Sub(a, b) => {
                let (x, y) = (a.simplify(), b.simplify());
                if is_zero(&y) {
                    x
                } else if x == y {
                    Nexp::Const(0)
                } else {
                    Nexp::Sub(Box::new(x), Box::new(y))
                }
            }
            Nexp::Mul(a, b) => {
                let (x, y) = (a.simplify(), b.simplify());
                if is_one(&x) {
                    y
                } else if is_one(&y) {
                    x
                } else {
                    Nexp::Mul(Box::new(x), Box::new(y))
                }
            }
            Nexp::Div(a, b) => {
                let (x, y) = (a.simplify(), b.simplify());
                if is_one(&y) {
                    x
                } else {
                    Nexp::Div(Box::new(x), Box::new(y))
                }
            }
            Nexp::Mod(a, b) => {
                let (x, y) = (a.simplify(), b.simplify());
                if is_one(&y) {
                    Nexp::Const(0)
                } else {
                    Nexp::Mod(Box::new(x), Box::new(y))
                }
            }
            Nexp::If(c, a, b) => {
                let (cc, x, y) = (c.simplify(), a.simplify(), b.simplify());
                match cc.as_bool() {
                    Some(true) => x,
                    Some(false) => y,
                    None if x == y => x,
                    _ => Nexp::If(Box::new(cc), Box::new(x), Box::new(y)),
                }
            }
        }
    }
}

/// Exact linear (affine) form over level variables: `constant + Σ coeff·var`
/// with nonzero coefficients in a canonical (sorted) map.
///
/// Built by [`linear_form`]; only linear skeletons decompose (`Add`/`Sub`,
/// `Neg`, `Mul` with a constant side, constant-foldable subterms).
/// `Div`/`Mod`/`Exp`/`Abs`/undecided `If`/nonlinear `Mul` yield `None` —
/// sound openness, never an approximation. All coefficient arithmetic is
/// checked: overflow yields `None` (→ `Unknown`), never a wrong proof.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinearForm {
    /// Constant term.
    pub constant: i128,
    /// Nonzero coefficients by variable, sorted for canonicity.
    pub coeffs: std::collections::BTreeMap<String, i128>,
}

impl LinearForm {
    /// The constant form `c` (no variables).
    #[must_use]
    pub fn constant(c: i128) -> Self {
        Self { constant: c, coeffs: std::collections::BTreeMap::new() }
    }

    /// `true` when no variable occurs (the form is just [`Self::constant`]).
    #[must_use]
    pub fn is_constant(&self) -> bool {
        self.coeffs.is_empty()
    }

    /// Negation (exact; `None` only on `i128` overflow).
    #[must_use]
    pub fn neg(&self) -> Option<Self> {
        let mut coeffs = std::collections::BTreeMap::new();
        for (v, c) in &self.coeffs {
            coeffs.insert(v.clone(), c.checked_neg()?);
        }
        Some(Self { constant: self.constant.checked_neg()?, coeffs })
    }

    /// Sum (exact; `None` only on `i128` overflow). Zero coefficients are
    /// dropped so `is_constant` and variable scans stay exact.
    #[must_use]
    pub fn add(&self, other: &Self) -> Option<Self> {
        let mut coeffs = self.coeffs.clone();
        for (v, c) in &other.coeffs {
            let next = coeffs.get(v).copied().unwrap_or(0).checked_add(*c)?;
            if next == 0 {
                coeffs.remove(v);
            } else {
                coeffs.insert(v.clone(), next);
            }
        }
        Some(Self { constant: self.constant.checked_add(other.constant)?, coeffs })
    }

    /// Difference (exact; `None` only on `i128` overflow).
    #[must_use]
    pub fn sub(&self, other: &Self) -> Option<Self> {
        self.add(&other.neg()?)
    }

    /// Scale by a constant (exact; `None` only on `i128` overflow).
    #[must_use]
    pub fn scale(&self, factor: i128) -> Option<Self> {
        if factor == 0 {
            return Some(Self::constant(0));
        }
        let mut coeffs = std::collections::BTreeMap::new();
        for (v, c) in &self.coeffs {
            coeffs.insert(v.clone(), c.checked_mul(factor)?);
        }
        Some(Self {
            constant: self.constant.checked_mul(factor)?,
            coeffs,
        })
    }

    /// Rebuild a level term denoting exactly this form:
    /// `((t₁ + t₂) + … + constant)`, where each `tᵢ` is `v`, `-v`, or
    /// `c * v`. A constant form rebuilds to [`Nexp::Const`]; a form with
    /// terms and a zero constant rebuilds without a trailing `+ 0`.
    #[must_use]
    pub fn into_nexp(self) -> Nexp {
        let mut acc: Option<Nexp> = None;
        let push = |term: Nexp, acc: &mut Option<Nexp>| {
            *acc = Some(match acc.take() {
                None => term,
                Some(prev) => Nexp::Add(Box::new(prev), Box::new(term)),
            });
        };
        for (v, c) in self.coeffs {
            let var = Nexp::Var(v);
            push(
                match c {
                    1 => var,
                    -1 => Nexp::Neg(Box::new(var)),
                    _ => Nexp::Mul(Box::new(Nexp::Const(c)), Box::new(var)),
                },
                &mut acc,
            );
        }
        if self.constant != 0 || acc.is_none() {
            push(Nexp::Const(self.constant), &mut acc);
        }
        acc.expect("LinearForm rebuilds to at least a constant")
    }
}

/// Decompose `n` into an exact [`LinearForm`]; `None` for nonlinear or
/// otherwise non-affine skeletons (sound openness — never approximate).
///
/// Constant-foldable subterms (including small powers and decided `if`)
/// fold via [`Nexp::as_const`] first, so `2 ^ 3 + 'n` decomposes while
/// `2 ^ 'n + 'm` stays `None`.
#[must_use]
pub fn linear_form(n: &Nexp) -> Option<LinearForm> {
    if let Some(c) = n.as_const() {
        return Some(LinearForm::constant(c));
    }
    match n {
        Nexp::Const(c) => Some(LinearForm::constant(*c)),
        Nexp::Var(v) => {
            let mut coeffs = std::collections::BTreeMap::new();
            coeffs.insert(v.clone(), 1);
            Some(LinearForm { constant: 0, coeffs })
        }
        Nexp::Neg(a) => linear_form(a)?.neg(),
        Nexp::Add(a, b) => linear_form(a)?.add(&linear_form(b)?),
        Nexp::Sub(a, b) => linear_form(a)?.sub(&linear_form(b)?),
        Nexp::Mul(a, b) => {
            if let Some(c) = a.as_const() {
                linear_form(b)?.scale(c)
            } else if let Some(c) = b.as_const() {
                linear_form(a)?.scale(c)
            } else {
                None
            }
        }
        // `Div`/`Mod` (integer division is not linear), `Exp` (powers need
        // elaboration before any proof, cf. the SMT emitter's sound
        // refusal), `Abs` (piecewise linear only), and undecided `If`
        // never decompose.
        Nexp::Div(..) | Nexp::Mod(..) | Nexp::Exp(..) | Nexp::Abs(..) | Nexp::If(..) => None,
    }
}

fn is_zero(n: &Nexp) -> bool {
    matches!(n, Nexp::Const(0))
}

fn is_one(n: &Nexp) -> bool {
    matches!(n, Nexp::Const(1))
}

impl fmt::Display for Nexp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Const(c) => write!(f, "{c}"),
            Self::Var(v) => write!(f, "'{v}"),
            Self::Neg(n) => write!(f, "-({n})"),
            Self::Abs(n) => write!(f, "abs({n})"),
            Self::Exp(n) => write!(f, "2^({n})"),
            Self::Add(a, b) => write!(f, "({a} + {b})"),
            Self::Sub(a, b) => write!(f, "({a} - {b})"),
            Self::Mul(a, b) => write!(f, "({a} * {b})"),
            Self::Div(a, b) => write!(f, "({a} / {b})"),
            Self::Mod(a, b) => write!(f, "({a} mod {b})"),
            Self::If(c, a, b) => write!(f, "(if {c} then {a} else {b})"),
        }
    }
}

/// Boolean / numeric constraint over [`Nexp`] (subset of Sail `n_constraint`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NConstraint {
    /// `true`
    True,
    /// `false`
    False,
    /// `n == m`
    Eq(Box<Nexp>, Box<Nexp>),
    /// `n != m`
    Neq(Box<Nexp>, Box<Nexp>),
    /// `n >= m`
    Ge(Box<Nexp>, Box<Nexp>),
    /// `n > m`
    Gt(Box<Nexp>, Box<Nexp>),
    /// `n <= m`
    Le(Box<Nexp>, Box<Nexp>),
    /// `n < m`
    Lt(Box<Nexp>, Box<Nexp>),
    /// `a /\ b`
    And(Box<NConstraint>, Box<NConstraint>),
    /// `a \/ b`
    Or(Box<NConstraint>, Box<NConstraint>),
    /// `~a`
    Not(Box<NConstraint>),
    /// `n in {v1, ..}` — Sail `NC_set` (Lane 107 prove-path: constant
    /// scrutinees fold by membership, `[]` folds to `False`; anything else
    /// goes to the [`Solver`](crate::solver::Solver) via the trait).
    Set(Box<Nexp>, Vec<i128>),
    /// Uninterpreted predicate `f(n1, ..)` — Sail `NC_app` over `A_nexp`
    /// arguments only (Lane 107: non-`NExp` `typ_arg`s have no local
    /// counterpart and stay abstracted by the caller, see [`crate::conv`]).
    /// Folding is always `None` (sound); backends decide via the trait.
    App(String, Vec<Nexp>),
    /// Boolean kind variable used as a constraint — Sail `NC_var`.
    Var(String),
    /// Opaque boolean identifier — Sail `NC_id`.
    Id(String),
}

impl NConstraint {
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::True | Self::False => 1,
            Self::Eq(a, b)
            | Self::Neq(a, b)
            | Self::Ge(a, b)
            | Self::Gt(a, b)
            | Self::Le(a, b)
            | Self::Lt(a, b) => 1 + a.depth().max(b.depth()),
            Self::And(a, b) | Self::Or(a, b) => 1 + a.depth().max(b.depth()),
            Self::Not(a) => 1 + a.depth(),
            Self::Set(n, _) => 1 + n.depth(),
            Self::App(_, args) => {
                1 + args.iter().map(Nexp::depth).max().unwrap_or(0)
            }
            Self::Var(_) | Self::Id(_) => 1,
        }
    }

    pub fn free_vars(&self, out: &mut Vec<String>) {
        match self {
            Self::True | Self::False | Self::Id(_) => {}
            // Boolean level variable: same flat namespace as `Nexp::Var`
            // in this skeleton (kind-checked against `K_bool`, see
            // [`crate::kind::check_constraint_kind`]).
            Self::Var(v) => out.push(v.clone()),
            Self::Eq(a, b)
            | Self::Neq(a, b)
            | Self::Ge(a, b)
            | Self::Gt(a, b)
            | Self::Le(a, b)
            | Self::Lt(a, b) => {
                a.free_vars(out);
                b.free_vars(out);
            }
            Self::And(a, b) | Self::Or(a, b) => {
                a.free_vars(out);
                b.free_vars(out);
            }
            Self::Not(a) => a.free_vars(out),
            Self::Set(n, _) => n.free_vars(out),
            Self::App(_, args) => {
                for a in args {
                    a.free_vars(out);
                }
            }
        }
    }

    /// Boolean constant folding. `None` = not decidable syntactically.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::True => Some(true),
            Self::False => Some(false),
            Self::Eq(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x == y),
                _ if a == b => Some(true),
                _ => None,
            },
            Self::Neq(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x != y),
                _ if a == b => Some(false),
                _ => None,
            },
            Self::Ge(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x >= y),
                _ if a == b => Some(true),
                _ => None,
            },
            Self::Gt(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x > y),
                _ if a == b => Some(false),
                _ => None,
            },
            Self::Le(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x <= y),
                _ if a == b => Some(true),
                _ => None,
            },
            Self::Lt(a, b) => match (a.as_const(), b.as_const()) {
                (Some(x), Some(y)) => Some(x < y),
                _ if a == b => Some(false),
                _ => None,
            },
            Self::And(a, b) => match (a.as_bool(), b.as_bool()) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            Self::Or(a, b) => match (a.as_bool(), b.as_bool()) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            Self::Not(a) => a.as_bool().map(|v| !v),
            Self::Set(n, vs) => match n.as_const() {
                Some(x) => Some(vs.contains(&x)),
                // Empty membership is unsatisfiable whatever the scrutinee.
                None if vs.is_empty() => Some(false),
                None => None,
            },
            // Uninterpreted predicates and opaque booleans never fold
            // (conservative `None`, like `Nexp` unknown functions).
            Self::App(_, _) | Self::Var(_) | Self::Id(_) => None,
        }
    }

    /// Sound boolean/arithmetic simplifier: embedded [`Nexp::simplify`] on
    /// every level term, `True`/`False` elimination in `And`/`Or`, double
    /// negation, then the [`NConstraint::as_bool`] decision (constant
    /// comparisons, constant `Set` membership, empty `Set`, syntactic
    /// reflexivity all collapse to `True`/`False`).
    ///
    /// Total and terminating (one structural pass). Uninterpreted
    /// predicates (`App`), opaque booleans (`Var`/`Id`), and symbolic
    /// `Set` scrutinees pass through untouched — they are never guessed,
    /// only propagated (rule `Z-UnknownSound`).
    #[must_use]
    pub fn simplify(&self) -> NConstraint {
        let rebuilt = match self {
            NConstraint::True | NConstraint::False | NConstraint::Var(_) | NConstraint::Id(_) => {
                self.clone()
            }
            NConstraint::Eq(a, b) => NConstraint::Eq(simplified(a), simplified(b)),
            NConstraint::Neq(a, b) => NConstraint::Neq(simplified(a), simplified(b)),
            NConstraint::Ge(a, b) => NConstraint::Ge(simplified(a), simplified(b)),
            NConstraint::Gt(a, b) => NConstraint::Gt(simplified(a), simplified(b)),
            NConstraint::Le(a, b) => NConstraint::Le(simplified(a), simplified(b)),
            NConstraint::Lt(a, b) => NConstraint::Lt(simplified(a), simplified(b)),
            NConstraint::And(a, b) => {
                let (x, y) = (a.simplify(), b.simplify());
                match (&x, &y) {
                    (NConstraint::False, _) | (_, NConstraint::False) => NConstraint::False,
                    (NConstraint::True, _) => y,
                    (_, NConstraint::True) => x,
                    _ => NConstraint::And(Box::new(x), Box::new(y)),
                }
            }
            NConstraint::Or(a, b) => {
                let (x, y) = (a.simplify(), b.simplify());
                match (&x, &y) {
                    (NConstraint::True, _) | (_, NConstraint::True) => NConstraint::True,
                    (NConstraint::False, _) => y,
                    (_, NConstraint::False) => x,
                    _ => NConstraint::Or(Box::new(x), Box::new(y)),
                }
            }
            NConstraint::Not(a) => match a.simplify() {
                NConstraint::True => NConstraint::False,
                NConstraint::False => NConstraint::True,
                NConstraint::Not(x) => *x,
                x => NConstraint::Not(Box::new(x)),
            },
            NConstraint::Set(n, vs) => NConstraint::Set(Box::new(n.simplify()), vs.clone()),
            NConstraint::App(func, args) => {
                NConstraint::App(func.clone(), args.iter().map(Nexp::simplify).collect())
            }
        };
        match rebuilt.as_bool() {
            Some(true) => NConstraint::True,
            Some(false) => NConstraint::False,
            None => rebuilt,
        }
    }
}

fn simplified(n: &Nexp) -> Box<Nexp> {
    Box::new(n.simplify())
}

impl fmt::Display for NConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::True => write!(f, "true"),
            Self::False => write!(f, "false"),
            Self::Eq(a, b) => write!(f, "({a} == {b})"),
            Self::Neq(a, b) => write!(f, "({a} != {b})"),
            Self::Ge(a, b) => write!(f, "({a} >= {b})"),
            Self::Gt(a, b) => write!(f, "({a} > {b})"),
            Self::Le(a, b) => write!(f, "({a} <= {b})"),
            Self::Lt(a, b) => write!(f, "({a} < {b})"),
            Self::And(a, b) => write!(f, "({a} & {b})"),
            Self::Or(a, b) => write!(f, "({a} | {b})"),
            Self::Not(a) => write!(f, "~({a})"),
            Self::Set(n, vs) => {
                write!(f, "({n} in {{")?;
                for (i, v) in vs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, "}})")
            }
            Self::App(func, args) => {
                write!(f, "{func}(")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{a}")?;
                }
                write!(f, ")")
            }
            Self::Var(v) => write!(f, "'{v}"),
            Self::Id(name) => write!(f, "{name}"),
        }
    }
}

/// Sail type (subset of Sail `typ`): enough for kind checking,
/// register types, and the `test/lean`-subset smoke tests.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MiniTyp {
    /// `int`
    Int,
    /// `bool`
    Bool,
    /// `bit`
    Bit,
    /// `bits('n)` / `bits(n)` — length-level nexp
    Bits(Box<Nexp>),
    /// `int('n)` — range-bounded int, Sail `atom` shorthand
    Range(Box<Nexp>, Box<Nexp>),
    /// Named type / abbreviation applied to nexps, e.g. `atom('n)`.
    Named {
        name: String,
        args: Vec<Nexp>,
    },
    /// `t1 -> t2`
    Fun(Box<MiniTyp>, Box<MiniTyp>),
    /// `(t1, t2, ...)` tuple
    Tuple(Vec<MiniTyp>),
}

impl MiniTyp {
    /// Collect all level nexps occurring in the type.
    pub fn nexps(&self, out: &mut Vec<Nexp>) {
        match self {
            Self::Int | Self::Bool | Self::Bit => {}
            Self::Bits(n) => out.push((**n).clone()),
            Self::Range(lo, hi) => {
                out.push((**lo).clone());
                out.push((**hi).clone());
            }
            Self::Named { args, .. } => out.extend(args.iter().cloned()),
            Self::Fun(a, b) => {
                a.nexps(out);
                b.nexps(out);
            }
            Self::Tuple(ts) => {
                for t in ts {
                    t.nexps(out);
                }
            }
        }
    }
}

impl fmt::Display for MiniTyp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int => write!(f, "int"),
            Self::Bool => write!(f, "bool"),
            Self::Bit => write!(f, "bit"),
            Self::Bits(n) => write!(f, "bits({n})"),
            Self::Range(lo, hi) => write!(f, "range({lo}, {hi})"),
            Self::Named { name, args } => {
                write!(f, "{name}")?;
                if !args.is_empty() {
                    write!(f, "(")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{a}")?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
            Self::Fun(a, b) => write!(f, "({a} -> {b})"),
            Self::Tuple(ts) => {
                write!(f, "(")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, ")")
            }
        }
    }
}

/// Universally quantified type scheme: `forall 'n 'm. nc => typ`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypQuant {
    /// Bound kinded variables: `('n, K_int)`.
    pub quants: Vec<(String, crate::kind::Kind)>,
    /// Precondition constraint.
    pub constraint: NConstraint,
    /// Body type.
    pub typ: MiniTyp,
}

impl TypQuant {
    #[must_use]
    pub fn monomorphic(typ: MiniTyp) -> Self {
        Self {
            quants: Vec::new(),
            constraint: NConstraint::True,
            typ,
        }
    }
}

/// Loop kind for [`MiniExp::Loop`] (mirrors Sail `loop`: `while`/`until`).
///
/// Lane-127-Notiz (Checker-Minimal, keine Logik-Aenderung): Wahrheit
/// ist `sail_loops::LoopKind`; dieses Minimal bleibt bis Lane 47
/// (Typvereinheitlichung, siehe `ARCHITECTURE.md`) bestehen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MiniLoop {
    While,
    Until,
}

/// Iteration order for [`MiniExp::For`] (mirrors Sail `order`: `inc`/`dec`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MiniOrder {
    Inc,
    Dec,
}

/// One struct field initializer: `field = value` (mirrors Sail `fexp`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniFExp {
    pub field: String,
    pub value: MiniExp,
}

/// One match/try arm: `pat [if guard] => body` (mirrors Sail `pexp`:
/// `PatExp` for `guard: None`, `PatWhen` for `guard: Some`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniPExp {
    pub pat: MiniPat,
    pub guard: Option<MiniExp>,
    pub body: MiniExp,
}

/// Left-hand-side expression for assignment targets (subset of Sail
/// `lexp_aux`; mirrors `sail_ast::LExpKind` without spans).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiniLexp {
    /// `x`
    Id(String),
    /// `*e`
    Deref(Box<MiniExp>),
    /// `(x : t)` typed binder
    Typed { typ: MiniTyp, id: String },
    /// `(l1, l2, ...)`
    Tuple(Vec<MiniLexp>),
    /// Concatenated vector targets
    VectorConcat(Vec<MiniLexp>),
    /// `l[i]`
    Vector { lexp: Box<MiniLexp>, index: Box<MiniExp> },
    /// `l[hi .. lo]`
    VectorRange {
        lexp: Box<MiniLexp>,
        lo: Box<MiniExp>,
        hi: Box<MiniExp>,
    },
    /// `l.field`
    Field { lexp: Box<MiniLexp>, field: String },
}

/// Minimal expression: covers every non-internal Sail `exp_aux` constructor
/// (`E_*` in `sail/src/lib/extraction/Ast.ml`, one `E_lit` shared by
/// [`MiniExp::IntLit`]/[`MiniExp::BoolLit`). The bidirectional checker
/// still synthesizes only the Lane-79 subset
/// (`IntLit`/`BoolLit`/`Var`/`Let`/`App`/`Tuple`/`Annot`/`If`); all other
/// forms are representable plus convertible (`conv.rs`) but report a
/// mismatch at `infer` (checking deferred, no checker rules added).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiniExp {
    /// Integer literal (`E_lit`)
    IntLit(i128),
    /// Boolean literal (`E_lit`)
    BoolLit(bool),
    /// Variable (`E_id`, also `E_ref` targets via [`MiniExp::Ref`])
    Var(String),
    /// `let x: t = e1 in e2` (`E_let`, `x` must be a plain/annotated id)
    Let {
        var: String,
        annot: Option<MiniTyp>,
        bound: Box<MiniExp>,
        body: Box<MiniExp>,
    },
    /// `f x` (`E_app` with a variable head; curried heads stay `None` in conv)
    App(Box<MiniExp>, Box<MiniExp>),
    /// `(e1, e2, ...)` (`E_tuple`)
    Tuple(Vec<MiniExp>),
    /// `e : t` annotation (`E_typ`)
    Annot(Box<MiniExp>, MiniTyp),
    /// `if c then a else b` (`E_if`)
    If(Box<MiniExp>, Box<MiniExp>, Box<MiniExp>),
    /// `e1; e2; ...` sequence (`E_block`)
    Block(Vec<MiniExp>),
    /// `while`/`until cond do body` (`E_loop`; `measure` is the
    /// `in_place_loop_measure`, `None` = `Measure_none`)
    Loop {
        kind: MiniLoop,
        measure: Option<Box<MiniExp>>,
        cond: Box<MiniExp>,
        body: Box<MiniExp>,
    },
    /// `for x = <from> to <to> step <step> (<order>) do <body>` (`E_for`)
    For {
        var: String,
        from: Box<MiniExp>,
        to: Box<MiniExp>,
        step: Box<MiniExp>,
        order: MiniOrder,
        body: Box<MiniExp>,
    },
    /// `[e1, e2, ...]` (`E_vector`)
    Vector(Vec<MiniExp>),
    /// `v1 @ v2` (`E_vector_append`)
    VectorAppend(Box<MiniExp>, Box<MiniExp>),
    /// `[e1, e2, ...]` list literal (`E_list`)
    List(Vec<MiniExp>),
    /// `h :: t` (`E_cons`)
    Cons(Box<MiniExp>, Box<MiniExp>),
    /// `struct { f = e, ... }` (`E_struct`; `name: None` = anonymous)
    Struct {
        name: Option<String>,
        fields: Vec<MiniFExp>,
    },
    /// `r with { f = e, ... }` (`E_struct_update`)
    StructUpdate {
        record: Box<MiniExp>,
        fields: Vec<MiniFExp>,
    },
    /// `r.field` (`E_field`)
    Field { record: Box<MiniExp>, field: String },
    /// `match scrut { pat [if g] => body, ... }` (`E_match`)
    Match {
        scrutinee: Box<MiniExp>,
        arms: Vec<MiniPExp>,
    },
    /// `l = v` (`E_assign`)
    Assign {
        lexp: Box<MiniLexp>,
        value: Box<MiniExp>,
    },
    /// `sizeof(n)` (`E_sizeof`)
    Sizeof(Nexp),
    /// `return e` (`E_return`)
    Return(Box<MiniExp>),
    /// `exit e` (`E_exit`)
    Exit(Box<MiniExp>),
    /// `%config` keys (`E_config`)
    Config(Vec<String>),
    /// `ref x` (`E_ref`)
    Ref(String),
    /// `throw e` (`E_throw`)
    Throw(Box<MiniExp>),
    /// `try e catch { pat [if g] => body, ... }` (`E_try`)
    Try {
        exp: Box<MiniExp>,
        arms: Vec<MiniPExp>,
    },
    /// `assert(cond, msg)` (`E_assert`)
    Assert { cond: Box<MiniExp>, msg: Box<MiniExp> },
    /// `var l = init in body` (`E_var`; distinct from [`MiniExp::Var`],
    /// which is the `E_id` variable reference)
    VarDecl {
        lexp: Box<MiniLexp>,
        init: Box<MiniExp>,
        body: Box<MiniExp>,
    },
    /// `undefined` (`E_undef`)
    Undef,
    /// `constraint(c)` (`E_constraint`)
    Constraint(NConstraint),
}

// ---------------------------------------------------------------------------
// Lane 10: canonical-path normalization, SMT-power detection, and
// arithmetic backward inference.
// ---------------------------------------------------------------------------
//
// Reference: OCaml `type_check.ml` (`nexp_simp`/`constraint_simp`,
// `constraint_power_variables`, `unify_nexp`); `sail-solver/src/canonical.rs`
// is read-only here. The canonical SMT emitter rejects `NExp::Exp` with a
// symbolic exponent and general numeric/constraint applications
// ("numeric application/power needs elaboration before proof",
// "constraint synonym needs expansion before proof"): those stay
// [`ProveResult::Unknown`](crate::solver::ProveResult) on the stub path and
// are decided only by stronger backends behind the [`Solver`] trait.
// What this section completes on the canonical path:
// - ground powers (`2^c`, `c` in `0..=126`) fold to constants;
// - symbolic powers are detected ([`Nexp::has_power`]/[`NConstraint::has_power`])
//   so callers can document/skip them instead of mis-proving;
// - cheap identities (`+0`, `-0`, `*0`, `*1`, `/1`, `mod 1`, double
//   negation, `if true/false`, boolean absorptions, `Set` sort/dedup)
//   are normalized away before proving, so `Eq`-reflexivity fires;
// - one-step backward inference ([`solve_for_var`]) inverts `Add`/`Sub`
//   (and constant `Mul`/`Div`) to solve omitted implicits from equations.

/// Bottom-up normalization of a level expression: ground subterms fold
/// via [`Nexp::as_const`], then one layer of total identities applies.
/// Pure, total, and shape-preserving for opaque subterms.
#[must_use]
pub fn normalize_nexp(n: &Nexp) -> Nexp {
    match n {
        Nexp::Const(_) | Nexp::Var(_) => n.clone(),
        Nexp::Neg(a) => {
            let a = normalize_nexp(a);
            if let Some(c) = Nexp::Neg(Box::new(a.clone())).as_const() {
                return Nexp::Const(c);
            }
            match a {
                Nexp::Neg(inner) => *inner,
                _ => Nexp::Neg(Box::new(a)),
            }
        }
        Nexp::Abs(a) => {
            let a = normalize_nexp(a);
            if let Some(c) = Nexp::Abs(Box::new(a.clone())).as_const() {
                return Nexp::Const(c);
            }
            match a {
                Nexp::Abs(inner) => Nexp::Abs(inner),
                _ => Nexp::Abs(Box::new(a)),
            }
        }
        Nexp::Exp(a) => {
            let a = normalize_nexp(a);
            if let Some(c) = Nexp::Exp(Box::new(a.clone())).as_const() {
                return Nexp::Const(c);
            }
            Nexp::Exp(Box::new(a))
        }
        Nexp::Add(a, b) => {
            let (x, y) = (normalize_nexp(a), normalize_nexp(b));
            if let Some(c) = Nexp::Add(Box::new(x.clone()), Box::new(y.clone())).as_const() {
                return Nexp::Const(c);
            }
            match (&x, &y) {
                (Nexp::Const(0), _) => y,
                (_, Nexp::Const(0)) => x,
                _ => Nexp::Add(Box::new(x), Box::new(y)),
            }
        }
        Nexp::Sub(a, b) => {
            let (x, y) = (normalize_nexp(a), normalize_nexp(b));
            if let Some(c) = Nexp::Sub(Box::new(x.clone()), Box::new(y.clone())).as_const() {
                return Nexp::Const(c);
            }
            if x == y {
                return Nexp::Const(0);
            }
            match &y {
                Nexp::Const(0) => x,
                _ => Nexp::Sub(Box::new(x), Box::new(y)),
            }
        }
        Nexp::Mul(a, b) => {
            let (x, y) = (normalize_nexp(a), normalize_nexp(b));
            if let Some(c) = Nexp::Mul(Box::new(x.clone()), Box::new(y.clone())).as_const() {
                return Nexp::Const(c);
            }
            match (&x, &y) {
                (Nexp::Const(0), _) | (_, Nexp::Const(0)) => Nexp::Const(0),
                (Nexp::Const(1), _) => y,
                (_, Nexp::Const(1)) => x,
                _ => Nexp::Mul(Box::new(x), Box::new(y)),
            }
        }
        Nexp::Div(a, b) => {
            let (x, y) = (normalize_nexp(a), normalize_nexp(b));
            if let Some(c) = Nexp::Div(Box::new(x.clone()), Box::new(y.clone())).as_const() {
                return Nexp::Const(c);
            }
            match &y {
                Nexp::Const(1) => x,
                _ => Nexp::Div(Box::new(x), Box::new(y)),
            }
        }
        Nexp::Mod(a, b) => {
            let (x, y) = (normalize_nexp(a), normalize_nexp(b));
            if let Some(c) = Nexp::Mod(Box::new(x.clone()), Box::new(y.clone())).as_const() {
                return Nexp::Const(c);
            }
            match (&x, &y) {
                (_, Nexp::Const(1)) => Nexp::Const(0),
                (Nexp::Const(0), Nexp::Const(d)) if *d != 0 => Nexp::Const(0),
                _ => Nexp::Mod(Box::new(x), Box::new(y)),
            }
        }
        Nexp::If(c, a, b) => {
            let (cc, x, y) = (
                normalize_constraint(c),
                normalize_nexp(a),
                normalize_nexp(b),
            );
            match cc {
                NConstraint::True => x,
                NConstraint::False => y,
                _ if x == y => x,
                _ => Nexp::If(Box::new(cc), Box::new(x), Box::new(y)),
            }
        }
    }
}

/// Bottom-up normalization of a constraint: numeric sides via
/// [`normalize_nexp`], boolean absorptions, double negation, and `Set`
/// sort/dedup. Ground goals fold via [`NConstraint::as_bool`].
/// Uninterpreted predicates (`App`) and opaque booleans (`Var`/`Id`)
/// are preserved (soundly opaque, like the canonical emitter).
#[must_use]
pub fn normalize_constraint(c: &NConstraint) -> NConstraint {
    match c {
        NConstraint::True | NConstraint::False | NConstraint::Var(_) | NConstraint::Id(_) => {
            c.clone()
        }
        NConstraint::Eq(a, b) => {
            let (x, y) = (normalize_nexp(a), normalize_nexp(b));
            let folded = NConstraint::Eq(Box::new(x.clone()), Box::new(y.clone()));
            if let Some(v) = folded.as_bool() {
                return if v {
                    NConstraint::True
                } else {
                    NConstraint::False
                };
            }
            folded
        }
        NConstraint::Neq(a, b) => {
            let (x, y) = (normalize_nexp(a), normalize_nexp(b));
            let folded = NConstraint::Neq(Box::new(x.clone()), Box::new(y.clone()));
            if let Some(v) = folded.as_bool() {
                return if v {
                    NConstraint::True
                } else {
                    NConstraint::False
                };
            }
            folded
        }
        NConstraint::Ge(a, b) => cmp_fold(NConstraint::Ge as fn(_, _) -> _, a, b),
        NConstraint::Gt(a, b) => cmp_fold(NConstraint::Gt as fn(_, _) -> _, a, b),
        NConstraint::Le(a, b) => cmp_fold(NConstraint::Le as fn(_, _) -> _, a, b),
        NConstraint::Lt(a, b) => cmp_fold(NConstraint::Lt as fn(_, _) -> _, a, b),
        NConstraint::And(a, b) => {
            let (x, y) = (normalize_constraint(a), normalize_constraint(b));
            match (&x, &y) {
                (NConstraint::False, _) | (_, NConstraint::False) => NConstraint::False,
                (NConstraint::True, _) => y,
                (_, NConstraint::True) => x,
                _ if x == y => x,
                _ => NConstraint::And(Box::new(x), Box::new(y)),
            }
        }
        NConstraint::Or(a, b) => {
            let (x, y) = (normalize_constraint(a), normalize_constraint(b));
            match (&x, &y) {
                (NConstraint::True, _) | (_, NConstraint::True) => NConstraint::True,
                (NConstraint::False, _) => y,
                (_, NConstraint::False) => x,
                _ if x == y => x,
                _ => NConstraint::Or(Box::new(x), Box::new(y)),
            }
        }
        NConstraint::Not(a) => match normalize_constraint(a) {
            NConstraint::True => NConstraint::False,
            NConstraint::False => NConstraint::True,
            NConstraint::Not(inner) => *inner,
            x => NConstraint::Not(Box::new(x)),
        },
        NConstraint::Set(n, vs) => {
            let x = normalize_nexp(n);
            let mut members = vs.clone();
            members.sort();
            members.dedup();
            let folded =
                NConstraint::Set(Box::new(x.clone()), members.clone());
            if let Some(v) = folded.as_bool() {
                return if v {
                    NConstraint::True
                } else {
                    NConstraint::False
                };
            }
            folded
        }
        NConstraint::App(func, args) => {
            NConstraint::App(func.clone(), args.iter().map(normalize_nexp).collect())
        }
    }
}

fn cmp_fold(
    mk: fn(Box<Nexp>, Box<Nexp>) -> NConstraint,
    a: &Nexp,
    b: &Nexp,
) -> NConstraint {
    let (x, y) = (normalize_nexp(a), normalize_nexp(b));
    let folded = mk(Box::new(x.clone()), Box::new(y.clone()));
    if let Some(v) = folded.as_bool() {
        return if v {
            NConstraint::True
        } else {
            NConstraint::False
        };
    }
    folded
}

fn free_in(var: &str, n: &Nexp) -> bool {
    let mut out = Vec::new();
    n.free_vars(&mut out);
    out.iter().any(|v| v == var)
}

/// Solve the equation `lhs == rhs` for `var` with one-step arithmetic
/// backward inference (OCaml `unify_nexp`, linear skeleton only).
///
/// Handles `Var`, `Add`/`Sub` inversion, `Neg`, constant `Mul`/`Div`
/// with exact division, and mirrored orientations. Returns the solved
/// expression (normalized) or `None` when the shape is non-linear,
/// under-constrained, or fails the occurs check. Total.
#[must_use]
pub fn solve_for_var(var: &str, lhs: &Nexp, rhs: &Nexp) -> Option<Nexp> {
    solve_side(var, lhs, rhs)
        .or_else(|| solve_side(var, rhs, lhs))
        .map(|n| normalize_nexp(&n))
}

fn solve_side(var: &str, lhs: &Nexp, rhs: &Nexp) -> Option<Nexp> {
    match lhs {
        Nexp::Var(v) if v == var => {
            if free_in(var, rhs) {
                None
            } else {
                Some(rhs.clone())
            }
        }
        Nexp::Neg(a) => solve_side(
            var,
            a,
            &Nexp::Neg(Box::new(rhs.clone())),
        ),
        Nexp::Add(a, b) => {
            let in_a = free_in(var, a);
            let in_b = free_in(var, b);
            match (in_a, in_b) {
                (true, false) => solve_side(
                    var,
                    a,
                    &Nexp::Sub(Box::new(rhs.clone()), Box::new((**b).clone())),
                ),
                (false, true) => solve_side(
                    var,
                    b,
                    &Nexp::Sub(Box::new(rhs.clone()), Box::new((**a).clone())),
                ),
                _ => None,
            }
        }
        Nexp::Sub(a, b) => {
            let in_a = free_in(var, a);
            let in_b = free_in(var, b);
            match (in_a, in_b) {
                (true, false) => solve_side(
                    var,
                    a,
                    &Nexp::Add(Box::new(rhs.clone()), Box::new((**b).clone())),
                ),
                (false, true) => {
                    // `a - var == rhs`  =>  `var == a - rhs`.
                    let solved = Nexp::Sub(Box::new((**a).clone()), Box::new(rhs.clone()));
                    if free_in(var, &solved) {
                        None
                    } else {
                        Some(solved)
                    }
                }
                _ => None,
            }
        }
        Nexp::Mul(a, b) => {
            let in_a = free_in(var, a);
            let in_b = free_in(var, b);
            match (in_a, in_b, (&**a, &**b)) {
                (true, false, (_, Nexp::Const(c))) if *c != 0 => {
                    match rhs.as_const() {
                        Some(r) if r % c == 0 => solve_side(
                            var,
                            a,
                            &Nexp::Const(r / c),
                        ),
                        _ => None,
                    }
                }
                (false, true, (Nexp::Const(c), _)) if *c != 0 => {
                    match rhs.as_const() {
                        Some(r) if r % c == 0 => solve_side(
                            var,
                            b,
                            &Nexp::Const(r / c),
                        ),
                        _ => None,
                    }
                }
                _ => None,
            }
        }
        Nexp::Div(a, b) => match (&**b, free_in(var, a), free_in(var, b)) {
            (Nexp::Const(c), true, false) if *c != 0 => solve_side(
                var,
                a,
                &Nexp::Mul(Box::new(rhs.clone()), Box::new(Nexp::Const(*c))),
            ),
            _ => None,
        },
        _ => None,
    }
}

impl Nexp {
    /// Whether this term contains a *symbolic* power (`2^n` with
    /// non-constant `n`). Ground powers fold via [`Nexp::as_const`]; only
    /// symbolic ones need SMT elaboration on the canonical path
    /// (mirrors OCaml `constraint_power_variables`).
    #[must_use]
    pub fn has_power(&self) -> bool {
        match self {
            Self::Const(_) | Self::Var(_) => false,
            Self::Exp(a) => a.as_const().is_none() || a.has_power(),
            Self::Neg(a) | Self::Abs(a) => a.has_power(),
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Mod(a, b) => a.has_power() || b.has_power(),
            Self::If(c, a, b) => c.has_power() || a.has_power() || b.has_power(),
        }
    }
}

impl NConstraint {
    /// Whether this goal mentions a symbolic power. Such goals are
    /// soundly `Unknown` on the stub path (never mis-proved); the SMT
    /// backend decides them.
    #[must_use]
    pub fn has_power(&self) -> bool {
        match self {
            Self::True | Self::False | Self::Var(_) | Self::Id(_) => false,
            Self::Eq(a, b)
            | Self::Neq(a, b)
            | Self::Ge(a, b)
            | Self::Gt(a, b)
            | Self::Le(a, b)
            | Self::Lt(a, b) => a.has_power() || b.has_power(),
            Self::And(a, b) | Self::Or(a, b) => a.has_power() || b.has_power(),
            Self::Not(a) => a.has_power(),
            Self::Set(n, _) => n.has_power(),
            Self::App(_, args) => args.iter().any(Nexp::has_power),
        }
    }
}
