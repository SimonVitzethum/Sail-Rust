#![forbid(unsafe_code)]
//! Simplification and canonical normal forms for [`NExp`]
//! and [`NConstraint`].
//!
//! All functions are pure and total. [`simplify`] applies one rewrite
//! pass (constant folding plus arithmetic identities);
//! [`normalize`] iterates it to a fixpoint (bounded) and sorts the
//! operands of associative-commutative `Sum`/`Times` for a canonical
//! syntactic form.
//!
//! Semantics contract: simplification never changes a *defined* value —
//! if `e.eval(env) == Some(v)` then `simplify(e).eval(env) == Some(v)`
//! (and likewise for [`normalize`]). Rules such as `n * 0 -> 0` may turn
//! an *undefined* valuation (`None` from an unbound variable) into a
//! defined one; under total environments evaluation is preserved
//! exactly.

use crate::expr::{NConstraint, NExp};

/// One rewrite pass over an [`NExp`].
pub fn simplify(e: &NExp) -> NExp {
    match e {
        NExp::Id(_) | NExp::Var(_) | NExp::Constant(_) => e.clone(),
        NExp::App(f, args) => simplify_app(f, args),
        NExp::If(c, t, b) => {
            let c = simplify_constraint(c);
            let t = simplify(t);
            let b = simplify(b);
            match c {
                NConstraint::True => t,
                NConstraint::False => b,
                _ if t == b => t,
                _ => NExp::If(Box::new(c), Box::new(t), Box::new(b)),
            }
        }
        NExp::Times(a, b) => {
            let (a, b) = (simplify(a), simplify(b));
            times(a, b)
        }
        NExp::Sum(a, b) => {
            let (a, b) = (simplify(a), simplify(b));
            sum(a, b)
        }
        NExp::Minus(a, b) => {
            let (a, b) = (simplify(a), simplify(b));
            minus(a, b)
        }
        NExp::Exp(x) => {
            let x = simplify(x);
            match x {
                NExp::Constant(0) => NExp::Constant(1),
                NExp::Constant(e) if e < 0 => NExp::Exp(Box::new(NExp::Constant(e))),
                NExp::Constant(e) if e < 127 => NExp::Constant(1i128 << (e as u32)),
                _ => NExp::Exp(Box::new(x)),
            }
        }
        NExp::Neg(x) => {
            let x = simplify(x);
            neg(x)
        }
    }
}

fn num(e: &NExp) -> Option<i128> {
    match e {
        NExp::Constant(v) => Some(*v),
        _ => None,
    }
}

fn is_zero(e: &NExp) -> bool {
    num(e) == Some(0)
}

fn is_one(e: &NExp) -> bool {
    num(e) == Some(1)
}

fn sum(a: NExp, b: NExp) -> NExp {
    match (a, b) {
        (NExp::Constant(x), NExp::Constant(y)) => match x.checked_add(y) {
            Some(v) => NExp::Constant(v),
            None => NExp::Sum(Box::new(NExp::Constant(x)), Box::new(NExp::Constant(y))),
        },
        (x, y) if is_zero(&x) => y,
        (x, y) if is_zero(&y) => x,
        // -(a) + -(b) -> -(a + b) (kept as one negation for canonicity:
        // both operand orders reach the same term).
        (NExp::Neg(x), NExp::Neg(y)) => neg(sum(*x, *y)),
        // x + (-y) -> x - y, in either operand order.
        (x, NExp::Neg(y)) => minus(x, *y),
        (NExp::Neg(x), y) => minus(y, *x),
        (x, y) => NExp::Sum(Box::new(x), Box::new(y)),
    }
}

