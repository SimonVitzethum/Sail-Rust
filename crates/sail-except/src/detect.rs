#![forbid(unsafe_code)]
//! Early-return detection.
//!
//! Ports `Rewriter.has_early_return` (`sail/src/lib/rewriter.ml`):
//!
//! ```ocaml
//! let has_early_return (e : 'a exp) =
//!   let e_app (id, args) = string_of_id id = "early_return"
//!     || List.fold_left ( || ) false args in
//!   let e_return _ = true in
//!   fold_exp { (pure_exp_alg false ( || )) with e_app; e_return } e
//! ```
//!
//! Only surface `return` (elaborated to `E_app (early_return, …)`) and the
//! internal `E_return` node count. Sail exceptions (`E_throw`, `E_try`) and
//! `E_exit` are a separate channel and return `false` here — the backend
//! relies on exactly this split when it picks `sailTryCatchE` over
//! `sailTryCatch` (only `has_early_return` of the tried expression matters).

/// Minimal expression shape sufficient for early-return analysis.
///
/// Generic fold nodes (`App`, `Seq`) stand in for the full typed AST: loop
/// combinators (`while#`, `foreach#`), register reads/writes and pure
/// operators never contribute an early return themselves, so they fold to
/// "any child".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exp {
    /// `E_app (early_return, [arg])` — surface `return e`.
    EarlyReturn(Box<Exp>),
    /// `E_return e` (ANF/internal return).
    Return(Box<Exp>),
    /// `E_throw e` — Sail exception, **not** an early return.
    Throw(Box<Exp>),
    /// `E_exit _` — **not** an early return.
    Exit,
    /// `E_try (tried, cases)`.
    Try { tried: Box<Exp>, arms: Vec<Exp> },
    /// Any other application (loops, register ops, pure operators, and a
    /// literal `early_return` call by name).
    App { name: String, args: Vec<Exp> },
    /// Sequencing / block children (`E_block`, `E_let` bodies, …).
    Seq(Vec<Exp>),
    /// Leaf: literals, variables, asserts, register references, …
    Leaf,
}

impl Exp {
    /// Application node helper.
    pub fn app(name: &str, args: Vec<Exp>) -> Self {
        Self::App {
            name: name.to_owned(),
            args,
        }
    }
}

/// Fold `||` over the tree: `true` iff some node is an early return.
///
/// Direct port of the OCaml fold quoted above.
pub fn has_early_return(e: &Exp) -> bool {
    match e {
        Exp::EarlyReturn(_) | Exp::Return(_) => true,
        Exp::Throw(_) | Exp::Exit | Exp::Leaf => false,
        Exp::Try { tried, arms } => has_early_return(tried) || arms.iter().any(has_early_return),
        Exp::App { name, args } => name == "early_return" || args.iter().any(has_early_return),
        Exp::Seq(es) => es.iter().any(has_early_return),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_return_counts() {
        let e = Exp::EarlyReturn(Box::new(Exp::Leaf));
        assert!(has_early_return(&e));
    }

    #[test]
    fn internal_return_counts() {
        let e = Exp::Return(Box::new(Exp::Leaf));
        assert!(has_early_return(&e));
    }

    #[test]
    fn named_early_return_app_counts() {
        let e = Exp::app("early_return", vec![Exp::Leaf]);
        assert!(has_early_return(&e));
    }

    #[test]
    fn throw_exit_and_try_alone_do_not_count() {
        assert!(!has_early_return(&Exp::Throw(Box::new(Exp::Leaf))));
        assert!(!has_early_return(&Exp::Exit));
        assert!(!has_early_return(&Exp::Leaf));
        let attempt = Exp::Try {
            tried: Box::new(Exp::Throw(Box::new(Exp::Leaf))),
            arms: vec![Exp::Leaf],
        };
        assert!(!has_early_return(&attempt));
    }

    #[test]
    fn nested_return_inside_loop_body_counts() {
        // `foreach (i …) { if i > 5 then return false; … }`
        let body = Exp::app(
            "foreach#",
            vec![Exp::app(
                "if#",
                vec![Exp::EarlyReturn(Box::new(Exp::Leaf)), Exp::Leaf],
            )],
        );
        assert!(has_early_return(&body));
    }

    #[test]
    fn return_in_try_arm_counts_but_tried_alone_does_not() {
        let t = Exp::Try {
            tried: Box::new(Exp::Leaf),
            arms: vec![Exp::EarlyReturn(Box::new(Exp::Leaf))],
        };
        assert!(has_early_return(&t));
    }

    #[test]
    fn seq_finds_return_in_tail() {
        let s = Exp::Seq(vec![Exp::Leaf, Exp::Exit, Exp::Return(Box::new(Exp::Leaf))]);
        assert!(has_early_return(&s));
        let clean = Exp::Seq(vec![Exp::Leaf, Exp::Exit]);
        assert!(!has_early_return(&clean));
    }
}
