#![forbid(unsafe_code)]
//! Syntactic provability predicates: the pure equivalent of the Lean
//! backend's `provably_nneg ctx x = prove env (x >= 0)`
//! (`sail/src/sail_lean_backend/pretty_print_lean.ml`).
//!
//! There is no solver here. Every predicate is conservative: it answers
//! `true` only when the fact follows syntactically from the shape of
//! the expression plus the assumed [`Facts`]. Unknown shapes answer
//! `false` (unknown), never a wrong `true`.

use crate::expr::{Kid, NConstraint, NExp};
use std::collections::BTreeMap;

/// Assumed context facts: lower/upper bounds per kind variable plus a
/// set of variables known to be non-zero.
///
/// Bounds are inclusive: `lower[n] = Some(0)` means `n >= 0` is
/// assumed. Keys are interned [`Kid`]s (Lane 71); intern names through
/// the caller's interner before inserting or querying.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    lower: BTreeMap<Kid, i128>,
    upper: BTreeMap<Kid, i128>,
    nonzero: BTreeMap<Kid, ()>,
}

impl Facts {
    /// Empty context: nothing is assumed.
    pub fn new() -> Self {
        Facts::default()
    }

    /// Assume `var >= bound`.
    pub fn with_lower(mut self, var: Kid, bound: i128) -> Self {
        self.lower
            .entry(var)
            .and_modify(|b| *b = (*b).max(bound))
            .or_insert(bound);
        self
    }

    /// Assume `var <= bound`.
    pub fn with_upper(mut self, var: Kid, bound: i128) -> Self {
        self.upper
            .entry(var)
            .and_modify(|b| *b = (*b).min(bound))
            .or_insert(bound);
        self
    }

    /// Assume `var != 0`.
    pub fn with_nonzero(mut self, var: Kid) -> Self {
        self.nonzero.insert(var, ());
        self
    }

    /// Collect simple bounds from a constraint tree (`And` of
    /// comparisons against constants, `x != 0`, `x == c` (both bounds),
    /// `x in {...}` with a single element). Anything else is ignored
    /// (conservative).
    pub fn from_constraint(c: &NConstraint) -> Self {
        let mut facts = Facts::new();
        facts.add(c);
        facts
    }

    fn add(&mut self, c: &NConstraint) {
        match c {
            NConstraint::And(a, b) => {
                self.add(a);
                self.add(b);
            }
            NConstraint::Ge(a, b) => self.add_cmp(a, b, false),
            NConstraint::Gt(a, b) => self.add_cmp(a, b, true),
            NConstraint::Le(a, b) => self.add_cmp(b, a, false),
            NConstraint::Lt(a, b) => self.add_cmp(b, a, true),
            NConstraint::NotEqual(a, b) => {
                if let (NExp::Var(k), NExp::Constant(0)) | (NExp::Constant(0), NExp::Var(k)) =
                    (&**a, &**b)
                {
                    self.nonzero.insert(*k, ());
                }
            }
            NConstraint::Equal(a, b) => {
                // `x == c` pins both bounds (max/min discipline keeps
                // tighter pre-existing bounds; contradictory facts stay
                // contradictory, which is vacuously sound).
                if let (NExp::Var(k), NExp::Constant(c))
                | (NExp::Constant(c), NExp::Var(k)) = (&**a, &**b)
                {
                    self.lower
                        .entry(*k)
                        .and_modify(|bound| *bound = (*bound).max(*c))
                        .or_insert(*c);
                    self.upper
                        .entry(*k)
                        .and_modify(|bound| *bound = (*bound).min(*c))
                        .or_insert(*c);
                }
            }
            NConstraint::Set(n, vs) => {
                if let ([v], NExp::Var(k)) = (vs.as_slice(), &**n) {
                    self.lower.insert(*k, *v);
                    self.upper.insert(*k, *v);
                }
            }
            _ => {}
        }
    }

    /// Record `lo >= hi` (or `lo > hi` for strict) when one side is a
    /// variable and the other a constant.
    fn add_cmp(&mut self, lo: &NExp, hi: &NExp, strict: bool) {
        let bump = i128::from(strict);
        match (lo, hi) {
            (NExp::Var(k), NExp::Constant(c)) => {
                let bound = c.saturating_add(bump);
                self.lower
                    .entry(*k)
                    .and_modify(|b| *b = (*b).max(bound))
                    .or_insert(bound);
            }
            (NExp::Constant(c), NExp::Var(k)) => {
                let bound = c.saturating_sub(bump);
                self.upper
                    .entry(*k)
                    .and_modify(|b| *b = (*b).min(bound))
                    .or_insert(bound);
            }
            _ => {}
        }
    }

