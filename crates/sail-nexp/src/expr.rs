#![forbid(unsafe_code)]
//! Numeric type-level expressions of Sail (`Nexp_*` in
//! `sail/src/lib/extraction/Ast.ml`).
//!
//! Constructors mirror the OCaml AST; `div`, `mod` and `abs` are
//! `Nexp_app` nodes there, exposed here via [`NExp::div`],
//! [`NExp::modulo`] and [`NExp::abs`] helpers.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use sail_ast::Symbol;

/// Numeric kind variable (`kid` in the Sail AST), e.g. `"n"`.
///
/// Interned [`Symbol`] (Lane 71 payload-intern): repeated occurrences share
/// one table entry. Build one via an [`Interner`][sail_ast::Interner] and
/// resolve it back with
/// [`resolve`][sail_ast::Interner::resolve]. (`Id`/`App` operator names stay
/// plain `String`s — see `docs/payload-intern-contract.md`.)
pub type Kid = Symbol;

/// Numeric type-level expression.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NExp {
    /// Type-level identifier that is not a kind variable.
    Id(String),
    /// `Nexp_var kid` — kind variable reference.
    Var(Kid),
    /// `Nexp_constant i`.
    Constant(i128),
    /// `Nexp_app (f, args)` — `div`, `mod`, `abs` and unknown functions.
    App(String, Vec<NExp>),
    /// `Nexp_if (cond, then, else)`.
    If(Box<NConstraint>, Box<NExp>, Box<NExp>),
    /// `Nexp_times (a, b)`.
    Times(Box<NExp>, Box<NExp>),
    /// `Nexp_sum (a, b)`.
    Sum(Box<NExp>, Box<NExp>),
    /// `Nexp_minus (a, b)`.
    Minus(Box<NExp>, Box<NExp>),
    /// `Nexp_exp x` — `2 ^ x`.
    Exp(Box<NExp>),
    /// `Nexp_neg x` — `-x`.
    Neg(Box<NExp>),
}

/// Type-level argument (`typ_arg` in the Sail AST, `Ast.ml`: `A_nexp n
/// | A_typ t | A_bool nc`).
///
/// ARM constraints apply `NC_app`/`NC_equal`/`NC_not_equal` to all three
/// sorts — modelling only numeric expressions is too narrow (Lane 108).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypArg {
    /// `A_nexp n` — numeric/size expression.
    NExp(NExp),
    /// `A_typ t` — type argument.
    Typ(NTyp),
    /// `A_bool nc` — boolean constraint argument.
    Bool(Box<NConstraint>),
}

/// Structural type payload for [`TypArg::Typ`], mirroring the `typ_aux`
/// shapes that can occur inside constraints. Predicate/identifier names
/// stay plain `String`s (like [`NExp::Id`]/[`NConstraint::Id`]); kind
/// variables stay interned [`Kid`]s.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NTyp {
    /// Placeholder the typechecker fills in later (`Typ_unknown`).
    Unknown,
    /// Nullary named type (`Typ_id`).
    Id(String),
    /// Type variable (`Typ_var`).
    Var(Kid),
    /// Function type (`Typ_fn (args, ret)`).
    Fn(Vec<NTyp>, Box<NTyp>),
    /// Bidirectional type (`Typ_bidir (lhs, rhs)`).
    Bidir(Box<NTyp>, Box<NTyp>),
    /// Tuple type (`Typ_tuple ts`).
    Tuple(Vec<NTyp>),
    /// Type application (`Typ_app (name, args)`).
    App(String, Vec<TypArg>),
    /// Existential (`Typ_exist (binders, constraint, typ)`).
    Exist(Vec<NQuant>, Box<NConstraint>, Box<NTyp>),
}

/// Kind-annotated binder for [`NTyp::Exist`] (mirrors `KOpt_kind`).
/// [`NKind`] carries its own `Ord` (unlike `sail_ast::Kind`) so
/// [`NTyp`] keeps the canonical ordering used by constraint
/// normalization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NQuant {
    pub kind: NKind,
    pub kid: Kid,
}

/// Sail kinds (`Type`, `Int`, `Bool`) for [`NQuant`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NKind {
    Type,
    Int,
    Bool,
}

