#![forbid(unsafe_code)]
//! Numeric type-level expressions of Sail (`Nexp_*` in
//! `sail/src/lib/extraction/Ast.ml`).
//!
//! Constructors mirror the OCaml AST; `div`, `mod` and `abs` are
//! `Nexp_app` nodes there, exposed here via [`NExp::div`],
//! [`NExp::modulo`] and [`NExp::abs`] helpers.

use std::collections::{BTreeMap, BTreeSet};

/// Numeric kind variable (`kid` in the Sail AST), e.g. `"n"`.
pub type Kid = String;

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

/// Numeric constraint (`n_constraint` in the Sail AST).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NConstraint {
    True,
    False,
    /// `NC_id i`.
    Id(String),
    /// Boolean kind variable used as a constraint.
    Var(Kid),
    Equal(Box<NExp>, Box<NExp>),
    NotEqual(Box<NExp>, Box<NExp>),
    Ge(Box<NExp>, Box<NExp>),
    Gt(Box<NExp>, Box<NExp>),
    Le(Box<NExp>, Box<NExp>),
    Lt(Box<NExp>, Box<NExp>),
    /// `NC_set (n, vs)` — `n` is one of `vs`.
    Set(Box<NExp>, Vec<i128>),
    And(Box<NConstraint>, Box<NConstraint>),
    Or(Box<NConstraint>, Box<NConstraint>),
    Not(Box<NConstraint>),
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
    pub fn var(name: impl Into<String>) -> NExp {
        NExp::Var(name.into())
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
                if seen.insert(k.clone()) {
                    acc.push(k.clone());
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
    pub fn subst(&self, subst: &BTreeMap<Kid, NExp>) -> NExp {
        match self {
            NExp::Id(_) | NExp::Constant(_) => self.clone(),
            NExp::Var(k) => subst.get(k).cloned().unwrap_or_else(|| self.clone()),
            NExp::App(f, args) => {
                NExp::App(f.clone(), args.iter().map(|a| a.subst(subst)).collect())
            }
            NExp::If(c, t, e) => NExp::If(
                Box::new(c.subst(subst)),
                Box::new(t.subst(subst)),
                Box::new(e.subst(subst)),
            ),
            NExp::Times(a, b) => NExp::Times(Box::new(a.subst(subst)), Box::new(b.subst(subst))),
            NExp::Sum(a, b) => NExp::Sum(Box::new(a.subst(subst)), Box::new(b.subst(subst))),
            NExp::Minus(a, b) => NExp::Minus(Box::new(a.subst(subst)), Box::new(b.subst(subst))),
            NExp::Exp(x) => NExp::Exp(Box::new(x.subst(subst))),
            NExp::Neg(x) => NExp::Neg(Box::new(x.subst(subst))),
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
                if seen.insert(k.clone()) {
                    acc.push(k.clone());
                }
            }
            NConstraint::Equal(a, b)
            | NConstraint::NotEqual(a, b)
            | NConstraint::Ge(a, b)
            | NConstraint::Gt(a, b)
            | NConstraint::Le(a, b)
            | NConstraint::Lt(a, b) => {
                a.collect_vars(acc, seen);
                b.collect_vars(acc, seen);
            }
            NConstraint::Set(n, _) => n.collect_vars(acc, seen),
            NConstraint::And(a, b) | NConstraint::Or(a, b) => {
                a.collect_vars(acc, seen);
                b.collect_vars(acc, seen);
            }
            NConstraint::Not(c) => c.collect_vars(acc, seen),
        }
    }

    /// Substitute kind variables according to `subst`.
    pub fn subst(&self, subst: &BTreeMap<Kid, NExp>) -> NConstraint {
        match self {
            NConstraint::True | NConstraint::False | NConstraint::Id(_) => self.clone(),
            NConstraint::Var(k) => self.clone().and_mark(k, subst),
            NConstraint::Equal(a, b) => {
                NConstraint::Equal(Box::new(a.subst(subst)), Box::new(b.subst(subst)))
            }
            NConstraint::NotEqual(a, b) => {
                NConstraint::NotEqual(Box::new(a.subst(subst)), Box::new(b.subst(subst)))
            }
            NConstraint::Ge(a, b) => {
                NConstraint::Ge(Box::new(a.subst(subst)), Box::new(b.subst(subst)))
            }
            NConstraint::Gt(a, b) => {
                NConstraint::Gt(Box::new(a.subst(subst)), Box::new(b.subst(subst)))
            }
            NConstraint::Le(a, b) => {
                NConstraint::Le(Box::new(a.subst(subst)), Box::new(b.subst(subst)))
            }
            NConstraint::Lt(a, b) => {
                NConstraint::Lt(Box::new(a.subst(subst)), Box::new(b.subst(subst)))
            }
            NConstraint::Set(n, vs) => NConstraint::Set(Box::new(n.subst(subst)), vs.clone()),
            NConstraint::And(a, b) => {
                NConstraint::And(Box::new(a.subst(subst)), Box::new(b.subst(subst)))
            }
            NConstraint::Or(a, b) => {
                NConstraint::Or(Box::new(a.subst(subst)), Box::new(b.subst(subst)))
            }
            NConstraint::Not(c) => NConstraint::Not(Box::new(c.subst(subst))),
        }
    }

    /// Helper: a substituted boolean variable that maps to a constant
    /// becomes `True`/`False` (non-zero is true).
    fn and_mark(self, k: &Kid, subst: &BTreeMap<Kid, NExp>) -> NConstraint {
        match subst.get(k) {
            Some(NExp::Constant(0)) => NConstraint::False,
            Some(NExp::Constant(_)) => NConstraint::True,
            _ => self,
        }
    }

    /// Total valuation under `env`; `None` if any sub-expression is
    /// unrepresentable (see [`NExp::eval`]).
    pub fn eval(&self, env: &BTreeMap<Kid, i128>) -> Option<bool> {
        match self {
            NConstraint::True => Some(true),
            NConstraint::False => Some(false),
            NConstraint::Id(_) => None,
            NConstraint::Var(_) => None,
            NConstraint::Equal(a, b) => Some(a.eval(env)? == b.eval(env)?),
            NConstraint::NotEqual(a, b) => Some(a.eval(env)? != b.eval(env)?),
            NConstraint::Ge(a, b) => Some(a.eval(env)? >= b.eval(env)?),
            NConstraint::Gt(a, b) => Some(a.eval(env)? > b.eval(env)?),
            NConstraint::Le(a, b) => Some(a.eval(env)? <= b.eval(env)?),
            NConstraint::Lt(a, b) => Some(a.eval(env)? < b.eval(env)?),
            NConstraint::Set(n, vs) => Some(vs.contains(&n.eval(env)?)),
            NConstraint::And(a, b) => Some(a.eval(env)? && b.eval(env)?),
            NConstraint::Or(a, b) => Some(a.eval(env)? || b.eval(env)?),
            NConstraint::Not(c) => Some(!c.eval(env)?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, i128)]) -> BTreeMap<Kid, i128> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn eval_arith_identities() {
        let e = env(&[("n", 6), ("m", 4)]);
        let sum = NExp::Sum(Box::new(NExp::var("n")), Box::new(NExp::var("m")));
        assert_eq!(sum.eval(&e), Some(10));
        assert_eq!(NExp::Exp(Box::new(NExp::constant(3))).eval(&e), Some(8));
        assert_eq!(
            NExp::div(NExp::var("n"), NExp::constant(2)).eval(&e),
            Some(3)
        );
        assert_eq!(
            NExp::modulo(NExp::var("n"), NExp::var("m")).eval(&e),
            Some(2)
        );
        assert_eq!(NExp::abs(NExp::constant(-5)).eval(&e), Some(5));
    }

    #[test]
    fn eval_total_on_partial_input() {
        let e = env(&[]);
        assert_eq!(NExp::var("n").eval(&e), None);
        assert_eq!(
            NExp::div(NExp::constant(1), NExp::constant(0)).eval(&e),
            None
        );
        assert_eq!(NExp::Exp(Box::new(NExp::constant(-1))).eval(&e), None);
        assert_eq!(NExp::Id("f".to_string()).eval(&e), None);
    }

    #[test]
    fn subst_and_vars() {
        let e = NExp::Sum(Box::new(NExp::var("n")), Box::new(NExp::var("n")));
        assert_eq!(e.vars(), vec!["n".to_string()]);
        let mut s = BTreeMap::new();
        s.insert("n".to_string(), NExp::constant(2));
        assert_eq!(
            e.subst(&s),
            NExp::Sum(Box::new(NExp::constant(2)), Box::new(NExp::constant(2)))
        );
    }

    #[test]
    fn constraint_eval() {
        let e = env(&[("n", 3)]);
        let c = NConstraint::Ge(Box::new(NExp::var("n")), Box::new(NExp::constant(0)));
        assert_eq!(c.eval(&e), Some(true));
        let set = NConstraint::Set(Box::new(NExp::var("n")), vec![1, 2, 3]);
        assert_eq!(set.eval(&e), Some(true));
    }
}
