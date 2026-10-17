//! Top-level annotations guide initializer checking; returns need a function.
use sail_ast::{SimpleInterner, Span};
use sail_parse::RealParser;
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
