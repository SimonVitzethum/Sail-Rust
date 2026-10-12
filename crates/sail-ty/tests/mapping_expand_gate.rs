//! Lane-53: Mapping-Expansion-Gate (`E_app` mapping arms).
//!
//! OCaml `type_check.ml` `check_exp` ~L2299 / `infer_exp` ~L3850: `f(xs)`
//! with `f` a declared mapping rewrites to `<f>_forwards(xs)`; on any
//! `Type_error` it retries `<f>_backwards(xs)`; when both fail the
//! insertion raises both errors (`Err_no_overloading`).
//!
//! These tests pin the public gate surface in `sail_ty::mapping_expand`
//! that the Main-Agent inserts at the top of both `ExpKind::App` arms in
//! `canonical_body.rs` (synthesis + check), before overload resolution:
//! - [`sail_ty::mapping_expand::expansion_names`]: `_forwards` /
//!   `_backwards` convention.
//! - [`sail_ty::mapping_expand::Expansion::from_gate`]: `None` (plain or
//!   overloaded function, `vector_*#` prims) means no rewrite, so the gate
//!   composes with the Lane-16 `vector_prims` hook (mapping gate first,
//!   then vector normalization); `Some` preserves caller-supplied names.
//! - [`sail_ty::mapping_expand::Expansion::expand`]: forwards-first,
//!   backwards fallback on any `TypeError`, combined double-failure error.
//! - [`sail_ty::mapping_expand::combine_errors`]: deterministic
//!   forwards-then-backwards `QuantError` naming the mapping plus both
//!   clause functions with their errors.

use sail_ty::Span;
use sail_ty::error::{TypeError, TypeErrorKind};
use sail_ty::mapping_expand::{Expansion, combine_errors, expand, expansion_names};

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
    let gate = Expansion::named("decode");
    assert_eq!(gate.mapping(), "decode");
    assert_eq!(gate.forwards(), "decode_forwards");
    assert_eq!(gate.backwards(), "decode_backwards");
}

#[test]
fn gate_passes_plain_functions_through() {
    // Plain functions, overloads and `vector_*#` prims (Lane-16 hook) are
    // never declared mappings: `Program::mapping_expansion` yields `None`
    // and the `App` arm keeps its current path.
    for name in ["f", "vector_access", "vector_access#", "vector_subrange#"] {
        assert_eq!(Expansion::from_gate(name, None), None, "{name} rewrites nothing");
    }
}

#[test]
fn gate_keeps_declared_mapping_names_verbatim() {
    let gate = Expansion::from_gate(
        "decode",
        Some(("decode_forwards".to_owned(), "decode_backwards".to_owned())),
    )
    .expect("declared mapping gate");
    assert_eq!(gate, Expansion::named("decode"));
}

#[test]
fn forwards_success_never_runs_backwards() {
    let gate = Expansion::named("decode");
    let mut backwards_ran = false;
    let result: Result<u32, TypeError> = gate.expand(
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
    let gate = Expansion::named("decode");
    let result: Result<String, TypeError> = gate.expand(
        span(),
        || Err(quant_error("forwards exhausted")),
        || Ok("backwards-ok".to_owned()),
    );
    assert_eq!(result, Ok("backwards-ok".to_owned()));
}

#[test]
fn unsupported_forwards_still_retries_backwards() {
    // OCaml catches every `Type_error`, including not-yet-elaborated
    // callees; the gate must not short-circuit on `Unsupported`.
    let gate = Expansion::named("m");
    let result: Result<u32, TypeError> = gate.expand(
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
fn double_failure_combines_both_errors_forwards_first() {
    let gate = Expansion::named("decode");
    let forwards_error = TypeError {
        span: span(),
        kind: TypeErrorKind::Mismatch {
            expected: "int".to_owned(),
            found: "bool".to_owned(),
        },
    };
    let backwards_error = quant_error("backwards constraint unproved");
    let error = gate
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
    assert!(message.contains("decode"), "names the mapping: {message}");
    assert!(message.contains("decode_forwards"), "names forwards: {message}");
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
fn free_functions_match_expansion_methods() {
    let forwards_error = quant_error("e1");
    let backwards_error = quant_error("e2");
    assert_eq!(
        Expansion::named("m").combine(span(), forwards_error.clone(), backwards_error.clone()),
        combine_errors(
            span(),
            "m",
            "m_forwards",
            &forwards_error,
            "m_backwards",
            &backwards_error
        )
    );
    let via_method: Result<u32, TypeError> = Expansion::named("ok").expand(span(), || Ok(3), || Ok(4));
    let via_free: Result<u32, TypeError> =
        expand(span(), "ok", "ok_forwards", "ok_backwards", || Ok(3), || Ok(4));
    assert_eq!(via_method, via_free);
    assert_eq!(via_free, Ok(3));
}
