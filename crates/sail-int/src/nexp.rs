#![forbid(unsafe_code)]
//! Minimal numeric exponent AST plus a conservative `provably_nneg` predicate.
//!
//! Local mirror of the rule used by the OCaml Lean backend
//! (`sail/src/sail_lean_backend/pretty_print_lean.ml`):
//! `provably_nneg ctx x = prove env (x >= 0)`.
//! Lane 04 (`sail-nexp`) implements the same predicate; it lives outside
//! this worktree (read-only), so the small fragment needed for
//! Int/Nat/Range/Atom classification is re-implemented here.
//!
//! Every predicate is conservative: it answers `true` only when the fact
//! follows syntactically from the expression shape plus the assumed
//! [`Facts`]. Unknown shapes answer `false`, never a wrong `true`.

use std::collections::BTreeMap;

/// Numeric type-level expression (fragment of Sail `Nexp_*`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NExp {
    /// Integer literal.
    Constant(i128),
    /// Kind/existential variable, e.g. `'n`.
    Var(String),
    /// `a + b`.
    Add(Box<NExp>, Box<NExp>),
    /// `a - b`.
    Sub(Box<NExp>, Box<NExp>),
    /// `a * b`.
    Mul(Box<NExp>, Box<NExp>),
    /// `-a`.
    Neg(Box<NExp>),
    /// `abs a` (always non-negative; backend prints `natAbs`).
    Abs(Box<NExp>),
    /// `2 ^ a` (always non-negative).
    Pow2(Box<NExp>),
    /// `a / b`.
    Div(Box<NExp>, Box<NExp>),
    /// `a % b`.
    Mod(Box<NExp>, Box<NExp>),
}

impl NExp {
    /// Variable expression.
    pub fn var(name: impl Into<String>) -> Self {
        NExp::Var(name.into())
    }

    /// Negation expression.
    pub fn negated(e: NExp) -> Self {
        NExp::Neg(Box::new(e))
    }

    /// Absolute value.
    pub fn abs(e: NExp) -> Self {
        NExp::Abs(Box::new(e))
    }
}

/// Assumed context facts: inclusive lower bounds per variable.
///
/// `lower["n"] = 0` means `n >= 0` is assumed (e.g. from a Sail
/// constraint `0 <= 'n` or a `Nat` binder).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    lower: BTreeMap<String, i128>,
}

impl Facts {
    /// Empty context: nothing is assumed.
    pub fn new() -> Self {
        Facts::default()
    }

    /// Assume `var >= bound` (keeps the strongest bound).
    pub fn with_lower(mut self, var: impl Into<String>, bound: i128) -> Self {
        let var = var.into();
        self.lower
            .entry(var)
            .and_modify(|b| *b = (*b).max(bound))
            .or_insert(bound);
        self
    }

    fn lower_bound(&self, var: &str) -> Option<i128> {
        self.lower.get(var).copied()
    }
}

/// `true` iff `e >= 0` follows syntactically from `e` and `facts`.
pub fn provably_nneg_in(e: &NExp, facts: &Facts) -> bool {
    match e {
        NExp::Constant(c) => *c >= 0,
        NExp::Var(v) => facts.lower_bound(v).is_some_and(|b| b >= 0),
        NExp::Add(a, b) => provably_nneg_in(a, facts) && provably_nneg_in(b, facts),
        // `a - b >= 0` needs an upper bound on `b`; not tracked: conservative.
        NExp::Sub(_, _) => false,
        NExp::Mul(a, b) => {
            (provably_nneg_in(a, facts) && provably_nneg_in(b, facts))
                || (provably_nonpos_in(a, facts) && provably_nonpos_in(b, facts))
        }
        NExp::Neg(a) => provably_nonpos_in(a, facts),
        NExp::Abs(_) | NExp::Pow2(_) => true,
        NExp::Div(a, b) => provably_nneg_in(a, facts) && provably_pos_in(b, facts),
        // Remainder sign is backend-specific; stay conservative.
        NExp::Mod(_, _) => false,
    }
}

/// `true` iff `e >= 0` with an empty context.
pub fn provably_nneg(e: &NExp) -> bool {
    provably_nneg_in(e, &Facts::new())
}

/// `true` iff `e <= 0` follows syntactically from `e` and `facts`.
pub fn provably_nonpos_in(e: &NExp, facts: &Facts) -> bool {
    match e {
        NExp::Constant(c) => *c <= 0,
        NExp::Neg(a) => provably_nneg_in(a, facts),
        NExp::Sub(a, b) => provably_nonpos_in(a, facts) && provably_nneg_in(b, facts),
        _ => false,
    }
}

/// `true` iff `e > 0` follows syntactically from `e` and `facts`.
pub fn provably_pos_in(e: &NExp, facts: &Facts) -> bool {
    match e {
        NExp::Constant(c) => *c > 0,
        NExp::Var(v) => facts.lower_bound(v).is_some_and(|b| b > 0),
        NExp::Add(a, b) => {
            (provably_pos_in(a, facts) && provably_nneg_in(b, facts))
                || (provably_nneg_in(a, facts) && provably_pos_in(b, facts))
        }
        NExp::Abs(a) => provably_pos_in(a, facts) || provably_neg_in(a, facts),
        NExp::Pow2(_) => true,
        _ => false,
    }
}

/// `true` iff `e < 0` follows syntactically from `e` and `facts`.
pub fn provably_neg_in(e: &NExp, facts: &Facts) -> bool {
    match e {
        NExp::Constant(c) => *c < 0,
        NExp::Neg(a) => provably_pos_in(a, facts),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants() {
        assert!(provably_nneg(&NExp::Constant(0)));
        assert!(provably_nneg(&NExp::Constant(31)));
        assert!(!provably_nneg(&NExp::Constant(-2)));
    }

    #[test]
    fn var_needs_fact() {
        let n = NExp::var("n");
        assert!(!provably_nneg(&n));
        let f = Facts::new().with_lower("n", 0);
        assert!(provably_nneg_in(&n, &f));
        let g = Facts::new().with_lower("n", 1);
        assert!(provably_nneg_in(&n, &g));
        let h = Facts::new().with_lower("n", -5);
        assert!(!provably_nneg_in(&n, &h));
    }

    #[test]
    fn shapes() {
        assert!(provably_nneg(&NExp::abs(NExp::var("n"))));
        assert!(provably_nneg(&NExp::Pow2(Box::new(NExp::var("n")))));
        assert!(provably_nneg(&NExp::negated(NExp::Constant(-3))));
        assert!(!provably_nneg(&NExp::negated(NExp::Constant(3))));
        assert!(!provably_nneg(&NExp::Sub(
            Box::new(NExp::var("a")),
            Box::new(NExp::var("b"))
        )));
    }
}