/// Numeric constraint (`n_constraint` in the Sail AST).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NConstraint {
    True,
    False,
    /// `NC_id i`.
    Id(String),
    /// Boolean kind variable used as a constraint.
    Var(Kid),
    /// `NC_equal (t1, t2)` over `typ_arg` sorts.
    Equal(Box<TypArg>, Box<TypArg>),
    /// `NC_not_equal (t1, t2)` over `typ_arg` sorts.
    NotEqual(Box<TypArg>, Box<TypArg>),
    Ge(Box<NExp>, Box<NExp>),
    Gt(Box<NExp>, Box<NExp>),
    Le(Box<NExp>, Box<NExp>),
    Lt(Box<NExp>, Box<NExp>),
    /// `NC_set (n, vs)` — `n` is one of `vs`.
    Set(Box<NExp>, Vec<i128>),
    /// `NC_app (f, args)` — uninterpreted predicate application over
    /// `typ_arg` sorts (`A_nexp`/`A_typ`/`A_bool`).
    App(String, Vec<TypArg>),
    And(Box<NConstraint>, Box<NConstraint>),
    Or(Box<NConstraint>, Box<NConstraint>),
    Not(Box<NConstraint>),
}

/// Rebuild a binary [`NExp`] node only when at least one side changed;
/// otherwise borrow the original node (no fresh `Box`, no `clone`).
fn subst_binary<'a>(
    subst: &BTreeMap<Kid, NExp>,
    orig: &'a NExp,
    a: &'a NExp,
    b: &'a NExp,
    rebuild: fn(Box<NExp>, Box<NExp>) -> NExp,
) -> Cow<'a, NExp> {
    let (ra, rb) = (a.subst_cow(subst), b.subst_cow(subst));
    if matches!(ra, Cow::Borrowed(_)) && matches!(rb, Cow::Borrowed(_)) {
        Cow::Borrowed(orig)
    } else {
        Cow::Owned(rebuild(Box::new(ra.into_owned()), Box::new(rb.into_owned())))
    }
}

/// Rebuild a unary [`NExp`] node only when the child changed.
fn subst_unary<'a>(
    subst: &BTreeMap<Kid, NExp>,
    orig: &'a NExp,
    x: &'a NExp,
    rebuild: fn(Box<NExp>) -> NExp,
) -> Cow<'a, NExp> {
    let rx = x.subst_cow(subst);
    if matches!(rx, Cow::Borrowed(_)) {
        Cow::Borrowed(orig)
    } else {
        Cow::Owned(rebuild(Box::new(rx.into_owned())))
    }
}

/// Rebuild a binary [`NConstraint`]-over-constraints node only when at least
/// one side changed (boolean connectives hold `Box<NConstraint>`, unlike the
/// [`NExp`] operands handled by [`subst_constraint_binary`]).
fn subst_constraint_pair<'a>(
    subst: &BTreeMap<Kid, NExp>,
    orig: &'a NConstraint,
    a: &'a NConstraint,
    b: &'a NConstraint,
    rebuild: fn(Box<NConstraint>, Box<NConstraint>) -> NConstraint,
) -> Cow<'a, NConstraint> {
    let (ra, rb) = (a.subst_cow(subst), b.subst_cow(subst));
    if matches!(ra, Cow::Borrowed(_)) && matches!(rb, Cow::Borrowed(_)) {
        Cow::Borrowed(orig)
    } else {
        Cow::Owned(rebuild(Box::new(ra.into_owned()), Box::new(rb.into_owned())))
    }
}
/// Rebuild a binary [`NConstraint`] node only when at least one side changed.
fn subst_constraint_binary<'a>(
    subst: &BTreeMap<Kid, NExp>,
    orig: &'a NConstraint,
    a: &'a NExp,
    b: &'a NExp,
    rebuild: fn(Box<NExp>, Box<NExp>) -> NConstraint,
) -> Cow<'a, NConstraint> {
    let (ra, rb) = (a.subst_cow(subst), b.subst_cow(subst));
    if matches!(ra, Cow::Borrowed(_)) && matches!(rb, Cow::Borrowed(_)) {
        Cow::Borrowed(orig)
    } else {
        Cow::Owned(rebuild(Box::new(ra.into_owned()), Box::new(rb.into_owned())))
    }
}

/// Rebuild a binary [`NConstraint`] node over [`TypArg`]s only when at
/// least one side changed (`NC_equal`/`NC_not_equal` hold
/// `Box<TypArg>`, unlike the [`NExp`] operands handled by
/// [`subst_constraint_binary`]).
fn subst_typarg_pair<'a>(
    subst: &BTreeMap<Kid, NExp>,
    orig: &'a NConstraint,
    a: &'a TypArg,
    b: &'a TypArg,
    rebuild: fn(Box<TypArg>, Box<TypArg>) -> NConstraint,
) -> Cow<'a, NConstraint> {
    let (ra, rb) = (a.subst_cow(subst), b.subst_cow(subst));
    if matches!(ra, Cow::Borrowed(_)) && matches!(rb, Cow::Borrowed(_)) {
        Cow::Borrowed(orig)
    } else {
        Cow::Owned(rebuild(Box::new(ra.into_owned()), Box::new(rb.into_owned())))
    }
}