fn minus(a: NExp, b: NExp) -> NExp {
    if a == b {
        return NExp::Constant(0);
    }
    match (a, b) {
        (NExp::Constant(x), NExp::Constant(y)) => match x.checked_sub(y) {
            Some(v) => NExp::Constant(v),
            None => NExp::Minus(Box::new(NExp::Constant(x)), Box::new(NExp::Constant(y))),
        },
        (x, y) if is_zero(&y) => x,
        (x, y) if is_zero(&x) => neg(y),
        // (x + c1) - c2 -> x + (c1 - c2)
        (NExp::Sum(x, c1), NExp::Constant(c2)) if matches!(*c1, NExp::Constant(_)) => {
            let NExp::Constant(c1v) = *c1 else {
                unreachable!()
            };
            match c1v.checked_sub(c2) {
                Some(0) => *x,
                Some(v) => sum(*x, NExp::Constant(v)),
                None => NExp::Minus(
                    Box::new(NExp::Sum(Box::new(*x), Box::new(NExp::Constant(c1v)))),
                    Box::new(NExp::Constant(c2)),
                ),
            }
        }
        (x, y) => NExp::Minus(Box::new(x), Box::new(y)),
    }
}

fn times(a: NExp, b: NExp) -> NExp {
    match (a, b) {
        (NExp::Constant(x), NExp::Constant(y)) => match x.checked_mul(y) {
            Some(v) => NExp::Constant(v),
            None => NExp::Times(Box::new(NExp::Constant(x)), Box::new(NExp::Constant(y))),
        },
        (x, y) if is_zero(&x) || is_zero(&y) => NExp::Constant(0),
        (x, y) if is_one(&x) => y,
        (x, y) if is_one(&y) => x,
        // (-1) * x -> -x
        (NExp::Constant(-1), x) | (x, NExp::Constant(-1)) => neg(x),
        // (-a) * (-b) -> a * b
        (NExp::Neg(a), NExp::Neg(b)) => times(*a, *b),
        (x, y) => NExp::Times(Box::new(x), Box::new(y)),
    }
}

fn neg(x: NExp) -> NExp {
    match x {
        NExp::Constant(v) => match v.checked_neg() {
            Some(w) => NExp::Constant(w),
            None => NExp::Neg(Box::new(NExp::Constant(v))),
        },
        NExp::Neg(y) => *y,
        // -(a - b) -> b - a
        NExp::Minus(a, b) => minus(*b, *a),
        y => NExp::Neg(Box::new(y)),
    }
}

fn simplify_app(f: &str, args: &[NExp]) -> NExp {
    let args: Vec<NExp> = args.iter().map(simplify).collect();
    match (f, args.as_slice()) {
        ("div", [a, b]) => match (num(a), num(b)) {
            (_, Some(0)) => NExp::div(a.clone(), b.clone()),
            (Some(x), Some(y)) => match x.checked_div(y) {
                Some(v) => NExp::Constant(v),
                None => NExp::div(NExp::Constant(x), NExp::Constant(y)),
            },
            _ if is_one(b) => a.clone(),
            // `0 div d -> 0` only for known non-zero `d` (`0 div 0`
            // is undefined and must stay unfolded).
            _ if is_zero(a) && matches!(num(b), Some(d) if d != 0) => NExp::Constant(0),
            _ => NExp::div(a.clone(), b.clone()),
        },
        ("mod", [a, b]) => match (num(a), num(b)) {
            (_, Some(0)) => NExp::modulo(a.clone(), b.clone()),
            (Some(x), Some(y)) => match x.checked_rem(y) {
                Some(v) => NExp::Constant(v),
                None => NExp::modulo(NExp::Constant(x), NExp::Constant(y)),
            },
            _ if is_one(b) => NExp::Constant(0),
            _ if is_zero(a) && matches!(num(b), Some(d) if d != 0) => NExp::Constant(0),
            _ => NExp::modulo(a.clone(), b.clone()),
        },
        ("abs", [a]) => match num(a) {
            Some(v) => match v.checked_abs() {
                Some(w) => NExp::Constant(w),
                None => NExp::abs(NExp::Constant(v)),
            },
            None => match a {
                // abs is idempotent on applications of abs and on 2^x.
                NExp::App(g, inner) if g == "abs" => {
                    NExp::abs(inner.first().cloned().unwrap_or(NExp::Constant(0)))
                }
                NExp::Exp(_) => a.clone(),
                NExp::Constant(_) => unreachable!(),
                _ => NExp::abs(a.clone()),
            },
        },
        _ => NExp::App(f.to_string(), args),
    }
}

