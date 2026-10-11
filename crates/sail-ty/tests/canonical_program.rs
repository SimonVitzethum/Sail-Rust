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
        matches!(error.kind, TypeErrorKind::ConstraintNotProved(_)),
        "{error}"
    );
}

#[test]
fn constructor_constraints_are_substituted_and_proved_from_rigid_assumptions() {
    let defs = parse(&[
        "struct Sized('n : Int), 'n > 0 = { field : bitvector('n) }",
        "val positive : forall ('k : Int), 'k > 0. Sized('k) -> unit",
        "val shifted : forall ('k : Int), 'k >= 0. Sized('k + 1) -> unit",
        "val huge : Sized(340282366920938463463374607431768211456) -> unit",
        "val existential : { ('k : Int), 'k > 0. Sized('k) } -> unit",
        "val bad : Sized(0) -> unit",
        "val missing : forall ('k : Int). Sized('k) -> unit",
    ]);
    let mut program = Program::new();
    for position in 0..5 {
        program
            .declare(&defs[position].0, defs[position].1.clone(), position)
            .unwrap();
    }
    for position in 5..defs.len() {
        let error = program
            .declare(&defs[position].0, defs[position].1.clone(), position)
            .unwrap_err();
        assert!(
            matches!(error.kind, TypeErrorKind::ConstraintNotProved(_)),
            "{error}"
        );
    }
}

#[test]
fn constructor_substitution_is_simultaneous_and_boolean_sorted() {
    let defs = parse(&[
        "struct Ordered('n : Int, 'm : Int), 'n < 'm = { field : unit }",
        "struct Enabled('b : Bool), 'b = { field : unit }",
        "val swapped : forall ('n 'm : Int), 'm < 'n. Ordered('m, 'n) -> unit",
        "val enabled : Enabled(true) -> unit",
        "val bool_scope : forall ('b : Bool), 'b. Enabled('b) -> unit",
        "val bad_swap : forall ('n 'm : Int), 'm < 'n. Ordered('n, 'm) -> unit",
        "val disabled : Enabled(false) -> unit",
    ]);
    let mut program = Program::new();
    for position in 0..5 {
        program
            .declare(&defs[position].0, defs[position].1.clone(), position)
            .unwrap();
    }
    for position in 5..defs.len() {
        let error = program
            .declare(&defs[position].0, defs[position].1.clone(), position)
            .unwrap_err();
        assert!(
            matches!(error.kind, TypeErrorKind::ConstraintNotProved(_)),
            "{error}"
        );
    }
}

#[test]
fn existential_shadowing_never_reuses_outer_proof_binders_or_leaks_assumptions() {
    let defs = parse(&[
        "struct Sized('n : Int), 'n > 0 = { field : bitvector('n) }",
        "val good : forall ('n : Int), 'n < 0. { ('n : Int), 'n > 0. Sized('n) } -> unit",
        "val shadow : forall ('n : Int), 'n > 0. { ('n : Int), true. Sized('n) } -> unit",
        "val leak : forall ('n : Int). ({ ('k : Int), 'k > 0. Sized('k) }, Sized('n)) -> unit",
    ]);
    let mut program = Program::new();
    for position in 0..2 {
        program
            .declare(&defs[position].0, defs[position].1.clone(), position)
            .unwrap();
    }
    for position in 2..defs.len() {
        let error = program
            .declare(&defs[position].0, defs[position].1.clone(), position)
            .unwrap_err();
        assert!(
            matches!(error.kind, TypeErrorKind::ConstraintNotProved(_)),
            "{error}"
        );
    }
}

#[test]
fn proofs_use_only_earlier_global_constraints_and_keep_annotation_scopes() {
    let defs = parse(&[
        "struct Sized('n : Int), 'n > 0 = { field : bitvector('n) }",
        "type N : Int",
        "val before : Sized(N) -> unit",
        "constraint N > 0",
        "val after : Sized(N) -> unit",
        "val f : forall ('n : Int), 'n > 0. Sized('n) -> Sized('n)",
        "function f(x) = (x : Sized('n))",
    ]);
    let mut program = Program::new();
    for (position, (def, it)) in defs.iter().enumerate() {
        let result = program.declare(def, it.clone(), position);
        if position == 2 {
            assert!(matches!(
                result.unwrap_err().kind,
                TypeErrorKind::ConstraintNotProved(_)
            ));
        } else {
            result.unwrap();
        }
    }
    program
        .check_definition(&defs[6].0, defs[6].1.clone(), 6)
        .unwrap();
    // Source-position views remain valid after the later assumption exists.
    assert!(program
        .check_scheme(
            match &defs[2].0.kind {
                DefKind::Val(v) => &v.typschm,
                _ => panic!(),
            },
            &defs[2].1,
            2
        )
        .is_err());
}