/// Rewrite a slice elementwise; `None` when every element borrows (no
/// allocation, no clone), `Some` with the materialized values otherwise.
///
/// Same Cow idiom as the `App`-argument loops: the temporary `Vec<Cow>`
/// is the only allocation on the unchanged path.
fn rewrite_vec<'a, T: Clone>(
    xs: &'a [T],
    f: impl Fn(&'a T) -> Cow<'a, T>,
) -> Option<Vec<T>> {
    let rewritten: Vec<Cow<'a, T>> = xs.iter().map(f).collect();
    if rewritten.iter().all(|c| matches!(c, Cow::Borrowed(_))) {
        None
    } else {
        Some(rewritten.into_iter().map(Cow::into_owned).collect())
    }
}

impl NExp {
    /// `n div m` as `Nexp_app ("div", [n, m])`.
    ///
    /// Named after the Sail function, not `std::ops::Div`: this is a
    /// constructor, not division of two `NExp` values.
    #[allow(clippy::should_implement_trait)]
    pub fn div(a: NExp, b: NExp) -> NExp {
        NExp::App("div".to_string(), vec![a, b])
    }

    /// `n mod m` as `Nexp_app ("mod", [n, m])`.
    pub fn modulo(a: NExp, b: NExp) -> NExp {
        NExp::App("mod".to_string(), vec![a, b])
    }

    /// `abs n` as `Nexp_app ("abs", [n])`.
    pub fn abs(a: NExp) -> NExp {
        NExp::App("abs".to_string(), vec![a])
    }

    /// `(n: Int)` variable.
    pub fn var(name: Kid) -> NExp {
        NExp::Var(name)
    }

    /// Integer literal.
    pub fn constant(v: i128) -> NExp {
        NExp::Constant(v)
    }

    /// Free kind variables in first-occurrence order.
    pub fn vars(&self) -> Vec<Kid> {
        let mut acc = Vec::new();
        let mut seen = BTreeSet::new();
        self.collect_vars(&mut acc, &mut seen);
        acc
    }

    fn collect_vars(&self, acc: &mut Vec<Kid>, seen: &mut BTreeSet<Kid>) {
        match self {
            NExp::Id(_) | NExp::Constant(_) => {}
            NExp::Var(k) => {
                if seen.insert(*k) {
                    acc.push(*k);
                }
            }
            NExp::App(_, args) => {
                for a in args {
                    a.collect_vars(acc, seen);
                }
            }
            NExp::If(c, t, e) => {
                c.collect_vars(acc, seen);
                t.collect_vars(acc, seen);
                e.collect_vars(acc, seen);
            }
            NExp::Times(a, b) | NExp::Sum(a, b) | NExp::Minus(a, b) => {
                a.collect_vars(acc, seen);
                b.collect_vars(acc, seen);
            }
            NExp::Exp(x) | NExp::Neg(x) => x.collect_vars(acc, seen),
        }
    }

    /// Substitute kind variables according to `subst`.
    ///
    /// Unchanged subtrees are shared, not rebuilt: use
    /// [`NExp::subst_cow`] directly when the caller can keep a borrow.
    pub fn subst(&self, subst: &BTreeMap<Kid, NExp>) -> NExp {
        self.subst_cow(subst).into_owned()
    }

