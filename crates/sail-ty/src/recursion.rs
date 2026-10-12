//! Iterative recursion-depth guards (Lane-21 audit O-7).
//!
//! [`Nexp::depth`](crate::nexp::Nexp::depth) and
//! [`NConstraint::depth`](crate::nexp::NConstraint::depth) are natively
//! recursive: on a 128 KiB worker stack a 10_000-deep input aborts the
//! process with `SIGABRT` *inside the guard itself* (proven by the Lane-21
//! 10k probes, see `evidence/2026-10-12/lane-21-recursion.md`), so the
//! [`CheckLimits::max_depth`](crate::check::CheckLimits::max_depth) fuel
//! check in `check.rs` cannot protect those paths.
//!
//! The helpers here compute the same accept/reject answer with an explicit
//! heap work-stack: they never recurse, never clone deep data (only
//! references are pushed), and short-circuit on the first node that
//! exceeds `limit`. Semantics match the recursive definitions exactly:
//! `*_exceeds_depth(x, limit)` is true iff `x.depth() > limit`.
//!
//! These are intentionally *checks*, not converters: the `conv` converters
//! keep their signatures (see the audit report for the full iterative
//! rewrite specs). Callers that must survive adversarial depth should gate
//! on these helpers *before* invoking recursive converters, `Display`
//! impls, or `.depth()`.

#![forbid(unsafe_code)]

use crate::nexp::{MiniTyp, NConstraint, Nexp, TypQuant};

/// Work item for the shared iterative traversal. The `usize` is the depth
/// of the referenced node (leaves of the whole tree have depth 1, matching
/// the recursive `depth()` definitions where `Const`/`True` count as 1).
enum Work<'a> {
    N(&'a Nexp, usize),
    C(&'a NConstraint, usize),
}

/// Returns `true` iff `n.depth() > limit`, computed iteratively.
///
/// Short-circuits: the first node deeper than `limit` stops the walk, so
/// adversarial inputs cost O(limit + branching) rather than O(size).
#[must_use]
pub fn nexp_exceeds_depth(n: &Nexp, limit: usize) -> bool {
    let mut stack = vec![Work::N(n, 1)];
    while let Some(item) = stack.pop() {
        match item {
            Work::N(node, depth) => {
                if depth > limit {
                    return true;
                }
                match node {
                    Nexp::Const(_) | Nexp::Var(_) => {}
                    Nexp::Neg(x) | Nexp::Exp(x) | Nexp::Abs(x) => {
                        stack.push(Work::N(x, depth + 1));
                    }
                    Nexp::Add(a, b)
                    | Nexp::Sub(a, b)
                    | Nexp::Mul(a, b)
                    | Nexp::Div(a, b)
                    | Nexp::Mod(a, b) => {
                        stack.push(Work::N(a, depth + 1));
                        stack.push(Work::N(b, depth + 1));
                    }
                    Nexp::If(c, a, b) => {
                        stack.push(Work::N(a, depth + 1));
                        stack.push(Work::N(b, depth + 1));
                        stack.push(Work::C(c, depth + 1));
                    }
                }
            }
            Work::C(node, depth) => {
                if depth > limit {
                    return true;
                }
                push_constraint_children(&mut stack, node, depth);
            }
        }
    }
    false
}

/// Returns `true` iff `c.depth() > limit`, computed iteratively.
#[must_use]
pub fn constraint_exceeds_depth(c: &NConstraint, limit: usize) -> bool {
    let mut stack = vec![Work::C(c, 1)];
    while let Some(item) = stack.pop() {
        match item {
            Work::C(node, depth) => {
                if depth > limit {
                    return true;
                }
                push_constraint_children(&mut stack, node, depth);
            }
            Work::N(node, depth) => {
                if depth > limit {
                    return true;
                }
                match node {
                    Nexp::Const(_) | Nexp::Var(_) => {}
                    Nexp::Neg(x) | Nexp::Exp(x) | Nexp::Abs(x) => {
                        stack.push(Work::N(x, depth + 1));
                    }
                    Nexp::Add(a, b)
                    | Nexp::Sub(a, b)
                    | Nexp::Mul(a, b)
                    | Nexp::Div(a, b)
                    | Nexp::Mod(a, b) => {
                        stack.push(Work::N(a, depth + 1));
                        stack.push(Work::N(b, depth + 1));
                    }
                    Nexp::If(cond, a, b) => {
                        stack.push(Work::N(a, depth + 1));
                        stack.push(Work::N(b, depth + 1));
                        stack.push(Work::C(cond, depth + 1));
                    }
                }
            }
        }
    }
    false
}

fn push_constraint_children<'a>(stack: &mut Vec<Work<'a>>, node: &'a NConstraint, depth: usize) {
    match node {
        NConstraint::True | NConstraint::False | NConstraint::Var(_) | NConstraint::Id(_) => {}
        NConstraint::Eq(a, b)
        | NConstraint::Neq(a, b)
        | NConstraint::Ge(a, b)
        | NConstraint::Gt(a, b)
        | NConstraint::Le(a, b)
        | NConstraint::Lt(a, b) => {
            stack.push(Work::N(a, depth + 1));
            stack.push(Work::N(b, depth + 1));
        }
        NConstraint::And(a, b) | NConstraint::Or(a, b) => {
            stack.push(Work::C(a, depth + 1));
            stack.push(Work::C(b, depth + 1));
        }
        NConstraint::Not(a) => stack.push(Work::C(a, depth + 1)),
        NConstraint::Set(n, _) => stack.push(Work::N(n, depth + 1)),
        NConstraint::App(_, args) => {
            for a in args {
                stack.push(Work::N(a, depth + 1));
            }
        }
    }
}

