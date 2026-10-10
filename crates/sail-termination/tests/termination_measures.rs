#![forbid(unsafe_code)]
//! End-to-end test over the `termination_measures.sail`-shaped fixture:
//! parse directives → run the move pass → emit `termination_by` Lean.

use sail_termination::{LoopMeasure, MeasureDirective, RecMeasure, TerminationPass, parse_measure_directives, render_termination_by};

const FIXTURE: &str = include_str!("fixtures/termination_measures.sail");

#[test]
fn fixture_parses_all_directives() {
    let ds = parse_measure_directives(FIXTURE).expect("fixture must parse");
    assert_eq!(ds.len(), 6, "{ds:?}");

    // Function-measure forms.
    assert!(matches!(
        &ds[0],
        MeasureDirective::Function { name, .. } if name == "Reduce__1"
    ));
    assert!(matches!(
        &ds[4],
        MeasureDirective::Function { name, pattern, expr }
        if name == "enabled" && pattern == "e" && expr == "measure(e)"
    ));

    // Loop-measure forms: single while, multiple whiles, repeats.
    assert_eq!(
        ds[1],
        MeasureDirective::Loops {
            name: "FloorPow2".to_string(),
            measures: vec![LoopMeasure::While("x - pow2(n)".to_string())],
        }
    );
    assert!(matches!(&ds[2], MeasureDirective::Loops { measures, .. } if measures.len() == 2));
    assert!(matches!(&ds[3], MeasureDirective::Loops { measures, .. }
        if *measures == vec![LoopMeasure::Repeat(100), LoopMeasure::Repeat(100)]));
}

#[test]
fn fixture_end_to_end_lean_matches_reference() {
    // Collect function measures like DEF_measure nodes, run the move pass,
    // then render termination_by clauses as doc_termination would.
    let ds = parse_measure_directives(FIXTURE).unwrap();
    let mut triples = Vec::new();
    for d in &ds {
        if let MeasureDirective::Function { name, pattern, expr } = d {
            triples.push((name.clone(), pattern.clone(), expr.clone()));
        }
    }
    let mut funs = vec![
        sail_termination::FunDef {
            name: "enabled".to_string(),
            measure: None,
            hoisted_specs: vec![],
        },
        sail_termination::FunDef {
            name: "enabled2".to_string(),
            measure: None,
            hoisted_specs: vec![],
        },
    ];
    let warnings =
        TerminationPass::run(triples, &mut funs, &|_| Vec::new()).expect("pass must succeed");
    // Reduce__1 has no fundef here → one unused-measure warning.
    assert_eq!(warnings.len(), 1, "{warnings:?}");

    // Override fn patterns to the real call shapes and check Lean output
    // against sail/test/lean/termination.expected.lean.
    let mut m0 = funs[0].measure.clone().unwrap();
    m0.fn_pattern = "e".to_string();
    assert_eq!(
        render_termination_by(&m0),
        "\ntermination_by (let e := e\n(measure(e))).toNat"
    );

    let mut m1 = funs[1].measure.clone().unwrap();
    m1.pattern = "(_, x)".to_string();
    m1.measure_expr = "measure2(x)".to_string();
    m1.fn_pattern = "(b, e)".to_string();
    assert_eq!(
        render_termination_by(&m1),
        "\ntermination_by (let (_, x) := (b, e)\n(measure2(x))).toNat"
    );

    // Sanity: explicit RecMeasure constructor agrees.
    let direct = RecMeasure::new("(_, x)", "measure2(x)", "(b, e)").unwrap();
    assert_eq!(render_termination_by(&direct), render_termination_by(&m1));
}
