//! Control expressions (OCaml `check_exp`): `assert` with continuation
//! facts, `throw` against the `exception` union, `exit` against `unit`,
//! `constraint(..)` as a fact source (impdefs-pattern).

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
    "type bits('n) = bitvector('n)",
    "val greater_equal : forall 'n 'm. (int('n), int('m)) -> bool",
    "overload operator >= = {greater_equal}",
    "val less_equal : forall 'n 'm. (int('n), int('m)) -> bool",
    "overload operator <= = {less_equal}",
    "val bitvector_length : forall 'n. bits('n) -> int('n)",
    "overload __size = {bitvector_length}",
    "val truncate : forall ('m : Int) ('n : Int), 'm >= 0 & 'm <= 'n. (bits('n), int('m)) -> bits('m)",
    "union exception = { Failure : unit }",
];

fn check_last(sources: &[&str]) -> Result<usize, sail_ty::error::TypeError> {
    let defs = parse(sources);
    let program = declare(&defs);
    let pos = defs.len() - 1;
    let (def, it) = &defs[pos];
    program.check_definition(def, (*it).clone(), pos)
}

fn check_with(bodies: &[&str]) -> Result<usize, sail_ty::error::TypeError> {
    let mut sources: Vec<&str> = PRELUDE.to_vec();
    sources.extend(bodies.iter().copied());
    check_last(&sources)
}

#[test]
fn assert_fact_flows_to_continuation() {
    // `assert` proves the following call (impdefs-pattern with an
    // explicit comparison instead of `constraint(..)`).
    check_with(&[
        "val f : forall 'n ('m : Int), 'm >= 0. (implicit('m), bits('n)) -> bits('m)",
        "function f(m, v) = {
  assert(m <= 'n, \"bound\");
  truncate(v, m)
}",
    ])
    .expect("asserted bound should discharge truncate");
}

#[test]
fn assert_constraint_form_flows_to_continuation() {
    // `assert(constraint(..))` as in ARM `impdefs.sail`.
    check_with(&[
        "val f : forall 'n ('m : Int), 'm >= 0. (implicit('m), bits('n)) -> bits('m)",
        "function f(m, v) = {
  assert(constraint('m <= 'n));
  truncate(v, m)
}",
    ])
    .expect("assert(constraint(..)) should discharge truncate");
}

#[test]
fn assert_wrong_cond_type_fails() {
    let err = check_with(&[
        "val f : unit -> int",
        "function f() = { assert(5, \"x\"); 0 }",
    ])
    .expect_err("int condition must fail");
    assert!(
        !matches!(err.kind, TypeErrorKind::Unsupported(_)),
        "wrong cond type should be a hard error, got {err:?}"
    );
}

#[test]
fn throw_exception_member_checks() {
    check_with(&[
        "val f : unit -> int",
        "function f() = throw(Failure(()))",
    ])
    .expect("throw of exception member should check");
}

#[test]
fn throw_non_exception_fails() {
    let err = check_with(&[
        "val f : unit -> int",
        "function f() = throw(5)",
    ])
    .expect_err("throw of int must fail");
    assert!(
        matches!(err.kind, TypeErrorKind::Mismatch { .. }),
        "non-exception payload should be Mismatch, got {err:?}"
    );
}

#[test]
fn exit_unit_checks() {
    check_with(&[
        "val f : unit -> int",
        "function f() = { exit(()); 0 }",
    ])
    .expect("exit(()) should check");
}

#[test]
fn exit_non_unit_fails() {
    let err = check_with(&[
        "val f : unit -> int",
        "function f() = { exit(5); 0 }",
    ])
    .expect_err("exit(5) must fail");
    assert!(
        !matches!(err.kind, TypeErrorKind::Unsupported(_)),
        "wrong exit arg should be a hard error, got {err:?}"
    );
}
