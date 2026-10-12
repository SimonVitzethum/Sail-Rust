#![forbid(unsafe_code)]
//! Lane-42: check-only `match` (OCaml `check_exp` E_match ~L2176).
//! Scrutinee is inferred, `mapping_match` checks `<= option(Expected)`,
//! guards `<= bool`, bodies `<= Expected`. Completeness stays an oracle
//! reference (see `crate::exp_match::MatchCompletenessOracle`); constructor
//! patterns remain honest `Unsupported` via the pattern checker.

use sail_ast::*;
use sail_parse::{ParserContext, RealParser};
use sail_ty::{error::TypeErrorKind, program::Program};
use std::sync::Arc;

fn parse(sources: &[&str]) -> Vec<(Def, Arc<SimpleInterner>)> {
    let mut context = ParserContext::default();
    sources
        .iter()
        .map(|source| {
            let mut parser = RealParser::with_context(context.clone());
            let def = parser.parse_def(source, Span::DUMMY).unwrap();
            context = parser.context();
            (def, Arc::new(parser.into_interner()))
        })
        .collect()
}

fn declare(defs: &[(Def, Arc<SimpleInterner>)]) -> Program {
    let mut program = Program::new();
    for (position, (def, it)) in defs.iter().enumerate() {
        program.declare(def, it.clone(), position).unwrap();
    }
    program
}

fn check_all(sources: &[&str]) -> Vec<Result<usize, sail_ty::error::TypeError>> {
    let defs = parse(sources);
    let program = declare(&defs);
    defs.iter()
        .enumerate()
        .map(|(position, (def, it))| program.check_definition(def, it.clone(), position))
        .collect()
}

#[test]
fn match_var_and_wild_bodies_check_against_expected() {
    let results = check_all(&["function f(x : int) -> int = match x { y => y, _ => x }"]);
    assert!(results[0].is_ok(), "var/wild match should check: {:?}", results[0]);
}

#[test]
fn match_guard_must_be_bool() {
    let ok = check_all(&["function g(x : int) -> int = match x { y if true => y, _ => x }"]);
    assert!(ok[0].is_ok(), "bool guard should check: {:?}", ok[0]);
    let bad = check_all(&["function g(x : int) -> int = match x { y if 0 => y, _ => x }"]);
    assert!(bad[0].is_err(), "non-bool guard must fail, got {:?}", bad[0]);
}

#[test]
fn match_body_must_match_expected() {
    let bad = check_all(&["function f(x : int) -> int = match x { _ => false }"]);
    assert!(bad[0].is_err(), "bool body vs int expected must fail: {:?}", bad[0]);
    let err = bad[0].as_ref().err().unwrap();
    assert!(
        matches!(err.kind, TypeErrorKind::Mismatch { .. }),
        "body mismatch should be Mismatch, got {err:?}"
    );
}

#[test]
fn match_scrutinee_is_inferred() {
    let bad = check_all(&["function f() -> int = match missing { _ => 0 }"]);
    assert!(bad[0].is_err(), "unbound scrutinee must fail: {:?}", bad[0]);
}

#[test]
fn match_constructor_pattern_is_honest_unsupported() {
    // `Wrap(y)` needs constructor/mapping resolution (Lane-09 oracle);
    // the checker must not silently accept it.
    let res = check_all(&["function f(x : int) -> int = match x { Wrap(y) => y, _ => x }"]);
    assert!(res[0].is_err(), "constructor pattern must not succeed: {:?}", res[0]);
    let err = res[0].as_ref().err().unwrap();
    assert!(
        matches!(err.kind, TypeErrorKind::Unsupported(_)),
        "constructor pattern should be honest Unsupported, got {err:?}"
    );
}

#[test]
fn mapping_match_attr_checks_option_of_expected() {
    // Same program succeeds without the attribute (plain infer) but takes
    // the `option(Expected)` path with `mapping_match` (int vs option(int)
    // cannot be compatible).
    let sources = ["function f(x : int) -> int = match x { y => y, _ => x }"];
    let defs = parse(&sources);
    let program = declare(&defs);
    let (def, it) = &defs[0];
    assert!(program.check_definition(def, it.clone(), 0).is_ok());
    // Clone and attach `mapping_match` to the match node itself.
    let mut def_attr = def.clone();
    let DefKind::Fundef { funcls, .. } = &mut def_attr.kind else {
        panic!("expected fundef");
    };
    let (PExpKind::PatExp { body, .. } | PExpKind::PatWhen { body, .. }) =
        &mut funcls[0].arm.kind;
    assert!(
        matches!(body.kind, ExpKind::Match { .. }),
        "body should be a match"
    );
    body.annotation_mut().attributes.push(Attribute {
        name: "mapping_match".to_owned(),
        data: None,
    });
    let res = program.check_definition(&def_attr, it.clone(), 0);
    assert!(res.is_err(), "mapping_match should check scrutinee <= option(Expected): {res:?}");
}

#[test]
fn match_guard_fact_reaches_body() {
    // `only_true` needs a true constraint; the guard fact must flow into
    // the body scope (`with_guard_fact`), like `if` branches.
    let decl = "val only_true : forall ('p : Bool), 'p. bool('p) -> unit";
    let good = [decl, "function f(x : bool) -> unit = match x { y if y => only_true(y), _ => () }"];
    let results = check_all(&good);
    assert!(results[1].is_ok(), "guard fact should enable only_true: {:?}", results[1]);
    let bad = [decl, "function f(x : bool) -> unit = match x { y => only_true(y), _ => () }"];
    let results = check_all(&bad);
    assert!(results[1].is_err(), "without guard fact only_true must fail: {:?}", results[1]);
}
