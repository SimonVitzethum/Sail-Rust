//! Fixture tests: SVE/SME shard parse, pattern catalog, fail spans.
//!
//! Fixtures are tiny but structurally faithful excerpts (val + execute +
//! decode with `foreach`, predication, `assert(constraint(...))`, `throw`,
//! `match VL`; SME with nested `foreach` + tile accessor).

use sail_sve_sme::{FailKind, PatternKind, analyze_file, analyze_files_parallel};

fn fixture(name: &str) -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}")).unwrap()
}

#[test]
fn sve_fixture_shard_shape() {
    let src = fixture("sve_mini.sail");
    let r = analyze_file("sve_mini.sail", &src).unwrap();
    assert_eq!(r.def_total, 3);
    assert_eq!(r.val_count, 1);
    assert_eq!(r.fun_count, 2);
    assert_eq!(r.other_count, 0);
    assert_eq!(r.lines, 24);
}

#[test]
fn sve_fixture_pattern_catalog() {
    let src = fixture("sve_mini.sail");
    let r = analyze_file("sve_mini.sail", &src).unwrap();
    let c = |k: PatternKind| r.pattern_count(k);
    assert_eq!(c(PatternKind::ForeachInc), 1);
    assert_eq!(c(PatternKind::ForeachDec), 0);
    assert_eq!(c(PatternKind::NestedForeach), 0);
    assert_eq!(c(PatternKind::ElemRead), 1);
    assert_eq!(c(PatternKind::ElemSet), 1);
    assert_eq!(c(PatternKind::PredicatedExec), 0);
    assert_eq!(c(PatternKind::MaskRead), 1);
    assert_eq!(c(PatternKind::VecRegWrite), 1);
    assert_eq!(c(PatternKind::VlDispatch), 1);
    assert_eq!(c(PatternKind::ConstraintAssert), 1);
    assert_eq!(c(PatternKind::AssertGeneric), 1);
    assert_eq!(c(PatternKind::ThrowUndef), 2);
    assert_eq!(c(PatternKind::UndefinedInit), 1);
    assert_eq!(c(PatternKind::CheckEnabled), 1);
    assert_eq!(c(PatternKind::VlQuery), 1);
    assert_eq!(c(PatternKind::DependentSig), 1);
    // No false positives: internal `execute_MINI(...)` call (lowercase
    // start) and `not_bool` must not count as extern helpers.
    assert_eq!(c(PatternKind::ExternHelper), 0);
    assert_eq!(c(PatternKind::TileAccess), 0);
}

#[test]
fn sve_fixture_fail_counts() {
    let src = fixture("sve_mini.sail");
    let r = analyze_file("sve_mini.sail", &src).unwrap();
    assert_eq!(r.fail_count(FailKind::LoopLowering), 1);
    assert_eq!(r.fail_count(FailKind::ConstraintSolving), 2);
    assert_eq!(r.fail_count(FailKind::ExceptionSemantics), 3);
    assert_eq!(r.fail_count(FailKind::ExternMapping), 6);
    assert_eq!(r.fail_count(FailKind::VlDispatch), 1);
    assert_eq!(r.fail_count(FailKind::UndefinedInit), 1);
    assert_eq!(r.total_fails(), 14);
    // Every fail carries its owning lane via the kind.
    for f in &r.fails {
        assert!(!f.kind.lane().is_empty());
        assert_eq!(f.span.file, "sve_mini.sail");
        assert!(!f.def_name.is_empty());
        assert!(!f.detail.is_empty());
    }
}

#[test]
fn sve_fixture_exact_spans() {
    let src = fixture("sve_mini.sail");
    let r = analyze_file("sve_mini.sail", &src).unwrap();
    let span_of = |k: PatternKind| {
        r.hits.iter().find(|h| h.kind == k).unwrap().span.to_string()
    };
    assert_eq!(span_of(PatternKind::ForeachInc), "sve_mini.sail:7:5-7:12");
    assert_eq!(span_of(PatternKind::VlDispatch), "sve_mini.sail:20:5-20:13");
    assert_eq!(span_of(PatternKind::UndefinedInit), "sve_mini.sail:6:26-6:35");
    let throws: Vec<_> =
        r.hits.iter().filter(|h| h.kind == PatternKind::ThrowUndef).collect();
    assert_eq!(throws.len(), 2);
    assert_eq!(throws[0].span.to_string(), "sve_mini.sail:16:9-16:14");
    assert_eq!(throws[1].span.to_string(), "sve_mini.sail:22:14-22:19");
}

#[test]
fn sme_fixture_nested_foreach_and_tile() {
    let src = fixture("sme_mini.sail");
    let r = analyze_file("sme_mini.sail", &src).unwrap();
    assert_eq!(r.def_total, 2);
    assert_eq!(r.fun_count, 1);
    assert_eq!(r.pattern_count(PatternKind::ForeachInc), 2);
    assert_eq!(r.pattern_count(PatternKind::NestedForeach), 1);
    assert_eq!(r.pattern_count(PatternKind::ElemRead), 1);
    assert_eq!(r.pattern_count(PatternKind::TileAccess), 1);
    assert_eq!(r.pattern_count(PatternKind::CheckEnabled), 1);
    assert_eq!(r.pattern_count(PatternKind::VlDispatch), 0);
    assert_eq!(r.fail_count(FailKind::LoopLowering), 2);

    let nested = r.hits.iter().find(|h| h.kind == PatternKind::NestedForeach).unwrap();
    assert_eq!(nested.span.to_string(), "sme_mini.sail:6:9-6:16");
    assert_eq!(nested.def_name, "execute_TILE");
    let tile = r.hits.iter().find(|h| h.kind == PatternKind::TileAccess).unwrap();
    assert_eq!(tile.span.to_string(), "sme_mini.sail:10:27-10:38");
}

#[test]
fn parallel_matches_sequential() {
    let sve = fixture("sve_mini.sail");
    let sme = fixture("sme_mini.sail");
    let seq_sve = analyze_file("sve_mini.sail", &sve).unwrap();
    let seq_sme = analyze_file("sme_mini.sail", &sme).unwrap();
    let inputs = [("sve_mini.sail", sve.as_str()), ("sme_mini.sail", sme.as_str())];
    let par = analyze_files_parallel(&inputs);
    assert_eq!(par.len(), 2);
    assert_eq!(par[0].as_ref().unwrap(), &seq_sve);
    assert_eq!(par[1].as_ref().unwrap(), &seq_sme);
}

#[test]
fn val_dependent_sig_points_at_forall() {
    let src = fixture("sve_mini.sail");
    let r = analyze_file("sve_mini.sail", &src).unwrap();
    let sig = r.hits.iter().find(|h| h.kind == PatternKind::DependentSig).unwrap();
    assert_eq!(sig.def_name, "execute_MINI");
    assert_eq!(sig.span.start_line, 1);
}
