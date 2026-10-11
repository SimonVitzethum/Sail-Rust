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
    let defs = parse(&[
        "val f : forall 'n. unit -> int('n)",
        "function f() = sizeof('n)",
    ]);
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
fn literals_and_rigid_dependent_identities_keep_their_refinements() {
    let defs = parse(&[
        "val number : unit -> int(7)",
        "function number() = 7",
        "val bits8 : unit -> bitvector(8)",
        "function bits8() = 0x01",
        "val bit1 : unit -> bitvector(1)",
        "function bit1() = 0b0",
        "val truth : unit -> bool(true)",
        "function truth() = true",
        "val dependent : forall 'n. int('n) -> int('n)",
        "function dependent(x) = x",
        "val dependent_bool : forall ('p : Bool). bool('p) -> bool('p)",
        "function dependent_bool(x) = x",
        "val wrong : unit -> bitvector(7)",
        "function wrong() = 0x01",
    ]);
    let program = declare(&defs);
    for i in [1, 3, 5, 7, 9, 11] {
        program
            .check_definition(&defs[i].0, defs[i].1.clone(), i)
            .unwrap();
    }
    assert!(matches!(
        program
            .check_definition(&defs[13].0, defs[13].1.clone(), 13)
            .unwrap_err()
            .kind,
        TypeErrorKind::ConstraintNotProved(_)
    ));
}

#[test]
fn annotations_validate_names_and_preserve_rigid_binder_identity() {
    let defs = parse(&[
        "val identity : forall 'a. 'a -> 'a",
        "function identity(x : 'a) = (x : 'a)",
        "val wrong : unit -> int",
        "function wrong() = let unused = (undefined : Missing) in 1",
        "val free : unit -> int",
        "function free() = let unused = (undefined : 'missing) in 1",
    ]);
    let program = declare(&defs);
    assert!(program
        .check_definition(&defs[1].0, defs[1].1.clone(), 1)
        .is_ok());
    for i in [3, 5] {
        let err = program
            .check_definition(&defs[i].0, defs[i].1.clone(), i)
            .unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::UnboundVar(_)), "{err}");
    }
}

#[test]
fn polymorphic_calls_are_fresh_and_share_one_binding_across_arguments() {
    let defs = parse(&[
        "val identity : forall 'a. 'a -> 'a",
        "val first : forall 'a. ('a, 'a) -> 'a",
        "val generic : forall 'b. 'b -> 'b",
        "function generic(x) = identity(x)",
        "val integer : int -> int",
        "function integer(x) = identity(identity(x))",
        "val boolean : bool -> bool",
        "function boolean(x) = identity(x)",
        "val lists : list(int) -> list(int)",
        "function lists(x) = identity(x)",
        "val wrong : unit -> int",
        "function wrong() = first(1, true)",
    ]);
    let program = declare(&defs);
    for i in [3, 5, 7, 9] {
        program
            .check_definition(&defs[i].0, defs[i].1.clone(), i)
            .unwrap();
    }
    assert!(matches!(
        program
            .check_definition(&defs[11].0, defs[11].1.clone(), 11)
            .unwrap_err()
            .kind,
        TypeErrorKind::Mismatch { .. }
    ));
}

#[test]
fn enumeration_patterns_are_constants_and_union_values_have_signatures() {
    let defs = parse(&[
        "enum Colour = { Red, Green }",
        "union Wrapped = { Wrap : int }",
        "val colour : Colour -> Colour",
        "function colour(Red) = Green",
        "val wrap : int -> Wrapped",
        "function wrap(x) = Wrap(x)",
        "val wrong : int -> int",
        "function wrong(Red) = 1",
        "register state : int",
        "val shadow : int -> int",
        "function shadow(state) = state",
    ]);
    let program = declare(&defs);
    for i in [3, 5] {
        program
            .check_definition(&defs[i].0, defs[i].1.clone(), i)
            .unwrap();
    }
    assert!(matches!(
        program
            .check_definition(&defs[7].0, defs[7].1.clone(), 7)
            .unwrap_err()
            .kind,
        TypeErrorKind::Mismatch { .. }
    ));
    assert!(matches!(
        program
            .check_definition(&defs[10].0, defs[10].1.clone(), 10)
            .unwrap_err()
            .kind,
        TypeErrorKind::RegisterError(_)
    ));
}

#[test]
fn recursive_types_and_bad_bitfield_index_names_are_rejected() {
    for source in [
        "struct Recursive = { next : Recursive }",
        "union Recursive = { Next : Recursive }",
        "bitfield Flags : bits(8) = { field : unknown }",
    ] {
        let defs = parse(&[source]);
        let mut program = Program::new();
        let error = program
            .declare(&defs[0].0, defs[0].1.clone(), 0)
            .unwrap_err();
        assert!(
            matches!(error.kind, TypeErrorKind::UnboundVar(_)),
            "{error}"
        );
        assert!(program.value("Next", 0).is_none());
    }
}

#[test]
fn constrained_type_applications_require_a_proof_instead_of_kind_only_success() {
    let mut defs = parse(&[
        "struct Sized('n : Int) = { field : bitvector('n) }",
        "val use : Sized(8) -> unit",
    ]);
    let DefKind::Type(typ) = &mut defs[0].0.kind else {
        panic!()
    };
    let TypeDefKind::Record { quant, .. } = &mut typ.kind else {
        panic!()
    };
    quant.push(QuantItem::Constraint(Node::dummy(ConstraintKind::False)));
    let mut program = Program::new();
    program.declare(&defs[0].0, defs[0].1.clone(), 0).unwrap();
    let error = program
        .declare(&defs[1].0, defs[1].1.clone(), 1)
        .unwrap_err();
    assert!(
        matches!(error.kind, TypeErrorKind::Unsupported(_)),
        "{error}"
    );
}

#[test]
fn ten_thousand_nested_polymorphic_calls_use_iterative_cached_synthesis() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut defs = parse(&[
                "val identity : forall 'a. 'a -> 'a",
                "val f : int -> int",
                "function f(x) = x",
            ]);
            let program = declare(&defs);
            let (mut def, it) = defs.pop().unwrap();
            let DefKind::Fundef { funcls, .. } = &mut def.kind else {
                panic!()
            };
            let PExpKind::PatExp { body, .. } = &mut funcls[0].arm.kind else {
                panic!()
            };
            let mut interner = it.as_ref().clone();
            let name = Name::dummy(interner.intern("identity"));
            for _ in 0..10_000 {
                let previous = std::mem::replace(body, Node::dummy(ExpKind::Undef));
                *body = Node::dummy(ExpKind::App {
                    func: name.clone(),
                    args: vec![previous],
                });
            }
            assert_eq!(
                program
                    .check_definition(&def, Arc::new(interner), 2)
                    .unwrap(),
                10_001
            );
            sail_ast::storage::drop_definition(def);
        })
        .unwrap()
        .join()
        .unwrap();
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