    /// Substitute kind variables, borrowing `self` wherever no variable
    /// below the node is hit by `subst`.
    ///
    /// RAM diet (Lane 68): the old `subst` re-boxed every node on the
    /// path even when nothing changed. A `Borrowed` result allocates
    /// nothing — no `clone`, no fresh `Box`. Values are identical to
    /// [`NExp::subst`].
    pub fn subst_cow(&self, subst: &BTreeMap<Kid, NExp>) -> Cow<'_, NExp> {
        match self {
            NExp::Id(_) | NExp::Constant(_) => Cow::Borrowed(self),
            NExp::Var(k) => match subst.get(k) {
                Some(repl) => Cow::Owned(repl.clone()),
                None => Cow::Borrowed(self),
            },
            NExp::App(f, args) => {
                let rewritten: Vec<Cow<'_, NExp>> =
                    args.iter().map(|a| a.subst_cow(subst)).collect();
                if rewritten.iter().any(|c| matches!(c, Cow::Owned(_))) {
                    Cow::Owned(NExp::App(
                        f.clone(),
                        rewritten.into_iter().map(Cow::into_owned).collect(),
                    ))
                } else {
                    Cow::Borrowed(self)
                }
            }
            NExp::If(c, t, e) => {
                let (cc, tc, ec) = (c.subst_cow(subst), t.subst_cow(subst), e.subst_cow(subst));
                if matches!(cc, Cow::Borrowed(_))
                    && matches!(tc, Cow::Borrowed(_))
                    && matches!(ec, Cow::Borrowed(_))
                {
                    Cow::Borrowed(self)
                } else {
                    Cow::Owned(NExp::If(
                        Box::new(cc.into_owned()),
                        Box::new(tc.into_owned()),
                        Box::new(ec.into_owned()),
                    ))
                }
            }
            NExp::Times(a, b) => subst_binary(subst, self, a, b, NExp::Times),
            NExp::Sum(a, b) => subst_binary(subst, self, a, b, NExp::Sum),
            NExp::Minus(a, b) => subst_binary(subst, self, a, b, NExp::Minus),
            NExp::Exp(x) => subst_unary(subst, self, x, NExp::Exp),
            NExp::Neg(x) => subst_unary(subst, self, x, NExp::Neg),
        }
    }

    /// Total valuation under `env`.
    ///
    /// Returns `None` when a variable is unbound, an unknown
    /// function/identifier occurs, division by zero happens, or checked
    /// arithmetic overflows. `2 ^ x` is `None` for `x < 0` or `x >= 127`.
    /// `div`/`mod` use truncating semantics (Rust `/`, `%`).
    pub fn eval(&self, env: &BTreeMap<Kid, i128>) -> Option<i128> {
        match self {
            NExp::Id(_) => None,
            NExp::Var(k) => env.get(k).copied(),
            NExp::Constant(v) => Some(*v),
            NExp::App(f, args) => match (f.as_str(), args.as_slice()) {
                ("div", [a, b]) => {
                    let (x, y) = (a.eval(env)?, b.eval(env)?);
                    if y == 0 {
                        None
                    } else {
                        x.checked_div(y)
                    }
                }
                ("mod", [a, b]) => {
                    let (x, y) = (a.eval(env)?, b.eval(env)?);
                    if y == 0 {
                        None
                    } else {
                        x.checked_rem(y)
                    }
                }
                ("abs", [a]) => a.eval(env)?.checked_abs(),
                _ => None,
            },
            NExp::If(c, t, e) => {
                if c.eval(env)? {
                    t.eval(env)
                } else {
                    e.eval(env)
                }
            }
            NExp::Times(a, b) => a.eval(env)?.checked_mul(b.eval(env)?),
            NExp::Sum(a, b) => a.eval(env)?.checked_add(b.eval(env)?),
            NExp::Minus(a, b) => a.eval(env)?.checked_sub(b.eval(env)?),
            NExp::Exp(x) => {
                let e = x.eval(env)?;
                if (0..127).contains(&e) {
                    1i128.checked_shl(e as u32)
                } else if e == 127 {
                    // `1 << 127` overflows i128; report unrepresentable.
                    None
                } else {
                    None
                }
            }
            NExp::Neg(x) => x.eval(env)?.checked_neg(),
        }
    }
}

impl TypArg {
    /// Free kind variables in first-occurrence order (see [`NExp::vars`];
    /// [`NTyp::Exist`] binders shadow like binders should: bound
    /// occurrences are not reported).
    pub fn vars(&self) -> Vec<Kid> {
        let mut acc = Vec::new();
        let mut seen = BTreeSet::new();
        self.collect_vars(&mut acc, &mut seen);
        acc
    }

    fn collect_vars(&self, acc: &mut Vec<Kid>, seen: &mut BTreeSet<Kid>) {
        match self {
            TypArg::NExp(e) => e.collect_vars(acc, seen),
            TypArg::Typ(t) => t.collect_vars(acc, seen),
            TypArg::Bool(c) => c.collect_vars(acc, seen),
        }
    }

    /// Substitute kind variables according to `subst` (see
    /// [`NExp::subst_cow`] for the borrowing contract).
    pub fn subst(&self, subst: &BTreeMap<Kid, NExp>) -> TypArg {
        self.subst_cow(subst).into_owned()
    }

    /// Substitute kind variables, borrowing `self` wherever no variable
    /// below the node is hit by `subst`.
    pub fn subst_cow(&self, subst: &BTreeMap<Kid, NExp>) -> Cow<'_, TypArg> {
        match self {
            TypArg::NExp(e) => {
                let r = e.subst_cow(subst);
                if matches!(r, Cow::Borrowed(_)) {
                    Cow::Borrowed(self)
                } else {
                    Cow::Owned(TypArg::NExp(r.into_owned()))
                }
            }
            TypArg::Typ(t) => {
                let r = t.subst_cow(subst);
                if matches!(r, Cow::Borrowed(_)) {
                    Cow::Borrowed(self)
                } else {
                    Cow::Owned(TypArg::Typ(r.into_owned()))
                }
            }
            TypArg::Bool(c) => {
                let r = c.subst_cow(subst);
                if matches!(r, Cow::Borrowed(_)) {
                    Cow::Borrowed(self)
                } else {
                    Cow::Owned(TypArg::Bool(Box::new(r.into_owned())))
                }
            }
        }
    }
}