    /// Assumed lower bound of `var`, if any.
    pub fn lower(&self, var: Kid) -> Option<i128> {
        self.lower.get(&var).copied()
    }

    /// Assumed upper bound of `var`, if any.
    pub fn upper(&self, var: Kid) -> Option<i128> {
        self.upper.get(&var).copied()
    }

    /// Whether `var` is assumed non-zero.
    pub fn is_nonzero_var(&self, var: Kid) -> bool {
        self.nonzero.contains_key(&var)
    }
}

/// Sign information derived syntactically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sign {
    /// Provably `> 0`.
    Pos,
    /// Provably `>= 0`.
    Nneg,
    /// Provably `< 0`.
    Neg,
    /// Provably `<= 0`.
    Nonpos,
    /// Provably `== 0`.
    Zero,
    /// Unknown.
    Unknown,
}

impl Sign {
    fn is_nneg(self) -> bool {
        matches!(self, Sign::Pos | Sign::Nneg | Sign::Zero)
    }
    fn is_pos(self) -> bool {
        matches!(self, Sign::Pos)
    }
    fn is_nonpos(self) -> bool {
        matches!(self, Sign::Neg | Sign::Nonpos | Sign::Zero)
    }
    fn is_neg(self) -> bool {
        matches!(self, Sign::Neg)
    }
    fn is_zero(self) -> bool {
        matches!(self, Sign::Zero)
    }
}

/// Syntactic sign analysis of `e` under `facts`.
fn sign(e: &NExp, facts: &Facts) -> Sign {
    match e {
        NExp::Constant(v) => {
            if *v > 0 {
                Sign::Pos
            } else if *v < 0 {
                Sign::Neg
            } else {
                Sign::Zero
            }
        }
        NExp::Var(k) => {
            let lo = facts.lower(*k);
            let hi = facts.upper(*k);
            match (lo, hi) {
                (Some(l), Some(h)) if l == h && l == 0 => Sign::Zero,
                (Some(l), Some(h)) if l == h && l > 0 => Sign::Pos,
                (Some(l), Some(h)) if l == h && l < 0 => Sign::Neg,
                (Some(l), _) if l > 0 => Sign::Pos,
                (Some(0), _) => Sign::Nneg,
                (_, Some(h)) if h < 0 => Sign::Neg,
                (_, Some(0)) => Sign::Nonpos,
                _ => Sign::Unknown,
            }
        }
        NExp::Id(_) => Sign::Unknown,
        NExp::Neg(x) => match sign(x, facts) {
            Sign::Pos => Sign::Neg,
            Sign::Neg => Sign::Pos,
            Sign::Nneg => Sign::Nonpos,
            Sign::Nonpos => Sign::Nneg,
            Sign::Zero => Sign::Zero,
            Sign::Unknown => Sign::Unknown,
        },
        NExp::Sum(a, b) => add_sign(sign(a, facts), sign(b, facts)),
        NExp::Minus(a, b) => add_sign(sign(a, facts), negate(sign(b, facts))),
        NExp::Times(a, b) => mul_sign(sign(a, facts), sign(b, facts), a, b, facts),
        NExp::Exp(_) => Sign::Pos,
        NExp::App(f, args) => match (f.as_str(), args.as_slice()) {
            ("abs", [_]) => Sign::Nneg,
            ("div", [a, _]) => {
                // `div` by a positive constant preserves the sign of `a`.
                if let NExp::Constant(d) = args[1] {
                    if d > 0 {
                        return sign(a, facts);
                    }
                }
                let _ = a;
                Sign::Unknown
            }
            ("mod", [_, b]) => {
                // `x mod m` with constant `m > 0` is in `[0, m)`.
                if let NExp::Constant(m) = b {
                    if *m > 0 {
                        return Sign::Nneg;
                    }
                }
                Sign::Unknown
            }
            _ => Sign::Unknown,
        },
        NExp::If(_, t, b) => {
            let (s, e) = (sign(t, facts), sign(b, facts));
            if s == e {
                s
            } else {
                Sign::Unknown
            }
        }
    }
}

