//! Fixture tests encoded from `sail/test/lean/match.sail` and
//! `sail/test/lean/match_bv.sail` (structures only, no Sail parsing).

use sail_patterns::{Arm, Pat, ScrutineeType, check_exhaustive, is_match_bv, pattern_exhaustivity_redundancy};

// ---------------------------------------------------------------------------
// match.sail: `enum E = A | B | C`, `match_enum` covers A | B | C
// ---------------------------------------------------------------------------

fn enum_e() -> ScrutineeType {
    ScrutineeType::enum_of(&["A", "B", "C"])
}

fn bv_arm() -> Arm {
    // `0b11 @ opc @ Rm`-style vector-concat clause from match_bv.sail.
    Arm::simple(Pat::VectorConcat(vec![
        Pat::Lit("0b11".to_string()),
        Pat::Var("opc".to_string()),
        Pat::Var("Rm".to_string()),
    ]))
}

#[test]
fn match_sail_full_enum_is_exhaustive_not_bv() {
    // function match_enum(x : E): A => .., B => .., C => ..
    let arms = vec![
        Arm::simple(Pat::ctor("A")),
        Arm::simple(Pat::ctor("B")),
        Arm::simple(Pat::ctor("C")),
    ];
    let report = check_exhaustive(&arms, &enum_e());
    assert!(report.exhaustive, "missing: {:?}", report.missing);
    assert!(report.missing.is_empty());
    assert!(report.redundant.is_empty());
    assert!(!report.needs_wildcard);
    assert!(!is_match_bv(&arms));

    let out = pattern_exhaustivity_redundancy(arms, &enum_e());
    assert_eq!(out.arms.len(), 3);
    assert!(!out.wildcard_added);
    assert!(out.redundant_warnings.is_empty());
}

#[test]
fn match_sail_partial_enum_gets_wildcard() {
    // function match_enum2-style: only A | B covered.
    let arms = vec![Arm::simple(Pat::ctor("A")), Arm::simple(Pat::ctor("B"))];
    let report = check_exhaustive(&arms, &enum_e());
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec!["C".to_string()]);
    assert!(report.needs_wildcard);

    let out = pattern_exhaustivity_redundancy(arms, &enum_e());
    assert!(out.wildcard_added);
    assert_eq!(out.arms.len(), 3);
    assert_eq!(out.arms[2].pat, Pat::Wild);
    // Idempotent: second run is exhaustive with no further change.
    let again = check_exhaustive(&out.arms, &enum_e());
    assert!(again.exhaustive);
}

#[test]
fn match_sail_duplicate_arm_is_redundant() {
    // A | B | C | C: second C is redundant and removed.
    let arms = vec![
        Arm::simple(Pat::ctor("A")),
        Arm::simple(Pat::ctor("B")),
        Arm::simple(Pat::ctor("C")),
        Arm::simple(Pat::ctor("C")),
    ];
    let report = check_exhaustive(&arms, &enum_e());
    assert!(report.exhaustive);
    assert_eq!(report.redundant, vec![3]);

    let out = pattern_exhaustivity_redundancy(arms, &enum_e());
    assert_eq!(out.redundant_warnings, vec![3]);
    assert_eq!(out.arms.len(), 3);
    assert!(!out.wildcard_added);
}

#[test]
fn match_sail_wildcard_shadows_tail() {
    // A | B | C | _: trailing wildcard shadows nothing new but is reachable
    // only if earlier arms are inexhaustive; here it IS redundant.
    let arms = vec![
        Arm::simple(Pat::ctor("A")),
        Arm::simple(Pat::ctor("B")),
        Arm::simple(Pat::ctor("C")),
        Arm::simple(Pat::Wild),
    ];
    let report = check_exhaustive(&arms, &enum_e());
    assert!(report.exhaustive);
    assert_eq!(report.redundant, vec![3]);

    // A | _: arm 1 reachable-check — tail after wildcard redundant.
    let arms2 = vec![
        Arm::simple(Pat::ctor("A")),
        Arm::simple(Pat::Wild),
        Arm::simple(Pat::ctor("C")),
    ];
    let report2 = check_exhaustive(&arms2, &enum_e());
    assert!(report2.exhaustive);
    assert_eq!(report2.redundant, vec![2]);
}

#[test]
fn match_sail_guarded_arm_gives_no_coverage() {
    // `A if x` does not cover A.
    let arms = vec![
        Arm::guarded(Pat::ctor("A")),
        Arm::simple(Pat::ctor("B")),
        Arm::simple(Pat::ctor("C")),
    ];
    let report = check_exhaustive(&arms, &enum_e());
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec!["A".to_string()]);
}

// ---------------------------------------------------------------------------
// match_bv.sail: `decode : bits(32) -> bool` with vector-concat clauses
// ---------------------------------------------------------------------------

#[test]
fn match_bv_sail_detected_as_bv() {
    // Four vector-concat clauses + trailing wildcard, like `decode`.
    let arms = vec![
        bv_arm(),
        bv_arm(),
        bv_arm(),
        bv_arm(),
        Arm::simple(Pat::Wild),
    ];
    assert!(is_match_bv(&arms));
    let report = check_exhaustive(&arms, &ScrutineeType::bitvector());
    assert!(report.exhaustive);
    // Bitvector value spaces are opaque without a solver: vector arms are
    // kept conservatively, so nothing is redundant here.
    assert!(report.redundant.is_empty());
    let out = pattern_exhaustivity_redundancy(arms, &ScrutineeType::bitvector());
    assert_eq!(out.arms.len(), 5);
    assert!(!out.wildcard_added);
    assert!(out.redundant_warnings.is_empty());
}

#[test]
fn match_bv_sail_without_wildcard_needs_catch_all() {
    // `write_CSR2`-style: single vector clause, no wildcard.
    let arms = vec![bv_arm()];
    assert!(is_match_bv(&arms));
    let report = check_exhaustive(&arms, &ScrutineeType::bitvector());
    assert!(!report.exhaustive);
    assert!(report.needs_wildcard);

    let out = pattern_exhaustivity_redundancy(arms, &ScrutineeType::bitvector());
    assert!(out.wildcard_added);
    assert_eq!(out.arms.len(), 2);
    assert_eq!(out.arms[1].pat, Pat::Wild);
    assert!(is_match_bv(&out.arms));
}

#[test]
fn match_bv_as_wrapped_vector_still_bv() {
    let arms = vec![Arm::simple(Pat::As(Box::new(bv_arm().pat), "v".to_string()))];
    assert!(is_match_bv(&arms));
}

#[test]
fn bool_match_needs_both_cases() {
    let ty = ScrutineeType::bool();
    let arms = vec![Arm::simple(Pat::Lit("true".to_string()))];
    let report = check_exhaustive(&arms, &ty);
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec!["false".to_string()]);

    let full = vec![
        Arm::simple(Pat::Lit("true".to_string())),
        Arm::simple(Pat::Lit("false".to_string())),
    ];
    assert!(check_exhaustive(&full, &ty).exhaustive);
    assert!(!is_match_bv(&full));
}

#[test]
fn empty_match_gets_single_wildcard() {
    let out = pattern_exhaustivity_redundancy(Vec::new(), &enum_e());
    assert!(out.wildcard_added);
    assert_eq!(out.arms.len(), 1);
    assert!(check_exhaustive(&out.arms, &enum_e()).exhaustive);
}