impl NTyp {
    /// Free kind variables in first-occurrence order. `Exist` binders
    /// are inserted into `seen` before the body is traversed, so bound
    /// occurrences are not reported as free (the reported *order* of a
    /// variable that is both free elsewhere and bound here is the free
    /// occurrence — binders themselves are never reported).
    pub fn vars(&self) -> Vec<Kid> {
        let mut acc = Vec::new();
        let mut seen = BTreeSet::new();
        self.collect_vars(&mut acc, &mut seen);
        acc
    }

    fn collect_vars(&self, acc: &mut Vec<Kid>, seen: &mut BTreeSet<Kid>) {
        match self {
            NTyp::Unknown | NTyp::Id(_) => {}
            NTyp::Var(k) => {
                if seen.insert(*k) {
                    acc.push(*k);
                }
            }
            NTyp::Fn(args, ret) => {
                for a in args {
                    a.collect_vars(acc, seen);
                }
                ret.collect_vars(acc, seen);
            }
            NTyp::Bidir(a, b) => {
                a.collect_vars(acc, seen);
                b.collect_vars(acc, seen);
            }
            NTyp::Tuple(ts) => {
                for t in ts {
                    t.collect_vars(acc, seen);
                }
            }
            NTyp::App(_, args) => {
                for a in args {
                    a.collect_vars(acc, seen);
                }
            }
            NTyp::Exist(binders, c, t) => {
                for b in binders {
                    seen.insert(b.kid);
                }
                c.collect_vars(acc, seen);
                t.collect_vars(acc, seen);
            }
        }
    }

    /// Substitute kind variables according to `subst` (see
    /// [`NExp::subst_cow`] for the borrowing contract).
    ///
    /// `Exist` binders shadow: kids bound by the existential are removed
    /// from the substitution inside its scope.
    pub fn subst(&self, subst: &BTreeMap<Kid, NExp>) -> NTyp {
        self.subst_cow(subst).into_owned()
    }

    /// Substitute kind variables, borrowing `self` wherever no variable
    /// below the node is hit by `subst`.
    pub fn subst_cow(&self, subst: &BTreeMap<Kid, NExp>) -> Cow<'_, NTyp> {
        match self {
            // Leaves: `Typ_var` has no numeric payload to splice a
            // substitution into (numeric mappings only matter for
            // `NExp`/`Bool` positions, handled by the callers), so the
            // node is always kept.
            NTyp::Unknown | NTyp::Id(_) | NTyp::Var(_) => Cow::Borrowed(self),
            NTyp::Fn(args, ret) => {
                let rargs = rewrite_vec(args, |a| a.subst_cow(subst));
                let rret = ret.subst_cow(subst);
                if rargs.is_none() && matches!(rret, Cow::Borrowed(_)) {
                    Cow::Borrowed(self)
                } else {
                    Cow::Owned(NTyp::Fn(
                        rargs.unwrap_or_else(|| args.clone()),
                        Box::new(rret.into_owned()),
                    ))
                }
            }
            NTyp::Bidir(a, b) => {
                let (ra, rb) = (a.subst_cow(subst), b.subst_cow(subst));
                if matches!(ra, Cow::Borrowed(_)) && matches!(rb, Cow::Borrowed(_)) {
                    Cow::Borrowed(self)
                } else {
                    Cow::Owned(NTyp::Bidir(
                        Box::new(ra.into_owned()),
                        Box::new(rb.into_owned()),
                    ))
                }
            }
            NTyp::Tuple(ts) => match rewrite_vec(ts, |t| t.subst_cow(subst)) {
                None => Cow::Borrowed(self),
                Some(ts) => Cow::Owned(NTyp::Tuple(ts)),
            },
            NTyp::App(f, args) => match rewrite_vec(args, |a| a.subst_cow(subst)) {
                None => Cow::Borrowed(self),
                Some(args) => Cow::Owned(NTyp::App(f.clone(), args)),
            },
            NTyp::Exist(binders, c, t) => {
                // Shadow bound kids inside the scope (see `subst`).
                let active: Cow<'_, BTreeMap<Kid, NExp>> =
                    if binders.iter().any(|b| subst.contains_key(&b.kid)) {
                        Cow::Owned(
                            subst
                                .iter()
                                .filter(|(k, _)| !binders.iter().any(|b| &b.kid == *k))
                                .map(|(k, v)| (*k, v.clone()))
                                .collect(),
                        )
                    } else {
                        Cow::Borrowed(subst)
                    };
                let (rc, rt) = (c.subst_cow(&active), t.subst_cow(&active));
                if matches!(rc, Cow::Borrowed(_)) && matches!(rt, Cow::Borrowed(_)) {
                    Cow::Borrowed(self)
                } else {
                    Cow::Owned(NTyp::Exist(
                        binders.clone(),
                        Box::new(rc.into_owned()),
                        Box::new(rt.into_owned()),
                    ))
                }
            }
        }
    }
}

