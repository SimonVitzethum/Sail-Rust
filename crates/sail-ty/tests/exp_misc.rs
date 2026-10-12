//! Lane 27: kleine `Exp`-Arme (`E_field`, `E_ref`, `E_config`, `E_exit`,
//! `E_throw`, `E_assert`, `E_vector_append` nach OCaml `type_check.ml`).
//! Reine `Mini`-Ebene +Resolve/Read-Only-Klassifikation;
//! keine `canonical_body`/`program`/`pat`-Aenderung.
use sail_ty::{
    error::{Span, TypeErrorKind},
    exp_misc::{
        assert_msg_action, assert_obligations, check_config, check_field_base, check_ref,
        classify_vector_append, diverging_obligation, diverging_payload_builtin,
        diverging_payload_child, field_record, mode, AssertMsg, BidirectionalMode, Diverging,
        MiscArm, RefBinding, VectorAppendAction, APPEND_FUN, ASSERT_COND_BUILTIN,
        ASSERT_MSG_BUILTIN, ASSERT_RESULT_BUILTIN, HOOK_NOTE,
    },
    nexp::{MiniTyp, Nexp},
};

fn span() -> Span {
    Span::dummy()
}

fn named(name: &str) -> MiniTyp {
    MiniTyp::Named {
        name: name.to_owned(),
        args: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Modus-Tabelle
// ---------------------------------------------------------------------------

#[test]
fn mode_table_pins_check_vs_infer_only() {
    assert_eq!(mode(MiscArm::Field), BidirectionalMode::InferOnly);
    assert_eq!(mode(MiscArm::Ref), BidirectionalMode::InferOnly);
    assert_eq!(mode(MiscArm::Assert), BidirectionalMode::InferOnly);
    assert_eq!(mode(MiscArm::Config), BidirectionalMode::CheckOnly);
    assert_eq!(mode(MiscArm::Exit), BidirectionalMode::CheckOnly);
    assert_eq!(mode(MiscArm::Throw), BidirectionalMode::CheckOnly);
    assert_eq!(mode(MiscArm::VectorAppend), BidirectionalMode::Both);
}

#[test]
fn hook_note_names_read_only_wiring() {
    assert!(HOOK_NOTE.contains("canonical_body"));
    assert!(HOOK_NOTE.contains("program.rs") || HOOK_NOTE.contains("Program"));
}

// ---------------------------------------------------------------------------
// E_field (infer ~L3748)
// ---------------------------------------------------------------------------

#[test]
fn field_accepts_named_heads_only() {
    let base = named("my_record");
    let (name, args) = field_record(&base).expect("Named ist Record-Kandidat");
    assert_eq!(name, "my_record");
    assert!(args.is_empty());

    let with_args = MiniTyp::Named {
        name: "r".to_owned(),
        args: vec![Nexp::Const(8)],
    };
    let (name, args) = field_record(&with_args).expect("App-Kopf ist Record-Kandidat");
    assert_eq!((name, args.len()), ("r", 1));

    for base in [
        MiniTyp::Int,
        MiniTyp::Bool,
        MiniTyp::Bit,
        MiniTyp::Bits(Box::new(Nexp::Const(8))),
        MiniTyp::Range(Box::new(Nexp::Const(0)), Box::new(Nexp::Const(7))),
        MiniTyp::Tuple(vec![MiniTyp::Int]),
        MiniTyp::Fun(Box::new(MiniTyp::Int), Box::new(MiniTyp::Bool)),
    ] {
        assert!(field_record(&base).is_none(), "kein Record-Kopf: {base}");
    }
}

#[test]
fn field_base_mismatch_names_struct() {
    let err = check_field_base(&MiniTyp::Int, span()).expect_err("int ist kein struct");
    assert!(
        matches!(err.kind, TypeErrorKind::Mismatch { .. }),
        "unerwartet: {:?}",
        err.kind
    );
    let TypeErrorKind::Mismatch { expected, found } = err.kind else {
        unreachable!();
    };
    assert_eq!(expected, "record");
    assert!(found.contains("not a struct"), "unerwartet: {found}");

    let base = named("r");
    let (name, _) = check_field_base(&base, span()).expect("record ok");
    assert_eq!(name, "r");
}

// ---------------------------------------------------------------------------
// E_ref (infer ~L4090)
// ---------------------------------------------------------------------------

#[test]
fn ref_accepts_only_registers() {
    let payload = MiniTyp::Int;
    let out = check_ref("r", &RefBinding::Register(payload.clone()), span()).expect("Register ok");
    assert_eq!(out, payload);

    let local = check_ref("x", &RefBinding::Local, span()).expect_err("kein Local-Fallback");
    assert!(
        matches!(local.kind, TypeErrorKind::RegisterError(_)),
        "unerwartet: {:?}",
        local.kind
    );

    let unbound = check_ref("y", &RefBinding::Unbound, span()).expect_err("unbound");
    assert!(
        matches!(unbound.kind, TypeErrorKind::UnboundVar(_)),
        "unerwartet: {:?}",
        unbound.kind
    );
}

// ---------------------------------------------------------------------------
// E_config (check ~L2365)
// ---------------------------------------------------------------------------

#[test]
fn config_accepts_any_key_list() {
    assert!(check_config(&[]).is_ok());
    assert!(check_config(&["a".to_owned(), "b".to_owned()]).is_ok());
}

// ---------------------------------------------------------------------------
// E_exit / E_throw (check ~L2391 / ~L2394)
// ---------------------------------------------------------------------------

#[test]
fn diverging_payloads_are_unit_and_exc() {
    assert_eq!(diverging_payload_builtin(Diverging::Exit), "unit");
    assert_eq!(diverging_payload_builtin(Diverging::Throw), "exc");
    assert_eq!(diverging_payload_child(Diverging::Exit), "payload");
    assert_eq!(diverging_payload_child(Diverging::Throw), "payload");
    assert_eq!(diverging_obligation(Diverging::Exit), ("payload", "unit"));
    assert_eq!(diverging_obligation(Diverging::Throw), ("payload", "exc"));
}

// ---------------------------------------------------------------------------
// E_assert (infer ~L4039)
// ---------------------------------------------------------------------------

#[test]
fn assert_obligations_are_bool_string_unit() {
    assert_eq!(assert_obligations(), [("cond", "bool"), ("msg", "string")]);
    assert_eq!(ASSERT_COND_BUILTIN, "bool");
    assert_eq!(ASSERT_MSG_BUILTIN, "string");
    assert_eq!(ASSERT_RESULT_BUILTIN, "unit");
}

#[test]
fn assert_msg_coerces_only_empty_string() {
    assert_eq!(assert_msg_action(Some("")), AssertMsg::Location);
    assert_eq!(assert_msg_action(Some("boom")), AssertMsg::Keep);
    assert_eq!(assert_msg_action(Some("  ")), AssertMsg::Keep);
    assert_eq!(assert_msg_action(None), AssertMsg::Keep);
}

// ---------------------------------------------------------------------------
// E_vector_append (check ~L2262 / infer ~L4019)
// ---------------------------------------------------------------------------

#[test]
fn vector_append_identity_only_for_empty_right_literal() {
    assert_eq!(classify_vector_append(true), VectorAppendAction::Identity);
    assert_eq!(
        classify_vector_append(false),
        VectorAppendAction::RewriteAppend
    );
    assert_eq!(APPEND_FUN, "append");
}