#[test]
fn dependent_calls_instantiate_numeric_boolean_and_compound_results() {
    let defs = parse(&[
        "val add : forall 'n 'm. (int('n), int('m)) -> int('n + 'm)",
        "val equal : forall 'n 'm. (int('n), int('m)) -> bool('n == 'm)",
        "val negate : forall ('p : Bool). bool('p) -> bool(not('p))",
        "val answer : unit -> int(42)",
        "function answer() = add(20, 22)",
        "val nested : unit -> int(43)",
        "function nested() = add(add(20, 22), 1)",
        "val is_equal : unit -> bool(true)",
        "function is_equal() = equal(add(20, 22), 42)",
        "val negated : unit -> bool(false)",
        "function negated() = negate(equal(42, 42))",
        "val rigid : forall 'n. int('n) -> int('n + 1)",
        "function rigid(x) = add(x, 1)",
        "val bad : unit -> int(44)",
        "function bad() = add(20, 22)",
    ]);
    let program = declare(&defs);
    for position in [4, 6, 8, 10, 12] {
        program
            .check_definition(&defs[position].0, defs[position].1.clone(), position)
            .unwrap();
    }
    let error = program
        .check_definition(&defs[14].0, defs[14].1.clone(), 14)
        .unwrap_err();
    assert!(
        matches!(error.kind, TypeErrorKind::ConstraintNotProved(_)),
        "{error}"
    );
}

#[test]
fn dependent_calls_prove_constraints_and_reject_unrefined_argument_shortcuts() {
    let defs = parse(&[
        "val positive : forall 'n, 'n > 0. int('n) -> int('n)",
        "val same : forall 'n. (int('n), int('n)) -> unit",
        "val good : forall 'n, 'n > 1. int('n) -> int('n)",
        "function good(x) = positive(x)",
        "val wrong : unit -> int(0)",
        "function wrong() = positive(0)",
        "val unequal : unit -> unit",
        "function unequal() = same(1, 2)",
        "val unrefined : int -> int(1)",
        "function unrefined(x) = positive(x)",
    ]);
    let program = declare(&defs);
    program
        .check_definition(&defs[3].0, defs[3].1.clone(), 3)
        .unwrap();
    for position in [5, 7] {
        let error = program
            .check_definition(&defs[position].0, defs[position].1.clone(), position)
            .unwrap_err();
        assert!(
            matches!(error.kind, TypeErrorKind::ConstraintNotProved(_)),
            "{error}"
        );
    }
    assert!(program
        .check_definition(&defs[9].0, defs[9].1.clone(), 9)
        .is_err());
}

#[test]
fn recursive_and_sibling_dependent_calls_keep_each_instantiation_separate() {
    let defs = parse(&[
        "val add : forall 'n 'm. (int('n), int('m)) -> int('n + 'm)",
        "function add(x, y) = add(y, x)",
        "val sibling : unit -> int(10)",
        "function sibling() = add(add(1, 2), add(3, 4))",
        "val wrong_sibling : unit -> int(6)",
        "function wrong_sibling() = add(add(1, 2), add(3, 4))",
        "val recursive : forall 'n 'm, 'n > 0 & 'm < 0. (int('n), int('m)) -> int('n)",
        "function recursive(x, y) = recursive(y, x)",
    ]);
    let program = declare(&defs);
    for position in [1, 3] {
        program
            .check_definition(&defs[position].0, defs[position].1.clone(), position)
            .unwrap();
    }
    for position in [5, 7] {
        let error = program
            .check_definition(&defs[position].0, defs[position].1.clone(), position)
            .unwrap_err();
        assert!(
            matches!(error.kind, TypeErrorKind::ConstraintNotProved(_)),
            "{error}"
        );
    }
}

#[test]
fn plain_values_open_fresh_witnesses_and_opaque_results_do_not_become_constants() {
    let defs = parse(&[
        "val equal : forall 'n 'm. (int('n), int('m)) -> bool('n == 'm)",
        "val negate : forall ('p : Bool). bool('p) -> bool(not('p))",
        "val predicate : unit -> bool",
        "val unknown_int : unit -> int",
        "val reflexive : int -> bool(true)",
        "function reflexive(x) = equal(x, x)",
        "val invert_unknown : unit -> bool",
        "function invert_unknown() = negate(predicate())",
        "val distinct : (int, int) -> bool(true)",
        "function distinct(x, y) = equal(x, y)",
        "val invented_bool : unit -> bool(true)",
        "function invented_bool() = negate(predicate())",
        "val invented_int : unit -> int(1)",
        "function invented_int() = unknown_int()",
    ]);
    let program = declare(&defs);
    for position in [5, 7] {
        program
            .check_definition(&defs[position].0, defs[position].1.clone(), position)
            .unwrap();
    }
    for position in [9, 11, 13] {
        let error = program
            .check_definition(&defs[position].0, defs[position].1.clone(), position)
            .unwrap_err();
        assert!(
            matches!(error.kind, TypeErrorKind::ConstraintNotProved(_)),
            "{error}"
        );
    }
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

#[test]
fn ten_thousand_composed_dependent_results_check_and_release_iteratively() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut defs = parse(&[
                "val successor : forall 'n. int('n) -> int('n + 1)",
                "val f : unit -> int",
                "function f() = 0",
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
            let successor = Name::dummy(interner.intern("successor"));
            for _ in 0..10_000 {
                let previous = std::mem::replace(body, Node::dummy(ExpKind::Undef));
                *body = Node::dummy(ExpKind::App {
                    func: successor.clone(),
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