impl NConstraint {
    /// Free kind variables in first-occurrence order.
    pub fn vars(&self) -> Vec<Kid> {
        let mut acc = Vec::new();
        let mut seen = BTreeSet::new();
        self.collect_vars(&mut acc, &mut seen);
        acc
    }

    fn collect_vars(&self, acc: &mut Vec<Kid>, seen: &mut BTreeSet<Kid>) {
        match self {
            NConstraint::True | NConstraint::False | NConstraint::Id(_) => {}
            NConstraint::Var(k) => {
                if seen.insert(*k) {
                    acc.push(*k);
                }
            }
            NConstraint::Equal(a, b) | NConstraint::NotEqual(a, b) => {
                a.collect_vars(acc, seen);
                b.collect_vars(acc, seen);
            }
            NConstraint::Ge(a, b)
            | NConstraint::Gt(a, b)
            | NConstraint::Le(a, b)
            | NConstraint::Lt(a, b) => {
                a.collect_vars(acc, seen);
                b.collect_vars(acc, seen);
            }
            NConstraint::Set(n, _) => n.collect_vars(acc, seen),
            NConstraint::App(_, args) => {
                for a in args {
                    a.collect_vars(acc, seen);
                }
            }
            NConstraint::And(a, b) | NConstraint::Or(a, b) => {
                a.collect_vars(acc, seen);
                b.collect_vars(acc, seen);
            }
            NConstraint::Not(c) => c.collect_vars(acc, seen),
        }
    }

    /// Substitute kind variables according to `subst`.
    ///
    /// Unchanged subtrees are shared, not rebuilt: use
    /// [`NConstraint::subst_cow`] directly when the caller can keep a borrow.
    pub fn subst(&self, subst: &BTreeMap<Kid, NExp>) -> NConstraint {
        self.subst_cow(subst).into_owned()
    }

