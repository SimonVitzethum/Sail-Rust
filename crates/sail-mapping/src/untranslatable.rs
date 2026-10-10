//! `untranslatable_mapping` port (`sail/src/sail_lean_backend/pretty_print_lean.ml`,
//! lines 1188–1214).
//!
//! The OCaml original:
//!
//! ```ocaml
//! let mapping_regex = Str.regexp "_\\(forward\\|backwards\\)\\(_matches\\)?$"
//!
//! (* Mappings with string concatenation on the LHS are not translated to
//! executable code by sail, so we detect the pattern to replace the forward mapping
//! direction by a function that throws an exception. cf #260 *)
//! let untranslatable_mapping id exp =
//!   let rec exp_disc e =
//!     match e with
//!     | E_aux (E_let (_, _, e), _) -> exp_disc e
//!     | E_aux (E_app (_, [E_aux (E_exit _, _)]), _) -> true
//!     | _ -> false
//!   in
//!   let rec exp_match e =
//!     match e with E_aux (E_let (_, _, e), _) -> exp_match e | E_aux (E_match (e, _), _) -> exp_match e | _ -> false
//!   in
//!
//!   let id = string_of_id id in
//!   if Str.string_match mapping_regex id 0 then false else exp_match exp
//! ```
//!
//! Note the two subtleties preserved below:
//!
//! 1. `Str.string_match ... id 0` anchors at the *start* of the name, so only
//!    identifiers literally beginning with `_forward` / `_backwards`
//!    (optionally suffixed by `_matches`) are exempt — i.e. the generated
//!    mapping helpers *themselves* (`*_forwards`, `*_backwards`, ...; note the
//!    OCaml regex uses the singular `forward`) never take the `throw` path
//!    just because of their name. Realized bodies such as
//!    `vtype_assembly_forwards` do *not* match the prefix and are therefore
//!    still examined.
//! 2. Both the outer body and the match scrutinee are unwrapped through
//!    leading `let` bindings before testing for `f(exit)`.
//!
//! The printer rule (`doc_funcl_body`) is: if `untranslatable_mapping id exp`
//! then emit `throw Error.Exit`, else emit the translated body.

#![forbid(unsafe_code)]

use crate::ast::Exp;

/// Exemption test for the OCaml `mapping_regex` anchored at position 0:
/// `_forward` | `_backwards`, each optionally followed by `_matches`, then
/// end of string.
pub fn is_mapping_helper_exempt(id: &str) -> bool {
    let rest = match id.strip_prefix('_') {
        Some(rest) => rest,
        None => return false,
    };
    let rest = match rest.strip_prefix("forward") {
        Some(rest) => rest,
        None => match rest.strip_prefix("backwards") {
            Some(rest) => rest,
            None => return false,
        },
    };
    rest.is_empty() || rest == "_matches"
}

/// Unwrap leading `let _ = _ in body` bindings.
fn strip_lets(exp: &Exp) -> &Exp {
    let mut e = exp;
    while let Exp::Let { body, .. } = e {
        e = body;
    }
    e
}

/// `exp_disc`: after stripping lets, is this `f(exit)`?
fn is_exit_app(exp: &Exp) -> bool {
    match strip_lets(exp) {
        Exp::App(_, args) => matches!(args.as_slice(), [single] if *single == Exp::Exit),
        _ => false,
    }
}

/// Whether the function body for `id` must be replaced by
/// `throw Error.Exit` (`untranslatable_mapping`).
pub fn untranslatable_mapping(id: &str, exp: &Exp) -> bool {
    if is_mapping_helper_exempt(id) {
        return false;
    }
    match strip_lets(exp) {
        Exp::Match { scrutinee, .. } => is_exit_app(scrutinee),
        _ => false,
    }
}

