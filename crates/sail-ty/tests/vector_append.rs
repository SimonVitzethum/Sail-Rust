//! `VectorAppend` (`@`, OCaml `E_vector_append`): beide Seiten muessen
//! denselben Vektor-Flavor tragen; das Ergebnis addiert die gegroundeten
//! Laengen. Wie OCaml braucht `@` eine `append`-Ueberladung im Scope.

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

const PRELUDE: &[&str] = &[
    "default Order dec",
    "type bits('n) = bitvector('n)",
    "val sail_zeros : forall 'n, 'n >= 0. int('n) -> bits('n)",
    "val subrange_bits : forall ('n : Int) ('m : Int) ('o : Int), 0 <= 'o <= 'm < 'n. (bits('n), int('m), int('o)) -> bits('m - 'o + 1)",
    "overload vector_subrange = {subrange_bits}",
    "val append_bits : forall 'n 'm. (bits('n), bits('m)) -> bits('n + 'm)",
    "overload append = {append_bits}",
];

fn check_with(prelude: &[&str], body: &str) -> Result<usize, sail_ty::error::TypeError> {
    let mut sources: Vec<&str> = Vec::with_capacity(prelude.len() + 1);
    sources.extend(prelude.iter().copied());
    sources.push(body);
    let defs = parse(&sources);
    let program = declare(&defs);
    let (def, it) = &defs[defs.len() - 1];
    program.check_definition(def, (*it).clone(), defs.len() - 1)
}

#[test]
fn append_bits_adds_lengths() {
    check_with(
        PRELUDE,
        "function rev2(x : bits(16)) -> bits(16) = x[7 .. 0] @ x[15 .. 8]",
    )
    .expect("bits(8) ++ bits(8) should check against bits(16)");
}

#[test]
fn append_wrong_length_fails() {
    let err = check_with(
        PRELUDE,
        "function rev2(x : bits(16)) -> bits(15) = x[7 .. 0] @ x[15 .. 8]",
    )
    .expect_err("bits(16) result must not check against bits(15)");
    assert!(
        !matches!(err.kind, TypeErrorKind::Unsupported(_)),
        "length arithmetic must decide honestly, got {err:?}"
    );
}

#[test]
fn append_needs_overload_in_scope() {
    // Wie OCaml (`Function append not found`): ohne `append`-Ueberladung
    // ist `@` ungebunden, nie stillschweigend akzeptiert.
    let prelude: Vec<&str> = PRELUDE
        .iter()
        .copied()
        .filter(|line| !line.contains("append_bits") && !line.contains("overload append"))
        .collect();
    let err = check_with(
        &prelude,
        "function rev2(x : bits(16)) -> bits(16) = x[7 .. 0] @ x[15 .. 8]",
    )
    .expect_err("append without overload must fail");
    assert!(
        matches!(err.kind, TypeErrorKind::UnboundVar(_)),
        "missing append overload should be UnboundVar, got {err:?}"
    );
}

#[test]
fn append_flavor_mismatch_fails() {
    let err = check_with(
        PRELUDE,
        "function bad(x : bits(8), v : vector(8, bits(1))) -> bits(16) = x @ v",
    )
    .expect_err("bits ++ vector must fail");
    assert!(
        matches!(err.kind, TypeErrorKind::Mismatch { .. }),
        "flavor mismatch should be Mismatch, got {err:?}"
    );
}