/// One rewrite pass over an [`NConstraint`].
pub fn simplify_constraint(c: &NConstraint) -> NConstraint {
    match c {
        NConstraint::True | NConstraint::False | NConstraint::Id(_) | NConstraint::Var(_) => {
            c.clone()
        }
        NConstraint::Equal(a, b) => {
            let (a, b) = (simplify(a), simplify(b));
            if a == b {
                NConstraint::True
            } else if let (Some(x), Some(y)) = (num(&a), num(&b)) {
                if x == y {
                    NConstraint::True
                } else {
                    NConstraint::False
                }
            } else {
                NConstraint::Equal(Box::new(a), Box::new(b))
            }
        }
        NConstraint::NotEqual(a, b) => {
            let (a, b) = (simplify(a), simplify(b));
            if a == b {
                NConstraint::False
            } else if let (Some(x), Some(y)) = (num(&a), num(&b)) {
                if x != y {
                    NConstraint::True
                } else {
                    NConstraint::False
                }
            } else {
                NConstraint::NotEqual(Box::new(a), Box::new(b))
            }
        }
        NConstraint::Ge(a, b) => simplify_cmp(a, b, |x, y| x >= y, NConstraint::Ge),
        NConstraint::Gt(a, b) => simplify_cmp(a, b, |x, y| x > y, NConstraint::Gt),
        NConstraint::Le(a, b) => simplify_cmp(a, b, |x, y| x <= y, NConstraint::Le),
        NConstraint::Lt(a, b) => simplify_cmp(a, b, |x, y| x < y, NConstraint::Lt),
        NConstraint::Set(n, vs) => {
            let n = simplify(n);
            if let Some(v) = num(&n) {
                if vs.contains(&v) {
                    NConstraint::True
                } else {
                    NConstraint::False
                }
            } else if vs.is_empty() {
                NConstraint::False
            } else {
                let mut sorted = vs.clone();
                sorted.sort();
                sorted.dedup();
                NConstraint::Set(Box::new(n), sorted)
            }
        }
        NConstraint::And(a, b) => {
            let (a, b) = (simplify_constraint(a), simplify_constraint(b));
            match (a, b) {
                (NConstraint::False, _) | (_, NConstraint::False) => NConstraint::False,
                (NConstraint::True, x) | (x, NConstraint::True) => x,
                (x, y) if x == y => x,
                (x, y) => NConstraint::And(Box::new(x), Box::new(y)),
            }
        }
        NConstraint::Or(a, b) => {
            let (a, b) = (simplify_constraint(a), simplify_constraint(b));
            match (a, b) {
                (NConstraint::True, _) | (_, NConstraint::True) => NConstraint::True,
                (NConstraint::False, x) | (x, NConstraint::False) => x,
                (x, y) if x == y => x,
                (x, y) => NConstraint::Or(Box::new(x), Box::new(y)),
            }
        }
        NConstraint::Not(x) => {
            let x = simplify_constraint(x);
            match x {
                NConstraint::True => NConstraint::False,
                NConstraint::False => NConstraint::True,
                NConstraint::Not(y) => *y,
                y => NConstraint::Not(Box::new(y)),
            }
        }
    }
}

fn simplify_cmp(
    a: &NExp,
    b: &NExp,
    decide: impl Fn(i128, i128) -> bool,
    rebuild: fn(Box<NExp>, Box<NExp>) -> NConstraint,
) -> NConstraint {
    let (a, b) = (simplify(a), simplify(b));
    if a == b {
        // a `cmp` a is constant for every comparison operator.
        let one = NExp::Constant(1);
        let zero = NExp::Constant(0);
        return if decide(1, 1) && decide(0, 0) {
            NConstraint::True
        } else {
            let _ = (one, zero);
            NConstraint::False
        };
    }
    if let (Some(x), Some(y)) = (num(&a), num(&b)) {
        if decide(x, y) {
            NConstraint::True
        } else {
            NConstraint::False
        }
    } else {
        rebuild(Box::new(a), Box::new(b))
    }
}

