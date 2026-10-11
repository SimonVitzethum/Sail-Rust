//! Real grammar -> canonical declarations -> actual expression type checking.
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

#[test]
fn canonical_checker_handles_calls_scopes_tuples_and_rigid_polymorphism() {
    let defs = parse(&[
        "val identity : int -> int",
        "function identity(x) = x",
        "val choose : int -> bool",
        "function choose(x) = if true then false else true",
        "val swap : (int, bool) -> (bool, int)",
        "function swap(x, y) = (y, x)",
        "val polymorphic : forall 'a. 'a -> 'a",
        "function polymorphic(x) = x",
        "val call : int -> int",
        "function call(x) = let y = identity(x) in y",
    ]);
    let program = declare(&defs);
    let mut expressions = 0;
    for (position, (def, it)) in defs.iter().enumerate() {
        expressions += program.check_definition(def, it.clone(), position).unwrap();
    }
    assert_eq!(expressions, 13);
}

#[test]
fn type_errors_are_real_and_later_val_specs_do_not_retype_earlier_bodies() {
    let defs = parse(&[
        "val f : int -> int",
        "function f(x) = x",
        "val f : bool -> bool",
        "val wrong : int -> bool",
        "function wrong(x) = x",
        "val unknown : int -> int",
        "function unknown(x) = absent",
        "val bad_call : unit -> int",
        "function bad_call() = f(1)",
    ]);
    let program = declare(&defs);
    assert!(program
        .check_definition(&defs[1].0, defs[1].1.clone(), 1)
        .is_ok());
    assert_eq!(
        program.value("f", 1).unwrap().interner.resolve(
            match &program.value("f", 1).unwrap().typschm.typ.kind {
                TypKind::Fn { ret, .. } => match &ret.kind {
                    TypKind::Id(name) => name.symbol,
                    _ => panic!(),
                },
                _ => panic!(),
            }
        ),
        Some("int")
    );
    for position in [4, 8] {
        let error = program
            .check_definition(&defs[position].0, defs[position].1.clone(), position)
            .unwrap_err();
        assert!(
            matches!(error.kind, TypeErrorKind::Mismatch { .. }),
            "{error}"
        );
    }
    let error = program
        .check_definition(&defs[6].0, defs[6].1.clone(), 6)
        .unwrap_err();
    assert!(
        matches!(error.kind, TypeErrorKind::UnboundVar(_)),
        "{error}"
    );
}

#[test]
fn declaration_kinds_and_source_order_are_checked() {
    let defs = parse(&[
        "val wrong : Later -> unit",
        "struct Later = { field : int }",
    ]);
    let mut program = Program::new();
    assert!(matches!(
        program
            .declare(&defs[0].0, defs[0].1.clone(), 0)
            .unwrap_err()
            .kind,
        TypeErrorKind::UnboundVar(_)
    ));
    let defs = parse(&[
        "struct Box('a : Type) = { field : 'a }",
        "val use : Box(int) -> unit",
    ]);
    declare(&defs);
    let defs = parse(&["val negate : forall ('p : Bool). bool('p) -> bool(not('p))"]);
    declare(&defs);
}

#[test]
fn unsupported_obligations_do_not_silently_accept_bodies() {
    let defs = parse(&["val f : unit -> bitvector(8)", "function f() = 0x01"]);
    let program = declare(&defs);
    assert!(matches!(
        program
            .check_definition(&defs[1].0, defs[1].1.clone(), 1)
            .unwrap_err()
            .kind,
        TypeErrorKind::Unsupported(_)
    ));
}

#[test]
fn ten_thousand_let_scopes_check_and_release_without_recursion() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut defs = parse(&["val f : unit -> unit", "function f() = ()"]);
            let program = declare(&defs);
            let (mut def, it) = defs.pop().unwrap();
            let DefKind::Fundef { funcls, .. } = &mut def.kind else {
                panic!()
            };
            let PExpKind::PatExp { body, .. } = &mut funcls[0].arm.kind else {
                panic!()
            };
            let mut interner = it.as_ref().clone();
            let x = Name::dummy(interner.intern("x"));
            for _ in 0..10_000 {
                let previous = std::mem::replace(body, Node::dummy(ExpKind::Undef));
                *body = Node::dummy(ExpKind::Let {
                    pat: Node::dummy(PatKind::Id(x.clone())),
                    bind: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Unit)))),
                    body: Box::new(previous),
                });
            }
            assert_eq!(
                program
                    .check_definition(&def, Arc::new(interner), 1)
                    .unwrap(),
                20_001
            );
            sail_ast::storage::drop_definition(def);
        })
        .unwrap()
        .join()
        .unwrap();
}