    /// Substitute kind variables, borrowing `self` wherever no variable
    /// below the node is hit by `subst` (see [`NExp::subst_cow`]).
    ///
    /// A substituted boolean variable mapping to a constant still folds
    /// to `True`/`False` exactly like [`NConstraint::subst`] did.
    pub fn subst_cow(&self, subst: &BTreeMap<Kid, NExp>) -> Cow<'_, NConstraint> {
        match self {
            NConstraint::True | NConstraint::False | NConstraint::Id(_) => Cow::Borrowed(self),
            NConstraint::Var(k) => match subst.get(k) {
                Some(NExp::Constant(0)) => Cow::Owned(NConstraint::False),
                Some(NExp::Constant(_)) => Cow::Owned(NConstraint::True),
                _ => Cow::Borrowed(self),
            },
            NConstraint::Equal(a, b) => {
                subst_typarg_pair(subst, self, a, b, NConstraint::Equal)
            }
            NConstraint::NotEqual(a, b) => {
                subst_typarg_pair(subst, self, a, b, NConstraint::NotEqual)
            }
            NConstraint::Ge(a, b) => subst_constraint_binary(subst, self, a, b, NConstraint::Ge),
            NConstraint::Gt(a, b) => subst_constraint_binary(subst, self, a, b, NConstraint::Gt),
            NConstraint::Le(a, b) => subst_constraint_binary(subst, self, a, b, NConstraint::Le),
            NConstraint::Lt(a, b) => subst_constraint_binary(subst, self, a, b, NConstraint::Lt),
            NConstraint::Set(n, vs) => {
                let rewritten = n.subst_cow(subst);
                if matches!(rewritten, Cow::Borrowed(_)) {
                    // Unchanged: also spares the `vs.clone()` the old
                    // `subst` did unconditionally.
                    Cow::Borrowed(self)
                } else {
                    Cow::Owned(NConstraint::Set(
                        Box::new(rewritten.into_owned()),
                        vs.clone(),
                    ))
                }
            }
            NConstraint::App(f, args) => {
                let rewritten: Vec<Cow<'_, TypArg>> =
                    args.iter().map(|a| a.subst_cow(subst)).collect();
                if rewritten.iter().any(|c| matches!(c, Cow::Owned(_))) {
                    Cow::Owned(NConstraint::App(
                        f.clone(),
                        rewritten.into_iter().map(Cow::into_owned).collect(),
                    ))
                } else {
                    Cow::Borrowed(self)
                }
            }
            NConstraint::And(a, b) => {
                subst_constraint_pair(subst, self, a, b, NConstraint::And)
            }
            NConstraint::Or(a, b) => {
                subst_constraint_pair(subst, self, a, b, NConstraint::Or)
            }
            NConstraint::Not(c) => {
                let rewritten = c.subst_cow(subst);
                if matches!(rewritten, Cow::Borrowed(_)) {
                    Cow::Borrowed(self)
                } else {
                    Cow::Owned(NConstraint::Not(Box::new(rewritten.into_owned())))
                }
            }
        }
    }

    /// Total valuation under `env`; `None` if any sub-expression is
    /// unrepresentable (see [`NExp::eval`]).
    ///
    /// `Equal`/`NotEqual` compare `typ_arg` sorts (see [`typarg_eq`]);
    /// `App` stays uninterpreted (`None`, like [`NExp::Id`]).
    pub fn eval(&self, env: &BTreeMap<Kid, i128>) -> Option<bool> {
        match self {
            NConstraint::True => Some(true),
            NConstraint::False => Some(false),
            NConstraint::Id(_) => None,
            NConstraint::Var(_) => None,
            NConstraint::Equal(a, b) => typarg_eq(a, b, env),
            NConstraint::NotEqual(a, b) => typarg_eq(a, b, env).map(|v| !v),
            NConstraint::Ge(a, b) => Some(a.eval(env)? >= b.eval(env)?),
            NConstraint::Gt(a, b) => Some(a.eval(env)? > b.eval(env)?),
            NConstraint::Le(a, b) => Some(a.eval(env)? <= b.eval(env)?),
            NConstraint::Lt(a, b) => Some(a.eval(env)? < b.eval(env)?),
            NConstraint::Set(n, vs) => Some(vs.contains(&n.eval(env)?)),
            // Uninterpreted predicate: no valuation (conservative `None`,
            // like [`NExp::Id`]).
            NConstraint::App(_, _) => None,
            NConstraint::And(a, b) => Some(a.eval(env)? && b.eval(env)?),
            NConstraint::Or(a, b) => Some(a.eval(env)? || b.eval(env)?),
            NConstraint::Not(c) => Some(!c.eval(env)?),
        }
    }
}

