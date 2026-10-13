//! Vektor-Patterns (OCaml `bind_pat`): `P_vector` bindet Elemente gegen
//! `bit`, `P_vector_concat` summiert konstante Teilbreiten (Literale via
//! Typ, Vektoren via Stelligkeit, hoechstens ein Wildcard nimmt den
//! Rest); die Summe muss der Skrutinee-Breite entsprechen.

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
    "type bit = bitvector(1)",
    "type bits('n) = bitvector('n)",
    "val classify : bits(8) -> int",
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
fn vector_pattern_binds_bits() {
    check_with(&["function classify(op) = match op {
  [bitzero, _, _] @ 0x0 @ _ => 0,
  [bitone, _, _] @ 0x1 @ _ => 1,
  _ => 2
}"])
    .expect("decode-style concat patterns should check");
}

#[test]
fn vector_pattern_width_mismatch_fails() {
    let err = check_with(&["function classify(op) = match op {
  [bitzero, _] => 0,
  _ => 1
}"])
    .expect_err("3-bit pattern against bits(8) must fail");
    assert!(
        !matches!(err.kind, TypeErrorKind::Unsupported(_)),
        "width mismatch should be decided, got {err:?}"
    );
}

#[test]
fn concat_multiple_wildcards_fail() {
    let err = check_with(&["function classify(op) = match op {
  _ @ _ => 0,
  _ => 1
}"])
    .expect_err("two unclear widths must fail");
    assert!(
        matches!(err.kind, TypeErrorKind::Unsupported(_)),
        "ambiguous widths should stay honest Unsupported, got {err:?}"
    );
}

#[test]
fn concat_overflow_fails() {
    let err = check_with(&["function classify(op) = match op {
  [bitzero, _, _, _, _, _, _, _, _] => 0,
  _ => 1
}"])
    .expect_err("9-bit pattern against bits(8) must fail");
    assert!(
        !matches!(err.kind, TypeErrorKind::Unsupported(_)),
        "width overflow should be decided, got {err:?}"
    );
}
