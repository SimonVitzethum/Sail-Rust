#![forbid(unsafe_code)]
//! `E_for` mapping for Lean emission.
//!
//! Ports the `E_for` case of `doc_exp` in
//! `sail/src/sail_lean_backend/pretty_print_lean.ml` (lines 919-934):
//! - `Ord_inc` uses the `foreach_Z_up` direction, `Ord_dec` uses
//!   `foreach_Z_down`;
//! - the bound bracket is `[from:to:step]i`; for `Ord_dec` the step is
//!   negated (`-step`);
//! - a pure body (`Id.run` context) prefixes the head with `Id.run`, an
//!   effectful body emits the bare `for` head (the caller is monadic).

/// Loop direction (`Ord_aux` in the Sail AST).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Order {
    /// `in inc` / ascending.
    Inc,
    /// `in dec` / descending.
    Dec,
}

impl Order {
    /// Sail prelude combinator for the direction.
    pub fn combinator(self) -> &'static str {
        match self {
            Order::Inc => "foreach_Z_up",
            Order::Dec => "foreach_Z_down",
        }
    }
}

/// An `E_for` loop head: already-printed Lean fragments for the bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForLoop {
    /// Loop variable name.
    pub var: String,
    /// Printed `from` expression.
    pub from: String,
    /// Printed `to` expression.
    pub to: String,
    /// Printed `step` expression (unsigned; negated for [`Order::Dec`]).
    pub step: String,
    /// Loop direction.
    pub order: Order,
    /// `true` when the body has an effect (register write, throw, ...).
    pub body_effect: bool,
}

impl ForLoop {
    /// Bound bracket `[from:to:step]i`, negating the step for `Ord_dec`.
    pub fn bracket(&self) -> String {
        let step = match self.order {
            Order::Inc => self.step.clone(),
            Order::Dec => format!("-{}", self.step),
        };
        format!("[{}:{}:{}]i", self.from, self.to, step)
    }

    /// Full head: `[Id.run ]for <var> in <bracket> do`.
    pub fn head(&self) -> String {
        let prefix = if self.body_effect { "" } else { "Id.run " };
        format!("{prefix}for {} in {} do", self.var, self.bracket())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pure_inc() -> ForLoop {
        ForLoop {
            var: "i".to_string(),
            from: "0".to_string(),
            to: "(m -i 1)".to_string(),
            step: "1".to_string(),
            order: Order::Inc,
            body_effect: false,
        }
    }

    #[test]
    fn combinators() {
        assert_eq!(Order::Inc.combinator(), "foreach_Z_up");
        assert_eq!(Order::Dec.combinator(), "foreach_Z_down");
    }

    #[test]
    fn pure_inc_head() {
        // Shape of the inner loop in `foreachpure` (`loop.sail`).
        assert_eq!(
            pure_inc().head(),
            "Id.run for i in [0:(m -i 1):1]i do"
        );
    }

    #[test]
    fn dec_negates_step() {
        let l = ForLoop {
            order: Order::Dec,
            body_effect: false,
            ..pure_inc()
        };
        assert_eq!(l.head(), "Id.run for i in [0:(m -i 1):-1]i do");
    }

    #[test]
    fn effectful_drops_id_run() {
        let l = ForLoop {
            body_effect: true,
            ..pure_inc()
        };
        assert_eq!(l.head(), "for i in [0:(m -i 1):1]i do");
    }
}
