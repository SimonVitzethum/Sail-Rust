//! Implizite Argumente (OCaml `infer_funapp`): `implicit('m)`-Formale
//! duerfen an Call-Sites fehlen (`extzv([bitzero])`); explizites
//! Mitgeben (gleiche Stelligkeit) funktioniert weiter; alles andere
//! ist harter Aritaetsfehler. Unloesbare Implizite bleiben ehrliche
//! Fehler (OCaml: `Cannot solve implicit`).

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
    "val sail_zeros : forall 'k, 'k >= 0. int('k) -> bits('k)",
    "val mk : forall 'n 'm, 'm >= 0. (implicit('m), bits('n)) -> bits('m)",
    "function mk(m, v) = sail_zeros(m)",
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
fn omitted_implicit_solves_from_expected() {
    check_with(&[
        "val use_implicit : forall 'n. bits('n) -> bits(8)",
        "function use_implicit(v) = mk(v)",
    ])
    .expect("omitted implicit should solve 'm := 8 from the expected type");
}

#[test]
fn explicit_implicit_still_checks() {
    check_with(&[
        "val use_explicit : forall 'n. (int(8), bits('n)) -> bits(8)",
        "function use_explicit(m, v) = mk(m, v)",
    ])
    .expect("explicitly passed implicit should keep checking");
}

#[test]
fn unsolvable_implicit_stays_honest_error() {
    // `'m` kommt weder aus Argumenten noch aus dem Erwartungstyp:
    // ehrlicher Fehler, nie ein geratenes Binding.
    let err = check_with(&[
        "val use_open : forall 'n 'a. bits('n) -> 'a",
        "function use_open(v) = mk(v)",
    ])
    .expect_err("unsolvable implicit must fail");
    assert!(
        !matches!(err.kind, TypeErrorKind::Mismatch { .. }),
        "unsolvable implicit should not be a plain mismatch, got {err:?}"
    );
}

#[test]
fn sibling_type_solves_omitted_implicit() {
    // `extzv(..)`-Resultat traegt freies `'m`; das Geschwister (`xs`)
    // loest es per Retry (`is_zero_subrange`-Muster, beidseitig gruen).
    check_with(&[
        "val eq_bits : forall 'n. (bits('n), bits('n)) -> bool",
        "overload operator == = {eq_bits}",
        "val extzv_like2 : forall 'n 'm, 'm >= 0. (implicit('m), bits('n)) -> bits('m)",
        "val bitvector_length : forall 'n. bits('n) -> int('n)",
        "overload __size = {bitvector_length}",
        "val is_zero : forall 'n, 'n >= 0. bits('n) -> bool",
        "function is_zero(xs) = { xs == extzv_like2([bitzero]) }",
    ])
    .expect("sibling type should solve the omitted implicit");
}

#[test]
fn wrong_arity_still_fails() {
    let err = check_with(&[
        "val use_wrong : forall 'n. bits('n) -> bits(8)",
        "function use_wrong(v) = mk(v, v, v)",
    ])
    .expect_err("over-application must fail");
    assert!(
        matches!(err.kind, TypeErrorKind::ArityMismatch { .. }),
        "over-application should be ArityMismatch, got {err:?}"
    );
}

#[test]
fn nested_overloads_propagate_sibling_types_to_omitted_implicits() {
    check_with(&[
        "val bool_and : (bool, bool) -> bool",
        "val bits_and : forall 'n. (bits('n), bits('n)) -> bits('n)",
        "overload operator & = {bool_and, bits_and}",
        "val bool_eq : (bool, bool) -> bool",
        "val bits_eq : forall 'n. (bits('n), bits('n)) -> bool",
        "overload operator == = {bool_eq, bits_eq}",
        "val mask : forall 'n, 'n >= 0. (implicit('n), int, int) -> bits('n)",
        "val is_zero : forall 'n, 'n >= 0. (bits('n), int, int) -> bool",
        "function is_zero(xs, i, j) = (xs & mask(i, j)) == mk([bitzero] : bits(1))",
    ])
    .expect("nested overloaded operands must share their inferred sibling types");
}

#[test]
fn nested_overload_inference_keeps_width_obligations() {
    let error = check_with(&[
        "val bool_and : (bool, bool) -> bool",
        "val bits_and : forall 'n. (bits('n), bits('n)) -> bits('n)",
        "overload operator & = {bool_and, bits_and}",
        "val bits_eq : forall 'n. (bits('n), bits('n)) -> bool",
        "overload operator == = {bits_eq}",
        "val is_zero : forall 'n, 'n >= 0. bits('n) -> bool",
        "function is_zero(xs) = (xs & (mk([bitzero]) : bits(2))) == xs",
    ])
    .expect_err("an arbitrary input width cannot be equated with two bits");
    assert!(
        !matches!(error.kind, TypeErrorKind::Unsupported(_)),
        "{error:?}"
    );
}
