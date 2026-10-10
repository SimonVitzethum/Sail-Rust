#![forbid(unsafe_code)]
//! Property tests for arithmetic identities (lane 04 done-criterion).
//!
//! - Simplification/normalization preserves total valuation.
//! - Normalization is idempotent and canonical for `Sum`/`Times`.
//! - `provably_*` predicates are sound w.r.t. evaluation.

use proptest::prelude::*;
use sail_nexp::{
    normalize, provably_nneg, provably_nonzero, provably_zero, simplify, Facts, NConstraint, NExp,
};
use std::collections::BTreeMap;

fn leaf() -> impl Strategy<Value = NExp> {
    prop_oneof![
        (-8i128..8).prop_map(NExp::Constant),
        prop_oneof![Just("n".to_string()), Just("m".to_string())].prop_map(NExp::Var),
    ]
}

fn nexp(depth: u32) -> BoxedStrategy<NExp> {
    if depth == 0 {
        return leaf().boxed();
    }
    prop_oneof![
        leaf().boxed(),
        (nexp(depth - 1), nexp(depth - 1))
            .prop_map(|(a, b)| NExp::Sum(Box::new(a), Box::new(b)))
            .boxed(),
        (nexp(depth - 1), nexp(depth - 1))
            .prop_map(|(a, b)| NExp::Minus(Box::new(a), Box::new(b)))
            .boxed(),
        (nexp(depth - 1), nexp(depth - 1))
            .prop_map(|(a, b)| NExp::Times(Box::new(a), Box::new(b)))
            .boxed(),
        nexp(depth - 1).prop_map(|a| NExp::Neg(Box::new(a))).boxed(),
        // Small exponents keep `2^x` representable.
        (0i128..5)
            .prop_map(|e| NExp::Exp(Box::new(NExp::Constant(e))))
            .boxed(),
        nexp(depth - 1).prop_map(NExp::abs).boxed(),
    ]
    .boxed()
}

fn small_env() -> impl Strategy<Value = BTreeMap<String, i128>> {
    (-8i128..8, -8i128..8).prop_map(|(n, m)| {
        [("n".to_string(), n), ("m".to_string(), m)]
            .into_iter()
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn simplify_preserves_eval(e in nexp(3), env in small_env()) {
        prop_assert_eq!(simplify(&e).eval(&env), e.eval(&env));
    }

    #[test]
    fn normalize_preserves_eval(e in nexp(3), env in small_env()) {
        prop_assert_eq!(normalize(&e).eval(&env), e.eval(&env));
    }

    #[test]
    fn normalize_is_idempotent(e in nexp(3)) {
        let once = normalize(&e);
        prop_assert_eq!(normalize(&once), once);
    }

    #[test]
    fn normalize_is_canonical_for_sum(a in nexp(2), b in nexp(2)) {
        let ab = NExp::Sum(Box::new(a.clone()), Box::new(b.clone()));
        let ba = NExp::Sum(Box::new(b), Box::new(a));
        prop_assert_eq!(normalize(&ab), normalize(&ba));
    }

    #[test]
    fn normalize_is_canonical_for_times(a in nexp(2), b in nexp(2)) {
        let ab = NExp::Times(Box::new(a.clone()), Box::new(b.clone()));
        let ba = NExp::Times(Box::new(b), Box::new(a));
        prop_assert_eq!(normalize(&ab), normalize(&ba));
    }

    #[test]
    fn provably_nneg_is_sound(e in nexp(3), env in small_env()) {
        if provably_nneg(&e) {
            if let Some(v) = e.eval(&env) {
                prop_assert!(v >= 0, "provably_nneg but eval = {v} for {e:?}");
            }
        }
    }

    #[test]
    fn provably_zero_and_nonzero_are_sound(e in nexp(3), env in small_env()) {
        if provably_zero(&e) {
            if let Some(v) = e.eval(&env) {
                prop_assert_eq!(v, 0);
            }
        }
        if provably_nonzero(&e) {
            if let Some(v) = e.eval(&env) {
                prop_assert!(v != 0, "provably_nonzero but eval = 0 for {e:?}");
            }
        }
    }

    #[test]
    fn constraint_simplify_preserves_eval(
        x in -8i128..8, y in -8i128..8, z in -8i128..8,
    ) {
        // `x + y >= z` normalizes without changing its truth value.
        let c = NConstraint::Ge(
            Box::new(NExp::Sum(Box::new(NExp::Constant(x)), Box::new(NExp::Constant(y)))),
            Box::new(NExp::Constant(z)),
        );
        let env = BTreeMap::new();
        prop_assert_eq!(
            sail_nexp::simplify_constraint(&c).eval(&env),
            c.eval(&env)
        );
        let _ = Facts::new();
    }
}