fn negate(s: Sign) -> Sign {
    match s {
        Sign::Pos => Sign::Neg,
        Sign::Neg => Sign::Pos,
        Sign::Nneg => Sign::Nonpos,
        Sign::Nonpos => Sign::Nneg,
        Sign::Zero => Sign::Zero,
        Sign::Unknown => Sign::Unknown,
    }
}

fn add_sign(a: Sign, b: Sign) -> Sign {
    use Sign::*;
    match (a, b) {
        (Unknown, _) | (_, Unknown) => Unknown,
        (Zero, s) | (s, Zero) => s,
        (Pos, Pos) | (Pos, Nneg) | (Nneg, Pos) => Pos,
        (Nneg, Nneg) => Nneg,
        (Neg, Neg) | (Neg, Nonpos) | (Nonpos, Neg) => Neg,
        (Nonpos, Nonpos) => Nonpos,
        // Mixed signs: no conclusion without magnitudes.
        _ => Unknown,
    }
}

fn mul_sign(a: Sign, b: Sign, ae: &NExp, be: &NExp, facts: &Facts) -> Sign {
    use Sign::*;
    // Zero annihilates regardless of the other side.
    if a == Zero || b == Zero {
        // `0 * x` is `0` even when `x` is unknown.
        if is_zero_expr(ae, facts) || is_zero_expr(be, facts) {
            return Zero;
        }
    }
    match (a, b) {
        (Unknown, _) | (_, Unknown) => Unknown,
        (Zero, _) | (_, Zero) => Zero,
        (Pos, Pos) | (Neg, Neg) => Pos,
        (Pos, Neg) | (Neg, Pos) => Neg,
        (Pos, Nneg) | (Nneg, Pos) => Nneg,
        (Neg, Nonpos) | (Nonpos, Neg) => Nneg,
        (Nneg, Nneg) | (Nonpos, Nonpos) => Nneg,
        (Pos, Nonpos) | (Nonpos, Pos) | (Neg, Nneg) | (Nneg, Neg) => Nonpos,
        (Nneg, Nonpos) | (Nonpos, Nneg) => Unknown,
    }
}

fn is_zero_expr(e: &NExp, facts: &Facts) -> bool {
    sign(e, facts) == Sign::Zero
}

/// `x >= 0` is syntactically provable (empty context).
pub fn provably_nneg(x: &NExp) -> bool {
    provably_nneg_in(x, &Facts::new())
}

/// `x >= 0` is syntactically provable under `facts`.
pub fn provably_nneg_in(x: &NExp, facts: &Facts) -> bool {
    sign(x, facts).is_nneg()
}

/// `x > 0` is syntactically provable (empty context).
pub fn provably_pos(x: &NExp) -> bool {
    provably_pos_in(x, &Facts::new())
}

/// `x > 0` is syntactically provable under `facts`.
pub fn provably_pos_in(x: &NExp, facts: &Facts) -> bool {
    sign(x, facts).is_pos()
}

/// `x <= 0` is syntactically provable (empty context).
pub fn provably_nonpos(x: &NExp) -> bool {
    provably_nonpos_in(x, &Facts::new())
}

/// `x <= 0` is syntactically provable under `facts`.
pub fn provably_nonpos_in(x: &NExp, facts: &Facts) -> bool {
    sign(x, facts).is_nonpos()
}

/// `x < 0` is syntactically provable (empty context).
pub fn provably_neg(x: &NExp) -> bool {
    provably_neg_in(x, &Facts::new())
}

/// `x < 0` is syntactically provable under `facts`.
pub fn provably_neg_in(x: &NExp, facts: &Facts) -> bool {
    sign(x, facts).is_neg()
}

/// `x == 0` is syntactically provable (empty context).
pub fn provably_zero(x: &NExp) -> bool {
    provably_zero_in(x, &Facts::new())
}

/// `x == 0` is syntactically provable under `facts`.
pub fn provably_zero_in(x: &NExp, facts: &Facts) -> bool {
    sign(x, facts).is_zero()
}

/// `x != 0` is syntactically provable (empty context).
pub fn provably_nonzero(x: &NExp) -> bool {
    provably_nonzero_in(x, &Facts::new())
}

