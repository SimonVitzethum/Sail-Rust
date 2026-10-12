#![forbid(unsafe_code)]
//! Mapping-application expansion gate (`E_app` mapping arms).
//!
//! OCaml `type_check.ml` `check_exp` ~L2299 and `infer_exp` ~L3850:
//! `f(xs)` with `f` a declared mapping rewrites to `<f>_forwards(xs)`;
//! on `Type_error` it retries `<f>_backwards(xs)`; when both fail the
//! insertion raises both errors (`Err_no_overloading`).
//!
//! This module is the pure gate the Main-Agent inserts at the top of both
//! `ExpKind::App` arms in `canonical_body.rs` (infer `Synthesis::Enter`
//! and check `Task`), before overload resolution:
//!
//! ```ignore
//! // Zuoberst im `ExpKind::App`-Arm (beide Modi), vor `call_candidates`:
//! let func_name = self.name(func)?;
//! if let Some(expansion) = crate::mapping_expand::Expansion::from_gate(
//!     func_name,
//!     self.program.mapping_expansion(func_name, self.position),
//! ) {
//!     // Re-dispatch: the insertion synthesizes the rewritten `App` node
//!     // (`<m>_forwards(xs)` / `<m>_backwards(xs)`) and feeds it back
//!     // through the `App` arm; check mode threads `Some(expected)` as
//!     // the return context, infer mode threads `None`.
//!     return expansion.expand(exp.span,
//!         || self.check_mapping_direction(&forwards_app, expected.clone(), scope, returning),
//!         || self.check_mapping_direction(&backwards_app, expected.clone(), scope, returning));
//! }
//! ```
//!
//! Requires the Lane-19 `Program::mappings` table (`program.rs` in the
//! main worktree): `is_mapping` gates the rewrite, `mapping_expansion`
//! supplies the generated clause-function names. This file stays pure
//! (`String` in, gate out, no `Program` access) so it compiles without
//! that table; [`Expansion::from_gate`] bridges the
//! `Program::mapping_expansion` `Option<(String, String)>` to the gate.

use crate::error::{Span, TypeError, TypeErrorKind, TypeResult};

/// Generated clause-function names for a mapping application.
///
/// Mirrors the OCaml `mk_id (name ^ "_forwards")` / `mk_id (name ^
/// "_backwards")` convention and matches `Program::mapping_expansion`
/// (main-worktree reference) as well as the `exp_arms` planner
/// convention: `(format!("{mapping}_forwards"), format!("{mapping}_backwards"))`.
#[must_use]
pub fn expansion_names(mapping: &str) -> (String, String) {
    (
        format!("{mapping}_forwards"),
        format!("{mapping}_backwards"),
    )
}

/// Forwards-first / backwards-fallback gate for one mapping application.
///
/// Constructed from the `Program::mapping_expansion` gate output via
/// [`Expansion::from_gate`]: `None` (not a declared mapping) means the
/// `App` arm continues with normal overload resolution; `Some` means the
/// arm re-dispatches through [`Expansion::expand`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expansion {
    mapping: String,
    forwards: String,
    backwards: String,
}

impl Expansion {
    /// Gate with explicit clause-function names.
    #[must_use]
    pub fn new(mapping: String, forwards: String, backwards: String) -> Self {
        Self {
            mapping,
            forwards,
            backwards,
        }
    }

    /// Gate deriving the clause names from the mapping name via
    /// [`expansion_names`].
    #[must_use]
    pub fn named(mapping: &str) -> Self {
        let (forwards, backwards) = expansion_names(mapping);
        Self {
            mapping: mapping.to_owned(),
            forwards,
            backwards,
        }
    }

    /// Bridge from the `Program::mapping_expansion` gate to this gate.
    ///
    /// Returns `None` exactly when `gate` is `None` (plain or overloaded
    /// function: no rewrite, the `App` arm keeps its current path).
    /// A `Some` gate with caller-supplied names is preserved verbatim so
    /// the insertion only rewrites declared mappings and never invents
    /// clause names for other functions.
    #[must_use]
    pub fn from_gate(mapping: &str, gate: Option<(String, String)>) -> Option<Self> {
        gate.map(|(forwards, backwards)| Self {
            mapping: mapping.to_owned(),
            forwards,
            backwards,
        })
    }