/// Iterate [`simplify`] to a fixpoint (at most 64 rounds) and sort the
/// operands of `Sum`/`Times` into a canonical order.
pub fn normalize(e: &NExp) -> NExp {
    let mut cur = e.clone();
    for _ in 0..64 {
        let next = canonical(&simplify(&cur));
        if next == cur {
            return next;
        }
        cur = next;
    }
    cur
}

/// Iterate [`simplify_constraint`] to a fixpoint (at most 64 rounds).
pub fn normalize_constraint(c: &NConstraint) -> NConstraint {
    let mut cur = c.clone();
    for _ in 0..64 {
        let next = simplify_constraint(&cur);
        if next == cur {
            return next;
        }
        cur = next;
    }
    cur
}

/// Flatten nested `Sum`/`Times` and sort operands canonically.
fn canonical(e: &NExp) -> NExp {
    match e {
        NExp::Sum(_, _) => {
            let mut terms = Vec::new();
            flatten_sum(e, &mut terms);
            let mut terms: Vec<NExp> = terms.into_iter().map(|t| canonical(&t)).collect();
            terms.sort();
            rebuild_sum(terms)
        }
        NExp::Times(_, _) => {
            let mut factors = Vec::new();
            flatten_times(e, &mut factors);
            let mut factors: Vec<NExp> = factors.into_iter().map(|f| canonical(&f)).collect();
            factors.sort();
            rebuild_times(factors)
        }
        NExp::Minus(a, b) => NExp::Minus(Box::new(canonical(a)), Box::new(canonical(b))),
        NExp::Neg(x) => NExp::Neg(Box::new(canonical(x))),
        NExp::Exp(x) => NExp::Exp(Box::new(canonical(x))),
        NExp::If(c, t, b) => NExp::If(
            Box::new(simplify_constraint(c)),
            Box::new(canonical(t)),
            Box::new(canonical(b)),
        ),
        NExp::App(f, args) => NExp::App(f.clone(), args.iter().map(canonical).collect()),
        _ => e.clone(),
    }
}

fn flatten_sum(e: &NExp, out: &mut Vec<NExp>) {
    match e {
        NExp::Sum(a, b) => {
            flatten_sum(a, out);
            flatten_sum(b, out);
        }
        _ => out.push(e.clone()),
    }
}

fn flatten_times(e: &NExp, out: &mut Vec<NExp>) {
    match e {
        NExp::Times(a, b) => {
            flatten_times(a, out);
            flatten_times(b, out);
        }
        _ => out.push(e.clone()),
    }
}

fn rebuild_sum(mut terms: Vec<NExp>) -> NExp {
    debug_assert!(!terms.is_empty());
    let mut acc = terms.remove(0);
    for t in terms {
        acc = NExp::Sum(Box::new(acc), Box::new(t));
    }
    acc
}