/// `x != 0` is syntactically provable under `facts`.
///
/// Besides sign analysis (`x > 0` or `x < 0` implies `x != 0`), this
/// honours [`Facts::with_nonzero`] assumptions.
pub fn provably_nonzero_in(x: &NExp, facts: &Facts) -> bool {
    if let NExp::Var(k) = x {
        if facts.is_nonzero_var(*k) {
            return true;
        }
    }
    matches!(sign(x, facts), Sign::Pos | Sign::Neg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Interner as _, SimpleInterner};

    fn v(k: Kid) -> NExp {
        NExp::Var(k)
    }
    fn c(i: i128) -> NExp {
        NExp::Constant(i)
    }
    fn table() -> SimpleInterner {
        SimpleInterner::new()
    }

    #[test]
    fn constants() {
        assert!(provably_nneg(&c(0)));
        assert!(provably_nneg(&c(5)));
        assert!(!provably_nneg(&c(-1)));
        assert!(provably_pos(&c(5)));
        assert!(!provably_pos(&c(0)));
        assert!(provably_zero(&c(0)));
        assert!(provably_nonzero(&c(3)));
        assert!(!provably_nonzero(&c(0)));
    }

    #[test]
    fn shapes_without_context() {
        let mut ix = table();
        let n = ix.intern("n");
        // Unknown variable: conservative `false` everywhere.
        assert!(!provably_nneg(&v(n)));
        assert!(!provably_pos(&v(n)));
        assert!(!provably_zero(&v(n)));
        assert!(!provably_nonzero(&v(n)));
        // `2^x` is always positive; `abs` is always non-negative.
        assert!(provably_pos(&NExp::Exp(Box::new(v(n)))));
        assert!(provably_nneg(&NExp::abs(v(n))));
        assert!(!provably_pos(&NExp::abs(v(n))));
        // Sums/products of non-negative constants.
        let e = NExp::Sum(Box::new(c(1)), Box::new(c(2)));
        assert!(provably_pos(&e));
        let z = NExp::Times(Box::new(c(0)), Box::new(v(n)));
        assert!(provably_zero(&z));
    }

    #[test]
    fn facts_drive_bounds() {
        let mut ix = table();
        let n = ix.intern("n");
        let f = Facts::new().with_lower(n, 0);
        assert!(provably_nneg_in(&v(n), &f));
        assert!(!provably_pos_in(&v(n), &f));
        let f = Facts::new().with_lower(n, 1);
        assert!(provably_pos_in(&v(n), &f));
        assert!(provably_nonzero_in(&v(n), &f));
        let f = Facts::new().with_nonzero(n);
        assert!(provably_nonzero_in(&v(n), &f));
        assert!(!provably_nneg_in(&v(n), &f));
    }

    #[test]
    fn from_constraint_collects_bounds() {
        let mut ix = table();
        let n = ix.intern("n");
        let m = ix.intern("m");
        let c = NConstraint::And(
            Box::new(NConstraint::Ge(Box::new(v(n)), Box::new(NExp::Constant(2)))),
            Box::new(NConstraint::Lt(Box::new(v(m)), Box::new(NExp::Constant(0)))),
        );
        let f = Facts::from_constraint(&c);
        assert_eq!(f.lower(n), Some(2));
        assert_eq!(f.upper(m), Some(-1));
        assert!(provably_pos_in(&v(n), &f));
        assert!(provably_neg_in(&v(m), &f));
    }

    #[test]
    fn sign_through_arith() {
        let mut ix = table();
        let n = ix.intern("n");
        let m = ix.intern("m");
        let f = Facts::new().with_lower(n, 0).with_lower(m, 0);
        let sum = NExp::Sum(Box::new(v(n)), Box::new(v(m)));
        assert!(provably_nneg_in(&sum, &f));
        let prod = NExp::Times(Box::new(v(n)), Box::new(v(m)));
        assert!(provably_nneg_in(&prod, &f));
        let diff = NExp::Minus(Box::new(v(n)), Box::new(v(m)));
        assert!(!provably_nneg_in(&diff, &f));
    }

    #[test]
    fn facts_key_on_symbols_not_strings() {
        // Lane 71: interning the same spelling twice hits the same `Facts`
        // entry (no per-occurrence `String` keys).
        let mut ix = table();
        let n = ix.intern("n");
        assert_eq!(ix.intern("n"), n);
        let f = Facts::new().with_lower(n, 2);
        assert_eq!(f.lower(ix.intern("n")), Some(2));
        assert!(provably_pos_in(&v(n), &f));
    }
}
