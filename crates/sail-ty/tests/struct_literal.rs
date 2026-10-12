#![forbid(unsafe_code)]
//! Lane-44 struct check+infer (OCaml `check_exp` E_struct ~L2218,
//! `infer_exp` E_struct ~L3774) via the canonical checker.
use sail_ast::{Def, SimpleInterner, Span};
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

#[test]
fn struct_check_named_anon_and_generic() {
    let defs = parse(&[
        "struct Point = { x : int, y : bool }",
        "function mk() -> Point = struct Point { x = 1, y = true }",
        "function anon_ok() -> Point = struct { x = 1, y = true }",
        "struct Box('a : Type) = { field : 'a }",
        "function mkbox() -> Box(int) = struct Box { field = 1 }",
    ]);
    let program = declare(&defs);
    for position in [1, 2, 4] {
        program
            .check_definition(&defs[position].0, defs[position].1.clone(), position)
            .unwrap();
    }
}

#[test]
fn struct_check_rejects_mismatch_and_coverage() {
    for source in [
        "function bad_type() -> int = struct Point { x = 1, y = true }",
        "function bad_name() -> Point = struct Other { x = 1, y = true }",
        "function unknown() -> Point = struct Point { x = 1, y = true, z = 0 }",
        "function dup() -> Point = struct Point { x = 1, x = 2, y = true }",
        "function missing() -> Point = struct Point { x = 1 }",
        "function wrong_field() -> Point = struct Point { x = true, y = true }",
    ] {
        let defs = parse(&[
            "struct Point = { x : int, y : bool }",
            "struct Other = { x : int, y : bool }",
            source,
        ]);
        let program = declare(&defs);
        let position = defs.len() - 1;
        assert!(
            program
                .check_definition(&defs[position].0, defs[position].1.clone(), position)
                .is_err(),
            "{source}"
        );
    }
}

#[test]
fn struct_infer_named_and_anon_never_infers() {
    let defs = parse(&[
        "struct Point = { x : int, y : bool }",
        "function via_let() -> Point = let p = struct Point { x = 1, y = true } in p",
    ]);
    let program = declare(&defs);
    program
        .check_definition(&defs[1].0, defs[1].1.clone(), 1)
        .unwrap();

    for source in [
        "function anon() -> Point = let p = struct { x = 1, y = true } in p",
        "function missing() -> Point = let p = struct Point { x = 1 } in p",
        "function unknown() -> Point = let p = struct Point { x = 1, y = true, z = 0 } in p",
    ] {
        let defs = parse(&["struct Point = { x : int, y : bool }", source]);
        let program = declare(&defs);
        let position = defs.len() - 1;
        assert!(
            program
                .check_definition(&defs[position].0, defs[position].1.clone(), position)
                .is_err(),
            "{source}"
        );
    }
}

#[test]
fn struct_infer_generic_binds_telescope() {
    let defs = parse(&[
        "struct Box('a : Type) = { field : 'a }",
        "function mk() -> Box(int) = let b = struct Box { field = 1 } in b",
    ]);
    let program = declare(&defs);
    program
        .check_definition(&defs[1].0, defs[1].1.clone(), 1)
        .unwrap();
    let defs = parse(&[
        "struct Box('a : Type) = { field : 'a }",
        "function bad() -> Box(bool) = let b = struct Box { field = 1 } in b",
    ]);
    let program = declare(&defs);
    assert!(program
        .check_definition(&defs[1].0, defs[1].1.clone(), 1)
        .is_err());
}
