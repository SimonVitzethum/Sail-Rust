//! Top-level annotations guide initializer checking; returns need a function.
use sail_ast::{SimpleInterner, Span};
use sail_parse::{ParserContext, RealParser};
use sail_ty::{program::Program, TypeResult};
use std::sync::Arc;

fn check(source: &str) -> TypeResult<usize> {
    let mut parser = RealParser::new();
    let definition = parser.parse_def(source, Span::DUMMY).unwrap();
    let interner: Arc<SimpleInterner> = Arc::new(parser.into_interner());
    let program = Program::new();
    program.check_definition(&definition, interner, 0)
}

#[test]
fn annotated_global_literals_check_against_the_annotation() {
    for source in [
        "let flag : bool = false",
        "let count : int = 4",
        "let empty : unit = ()",
    ] {
        check(source).unwrap();
    }
}

#[test]
fn annotated_global_initializers_cannot_change_type() {
    for source in [
        "let flag : bool = 4",
        "let count : int = false",
        "let empty : unit = true",
    ] {
        assert!(check(source).is_err(), "{source}");
    }
}

#[test]
fn a_global_initializer_cannot_return_from_a_function() {
    assert!(check("let count : int = return(4)").is_err());
}

fn check_sources(sources: &[&str]) -> TypeResult<()> {
    let mut context = ParserContext::default();
    let mut program = Program::new();
    let mut defs = Vec::new();
    for (position, source) in sources.iter().enumerate() {
        let mut parser = RealParser::with_context(context.clone());
        let def = parser.parse_def(source, Span::DUMMY).unwrap();
        context = parser.context();
        let it = Arc::new(parser.into_interner());
        program.declare(&def, it.clone(), position)?;
        defs.push((position, def, it));
    }
    // Body checks deliberately run in reverse order against frozen histories.
    for (position, def, it) in defs.into_iter().rev() {
        program.check_definition(&def, it, position)?;
    }
    Ok(())
}

#[test]
fn annotated_globals_are_available_in_subsequent_bodies() {
    check_sources(&[
        "let flag : bool = false",
        "let (count, other) : (int, bool) = (4, true)",
        "val use : int -> int",
        "function use(x) = if flag then count else x",
        "outcome read : int -> bool = { impl emulator(x) = other }",
    ])
    .unwrap();
}

#[test]
fn a_global_is_not_visible_in_its_own_initializer() {
    assert!(check_sources(&["let flag : bool = flag"]).is_err());
    assert!(check_sources(&[
        "val use : unit -> bool",
        "function use() = flag",
        "let flag : bool = false",
    ])
    .is_err());
}

#[test]
fn duplicate_global_names_cannot_change_earlier_body_views() {
    assert!(check_sources(&[
        "let flag : bool = false",
        "val first : unit -> bool",
        "function first() = flag",
        "let flag : int = 4",
        "val second : unit -> int",
        "function second() = flag",
    ])
    .is_err());
}

#[test]
fn a_global_name_cannot_be_rebound_in_its_next_initializer() {
    assert!(check_sources(&["let flag : bool = false", "let flag : bool = flag"]).is_err());
    assert!(check_sources(&["let flag = false", "let flag : bool = true"]).is_err());
    assert!(check_sources(&["let flag : bool = false", "let flag = true"]).is_err());
}

#[test]
fn declarations_do_not_make_bad_initializers_valid() {
    assert!(check_sources(&[
        "let flag : bool = 4",
        "val use : unit -> bool",
        "function use() = flag",
    ])
    .is_err());
}