    /// The applied mapping name (OCaml `Err_no_overloading` head).
    #[must_use]
    pub fn mapping(&self) -> &str {
        &self.mapping
    }

    /// The forwards clause-function name (tried first).
    #[must_use]
    pub fn forwards(&self) -> &str {
        &self.forwards
    }

    /// The backwards clause-function name (fallback).
    #[must_use]
    pub fn backwards(&self) -> &str {
        &self.backwards
    }

    /// Combined double-failure error (`Err_no_overloading` equivalent).
    ///
    /// Deterministic `QuantError` naming the mapping plus both clause
    /// functions with their errors, forwards first:
    /// `no matching overload for mapping '<m>': <m>_forwards: <e1>;
    /// <m>_backwards: <e2>`.
    #[must_use]
    pub fn combine(
        &self,
        span: Span,
        forwards_error: TypeError,
        backwards_error: TypeError,
    ) -> TypeError {
        combine_errors(
            span,
            &self.mapping,
            &self.forwards,
            &forwards_error,
            &self.backwards,
            &backwards_error,
        )
    }

    /// Forwards-first with backwards fallback.
    ///
    /// Tries `try_forwards` (`<m>_forwards(xs)` re-dispatch); on any
    /// `TypeError` (OCaml catches every `Type_error`) retries with
    /// `try_backwards` (`<m>_backwards(xs)`); when both fail returns the
    /// combined [`Expansion::combine`] error. Generic over `T` so both
    /// `App` arms share it (check mode threads `Some(expected)`,
    /// infer mode threads `None` inside the closures).
    pub fn expand<T>(
        &self,
        span: Span,
        try_forwards: impl FnOnce() -> TypeResult<T>,
        try_backwards: impl FnOnce() -> TypeResult<T>,
    ) -> TypeResult<T> {
        match try_forwards() {
            ok @ Ok(_) => ok,
            Err(forwards_error) => match try_backwards() {
                ok @ Ok(_) => ok,
                Err(backwards_error) => Err(self.combine(span, forwards_error, backwards_error)),
            },
        }
    }
}

/// Combined double-failure error without an [`Expansion`] value.
///
/// Same format as [`Expansion::combine`]; the free function keeps call
/// sites readable when the names are already at hand.
#[must_use]
pub fn combine_errors(
    span: Span,
    mapping: &str,
    forwards: &str,
    forwards_error: &TypeError,
    backwards: &str,
    backwards_error: &TypeError,
) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::QuantError(format!(
            "no matching overload for mapping '{mapping}': {forwards}: {forwards_error}; {backwards}: {backwards_error}"
        )),
    }
}