/// Returns `true` iff any level nexp inside `t` exceeds `limit`.
///
/// Mirrors the `check.rs` guard semantics (`MiniTyp::nexps` + per-nexp
/// `depth()`), but the type walk itself is iterative and no deep data is
/// cloned — `nexps()` clones every level term (recursive `Clone` walks),
/// which overflows on 10k-deep arguments exactly like `depth()` does.
#[must_use]
pub fn mini_typ_exceeds_depth(t: &MiniTyp, limit: usize) -> bool {
    let mut stack = vec![t];
    while let Some(node) = stack.pop() {
        match node {
            MiniTyp::Int | MiniTyp::Bool | MiniTyp::Bit => {}
            MiniTyp::Bits(n) => {
                if nexp_exceeds_depth(n, limit) {
                    return true;
                }
            }
            MiniTyp::Range(lo, hi) => {
                if nexp_exceeds_depth(lo, limit) || nexp_exceeds_depth(hi, limit) {
                    return true;
                }
            }
            MiniTyp::Named { args, .. } => {
                for a in args {
                    if nexp_exceeds_depth(a, limit) {
                        return true;
                    }
                }
            }
            MiniTyp::Fun(a, b) => {
                stack.push(a);
                stack.push(b);
            }
            MiniTyp::Tuple(ts) => {
                for e in ts {
                    stack.push(e);
                }
            }
        }
    }
    false
}

