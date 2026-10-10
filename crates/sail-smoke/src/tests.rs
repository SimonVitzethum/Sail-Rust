use super::*;

#[test]
fn all_cases_have_documented_expectations() {
    assert_eq!(SMOKE_CASES.len(), 10, "must hold exactly 10 smoke cases");
    for case in SMOKE_CASES.iter() {
        assert!(!case.id.is_empty(), "id must be set");
        assert!(!case.sail_source.is_empty(), "sail_source must be set");
        assert!(!case.sail_function.is_empty(), "sail_function must be set");
        assert!(!case.description.is_empty(), "description must be set");
        assert!(!case.lean_expect.is_empty(), "lean_expect must be set");
        assert!(!case.c_expect.is_empty(), "c_expect must be set");
        assert!(!case.lean_log.is_empty(), "lean_log must be set");
        assert!(!case.c_log.is_empty(), "c_log must be set");
    }
}

#[test]
fn case_ids_are_unique() {
    let mut ids: Vec<&str> = SMOKE_CASES.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), SMOKE_CASES.len());
}

#[test]
fn normalize_collapses_whitespace_and_blank_lines() {
    let raw = "  bitvector_add(0xFFFF,   0x0001)  =  0x0000 \r\n\n  extra   spaces\rline2  ";
    assert_eq!(
        normalize_log(raw),
        "bitvector_add(0xFFFF, 0x0001) = 0x0000\nextra spaces\nline2"
    );
    assert_eq!(normalize_log("   \n \r\n "), "");
}

#[test]
fn identical_logs_match_after_normalization() {
    let case = &SMOKE_CASES[0];
    let a = "  bitvector_eq(0x00FF, 0x00FF) = true  ";
    let b = "bitvector_eq(0x00FF,\t0x00FF) = true\r\n";
    assert_eq!(compare_logs(case, a, b), Comparison::Match);
}

#[test]
fn differing_logs_mismatch_with_detail() {
    let case = &SMOKE_CASES[0];
    let res = compare_logs(case, "result = true", "result = false");
    match res {
        Comparison::Mismatch { detail } => {
            assert_eq!(detail.case_id, case.id);
            assert_eq!(detail.lean_normalized, "result = true");
            assert_eq!(detail.c_normalized, "result = false");
        }
        Comparison::Match => panic!("expected mismatch"),
    }
}

#[test]
fn all_fixtures_match_themselves() {
    let result = run_all();
    assert_eq!(
        result.mismatched, 0,
        "fixtures must agree: {:?}",
        result.mismatches
    );
    assert_eq!(result.matched, SMOKE_CASES.len());
}

#[test]
fn find_case_roundtrip() {
    for case in SMOKE_CASES.iter() {
        assert_eq!(find_case(case.id), Some(case));
    }
    assert_eq!(find_case("does-not-exist"), None);
}

#[test]
fn runner_plan_covers_all_cases() {
    let cfg = RunnerConfig::new("target/lean-runner", "target/c-runner");
    let plans = cfg.plan_all(&["--smoke"]);
    assert_eq!(plans.len(), SMOKE_CASES.len());
    for (plan, case) in plans.iter().zip(SMOKE_CASES.iter()) {
        assert_eq!(plan.case_id, case.id);
        assert_eq!(plan.lean_bin, "target/lean-runner");
        assert_eq!(plan.c_bin, "target/c-runner");
        assert_eq!(plan.args, vec!["--smoke".to_string()]);
    }
    // Single-case plan keeps both binaries with shared args.
    let one = cfg.plan_for(&SMOKE_CASES[0], &["--case", "bitvec-eq"]);
    assert_eq!(one.case_id, "bitvec-eq");
    assert_eq!(
        one.args,
        vec!["--case".to_string(), "bitvec-eq".to_string()]
    );
}
