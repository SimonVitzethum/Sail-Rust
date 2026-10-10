#![forbid(unsafe_code)]
//! Loop-combinator kinds and `foreach` step order.
//!
//! Mirrors the dispatch in `doc_exp` / `doc_loop`
//! (`pretty_print_lean.ml`):
//!
//! ```ocaml
//! | E_app (Id_aux (Id "while#", _), args) -> doc_loop l as_monadic ctx `While args
//! | E_app (Id_aux (Id "while#t", _), args) -> doc_loop l as_monadic ctx `WhileFuel args
//! | E_app (Id_aux (Id "until#", _), args) -> doc_loop l as_monadic ctx `Until args
//! | E_app (Id_aux (Id "until#t", _), args) -> doc_loop l as_monadic ctx `UntilFuel args
//! | E_app (Id_aux (Id "foreach#", _), args) -> (* for-idiom *)
//! | E_for (loopvar, from_exp, to_exp, step_exp, order, body) -> (* for-idiom *)
//! ```
//!
//! and the `is_loop` predicate used by `has_loop`:
//!
//! ```ocaml
//! let is_loop id =
//!   match string_of_id id with "while#" | "while#t" | "foreach#" | "until#" | "until#t" -> true | _ -> false
//! ```

/// ANF loop combinator produced by the loop-rewrite pass, plus the surface
/// `for` expression (OCaml `E_for`, which never goes through `doc_loop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoopKind {
    /// `while#`: `while (λ vars => cond) vars do …`.
    While,
    /// `while#t`: `whileFuelM (fuel := m) (fun vars => cond) init …`.
    WhileFuel,
    /// `until#`: `repeat … until (λ vars => cond) vars`.
    Until,
    /// `until#t`: `untilFuelM (fuel := m) (fun vars => cond) init …`.
    UntilFuel,
    /// `foreach#`: `for i in [lo:hi:step]i do …` with `loop_vars` state.
    Foreach,
    /// Surface `E_for`: `for i in [lo:hi:step]i do …` without `loop_vars`.
    ForSurface,
}

impl LoopKind {
    /// Sail-level combinator name (`None` for the surface `for`, which has
    /// no `#`-combinator).
    pub fn sail_name(self) -> Option<&'static str> {
        match self {
            LoopKind::While => Some("while#"),
            LoopKind::WhileFuel => Some("while#t"),
            LoopKind::Until => Some("until#"),
            LoopKind::UntilFuel => Some("until#t"),
            LoopKind::Foreach => Some("foreach#"),
            LoopKind::ForSurface => None,
        }
    }

    /// Whether this kind carries a fuel measure (`Option.get measure` in
    /// OCaml — rendering without one is a compiler bug).
    pub fn needs_fuel(self) -> bool {
        matches!(self, LoopKind::WhileFuel | LoopKind::UntilFuel)
    }

    /// Whether the body is threaded through explicit `loop_vars` state.
    /// Only the surface `for` carries no state.
    pub fn has_loop_vars(self) -> bool {
        !matches!(self, LoopKind::ForSurface)
    }
}

/// Fuel-bounded loop combinator (`while#t` / `until#t`).
///
/// Reference signatures from `lean-sail/Sail/Sail.lean`:
/// `whileFuelM (fuel : Nat) (cond : α → m Bool) (init : α) (f : α → m α)`,
/// same for `untilFuelM`. `whileFuelM` checks the condition *before* each
/// step; `untilFuelM` runs the body first and stops once the condition holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FuelKind {
    WhileFuel,
    UntilFuel,
}

impl FuelKind {
    /// Lean combinator name.
    pub fn combinator(self) -> &'static str {
        match self {
            FuelKind::WhileFuel => "whileFuelM",
            FuelKind::UntilFuel => "untilFuelM",
        }
    }
}

impl From<FuelKind> for LoopKind {
    fn from(kind: FuelKind) -> Self {
        match kind {
            FuelKind::WhileFuel => LoopKind::WhileFuel,
            FuelKind::UntilFuel => LoopKind::UntilFuel,
        }
    }
}

/// Step direction of a `foreach`/`for` range.
///
/// OCaml `foreach#` case: `if is_true ord_exp then step_pp else minus ^^ step_pp`;
/// OCaml `E_for` case: `Ord_inc` keeps the step, `Ord_dec` negates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Order {
    Inc,
    Dec,
}

/// Apply the range direction to an already-printed step expression.
pub fn apply_step(order: Order, step: &str) -> String {
    match order {
        Order::Inc => step.to_owned(),
        Order::Dec => format!("-{step}"),
    }
}

/// Sail `#`-combinator names that count as loops (`is_loop` in OCaml,
/// the predicate behind `has_loop`).
pub fn is_loop_name(name: &str) -> bool {
    matches!(name, "while#" | "while#t" | "foreach#" | "until#" | "until#t")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sail_names_cover_all_combinators() {
        assert_eq!(LoopKind::While.sail_name(), Some("while#"));
        assert_eq!(LoopKind::WhileFuel.sail_name(), Some("while#t"));
        assert_eq!(LoopKind::Until.sail_name(), Some("until#"));
        assert_eq!(LoopKind::UntilFuel.sail_name(), Some("until#t"));
        assert_eq!(LoopKind::Foreach.sail_name(), Some("foreach#"));
        assert_eq!(LoopKind::ForSurface.sail_name(), None);
    }

    #[test]
    fn sail_names_are_loop_names() {
        for kind in [
            LoopKind::While,
            LoopKind::WhileFuel,
            LoopKind::Until,
            LoopKind::UntilFuel,
            LoopKind::Foreach,
        ] {
            assert!(is_loop_name(kind.sail_name().unwrap()), "{kind:?}");
        }
    }

    #[test]
    fn non_loop_names_rejected() {
        for name in ["while", "for", "foreach", "early_return", "foreach_Z_up", ""] {
            assert!(!is_loop_name(name), "{name}");
        }
    }

    #[test]
    fn fuel_kinds_need_fuel() {
        assert!(LoopKind::WhileFuel.needs_fuel());
        assert!(LoopKind::UntilFuel.needs_fuel());
        assert!(!LoopKind::While.needs_fuel());
        assert!(!LoopKind::Until.needs_fuel());
        assert!(!LoopKind::Foreach.needs_fuel());
        assert!(!LoopKind::ForSurface.needs_fuel());
    }

    #[test]
    fn fuel_combinator_names() {
        assert_eq!(FuelKind::WhileFuel.combinator(), "whileFuelM");
        assert_eq!(FuelKind::UntilFuel.combinator(), "untilFuelM");
    }

    #[test]
    fn only_surface_for_has_no_loop_vars() {
        for kind in [
            LoopKind::While,
            LoopKind::WhileFuel,
            LoopKind::Until,
            LoopKind::UntilFuel,
            LoopKind::Foreach,
        ] {
            assert!(kind.has_loop_vars(), "{kind:?}");
        }
        assert!(!LoopKind::ForSurface.has_loop_vars());
    }

    #[test]
    fn step_direction() {
        assert_eq!(apply_step(Order::Inc, "1"), "1");
        assert_eq!(apply_step(Order::Dec, "1"), "-1");
        assert_eq!(apply_step(Order::Dec, "(m -i 1)"), "-(m -i 1)");
    }
}
