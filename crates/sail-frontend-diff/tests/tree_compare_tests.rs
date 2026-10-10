#![forbid(unsafe_code)]
//! Positional tree-comparison tests (Lane 86, E3: code only, no runs).

use sail_frontend_diff::norm_tree::{NormDef, NormKind};
use sail_frontend_diff::tree_compare::{DefVerdict, compare, render_seq_diff, summarize};

fn def(kind: NormKind, name: &str, head: &str) -> NormDef {
    NormDef {
        kind,
        name: Some(name.to_string()),
        head_norm: head.to_string(),
    }
}

fn val_f() -> NormDef {
    def(NormKind::Val, "f", "f : int -> int")
}

#[test]
fn identical_is_clean() {
    let r = compare(&[val_f()], &[val_f()]);
    assert!(r.is_clean());
    assert_eq!(r.mismatch_count(), 0);
    assert_eq!(render_seq_diff(&[val_f()], &[val_f()]), "");
    assert!(summarize(&r).contains("1/1"));
}

#[test]
fn kind_mismatch() {
    let r = compare(&[val_f()], &[def(NormKind::Register, "f", "f : int")]);
    assert!(!r.is_clean());
    assert!(matches!(r.verdicts[0], DefVerdict::KindMismatch { .. }));
    assert_eq!(r.verdicts[0].reason(), "def-kind mismatch");
}

#[test]
fn name_mismatch() {
    let r = compare(&[val_f()], &[def(NormKind::Val, "g", "f : int -> int")]);
    assert!(matches!(r.verdicts[0], DefVerdict::NameMismatch { .. }));
}

#[test]
fn head_mismatch_same_kind_and_name() {
    let r = compare(&[val_f()], &[def(NormKind::Val, "f", "f : bool")]);
    assert!(matches!(r.verdicts[0], DefVerdict::HeadMismatch { .. }));
    let d = render_seq_diff(&[val_f()], &[def(NormKind::Val, "f", "f : bool")]);
    assert!(d.contains("-val:f"), "diff: {d}");
    assert!(d.contains("+val:f"), "diff: {d}");
}

#[test]
fn length_tails() {
    let two = vec![val_f(), def(NormKind::Type, "t", "t = int")];
    let r = compare(&two, &[val_f()]);
    assert!(matches!(r.verdicts[1], DefVerdict::MissingInActual { .. }));
    let r2 = compare(&[val_f()], &two);
    assert!(matches!(r2.verdicts[1], DefVerdict::ExtraInActual { .. }));
}

#[test]
fn empty_both_is_clean() {
    let r = compare(&[], &[]);
    assert!(r.is_clean());
    assert_eq!(r.mismatch_count(), 0);
}

#[test]
fn positional_not_set_semantics() {
    // Same multiset, swapped order: positional compare must flag both slots.
    let a = vec![val_f(), def(NormKind::Type, "t", "t = int")];
    let b = vec![def(NormKind::Type, "t", "t = int"), val_f()];
    let r = compare(&a, &b);
    assert_eq!(r.mismatch_count(), 2);
}

#[test]
fn verdict_reasons_cover_all_variants() {
    let kinds = [
        DefVerdict::Match,
        DefVerdict::KindMismatch { index: 0, expected: "val", actual: "type" },
        DefVerdict::NameMismatch { index: 0, expected: None, actual: None },
        DefVerdict::HeadMismatch { index: 0, name: None },
        DefVerdict::MissingInActual { index: 0, expected: String::new() },
        DefVerdict::ExtraInActual { index: 0, actual: String::new() },
    ];
    for v in &kinds {
        assert!(!v.reason().is_empty());
    }
    assert!(DefVerdict::Match.is_match());
}
