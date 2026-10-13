//! `E_field` (`record.field`, OCaml `infer_exp`): Basis inferieren,
//! Record aufloesen, Accessor-Teleskop gegen die Basis unifizieren.

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
            let def = parser
                .parse_def(source, Span::DUMMY)
                .unwrap_or_else(|e| panic!("parse failed for {source:?}: {e}"));
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

const PRELUDE: &[&str] = &[
    "default Order dec",
    "struct Point = { x : int, y : int }",
    "val get_x : Point -> int",
];

fn check_last(sources: &[&str]) -> Result<usize, sail_ty::error::TypeError> {
    let defs = parse(sources);
    let program = declare(&defs);
    let pos = defs.len() - 1;
    let (def, it) = &defs[pos];
    program.check_definition(def, (*it).clone(), pos)
}

fn check_with(body: &str) -> Result<usize, sail_ty::error::TypeError> {
    let mut sources: Vec<&str> = PRELUDE.to_vec();
    sources.push(body);
    check_last(&sources)
}

#[test]
fn field_read_checks() {
    check_with("function get_x(p) = p.x").expect("p.x should check");
}

#[test]
fn field_result_flows_into_operators() {
    let mut sources: Vec<&str> = PRELUDE.to_vec();
    sources.extend([
        "val add_atom : forall 'n 'm. (int('n), int('m)) -> int('n + 'm)",
        "val add_int : (int, int) -> int",
        "overload operator + = {add_atom, add_int}",
        "val sum_xy : Point -> int",
        "function sum_xy(p) = p.x + p.y",
    ]);
    check_last(&sources).expect("p.x + p.y should check");
}

#[test]
fn field_of_non_record_fails() {
    let err = check_with("function bad(n : int) -> int = n.x")
        .expect_err("field of int must fail");
    assert!(
        matches!(err.kind, TypeErrorKind::QuantError(_)),
        "non-record base should be QuantError, got {err:?}"
    );
}

#[test]
fn unknown_field_fails() {
    let err = check_with("function bad(p : Point) -> int = p.z")
        .expect_err("unknown field must fail");
    assert!(
        matches!(err.kind, TypeErrorKind::UnboundVar(_)),
        "unknown field should be UnboundVar, got {err:?}"
    );
}
