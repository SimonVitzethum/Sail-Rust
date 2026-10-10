#![forbid(unsafe_code)]
//! `Id.run`-vs-`do`-Entscheidung nach Effekt.
//!
//! Ports the wrappers and the function-frame choice
//! (cf. `pretty_print_lean.ml`):
//!
//! ```ocaml
//! let wrap_with_pure (needs_return : bool) ?(with_parens = false) (d : document) =
//!   if needs_return then parens (nest 2 (flow space [string "pure"; d])) else d
//!
//! let wrap_with_left_arrow (needs_left_arrow : bool) (d : document) =
//!   if needs_left_arrow then parens (nest 2 (flow space [leftarrow; d])) else d
//!
//! let wrap_with_do (with_arrow : bool) (needs_return : bool) (d : document) =
//!   let ar_do = if with_arrow then string "← do" else string "do" in
//!   if needs_return then parens (nest 2 (flow hardline [ar_do; d])) else d
//! ```
//!
//! ```ocaml
//! let dec_val_end =
//!   match (is_monadic, early_return, has_loop) with
//!   | true, true, _ -> [string "SailME.run"; string "do"]
//!   | true, _, _ -> [string "do"]
//!   | false, false, true -> [string "Id.run"; string "do"]
//!   | false, true, _ -> [string "ExceptM.run"; string "do"]
//!   | _ -> []
//! ```
//!
//! Effect inputs (`has_effect body`, `has_effect cond`) are plain `bool`s
//! here: the OCaml predicates fold over compiler effect info
//! (`effectful (effect_of e) || has_effect_app e`), which this crate
//! deliberately does not depend on.
//!
//! `E_for` without effects is wrapped in `Id.run`
//! (`let enter_monad = if body_effect then empty else string "Id.run"`),
//! e.g. `Id.run for i in [0:(m -i 1):1]i do (nothing ())` vs. effectful
//! `for i in [0:(m -i 1):1]i do writeReg r x`.

/// `(pure d)` when the fragment sits in monadic position (`as_monadic`),
/// `d` unchanged otherwise (`wrap_with_pure`).
pub fn wrap_pure(as_monadic: bool, doc: &str) -> String {
    if as_monadic {
        format!("(pure {doc})")
    } else {
        doc.to_owned()
    }
}

/// `(← d)` when a pure fragment needs monadic binding
/// (`wrap_with_left_arrow`, e.g. `readReg` in pure position).
pub fn wrap_left_arrow(needs_arrow: bool, doc: &str) -> String {
    if needs_arrow {
        format!("(← {doc})")
    } else {
        doc.to_owned()
    }
}

/// `(← do d)` / `(do d)` for loop-valued arguments in non-monadic position
/// (`wrap_with_do`; used when `has_loop` forces a `do`-block).
pub fn wrap_do(with_arrow: bool, needs_return: bool, doc: &str) -> String {
    if !needs_return {
        return doc.to_owned();
    }
    if with_arrow {
        format!("(← do\n{doc})")
    } else {
        format!("(do\n{doc})")
    }
}

/// The `← do` (effectful step) vs `:=` (pure step) choice for the
/// `loop_vars` update line (`leftarrowdo` vs `coloneq` in OCaml).
pub fn update_arrow(body_effects: bool) -> &'static str {
    if body_effects {
        "← do"
    } else {
        ":="
    }
}

/// Function-body frame from the `(is_monadic, early_return, has_loop)`
/// triple (`dec_val_end` in OCaml).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Frame {
    /// `SailME.run do` — stateful with early return.
    SailMeRun,
    /// `do` — plain stateful (`SailM`).
    SailM,
    /// `Id.run do` — pure function containing a loop.
    IdRun,
    /// `ExceptM.run do` — pure function with early return.
    ExceptMRun,
    /// No wrapper — pure, no loop, no early return.
    Pure,
}

impl Frame {
    /// Lean tokens appended after `def f … : T :=`, joined with a space.
    /// Empty for [`Frame::Pure`].
    pub fn suffix(self) -> &'static str {
        match self {
            Frame::SailMeRun => "SailME.run do",
            Frame::SailM => "do",
            Frame::IdRun => "Id.run do",
            Frame::ExceptMRun => "ExceptM.run do",
            Frame::Pure => "",
        }
    }
}

/// Decide the function frame (`dec_val_end` match in OCaml).
pub fn function_frame(is_monadic: bool, early_return: bool, has_loop: bool) -> Frame {
    match (is_monadic, early_return, has_loop) {
        (true, true, _) => Frame::SailMeRun,
        (true, _, _) => Frame::SailM,
        (false, false, true) => Frame::IdRun,
        (false, true, _) => Frame::ExceptMRun,
        (false, false, false) => Frame::Pure,
    }
}

/// `Id.run` prefix for a surface `for` with a pure body
/// (`enter_monad` in the OCaml `E_for` case); `None` when the body has
/// effects and the ambient `do` suffices.
pub fn for_enter_monad(body_effects: bool) -> Option<&'static str> {
    if body_effects {
        None
    } else {
        Some("Id.run")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_pure_adds_pure() {
        assert_eq!(wrap_pure(true, "loop_vars"), "(pure loop_vars)");
        assert_eq!(wrap_pure(false, "loop_vars"), "loop_vars");
    }

    #[test]
    fn left_arrow_wrapping() {
        assert_eq!(wrap_left_arrow(true, "readReg r"), "(← readReg r)");
        assert_eq!(wrap_left_arrow(false, "readReg r"), "readReg r");
    }

    #[test]
    fn do_wrapping_forms() {
        assert_eq!(wrap_do(true, true, "d"), "(← do\nd)");
        assert_eq!(wrap_do(false, true, "d"), "(do\nd)");
        assert_eq!(wrap_do(true, false, "d"), "d");
    }

    #[test]
    fn arrow_follows_body_effects() {
        assert_eq!(update_arrow(true), "← do");
        assert_eq!(update_arrow(false), ":=");
    }

    #[test]
    fn frame_matrix_matches_ocaml() {
        assert_eq!(function_frame(true, true, false), Frame::SailMeRun);
        assert_eq!(function_frame(true, true, true), Frame::SailMeRun);
        assert_eq!(function_frame(true, false, false), Frame::SailM);
        assert_eq!(function_frame(true, false, true), Frame::SailM);
        assert_eq!(function_frame(false, false, true), Frame::IdRun);
        assert_eq!(function_frame(false, true, false), Frame::ExceptMRun);
        assert_eq!(function_frame(false, true, true), Frame::ExceptMRun);
        assert_eq!(function_frame(false, false, false), Frame::Pure);
    }

    #[test]
    fn frame_suffixes() {
        assert_eq!(Frame::SailMeRun.suffix(), "SailME.run do");
        assert_eq!(Frame::SailM.suffix(), "do");
        assert_eq!(Frame::IdRun.suffix(), "Id.run do");
        assert_eq!(Frame::ExceptMRun.suffix(), "ExceptM.run do");
        assert_eq!(Frame::Pure.suffix(), "");
    }

    #[test]
    fn surface_for_needs_id_run_only_when_pure() {
        assert_eq!(for_enter_monad(false), Some("Id.run"));
        assert_eq!(for_enter_monad(true), None);
    }
}