/// Returns `true` iff the scheme's constraint or body type exceeds `limit`.
/// Mirrors the `check_scheme` guard (`scheme.constraint.depth()` plus the
/// `MiniTyp::nexps` walk) without recursing.
#[must_use]
pub fn typ_quant_exceeds_depth(q: &TypQuant, limit: usize) -> bool {
    constraint_exceeds_depth(&q.constraint, limit) || mini_typ_exceeds_depth(&q.typ, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_chain(depth: usize) -> Nexp {
        let mut n = Nexp::Const(0);
        for _ in 0..depth {
            n = Nexp::Add(Box::new(Nexp::Const(1)), Box::new(n));
        }
        n
    }

    fn and_chain(depth: usize) -> NConstraint {
        let mut c = NConstraint::True;
        for _ in 0..depth {
            c = NConstraint::And(Box::new(NConstraint::True), Box::new(c));
        }
        c
    }

    #[test]
    fn boundary_matches_recursive_depth() {
        // depth(Add(Const,Const)) == 2; If adds the constraint level.
        let n = Nexp::Add(Box::new(Nexp::Const(1)), Box::new(Nexp::Const(2)));
        assert_eq!(n.depth(), 2);
        assert!(nexp_exceeds_depth(&n, 1));
        assert!(!nexp_exceeds_depth(&n, 2));
        assert!(!nexp_exceeds_depth(&n, 256));
        let c = NConstraint::Not(Box::new(NConstraint::True));
        assert_eq!(c.depth(), 2);
        assert!(constraint_exceeds_depth(&c, 1));
        assert!(!constraint_exceeds_depth(&c, 2));
        // `If` nests a constraint one level deeper than its branches.
        let mixed = Nexp::If(
            Box::new(NConstraint::And(
                Box::new(NConstraint::True),
                Box::new(NConstraint::True),
            )),
            Box::new(Nexp::Const(1)),
            Box::new(Nexp::Const(2)),
        );
        assert_eq!(mixed.depth(), 3);
        assert!(nexp_exceeds_depth(&mixed, 2));
        assert!(!nexp_exceeds_depth(&mixed, 3));
        // `App` over no args counts 1; empty MiniTyp leaves never exceed.
        let empty_app = NConstraint::App("p".to_owned(), vec![]);
        assert_eq!(empty_app.depth(), 1);
        assert!(!constraint_exceeds_depth(&empty_app, 1));
        assert!(!mini_typ_exceeds_depth(&MiniTyp::Int, 0));
    }

    #[test]
    fn agrees_with_recursive_depth_on_nested_shapes() {
        // Every constructor, nested 3 levels, checked against the recursive
        // `depth()` on the normal test stack (shallow => no overflow).
        let deep = add_chain(3);
        let cases: Vec<Nexp> = vec![
            Nexp::Neg(Box::new(deep.clone())),
            Nexp::Exp(Box::new(deep.clone())),
            Nexp::Abs(Box::new(deep.clone())),
            Nexp::Sub(Box::new(deep.clone()), Box::new(Nexp::Const(0))),
            Nexp::Mul(Box::new(deep.clone()), Box::new(Nexp::Const(0))),
            Nexp::Div(Box::new(deep.clone()), Box::new(Nexp::Const(1))),
            Nexp::Mod(Box::new(deep.clone()), Box::new(Nexp::Const(1))),
            Nexp::If(
                Box::new(and_chain(2)),
                Box::new(deep.clone()),
                Box::new(Nexp::Const(0)),
            ),
        ];
        for n in &cases {
            let d = n.depth();
            assert!(nexp_exceeds_depth(n, d - 1), "depth {d}");
            assert!(!nexp_exceeds_depth(n, d), "depth {d}");
        }
        let c_cases: Vec<NConstraint> = vec![
            NConstraint::Eq(Box::new(deep.clone()), Box::new(Nexp::Const(0))),
            NConstraint::Neq(Box::new(deep.clone()), Box::new(Nexp::Const(0))),
            NConstraint::Ge(Box::new(deep.clone()), Box::new(Nexp::Const(0))),
            NConstraint::Gt(Box::new(deep.clone()), Box::new(Nexp::Const(0))),
            NConstraint::Le(Box::new(deep.clone()), Box::new(Nexp::Const(0))),
            NConstraint::Lt(Box::new(deep.clone()), Box::new(Nexp::Const(0))),
            NConstraint::Or(Box::new(and_chain(2)), Box::new(NConstraint::False)),
            NConstraint::Not(Box::new(and_chain(2))),
            NConstraint::Set(Box::new(deep.clone()), vec![1, 2]),
            NConstraint::App("p".to_owned(), vec![deep.clone()]),
        ];
        for c in &c_cases {
            let d = c.depth();
            assert!(constraint_exceeds_depth(c, d - 1), "depth {d}");
            assert!(!constraint_exceeds_depth(c, d), "depth {d}");
        }
        let t = MiniTyp::Fun(
            Box::new(MiniTyp::Bits(Box::new(deep.clone()))),
            Box::new(MiniTyp::Tuple(vec![
                MiniTyp::Range(Box::new(deep.clone()), Box::new(Nexp::Const(9))),
                MiniTyp::Named {
                    name: "atom".to_owned(),
                    args: vec![deep.clone()],
                },
            ])),
        );
        assert!(!mini_typ_exceeds_depth(&t, 256));
        assert!(mini_typ_exceeds_depth(&t, 3));
        let q = TypQuant {
            quants: vec![],
            constraint: and_chain(2),
            typ: MiniTyp::Int,
        };
        assert!(!typ_quant_exceeds_depth(&q, 256));
        assert!(typ_quant_exceeds_depth(&q, 2));
    }
}
