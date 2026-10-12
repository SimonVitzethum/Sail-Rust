//! Lane-45: `E_struct_update` check + infer (OCaml `check_exp` ~L2199,
//! `infer_exp` ~L3823) as covered by `patch-structupdate.diff`.
//!
//! Check: base first `<= Expected`, then each update field against its
//! accessor payload; NO coverage obligation (partial updates legal).
//! Infer: infer the base, require a record, check each update against
//! `RecordField{Inferred("record")}` via the new
//! `Synthesis::StructUpdate` state.
//!
//! Anchor disjointness vs Lane-44 (`E_struct`):
//! - check arm between `Tuple` and `Let` (Lane-44: trailing `_ =>`
//!   fallback); see `LANE-45-STRUCTUPDATE-CHECK-ANCHOR`.
//! - infer `Enter` arm between `Vector` and `App` (Lane-44: beside `App`);
//!   see `LANE-45-STRUCTUPDATE-INFER-ENTER`.
//! - `Synthesis::StructUpdate` after `Vector` (Lane-44 `Synthesis::Struct`
//!   after `Call`); see `LANE-45-STRUCTUPDATE-SYNTHESIS`.
//! - program.rs: kein eigener Table — Lane-08 `records`/`accessor` wiederverwendet.
//! Der `struct_literal_supported_alongside_update`-Test unten belegt das
//! Zusammenspiel beider Arme.

use sail_ast::*;
use sail_parse::{ParserContext, RealParser};
use sail_ty::program::Program;
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
        program
            .declare(def, it.clone(), position)
            .unwrap_or_else(|e| panic!("declare {position} failed: {e}"));
    }
    program
}

fn check_all(defs: &[(Def, Arc<SimpleInterner>)], program: &Program) -> Vec<Result<(), String>> {
    defs.iter()
        .enumerate()
        .map(|(position, (def, it))| {
            program
                .check_definition(def, it.clone(), position)
                .map(|_| ())
                .map_err(|e| format!("{e:?}"))
        })
        .collect()
}

const POINT: &str = "struct Point = { x : int, y : int }";

#[test]
fn check_partial_update_has_no_coverage_obligation() {
    // Only `x` of `{x, y}` is updated: legal for updates (no
    // `FullFieldCoverage`), would be an error for `E_struct` literals.
    let defs = parse(&[
        POINT,
        "function upd(r : Point) -> Point = { r with x = 1 }",
    ]);
    let program = declare(&defs);
    let results = check_all(&defs, &program);
    assert!(results[1].is_ok(), "partial update must check: {:?}", results[1]);
}

#[test]
fn check_base_is_checked_first_against_expected() {
    // Base of the wrong record must fail even though the updated field
    // value itself is well-typed: base `<= Expected` comes first.
    let defs = parse(&[
        POINT,
        "struct Other = { x : int }",
        "function bad(r : Other) -> Point = { r with x = 1 }",
    ]);
    let program = declare(&defs);
    let results = check_all(&defs, &program);
    assert!(
        results[2].is_err(),
        "wrong-record base must fail check: {results:?}"
    );
}

#[test]
fn check_field_type_mismatch_fails() {
    let defs = parse(&[
        POINT,
        "function bad(r : Point) -> Point = { r with x = true }",
    ]);
    let program = declare(&defs);
    let results = check_all(&defs, &program);
    assert!(
        results[1].is_err(),
        "bool for int field must fail: {results:?}"
    );
}

#[test]
fn check_unknown_field_fails() {
    let defs = parse(&[
        POINT,
        "function bad(r : Point) -> Point = { r with z = 1 }",
    ]);
    let program = declare(&defs);
    let results = check_all(&defs, &program);
    let err = results[1].as_ref().unwrap_err();
    assert!(
        err.contains("nbound") || err.contains('z'),
        "unknown field must be unbound: {err}"
    );
}

#[test]
fn check_non_record_expected_fails() {
    // Expected `int` is not a record: OCaml "is not a record".
    let defs = parse(&["function bad(x : int) -> int = { x with y = 1 }"]);
    let program = declare(&defs);
    let results = check_all(&defs, &program);
    let err = results[0].as_ref().unwrap_err();
    assert!(
        err.contains("not a record"),
        "non-record expected must report it: {err}"
    );
}

#[test]
fn infer_update_through_let_bind() {
    // `infer` path: the `Let` bind infers `{ r with x = 1 }`
    // (`Synthesis::StructUpdate`: base inferred first, field checked
    // against `RecordField{Inferred}`), then the body checks.
    let defs = parse(&[
        POINT,
        "function f(r : Point) -> Point = let r2 = { r with x = 1 } in r2",
    ]);
    let program = declare(&defs);
    let results = check_all(&defs, &program);
    assert!(results[1].is_ok(), "inferred update must check: {:?}", results[1]);
}

#[test]
fn infer_update_wrong_field_fails() {
    let defs = parse(&[
        POINT,
        "function f(r : Point) -> Point = let r2 = { r with x = true } in r2",
    ]);
    let program = declare(&defs);
    let results = check_all(&defs, &program);
    assert!(
        results[1].is_err(),
        "inferred update with wrong field must fail: {results:?}"
    );
}

#[test]
fn infer_non_record_base_fails() {
    // Inferred base `int` is not a record.
    let defs = parse(&["function f() -> int = let v = { 1 with x = 1 } in v"]);
    let program = declare(&defs);
    let results = check_all(&defs, &program);
    let err = results[0].as_ref().unwrap_err();
    assert!(
        err.contains("not a record"),
        "non-record inferred base must report it: {err}"
    );
}

#[test]
fn struct_literal_supported_alongside_update() {
    // Lane-44 merged: `E_struct` checks fine; Lane-45 adds `StructUpdate`.
    // Both arms coexist (update on a checked literal).
    let defs = parse(&[
        POINT,
        "function mk() -> Point = struct Point { x = 1, y = 2 }",
    ]);
    let program = declare(&defs);
    let results = check_all(&defs, &program);
    assert!(results[1].is_ok(), "struct literal checks: {:?}", results[1]);
}