/// Free-function twin of [`Expansion::expand`] for call sites holding
/// plain names instead of an [`Expansion`] value.
pub fn expand<T>(
    span: Span,
    mapping: &str,
    forwards: &str,
    backwards: &str,
    try_forwards: impl FnOnce() -> TypeResult<T>,
    try_backwards: impl FnOnce() -> TypeResult<T>,
) -> TypeResult<T> {
    Expansion::new(
        mapping.to_owned(),
        forwards.to_owned(),
        backwards.to_owned(),
    )
    .expand(span, try_forwards, try_backwards)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::TypeErrorKind;

    fn span() -> Span {
        Span::DUMMY
    }

    fn quant_error(message: &str) -> TypeError {
        TypeError {
            span: span(),
            kind: TypeErrorKind::QuantError(message.to_owned()),
        }
    }

    #[test]
    fn expansion_names_follow_forwards_backwards_convention() {
        assert_eq!(
            expansion_names("decode"),
            ("decode_forwards".to_owned(), "decode_backwards".to_owned())
        );
        assert_eq!(Expansion::named("decode").forwards(), "decode_forwards");
        assert_eq!(Expansion::named("decode").backwards(), "decode_backwards");
        assert_eq!(Expansion::named("decode").mapping(), "decode");
    }

    #[test]
    fn from_gate_passes_none_through_and_keeps_names() {
        assert_eq!(Expansion::from_gate("f", None), None);
        let expansion = Expansion::from_gate(
            "decode",
            Some(("decode_forwards".to_owned(), "decode_backwards".to_owned())),
        )
        .expect("declared mapping gate");
        assert_eq!(expansion, Expansion::named("decode"));
        // Caller-supplied names are preserved verbatim, never re-derived.
        let custom = Expansion::from_gate("m", Some(("fwd".to_owned(), "bwd".to_owned())))
            .expect("custom gate");
        assert_eq!(custom.forwards(), "fwd");
        assert_eq!(custom.backwards(), "bwd");
    }

    #[test]
    fn forwards_success_never_runs_backwards() {
        let expansion = Expansion::named("decode");
        let mut backwards_ran = false;
        let result: TypeResult<u32> = expansion.expand(
            span(),
            || Ok(7),
            || {
                backwards_ran = true;
                Ok(8)
            },
        );
        assert_eq!(result, Ok(7));
        assert!(!backwards_ran, "forwards-first: no fallback on success");
    }

    #[test]
    fn backwards_fallback_returns_backwards_value() {
        let expansion = Expansion::named("decode");
        let result: TypeResult<String> = expansion.expand(
            span(),
            || Err(quant_error("forwards exhausted")),
            || Ok("backwards-ok".to_owned()),
        );
        assert_eq!(result, Ok("backwards-ok".to_owned()));
    }

    #[test]
    fn double_failure_combines_both_errors_forwards_first() {
        let expansion = Expansion::named("decode");
        let forwards_error = TypeError {
            span: span(),
            kind: TypeErrorKind::Mismatch {
                expected: "int".to_owned(),
                found: "bool".to_owned(),
            },
        };
        let backwards_error = quant_error("backwards constraint unproved");
        let error = expansion
            .expand::<u32>(
                span(),
                || Err(forwards_error.clone()),
                || Err(backwards_error.clone()),
            )
            .expect_err("both directions fail");
        assert_eq!(error.span, span());
        let TypeErrorKind::QuantError(message) = &error.kind else {
            panic!("double failure must be a QuantError, got {error:?}");
        };
        let message = message.clone();
        assert!(message.contains("decode"), "names the mapping: {message}");
        assert!(
            message.contains("decode_forwards"),
            "names forwards: {message}"
        );
        assert!(
            message.contains("decode_backwards"),
            "names backwards: {message}"
        );
        assert!(
            message.contains(&forwards_error.to_string()),
            "keeps forwards error: {message}"
        );
        assert!(
            message.contains(&backwards_error.to_string()),
            "keeps backwards error: {message}"
        );
        assert!(
            message.find("decode_forwards").expect("forwards named")
                < message.find("decode_backwards").expect("backwards named"),
            "deterministic forwards-then-backwards order: {message}"
        );
    }

    #[test]
    fn unsupported_forwards_still_retries_backwards() {
        // OCaml catches every `Type_error`, including not-yet-elaborated
        // callees; the gate must not short-circuit on `Unsupported`.
        let expansion = Expansion::named("m");
        let result: TypeResult<u32> = expansion.expand(
            span(),
            || {
                Err(TypeError {
                    span: span(),
                    kind: TypeErrorKind::Unsupported("mapping expansion pending".to_owned()),
                })
            },
            || Ok(1),
        );
        assert_eq!(result, Ok(1));
    }

    #[test]
    fn free_functions_match_expansion_methods() {
        let span = span();
        let forwards_error = quant_error("e1");
        let backwards_error = quant_error("e2");
        let via_method =
            Expansion::named("m").combine(span, forwards_error.clone(), backwards_error.clone());
        let via_free = combine_errors(
            span,
            "m",
            "m_forwards",
            &forwards_error,
            "m_backwards",
            &backwards_error,
        );
        assert_eq!(via_method, via_free);
        let via_method_run: TypeResult<u32> =
            Expansion::named("ok").expand(span, || Ok(3), || Ok(4));
        let via_free_run: TypeResult<u32> = expand(
            span,
            "ok",
            "ok_forwards",
            "ok_backwards",
            || Ok(3),
            || Ok(4),
        );
        assert_eq!(via_method_run, via_free_run);
        assert_eq!(via_free_run, Ok(3));
    }
}
