#![forbid(unsafe_code)]
//! Fuel variants for Sail loops: `whileFuelM` / `untilFuelM`.
//!
//! Mirrors `doc_loop` in `pretty_print_lean.ml` for the `` `WhileFuel`` and
//! `` `UntilFuel`` cases (Sail `while#t` / `until#t` after the loop-measure
//! rewrites):
//!
//! ```ocaml
//! | `WhileFuel | `UntilFuel ->
//!     let measure = Option.get measure in
//!     ...
//!     let loop_fn = match loop_kind with `WhileFuel -> "whileFuelM" | _ -> "untilFuelM" in
//!     let loop_head = string loop_fn ^^ space ^^ measure ^^ space ^^ cond_pp ^^ space ^^ init in
//! ```
//!
//! Reference signatures from `lean-sail/Sail/Sail.lean`:
//!
//! ```lean
//! def whileFuelM {α} [Monad m] (fuel : Nat) (cond : α → m Bool) (init : α) (f : α → m α)
//! def untilFuelM {α} [Monad m] (fuel : Nat) (cond : α → m Bool) (init : α) (f : α → m α)
//! ```
//!
//! Semantics (from `Sail.lean`): both run at most `fuel` iterations.
//! `whileFuelM` checks the condition *before* each step (returns `init`
//! unchanged when the condition is initially false); `untilFuelM` runs the
//! body *first* and stops once the condition holds afterwards.

use std::fmt;

/// Which fuel-bounded loop combinator to emit.
///
/// Lane-127-Wahrheit: Die vollstaendige LoopKind-Wahrheit ist
/// `sail_loops::LoopKind` (`sail-loops`: `While/WhileFuel/Until/
/// UntilFuel/Foreach/ForSurface` plus `sail_name()`, `needs_fuel()`,
/// `has_loop_vars()`, `FuelKind`, `Order`, `is_loop_name`). Dieser Typ
/// hier ist die Fuel-Teilsicht der Terminierungs-Lane (plain `While`/
/// `Until` nur der Vollstaendigkeit halber; der Renderer
/// [`FuelLoop`] bleibt terminierungs-eigen). Abbildung:
/// `WhileFuel <-> FuelKind::WhileFuel` (`"whileFuelM"`),
/// `UntilFuel <-> FuelKind::UntilFuel` (`"untilFuelM"`),
/// `needs_fuel() == combinator().is_some()` beidseitig; Paritaet pinnt
/// `tests/loopkind_parity.rs` (Code, E3: ungeprueft). Parse-Ebene
/// (`sail-grammar`, nur `While/Until`) und Checker-Minimal
/// (`sail-ty::MiniLoop/MiniOrder`) bleiben eigene Sichten mit
/// Owner-Migration (Rest-Owner-Tabelle Lane 127).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopKind {
    /// Plain `while` (no fuel) — not a fuel variant, listed for completeness.
    While,
    /// `whileFuelM`: check-then-step, at most `fuel` iterations.
    WhileFuel,
    /// Plain `repeat … until` (no fuel).
    Until,
    /// `untilFuelM`: step-then-check, at most `fuel` iterations.
    UntilFuel,
}

impl LoopKind {
    /// Lean combinator name, or `None` for the non-fuel variants.
    pub fn combinator(self) -> Option<&'static str> {
        match self {
            LoopKind::WhileFuel => Some("whileFuelM"),
            LoopKind::UntilFuel => Some("untilFuelM"),
            LoopKind::While | LoopKind::Until => None,
        }
    }

    /// Whether this loop kind carries a fuel measure.
    pub fn needs_fuel(self) -> bool {
        self.combinator().is_some()
    }
}

/// A fuel-bounded loop call site: combinator + pretty-printed arguments.
///
/// `cond_fn` is the already-printed `(fun <vars> => <cond>)` lambda,
/// `init` the initial loop state, `body_fn` the `(fun <vars> => do <body>)`
/// step function, and `fuel` the `(fuel := <measure>)` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuelLoop {
    pub kind: LoopKind,
    pub fuel: String,
    pub cond_fn: String,
    pub init: String,
    pub body_fn: String,
}

