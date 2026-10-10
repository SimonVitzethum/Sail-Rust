#![forbid(unsafe_code)]
//! `int`/`nat`/`atom`/`range` classification for Lean emission.
//!
//! Mirrors `doc_typ` in `pretty_print_lean.ml`:
//! - `int` maps to `Int`, `nat` maps to `Nat`;
//! - `atom(x)` maps to `Nat` when `x` is provably non-negative, else `Int`;
//! - `range(low, high)` maps to `Nat` when `low` is provably non-negative,
//!   else `Int`;
//! - an integer kind variable maps to `Nat` when it is provably
//!   non-negative, else `Int`.

use crate::nexp::{provably_nneg_in, Facts, NExp};

/// Lean integer type selected for a numeric Sail type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntKind {
    /// Emits as `Nat`.
    Nat,
    /// Emits as `Int`.
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

/// Numeric Sail type relevant for Int/Nat selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NumType {
    /// `int`.
    Int,
    /// `nat`.
    Nat,
    /// `atom(x)` / `int(x)`.
    Atom(NExp),
    /// `range(low, high)`.
    Range {
        /// Lower bound; decides `Nat` vs `Int`.
        lo: NExp,
        /// Upper bound (kept for completeness; unused for the mapping).
        hi: NExp,
    },
    /// Integer kind variable `'n`.
    KindVar(String),
}

impl NumType {
    /// Classify under `facts`: [`IntKind::Nat`] iff the value is provably
    /// non-negative.
    pub fn int_kind_in(&self, facts: &Facts) -> IntKind {
        let nat = match self {
            NumType::Int | NumType::Nat => matches!(self, NumType::Nat),
            NumType::Atom(x) => provably_nneg_in(x, facts),
            NumType::Range { lo, .. } => provably_nneg_in(lo, facts),
            NumType::KindVar(v) => provably_nneg_in(&NExp::Var(v.clone()), facts),
        };
        if nat { IntKind::Nat } else { IntKind::Int }
    }

    /// Classify with an empty context.
    pub fn int_kind(&self) -> IntKind {
        self.int_kind_in(&Facts::new())
    }

    /// Lean type name (`"Nat"`/`"Int"`) under `facts`.
    pub fn lean_name_in(&self, facts: &Facts) -> &'static str {
        self.int_kind_in(facts).lean_name()
    }

    /// Lean type name (`"Nat"`/`"Int"`) with an empty context.
    pub fn lean_name(&self) -> &'static str {
        self.int_kind().lean_name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_types() {
        assert_eq!(NumType::Int.lean_name(), "Int");
        assert_eq!(NumType::Nat.lean_name(), "Nat");
    }

    #[test]
    fn range_low_decides() {
        let nat_range = NumType::Range {
            lo: NExp::Constant(0),
            hi: NExp::Constant(31),
        };
        assert_eq!(nat_range.lean_name(), "Nat");
        let int_range = NumType::Range {
            lo: NExp::Constant(-2),
            hi: NExp::Constant(2),
        };
        assert_eq!(int_range.lean_name(), "Int");
        // `range('n, 'm)` with no facts: unknown low -> `Int`.
        let unkn = NumType::Range {
            lo: NExp::var("n"),
            hi: NExp::var("m"),
        };
        assert_eq!(unkn.lean_name(), "Int");
    }

    #[test]
    fn atom_and_kindvar() {
        assert_eq!(NumType::Atom(NExp::Constant(5)).lean_name(), "Nat");
        assert_eq!(NumType::Atom(NExp::Constant(-1)).lean_name(), "Int");
        assert_eq!(NumType::KindVar("n".to_string()).lean_name(), "Int");
        let facts = Facts::new().with_lower("n", 0);
        assert_eq!(
            NumType::KindVar("n".to_string()).lean_name_in(&facts),
            "Nat"
        );
        assert_eq!(NumType::Atom(NExp::var("n")).lean_name_in(&facts), "Nat");
    }
}
