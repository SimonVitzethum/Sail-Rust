#![forbid(unsafe_code)]
//! Rendering of `early_return` / `exit` / `throw` / `try-catch` / `assert`.
//!
//! Ports the corresponding `doc_exp` cases (`pretty_print_lean.ml`):
//!
//! ```ocaml
//! (* ~line 935 *)
//! | E_app ((Id_aux (Id "early_return", _) as f), [arg]) ->
//!     let throw = if ctx.in_sail_monad then string "SailME.throw "
//!       else string "throw " in
//!     nest 2 (throw ^^ d_of_arg ctx arg)
//! ```
//!
//! ```ocaml
//! (* ~lines 1059-1074 *)
//! | E_exit _ -> string "throw Error.Exit"
//! | E_throw e ->
//!     let arrow = if as_monadic then empty else leftarrow in
//!     arrow ^^ string "sailThrow " ^^ parens (doc_exp false ctx e)
//! | E_try (e, cases) ->
//!     let x = E_aux (E_id (Id_aux (Id "the_exception", Unknown)), …) in
//!     let cases = nest 2 (doc_exp true ctx …) in
//!     let try_catch = if has_early_return e then string "sailTryCatchE "
//!       else string "sailTryCatch " in
//!     let arrow = if as_monadic then empty else leftarrow in
//!     nest 2 (arrow ^^ try_catch
//!       ^^ parens (d_of_arg ~with_arrow:false ctx e)
//!       ^^ space
//!       ^^ parens (string "fun the_exception => " ^^ hardline ^^ cases))
//! | E_assert (e1, e2) -> string "assert " ^^ d_of_arg ctx e1
//!     ^^ space ^^ d_of_arg ctx e2
//! ```
//!
//! Arrow discipline ("keine falschen `←`", the lane's Done-criterion):
//! `E_exit` and `E_assert` never take an arrow; `E_throw` / `E_try` take a
//! `←` prefix **only** in non-monadic position. The prefix is concatenated
//! directly (`^^` in OCaml — no space, no parens), which this port mirrors
//! exactly (`←sailThrow (…)`, `←sailTryCatch (…) (…)`).
//!
//! All functions take already-rendered [`crate::Fragment`]s: `d_of_arg`
//! wrapping (effectful arguments become `(← do …)`, pure ones `(…)`) and
//! match-arm printing stay with the caller.

/// `SailME.throw ` inside `SailM`, plain `throw ` otherwise
/// (`ctx.in_sail_monad` branch of the `early_return` case).
pub fn early_return_throw(in_sail_monad: bool) -> &'static str {
    if in_sail_monad {
        "SailME.throw "
    } else {
        "throw "
    }
}

/// Render `early_return <arg>` with an already-wrapped argument fragment.
pub fn render_early_return(in_sail_monad: bool, arg: &str) -> String {
    format!("{}{arg}", early_return_throw(in_sail_monad))
}

/// Render `E_exit`: bare `throw Error.Exit` in every position.
///
/// The OCaml case applies neither `wrap_with_pure` nor a `←` arrow, so an
/// exit never gains `(pure …)` or `(← …)` — it diverges.
pub fn render_exit() -> &'static str {
    "throw Error.Exit"
}

/// Render `E_throw` with an already-rendered (`as_monadic = false`)
/// payload fragment, e.g. `sailThrow ((E_bool true))`.
pub fn render_throw(as_monadic: bool, payload: &str) -> String {
    if as_monadic {
        format!("sailThrow ({payload})")
    } else {
        format!("←sailThrow ({payload})")
    }
}

/// `sailTryCatchE` when the tried expression contains an early return
/// (`has_early_return e`), else `sailTryCatch`.
pub fn try_catch_combinator(tried_has_early_return: bool) -> &'static str {
    if tried_has_early_return {
        "sailTryCatchE"
    } else {
        "sailTryCatch"
    }
}

/// Render `E_try`: `{arrow}sailTryCatch(E) (<tried>) (fun the_exception =>`
/// followed by the already-rendered match arms on the next line.
///
/// The line keeps OCaml's trailing space (`string "fun the_exception => "`
/// before `hardline`), so output is byte-identical to the reference
/// backend.
///
/// `tried` must be rendered with `with_arrow = false` (for `sailTryCatch`
/// the argument is a computation); `arms` holds the `match the_exception
/// with …` body printed in monadic position.
pub fn render_try_catch(
    as_monadic: bool,
    tried_has_early_return: bool,
    tried: &str,
    arms: &str,
) -> String {
    let arrow = if as_monadic { "" } else { "←" };
    let comb = try_catch_combinator(tried_has_early_return);
    format!("{arrow}{comb} ({tried}) (fun the_exception => \n{arms})")
}

/// Render `E_assert`: `assert <cond> <msg>` with already-rendered fragments.
pub fn render_assert(cond: &str, msg: &str) -> String {
    format!("assert {cond} {msg}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_return_uses_sail_me_throw_in_sail_monad() {
        assert_eq!(early_return_throw(true), "SailME.throw ");
        assert_eq!(early_return_throw(false), "throw ");
        assert_eq!(
            render_early_return(true, "(false : Bool)"),
            "SailME.throw (false : Bool)"
        );
        assert_eq!(
            render_early_return(false, "(false : Bool)"),
            "throw (false : Bool)"
        );
    }

    #[test]
    fn exit_is_bare_everywhere() {
        assert_eq!(render_exit(), "throw Error.Exit");
    }

    #[test]
    fn throw_arrow_only_outside_monad() {
        assert_eq!(
            render_throw(true, "(E_bool true)"),
            "sailThrow ((E_bool true))"
        );
        assert_eq!(
            render_throw(false, "(E_bool true)"),
            "←sailThrow ((E_bool true))"
        );
    }

    #[test]
    fn try_catch_combinator_follows_early_return() {
        assert_eq!(try_catch_combinator(true), "sailTryCatchE");
        assert_eq!(try_catch_combinator(false), "sailTryCatch");
    }

    #[test]
    fn try_catch_shape() {
        let out = render_try_catch(
            true,
            false,
            "sailThrow ((E_bool true))",
            "match the_exception with\n| E_bool b => (pure b)",
        );
        assert_eq!(
            out,
            "sailTryCatch (sailThrow ((E_bool true))) (fun the_exception => \nmatch the_exception with\n| E_bool b => (pure b))"
        );
        let out_e = render_try_catch(true, true, "t", "arms");
        assert!(out_e.starts_with("sailTryCatchE (t)"), "{out_e}");
        let out_pure = render_try_catch(false, false, "t", "arms");
        assert!(out_pure.starts_with("←sailTryCatch (t)"), "{out_pure}");
    }

    #[test]
    fn assert_shape() {
        assert_eq!(
            render_assert("b", "\"b is false\""),
            "assert b \"b is false\""
        );
    }
}
