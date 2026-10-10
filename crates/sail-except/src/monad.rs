#![forbid(unsafe_code)]
//! `SailM` / `SailME` / `ExceptM` monad mapping.
//!
//! Ports the signature and frame rules from `pretty_print_lean.ml`:
//!
//! ```ocaml
//! (* doc_funcl_init, ~line 1164 *)
//! let doc_ret_typ = if is_monadic then string "SailM " ^^ doc_ret_typ_orig
//!   else doc_ret_typ_orig
//! ```
//!
//! ```ocaml
//! (* doc_funcl_init, ~line 1167 *)
//! let dec_val_end =
//!   match (is_monadic, early_return, has_loop) with
//!   | true, true, _ -> [string "SailME.run"; string "do"]
//!   | true, _, _ -> [string "do"]
//!   | false, false, true -> [string "Id.run"; string "do"]
//!   | false, true, _ -> [string "ExceptM.run"; string "do"]
//!   | _ -> []
//! ```
//!
//! ```ocaml
//! (* prepend_monad, ~line 726 *)
//! match (ctx.in_sail_monad, ctx.in_except_monad) with
//! | true, Some ty -> [string "SailME"; ty; doc]
//! | true, None -> [string "SailM"; doc]
//! | false, Some ty -> [string "ExceptM"; ty; doc]
//! | false, None -> [string "Id"; doc]
//! ```
//!
//! Effect inputs are plain `bool`s: the OCaml predicates fold over compiler
//! effect info (`effectful (effect_of e)`, `Effects.function_is_pure`),
//! which this crate deliberately does not depend on.

/// Lean return type of a translated function.
///
/// Stateful functions return `SailM Ret`; pure ones return the plain `Ret`
/// (an early return then runs in `ExceptM`, see [`function_frame`]).
pub fn return_type(is_monadic: bool, ret: &str) -> String {
    if is_monadic {
        format!("SailM {ret}")
    } else {
        ret.to_owned()
    }
}

/// Function-body frame from the `(is_monadic, early_return, has_loop)`
/// triple (`dec_val_end` in OCaml): the tokens appended after
/// `def f … : T :=`, joined with a space.
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
    /// Lean tokens appended after `def f … : T :=`. Empty for [`Frame::Pure`].
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

/// Monad ascription for a `let`-bound typed binding (`prepend_monad`).
///
/// `except_ret` is `Some` exactly when the enclosing function has an early
/// return (OCaml `in_except_monad`), holding the function's original return
/// type; `ascribed` is the binding's own type ascription. Produces e.g.
/// `SailME E E` (as in `let y ← ((do …) : SailME E E)`) or
/// `ExceptM Bool Nat`.
pub fn ascribed_monad(in_sail_monad: bool, except_ret: Option<&str>, ascribed: &str) -> String {
    match (in_sail_monad, except_ret) {
        (true, Some(ret)) => format!("SailME {ret} {ascribed}"),
        (true, None) => format!("SailM {ascribed}"),
        (false, Some(ret)) => format!("ExceptM {ret} {ascribed}"),
        (false, None) => format!("Id {ascribed}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_type_gains_sail_m_only_when_monadic() {
        assert_eq!(return_type(true, "Bool"), "SailM Bool");
        assert_eq!(return_type(false, "Bool"), "Bool");
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
    fn ascription_matrix_matches_prepend_monad() {
        // `let y ← ((do …) : SailME E E)` (match_early_return_inloop_2).
        assert_eq!(ascribed_monad(true, Some("E"), "E"), "SailME E E");
        // `… : ExceptM Bool Nat` (foreach_earlyreturnpure loop header).
        assert_eq!(
            ascribed_monad(false, Some("Bool"), "Nat"),
            "ExceptM Bool Nat"
        );
        assert_eq!(ascribed_monad(true, None, "Unit"), "SailM Unit");
        assert_eq!(ascribed_monad(false, None, "Nat"), "Id Nat");
    }
}