/// Printer rule for a function-clause body (`doc_funcl_body`): either the
/// `throw` replacement or a marker carrying the translated body.
///
/// The `Ok` payload stands in for `doc_exp ... exp`; the printer crate owns
/// the real rendering.
pub fn render_funcl_body<'a>(id: &str, exp: &'a Exp) -> Result<&'a Exp, &'static str> {
    if untranslatable_mapping(id, exp) {
        Err("throw Error.Exit")
    } else {
        Ok(exp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Arm, Pat};

    fn exit_app() -> Exp {
        Exp::App(
            "internal_exit".to_string(),
            vec![Exp::Exit],
        )
    }

    fn match_on(exp: Exp) -> Exp {
        Exp::Match {
            scrutinee: Box::new(exp),
            arms: vec![Arm::plain(Pat::Wild, Exp::Bool(true))],
        }
    }

    /// `mapping.expected.lean` line 347-348: `vtype_assembly_forwards` is
    /// `throw Error.Exit` — string concatenation on the LHS leaves an
    /// `exit` scrutinee in the realized forwards body.
    #[test]
    fn string_append_forwards_is_untranslatable() {
        let body = match_on(exit_app());
        assert!(untranslatable_mapping("vtype_assembly_forwards", &body));
        assert_eq!(
            render_funcl_body("vtype_assembly_forwards", &body),
            Err("throw Error.Exit")
        );
    }

    /// Same file, lines 358-359: `vtype_assembly_forwards_matches` likewise.
    /// The `_matches` suffix on a *realized* name does not exempt it (the
    /// OCaml regex only matches names *starting* with `_forward`).
    #[test]
    fn realized_matches_suffix_is_not_exempt() {
        assert!(!is_mapping_helper_exempt("vtype_assembly_forwards_matches"));
        let body = match_on(exit_app());
        assert!(untranslatable_mapping(
            "vtype_assembly_forwards_matches",
            &body
        ));
    }

    /// `mapping.expected.lean` lines 350-356: `vtype_assembly_backwards` keeps
    /// its real body — a match on the tuple argument, not on `exit`.
    #[test]
    fn backwards_direction_stays_translatable() {
        let body = match_on(Exp::Id("arg_".to_string()));
        assert!(!untranslatable_mapping("vtype_assembly_backwards", &body));
        assert!(render_funcl_body("vtype_assembly_backwards", &body).is_ok());
    }

    /// Leading `let`s are transparent, on both the body and the scrutinee.
    #[test]
    fn lets_are_stripped_on_both_levels() {
        let body = Exp::Let {
            bound: Pat::Var("head_exp_".to_string()),
            value: Box::new(Exp::Id("arg_".to_string())),
            body: Box::new(Exp::Let {
                bound: Pat::Wild,
                value: Box::new(Exp::Bool(true)),
                body: Box::new(match_on(Exp::Let {
                    bound: Pat::Wild,
                    value: Box::new(Exp::Bool(false)),
                    body: Box::new(exit_app()),
                })),
            }),
        };
        assert!(untranslatable_mapping("vtype_assembly3_forwards", &body));
    }

    #[test]
    fn non_match_bodies_are_translatable() {
        assert!(!untranslatable_mapping("size_bits_forwards", &Exp::Id("x".to_string())));
        assert!(!untranslatable_mapping(
            "size_bits_forwards",
            &Exp::Let {
                bound: Pat::Wild,
                value: Box::new(Exp::Bool(true)),
                body: Box::new(Exp::Id("x".to_string())),
            }
        ));
    }

    /// Only names starting with `_forward` / `_backwards` (singular
    /// `forward`, optional `_matches` tail) are exempt — exactly the anchored
    /// OCaml regex.
    #[test]
    fn exemption_matches_anchored_regex() {
        assert!(is_mapping_helper_exempt("_forward"));
        assert!(is_mapping_helper_exempt("_backwards"));
        assert!(is_mapping_helper_exempt("_forward_matches"));
        assert!(is_mapping_helper_exempt("_backwards_matches"));
        assert!(!is_mapping_helper_exempt("size_bits_forwards"));
        assert!(!is_mapping_helper_exempt("hex_bits_forwards"));
        // Plural `_forwards` with a leading underscore still does not match:
        // the regex spells the singular `forward` followed by end/`_matches`.
        assert!(!is_mapping_helper_exempt("_forwards"));
        assert!(!is_mapping_helper_exempt("forward"));
        assert!(!is_mapping_helper_exempt("_forward_matches_extra"));
    }
}
