//! Outcome implementations use the outcome signature and sequential local vals.
use sail_ast::{SimpleInterner, Span};
use sail_parse::{ParserContext, RealParser};
use sail_ty::{program::Program, TypeResult};
use std::sync::Arc;

fn check(sources: &[&str]) -> TypeResult<usize> {
    let mut parser_context = ParserContext::default();
    let mut program = Program::new();
    let mut count = 0;
    for (position, source) in sources.iter().enumerate() {
        let mut parser = RealParser::with_context(parser_context.clone());
        let def = parser.parse_def(source, Span::DUMMY).unwrap();
        parser_context = parser.context();
        let interner: Arc<SimpleInterner> = Arc::new(parser.into_interner());
        program.declare(&def, interner.clone(), position)?;
        count += program.check_definition(&def, interner, position)?;
    }
    Ok(count)
}

#[test]
fn implementation_names_do_not_need_global_signatures() {
    let count =
        check(&["outcome read : int -> int = { impl emulator(x) = x impl symbolic(x) = 4 }"])
            .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn implementations_check_result_and_argument_types() {
    for source in [
        "outcome read : int -> int = { impl emulator(x) = false }",
        "outcome read : (int, bool) -> int = { impl emulator(x) = x }",
        "outcome read : int -> int = { impl emulator(x : bool) = 4 }",
    ] {
        assert!(check(&[source]).is_err(), "{source}");
    }
}

#[test]
fn outcome_parameters_and_local_polymorphism_remain_distinct() {
    check(&["outcome read : 'a -> 'a with 'a : Type = { val identity : forall 'a. 'a -> 'a impl emulator(x) = identity(x) }"])
        .unwrap();
    assert!(check(&["outcome read : 'a -> 'a with 'a : Type = { val identity : forall 'a. 'a -> 'a impl emulator(x) = identity(false) }"]).is_err());
    check(&["outcome read : 'a -> int with 'a : Type = { val convert : 'a -> int impl emulator(x) = convert(x) }"])
        .unwrap();
}

#[test]
fn local_specs_are_visible_only_after_their_declarations() {
    assert!(check(&[
        "outcome read : int -> int = { impl emulator(x) = helper(x) val helper : int -> int }"
    ])
    .is_err());
    assert!(check(&[
        "outcome read : int -> int = { val helper : int -> int impl emulator(x) = helper(x) }",
        "val outside : int -> int",
        "function outside(x) = helper(x)",
    ])
    .is_err());
}

#[test]
fn local_spec_does_not_override_global_signature_outside_the_block() {
    check(&[
        "val helper : bool -> bool",
        "outcome read : int -> int = { val helper : int -> int impl emulator(x) = helper(x) }",
        "val outside : bool -> bool",
        "function outside(x) = helper(x)",
    ])
    .unwrap();
}

#[test]
fn header_constraints_are_available_in_implementations() {
    check(&["outcome read : forall 'n, 'n >= 4. int('n) -> { 'm, 'm >= 4. int('m) } = { impl emulator(x) = x }"])
        .unwrap();
    assert!(check(&["outcome read : forall 'n, 'n >= 4. int('n) -> { 'm, 'm >= 5. int('m) } = { impl emulator(x) = x }"]).is_err());
}

#[test]
fn implementation_requires_an_enclosing_outcome() {
    assert!(check(&["impl emulator(x) = x"]).is_err());
}

#[test]
fn header_quantifiers_are_not_in_scope_in_local_specs() {
    assert!(check(&["outcome read : forall 'n, 'n >= 4. int('n) -> int = { val helper : int('n) -> int impl emulator(x) = helper(x) }"]).is_err());
}
