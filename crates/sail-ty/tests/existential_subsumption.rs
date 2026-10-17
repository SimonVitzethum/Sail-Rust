//! Existential witnesses must prove their predicates, including at returns.
use sail_ast::{Def, SimpleInterner, Span};
use sail_parse::{ParserContext, RealParser};
use sail_ty::{program::Program, TypeResult};
use std::sync::Arc;

fn check(sources: &[&str]) -> TypeResult<usize> {
    let mut context = ParserContext::default();
    let definitions: Vec<(Def, Arc<SimpleInterner>)> = sources
        .iter()
        .map(|source| {
            let mut parser = RealParser::with_context(context.clone());
            let definition = parser.parse_def(source, Span::DUMMY).unwrap();
            context = parser.context();
            (definition, Arc::new(parser.into_interner()))
        })
        .collect();
    let mut program = Program::new();
    for (position, (definition, interner)) in definitions.iter().enumerate() {
        program.declare(definition, interner.clone(), position)?;
    }
    let position = definitions.len() - 1;
    program.check_definition(
        &definitions[position].0,
        definitions[position].1.clone(),
        position,
    )
}

#[test]
fn existential_return_accepts_a_witness_satisfying_its_predicate() {
    check(&[
        "val within : unit -> { 'n, 0 <= 'n & 'n < 4. int('n) }",
        "function within() = 1",
    ])
    .unwrap();
}

#[test]
fn existential_return_rejects_a_witness_outside_its_predicate() {
    assert!(
        check(&[
            "val outside : unit -> { 'n, 0 <= 'n & 'n < 4. int('n) }",
            "function outside() = 4",
        ])
        .is_err(),
        "the result predicate must be proved, never discarded"
    );
}

#[test]
fn existential_return_may_weaken_existing_witness_bounds() {
    check(&[
        "val small : unit -> { 'n, 0 <= 'n & 'n < 4. int('n) }",
        "val wider : unit -> { 'm, 0 <= 'm & 'm < 8. int('m) }",
        "function wider() = small()",
    ])
    .unwrap();
}

#[test]
fn existential_return_cannot_strengthen_existing_witness_bounds() {
    assert!(
        check(&[
            "val wide : unit -> { 'n, 0 <= 'n & 'n < 8. int('n) }",
            "val small : unit -> { 'm, 0 <= 'm & 'm < 4. int('m) }",
            "function small() = wide()",
        ])
        .is_err(),
        "an unknown witness cannot gain a stronger bound"
    );
}

#[test]
fn quantified_call_can_return_a_weaker_existential() {
    check(&[
        "val produce : forall 'n, 'n > 0. int('n) -> { 'r, 0 <= 'r & 'r < 'n. int('r) }",
        "val wider : forall 'm, 'm > 0. int('m) -> { 's, 0 <= 's & 's < 'm + 1. int('s) }",
        "function wider(m) = produce(m)",
    ])
    .unwrap();
}

#[test]
fn quantified_call_cannot_strengthen_an_existential() {
    assert!(check(&[
        "val produce : forall 'n, 'n > 1. int('n) -> { 'r, 0 <= 'r & 'r < 'n. int('r) }",
        "val smaller : forall 'm, 'm > 1. int('m) -> { 's, 0 <= 's & 's < 'm - 1. int('s) }",
        "function smaller(m) = produce(m)",
    ])
    .is_err());
}

#[test]
fn ordinary_call_does_not_assume_its_desired_result_predicate() {
    assert!(check(&[
        "val identity : forall 'n. int('n) -> int('n)",
        "val impossible : unit -> { 'r, false. int('r) }",
        "function impossible() = identity(0)",
    ])
    .is_err());
}

#[test]
fn ordinary_call_result_can_supply_a_valid_existential_witness() {
    check(&[
        "val identity : forall 'n. int('n) -> int('n)",
        "val within : unit -> { 'r, 0 <= 'r & 'r < 4. int('r) }",
        "function within() = identity(1)",
    ])
    .unwrap();
}

#[test]
fn existential_shadowing_does_not_reuse_the_call_argument() {
    assert!(
        check(&[
            "val produce : forall 'n, 'n > 0. int('n) -> { 'n, 'n == 0. int('n) }",
            "val wrong : int(42) -> int(42)",
            "function wrong(n) = produce(n)",
        ])
        .is_err(),
        "the existential witness shadows the quantified argument"
    );
}

#[test]
fn existential_shadowing_retains_its_own_witness_predicate() {
    check(&[
        "val produce : forall 'n, 'n > 0. int('n) -> { 'n, 'n == 0. int('n) }",
        "val zero : int(42) -> int(0)",
        "function zero(n) = produce(n)",
    ])
    .unwrap();
}

#[test]
fn nested_existentials_are_rejected_before_opening_witnesses() {
    let error = check(&[
        "val produce : unit -> { 'n, 'n == 0. { 'n, 'n == 1. int('n) } }",
        "val one : unit -> int(1)",
        "function one() = produce()",
    ])
    .expect_err("Sail forbids nested existential types");
    assert!(matches!(
        error.kind,
        sail_ty::error::TypeErrorKind::QuantError(_)
    ));
}
