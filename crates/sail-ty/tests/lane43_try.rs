//! Lane-43: `try … catch` check-only (OCaml `check_exp E_try ~L2196`).
//!
//! Body `<= Expected` in the same scope; every arm binds against
//! `exc_typ` (`exception`, repo shorthand "exc") exactly like a match
//! arm: optional guard `<= bool` in the arm scope, body `<= Expected`
//! with the guard fact. No infer plan.

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
        program.declare(def, it.clone(), position).unwrap();
    }
    program
}

fn check_source(source: &str) -> Result<usize, sail_ty::error::TypeError> {
    let defs = parse(&[source]);
    let program = declare(&defs);
    let (def, it) = &defs[0];
    program.check_definition(def, it.clone(), 0)
}

#[test]
fn try_body_checks_against_expected() {
    // Body `1 <= int` ok, arm `_ => 2 <= int` ok.
    assert!(check_source("function f() -> int = try 1 catch { _ => 2 }").is_ok());
    // Body `true : bool` against `int` must fail (body <= Expected).
    assert!(check_source("function f() -> int = try true catch { _ => 2 }").is_err());
}

#[test]
fn try_arms_check_body_against_expected() {
    // Arm body `true : bool` against `int` must fail.
    assert!(check_source("function f() -> int = try 1 catch { _ => true }").is_err());
    // Multiple arms, all bodies <= Expected.
    assert!(check_source("function f() -> int = try 1 catch { _ => 1, _ => 2 }").is_ok());
}

#[test]
fn try_arms_bind_against_exc_not_expected() {
    // `e` binds against `exception` (exc_typ), so returning it where
    // `int` is expected must fail. If arms bound against Expected,
    // this would succeed; it must not.
    assert!(check_source("function f() -> int = try 1 catch { e => e }").is_err());
}

#[test]
fn try_guard_checks_bool_in_arm_scope() {
    // Guard `true <= bool` ok.
    assert!(check_source("function f() -> int = try 1 catch { e if true => 2 }").is_ok());
    // Guard `1 : int` against `bool` must fail.
    assert!(check_source("function f() -> int = try 1 catch { e if 1 => 2 }").is_err());
}

#[test]
fn try_is_check_only_infer_stays_unsupported() {
    // `let`-binding synthesizes; Try has no infer plan (OCaml infer
    // unsupported), so this must fail even though the same Try checks
    // fine against `int` in body position.
    assert!(
        check_source("function f() -> int = let x = try 1 catch { _ => 2 } in x").is_err()
    );
}