impl FuelLoop {
    /// Build a fuel loop; fails for non-fuel kinds or empty arguments.
    pub fn new(
        kind: LoopKind,
        fuel: impl Into<String>,
        cond_fn: impl Into<String>,
        init: impl Into<String>,
        body_fn: impl Into<String>,
    ) -> Result<Self, String> {
        if !kind.needs_fuel() {
            return Err(format!("loop kind {kind:?} takes no fuel argument"));
        }
        let (fuel, cond_fn, init, body_fn) =
            (fuel.into(), cond_fn.into(), init.into(), body_fn.into());
        for (name, value) in
            [("fuel", &fuel), ("cond_fn", &cond_fn), ("init", &init), ("body_fn", &body_fn)]
        {
            if value.trim().is_empty() {
                return Err(format!("fuel loop {name} must not be empty"));
            }
        }
        Ok(Self { kind, fuel, cond_fn, init, body_fn })
    }

    /// Render the loop head + body, mirroring `doc_loop`:
    ///
    /// ```text
    /// let <vars> ← whileFuelM (fuel := <fuel>) (fun <vars> => <cond>) <init>
    ///   fun <vars> => do
    ///     <body>
    /// ```
    pub fn render(&self, vars: &str) -> String {
        let combinator = self.kind.combinator().expect("fuel loop has a combinator");
        format!(
            "let {vars} ← {combinator} (fuel := {}) ({}) {}\n  fun {vars} => do\n    {}",
            self.fuel, self.cond_fn, self.init, self.body_fn
        )
    }
}

impl fmt::Display for FuelLoop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.render("loop_vars"))
    }
}

/// The reference signatures of the fuel combinators in `Sail/Sail.lean`,
/// for documentation and conformance tests.
pub fn fuel_signatures() -> [&'static str; 2] {
    [
        "def whileFuelM {α} [Monad m] (fuel : Nat) (cond : α → m Bool) (init : α) (f : α → m α)",
        "def untilFuelM {α} [Monad m] (fuel : Nat) (cond : α → m Bool) (init : α) (f : α → m α)",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combinator_names() {
        assert_eq!(LoopKind::WhileFuel.combinator(), Some("whileFuelM"));
        assert_eq!(LoopKind::UntilFuel.combinator(), Some("untilFuelM"));
        assert_eq!(LoopKind::While.combinator(), None);
        assert_eq!(LoopKind::Until.combinator(), None);
    }

    #[test]
    fn renders_while_fuel_head() {
        let l = FuelLoop::new(
            LoopKind::WhileFuel,
            "nbits",
            "fun x => x > 0",
            "nbits",
            "x - 1",
        )
        .unwrap();
        let out = l.render("x");
        assert!(out.starts_with("let x ← whileFuelM (fuel := nbits)"), "{out}");
        assert!(out.contains("fun x => do"), "{out}");
    }

    #[test]
    fn renders_until_fuel_head() {
        let l =
            FuelLoop::new(LoopKind::UntilFuel, "100", "fun s => done s", "s0", "step s").unwrap();
        assert!(l.render("s").contains("untilFuelM (fuel := 100)"), "{}", l.render("s"));
    }

    #[test]
    fn non_fuel_kind_rejected() {
        assert!(FuelLoop::new(LoopKind::While, "f", "c", "i", "b").is_err());
    }

    #[test]
    fn signatures_match_sail_lean() {
        let sigs = fuel_signatures();
        assert!(sigs[0].starts_with("def whileFuelM"));
        assert!(sigs[1].starts_with("def untilFuelM"));
        for s in sigs {
            assert!(s.contains("(fuel : Nat)"), "{s}");
        }
    }
}
