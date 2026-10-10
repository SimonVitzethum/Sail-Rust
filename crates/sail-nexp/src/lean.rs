#![forbid(unsafe_code)]
//! `Nat`-vs-`Int` mapping for Lean emission.
//!
//! Mirrors the Lean backend rule (`pretty_print_lean.ml`): an `atom`
//! type argument maps to `Nat` when its [`NExp`] is
//! provably non-negative, otherwise to `Int`. The same rule decides
//! range types (`low..high` maps to `Nat` when `low` is non-negative)
//! and integer kind variables.

use crate::expr::NExp;
use crate::pred::{provably_nneg_in, Facts};

/// Lean integer type selected for a numeric expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntKind {
    /// Fits in `Nat` (`x >= 0` is provable).
    Nat,
    /// Needs `Int`.
    Int,
}

impl IntKind {
    /// Lean type name: `"Nat"` or `"Int"`.
    pub fn lean_name(self) -> &'static str {
        match self {
            IntKind::Nat => "Nat",
            IntKind::Int => "Int",
        }
    }
}

/// Classify `e` under `facts`: [`IntKind::Nat`] iff `e >= 0` is
/// syntactically provable.
pub fn int_kind_in(e: &NExp, facts: &Facts) -> IntKind {
    if provably_nneg_in(e, facts) {
        IntKind::Nat
    } else {
        IntKind::Int
    }
}

/// Classify `e` with an empty context.
pub fn int_kind(e: &NExp) -> IntKind {
    int_kind_in(e, &Facts::new())
}

/// Lean type name (`"Nat"`/`"Int"`) for `e` under `facts`.
pub fn lean_type_name_in(e: &NExp, facts: &Facts) -> &'static str {
    int_kind_in(e, facts).lean_name()
}

/// Lean type name (`"Nat"`/`"Int"`) for `e` with an empty context.
pub fn lean_type_name(e: &NExp) -> &'static str {
    int_kind(e).lean_name()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::Kid;
    use sail_ast::{Interner as _, SimpleInterner};

    /// Fresh table with `n` pre-interned. The table is dropped on return:
    /// [`Kid`]s are plain `Copy` handles.
    fn n() -> Kid {
        SimpleInterner::new().intern("n")
    }
    fn table() -> SimpleInterner {
        SimpleInterner::new()
    }

    #[test]
    fn constants_map() {
        assert_eq!(lean_type_name(&NExp::Constant(0)), "Nat");
        assert_eq!(lean_type_name(&NExp::Constant(7)), "Nat");
        assert_eq!(lean_type_name(&NExp::Constant(-1)), "Int");
    }

    #[test]
    fn var_needs_facts_for_nat() {
        let mut ix = table();
        let kid = ix.intern("n");
        assert_eq!(kid, n());
        let v = NExp::Var(kid);
        assert_eq!(int_kind(&v), IntKind::Int);
        let f = Facts::new().with_lower(kid, 0);
        assert_eq!(int_kind_in(&v, &f), IntKind::Nat);
        assert_eq!(lean_type_name_in(&v, &f), "Nat");
    }

    #[test]
    fn shapes_map() {
        let kid = n();
        assert_eq!(lean_type_name(&NExp::Exp(Box::new(NExp::Var(kid)))), "Nat");
        assert_eq!(lean_type_name(&NExp::abs(NExp::Var(kid))), "Nat");
        assert_eq!(
            lean_type_name(&NExp::Neg(Box::new(NExp::Constant(1)))),
            "Int"
        );
    }
}