fn rebuild_times(mut factors: Vec<NExp>) -> NExp {
    debug_assert!(!factors.is_empty());
    let mut acc = factors.remove(0);
    for f in factors {
        acc = NExp::Times(Box::new(acc), Box::new(f));
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::{Kid, NExp::*};
    use sail_ast::{Interner as _, SimpleInterner};
    use std::collections::BTreeMap;

    fn v(k: Kid) -> NExp {
        NExp::Var(k)
    }
    fn c(i: i128) -> NExp {
        NExp::Constant(i)
    }
    /// Fresh table with `n`/`m`/`a`/`b` pre-interned (tuple order).
    /// The table itself is dropped: [`Kid`]s are plain `Copy` handles.
    fn kids() -> (Kid, Kid, Kid, Kid) {
        let mut ix = SimpleInterner::new();
        (
            ix.intern("n"),
            ix.intern("m"),
            ix.intern("a"),
            ix.intern("b"),
        )
    }

    #[test]
    fn folds_constants() {
        let e = Sum(Box::new(c(2)), Box::new(c(3)));
        assert_eq!(simplify(&e), c(5));
        assert_eq!(simplify(&NExp::div(c(7), c(2))), c(3));
        assert_eq!(simplify(&NExp::abs(c(-4))), c(4));
        assert_eq!(simplify(&NExp::Exp(Box::new(c(4)))), c(16));
    }

    #[test]
    fn identities() {
        let (n, _m, _a, _b) = kids();
        assert_eq!(simplify(&Sum(Box::new(v(n)), Box::new(c(0)))), v(n));
        assert_eq!(simplify(&Times(Box::new(v(n)), Box::new(c(1)))), v(n));
        assert_eq!(simplify(&Times(Box::new(v(n)), Box::new(c(0)))), c(0));
        assert_eq!(simplify(&Minus(Box::new(v(n)), Box::new(v(n)))), c(0));
        assert_eq!(simplify(&Neg(Box::new(Neg(Box::new(v(n)))))), v(n));
    }

    #[test]
    fn if_true_false_branches() {
        let (n, _m, _a, b) = kids();
        let t = NExp::If(Box::new(NConstraint::True), Box::new(c(1)), Box::new(c(2)));
        assert_eq!(simplify(&t), c(1));
        let same = NExp::If(
            Box::new(NConstraint::Var(b)),
            Box::new(v(n)),
            Box::new(v(n)),
        );
        assert_eq!(simplify(&same), v(n));
    }

    #[test]
    fn negated_sum_is_order_independent() {
        let (_n, _m, a, b) = kids();
        let neg_a = Neg(Box::new(v(a)));
        let neg_b = Neg(Box::new(NExp::abs(v(b))));
        let ab = Sum(Box::new(neg_a.clone()), Box::new(neg_b.clone()));
        let ba = Sum(Box::new(neg_b), Box::new(neg_a));
        // `simplify` alone does not sort; `normalize` must agree.
        assert_eq!(normalize(&ab), normalize(&ba));
        // Mixed negated / plain operands agree as well.
        let an = Sum(Box::new(Neg(Box::new(v(a)))), Box::new(v(b)));
        let na = Sum(Box::new(v(b)), Box::new(Neg(Box::new(v(a)))));
        assert_eq!(simplify(&an), simplify(&na));
        assert_eq!(normalize(&an), normalize(&na));
    }

    #[test]
    fn normalize_is_canonical_for_ac() {
        let (_n, _m, a, b) = kids();
        let x = Sum(
            Box::new(v(b)),
            Box::new(Sum(Box::new(v(a)), Box::new(c(1)))),
        );
        let y = Sum(
            Box::new(c(1)),
            Box::new(Sum(Box::new(v(a)), Box::new(v(b)))),
        );
        assert_eq!(normalize(&x), normalize(&y));
    }

    #[test]
    fn normalize_preserves_semantics() {
        let (n, _m, _a, _b) = kids();
        let e = Sum(
            Box::new(Times(Box::new(v(n)), Box::new(c(0)))),
            Box::new(Minus(Box::new(c(5)), Box::new(c(5)))),
        );
        assert_eq!(normalize(&e), c(0));
        let env: BTreeMap<Kid, i128> = [(n, 7)].into_iter().collect();
        let orig = Sum(Box::new(v(n)), Box::new(c(0)));
        assert_eq!(orig.eval(&env), normalize(&orig).eval(&env));
    }

    #[test]
    fn constraint_simplification() {
        let (_n, _m, _a, b) = kids();
        assert_eq!(
            simplify_constraint(&NConstraint::And(
                Box::new(NConstraint::True),
                Box::new(NConstraint::Var(b))
            )),
            NConstraint::Var(b)
        );
        assert_eq!(
            simplify_constraint(&NConstraint::Equal(Box::new(c(2)), Box::new(c(3)))),
            NConstraint::False
        );
    }
}