/// Equality valuation over `typ_arg` sorts under `env`.
///
/// - Syntactically equal arguments are equal (`Some(true)`); this is the
///   valuation twin of the `a == b -> True` simplification rule and holds
///   under every total environment.
/// - `(NExp, NExp)` and `(Bool, Bool)` pairs compare by recursive
///   valuation.
/// - Mixed sorts are never equal (`Some(false)`): different `typ_arg`
///   constructors cannot denote the same value.
/// - Syntactically different types stay conservative (`None`): type
///   identity has no numeric valuation.
fn typarg_eq(a: &TypArg, b: &TypArg, env: &BTreeMap<Kid, i128>) -> Option<bool> {
    if a == b {
        return Some(true);
    }
    match (a, b) {
        (TypArg::NExp(x), TypArg::NExp(y)) => Some(x.eval(env)? == y.eval(env)?),
        (TypArg::Bool(x), TypArg::Bool(y)) => Some(x.eval(env)? == y.eval(env)?),
        (TypArg::Typ(_), TypArg::Typ(_)) => None,
        _ => Some(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Interner as _, SimpleInterner};

    fn env(ix: &mut SimpleInterner, pairs: &[(&str, i128)]) -> BTreeMap<Kid, i128> {
        pairs.iter().map(|(k, v)| (ix.intern(k), *v)).collect()
    }

    #[test]
    fn eval_arith_identities() {
        let mut ix = SimpleInterner::new();
        let n = ix.intern("n");
        let m = ix.intern("m");
        let e = env(&mut ix, &[("n", 6), ("m", 4)]);
        let sum = NExp::Sum(Box::new(NExp::var(n)), Box::new(NExp::var(m)));
        assert_eq!(sum.eval(&e), Some(10));
        assert_eq!(NExp::Exp(Box::new(NExp::constant(3))).eval(&e), Some(8));
        assert_eq!(NExp::div(NExp::var(n), NExp::constant(2)).eval(&e), Some(3));
        assert_eq!(NExp::modulo(NExp::var(n), NExp::var(m)).eval(&e), Some(2));
        assert_eq!(NExp::abs(NExp::constant(-5)).eval(&e), Some(5));
    }

    #[test]
    fn eval_total_on_partial_input() {
        let mut ix = SimpleInterner::new();
        let n = ix.intern("n");
        let e = env(&mut ix, &[]);
        assert_eq!(NExp::var(n).eval(&e), None);
        assert_eq!(
            NExp::div(NExp::constant(1), NExp::constant(0)).eval(&e),
            None
        );
        assert_eq!(NExp::Exp(Box::new(NExp::constant(-1))).eval(&e), None);
        assert_eq!(NExp::Id("f".to_string()).eval(&e), None);
    }

    #[test]
    fn subst_and_vars() {
        let mut ix = SimpleInterner::new();
        let n = ix.intern("n");
        let e = NExp::Sum(Box::new(NExp::var(n)), Box::new(NExp::var(n)));
        assert_eq!(e.vars(), vec![n]);
        let mut s = BTreeMap::new();
        s.insert(n, NExp::constant(2));
        assert_eq!(
            e.subst(&s),
            NExp::Sum(Box::new(NExp::constant(2)), Box::new(NExp::constant(2)))
        );
    }

    #[test]
    fn subst_cow_borrows_fully_unchanged_tree() {
        let mut ix = SimpleInterner::new();
        let n = ix.intern("n");
        let m = ix.intern("m");
        let e = NExp::Sum(Box::new(NExp::var(n)), Box::new(NExp::constant(1)));
        let empty: BTreeMap<Kid, NExp> = BTreeMap::new();
        assert!(matches!(e.subst_cow(&empty), Cow::Borrowed(_)));
        // A miss in a non-empty map still borrows (no clone, no re-box).
        let mut s = BTreeMap::new();
        s.insert(m, NExp::constant(2));
        assert!(matches!(e.subst_cow(&s), Cow::Borrowed(_)));
    }

    #[test]
    fn subst_cow_rebuilds_only_on_hit_and_matches_subst() {
        let mut ix = SimpleInterner::new();
        let n = ix.intern("n");
        let e = NExp::Sum(Box::new(NExp::var(n)), Box::new(NExp::constant(1)));
        let mut s = BTreeMap::new();
        s.insert(n, NExp::constant(2));
        let cow = e.subst_cow(&s);
        assert!(matches!(cow, Cow::Owned(_)));
        assert_eq!(cow.into_owned(), e.subst(&s));
        assert_eq!(
            e.subst(&s),
            NExp::Sum(Box::new(NExp::constant(2)), Box::new(NExp::constant(1)))
        );
    }

    #[test]
    fn subst_cow_constraint_keeps_bool_var_folding() {
        let mut ix = SimpleInterner::new();
        let b = ix.intern("b");
        let n = ix.intern("n");
        let v = NConstraint::Var(b);
        let mut s = BTreeMap::new();
        s.insert(b, NExp::constant(0));
        assert_eq!(v.subst_cow(&s).into_owned(), NConstraint::False);
        s.insert(b, NExp::constant(7));
        assert_eq!(v.subst_cow(&s).into_owned(), NConstraint::True);
        // Non-constant mapping leaves the var borrowed (unchanged, as before).
        s.insert(b, NExp::var(n));
        assert!(matches!(v.subst_cow(&s), Cow::Borrowed(_)));
        assert_eq!(v.subst(&s), NConstraint::Var(b));
        // Unchanged set-constraint borrows (old `subst` cloned `vs` always).
        let set = NConstraint::Set(Box::new(NExp::var(n)), vec![1, 2, 3]);
        let empty: BTreeMap<Kid, NExp> = BTreeMap::new();
        assert!(matches!(set.subst_cow(&empty), Cow::Borrowed(_)));
        assert_eq!(set.subst(&empty), set);
    }

    #[test]
    fn constraint_eval() {
        let mut ix = SimpleInterner::new();
        let n = ix.intern("n");
        let e = env(&mut ix, &[("n", 3)]);
        let c = NConstraint::Ge(Box::new(NExp::var(n)), Box::new(NExp::constant(0)));
        assert_eq!(c.eval(&e), Some(true));
        let set = NConstraint::Set(Box::new(NExp::var(n)), vec![1, 2, 3]);
        assert_eq!(set.eval(&e), Some(true));
    }

    #[test]
    fn kid_symbols_share_one_table_entry() {
        // Lane 71: interning the same name twice yields one symbol, and the
        // `vars` order contract (first occurrence) is unchanged.
        let mut ix = SimpleInterner::new();
        let n = ix.intern("n");
        assert_eq!(ix.intern("n"), n);
        assert_eq!(ix.len(), 1);
        let e = NExp::Sum(Box::new(NExp::var(n)), Box::new(NExp::var(n)));
        assert_eq!(e.vars(), vec![n]);
        assert_eq!(ix.resolve(n), Some("n"));
    }
}
