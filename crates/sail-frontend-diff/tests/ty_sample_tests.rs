#![forbid(unsafe_code)]
//! Type-sampling tests (Lane 86, E3: code only, no runs).

use sail_frontend_diff::norm_tree::{NormDef, NormKind};
use sail_frontend_diff::ty_sample::{
    TyDefKind, compare_samples, normalize_scheme, sample_requests, sift_tc_dump,
};

fn def(kind: NormKind, name: Option<&str>) -> NormDef {
    NormDef {
        kind,
        name: name.map(|s| s.to_string()),
        head_norm: "h".to_string(),
    }
}

#[test]
fn normalize_scheme_collapses() {
    assert_eq!(normalize_scheme("  forall 'n,  int ->\r\n bits('n) "), "forall 'n, int -> bits('n)");
}

#[test]
fn sample_requests_first_n_eligible_in_order() {
    let defs = vec![
        def(NormKind::Pragma, Some("x")),
        def(NormKind::Val, Some("f")),
        def(NormKind::Register, Some("R")),
        def(NormKind::Type, Some("t")),
        def(NormKind::Fundef, Some("g")),
        def(NormKind::Val, None), // nameless: skipped
        def(NormKind::Val, Some("h")),
    ];
    let reqs = sample_requests(&defs, 2);
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].def_name, "f");
    assert_eq!(reqs[0].def_kind, TyDefKind::Val);
    assert_eq!(reqs[1].def_name, "R");
    assert_eq!(reqs[1].def_kind, TyDefKind::Register);
}

#[test]
fn sample_requests_zero_is_empty() {
    let defs = vec![def(NormKind::Val, Some("f"))];
    assert!(sample_requests(&defs, 0).is_empty());
}

#[test]
fn compare_samples_match_and_mismatch() {
    let exp = vec![("f".to_string(), "int -> int".to_string())];
    let ok = compare_samples(&exp, &exp);
    assert!(ok.iter().all(|v| v.is_match()));

    let got = vec![("f".to_string(), "bool -> bool".to_string())];
    let v = compare_samples(&exp, &got);
    assert_eq!(v.len(), 1);
    assert!(matches!(v[0].kind, sail_frontend_diff::ty_sample::TyVerdictKind::SchemeMismatch));
    assert!(v[0].detail.contains("-int -> int"), "detail: {}", v[0].detail);
}

#[test]
fn compare_samples_missing_both_directions() {
    let exp = vec![("f".to_string(), "s".to_string())];
    let v = compare_samples(&exp, &[]);
    assert!(matches!(v[0].kind, sail_frontend_diff::ty_sample::TyVerdictKind::MissingInActual));
    let v2 = compare_samples(&[], &exp);
    assert!(matches!(v2[0].kind, sail_frontend_diff::ty_sample::TyVerdictKind::MissingInOracle));
}

#[test]
fn compare_samples_duplicate_is_loud() {
    let exp = vec![("f".to_string(), "s".to_string())];
    let dup = vec![
        ("f".to_string(), "s".to_string()),
        ("f".to_string(), "s".to_string()),
    ];
    let v = compare_samples(&exp, &dup);
    assert!(!v[0].is_match());
    assert!(v[0].detail.contains("duplicate"));
}

#[test]
fn sift_tc_dump_keeps_marked_lines() {
    let dump = "Sail 0.20.3\nval_spec f : typquant forall 'n l3\nplain line\n";
    let got = sift_tc_dump(dump);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "val_spec");
    assert!(got[0].1.contains("typquant"), "scheme: {}", got[0].1);
    // Location fragment stripped.
    assert!(!got[0].1.split_whitespace().any(|t| t == "l3"), "scheme: {}", got[0].1);
}

#[test]
fn ty_def_kind_labels() {
    assert_eq!(TyDefKind::Val.label(), "val");
    assert_eq!(TyDefKind::Register.label(), "register");
    assert_eq!(TyDefKind::Fundef.label(), "function");
}
