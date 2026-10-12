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

const FLOW_DECLARATIONS: &[&str] = &[
    "val ge : forall 'n 'm. (int('n), int('m)) -> bool('n >= 'm)",
    "val lt : forall 'n 'm. (int('n), int('m)) -> bool('n < 'm)",
    "val eq : forall 'n 'm. (int('n), int('m)) -> bool('n == 'm)",
    "val neg : forall 'n. int('n) -> int(- 'n)",
    "overload negate = {neg}",
    "val not_boolean : forall ('p : Bool). bool('p) -> bool(not('p))",
    "val nonnegative : forall 'n, 'n >= 0. int('n) -> int",
    "val bounded : forall 'n, 0 <= 'n <= 4. int('n) -> int",
    "val only_true : forall ('p : Bool), 'p. bool('p) -> unit",
    "val zero : int(0) -> int",
];

const MUTABLE_DECLARATIONS: &[&str] = &[
    "register R : int",
    "val opaque_int : unit -> int",
    "val opaque_bool : unit -> bool",
    "val bounded_index : forall 'n, 0 <= 'n <= 2. int('n) -> unit",
    "val at_least_zero : forall 'n, 'n >= 0. int('n) -> unit",
    "val ge_zero : forall 'n. int('n) -> bool('n >= 0)",
    "val only_true : forall ('p : Bool), 'p. bool('p) -> unit",
];
const MUTABLE_POSITIVE: &[&str] = &[
    "function mutate() -> int = { var x : int = 0; x = 1; x }",
    "function inferred_mutate() -> int = { var x = opaque_int(); x = 7; x }",
    "function inferred_boolean() -> bool = { var x = opaque_bool(); x = false; x }",
    "function inferred_constant() -> int = { var x = 0; x = 0; x }",
    "function tuple_mutate() -> int = { var (x, y) = (opaque_int(), true); (x, y) = (1, true); x }",
    "function write_register() -> int = { R = 3; R }",
    "function vector_inference() -> unit = let values = [opaque_int(), 1] in ()",
    "function let_scope() -> int = { let x = 1; let y = x; y }",
    "function ascending() -> unit = foreach (i from 0 to 2) bounded_index(i)",
    "function descending() -> unit = foreach (i from 2 downto 0) bounded_index(i)",
    "function bounded_mutate() -> int = { var x : int = 0; foreach (i from 0 to 2) { x = i }; x }",
    "function while_loop() -> unit = while true do ()",
    "function repeat_loop() -> unit = repeat () until true",
    "function while_fact(x : int) -> unit = while ge_zero(x) do at_least_zero(x)",
    "function mutable_condition() -> unit = { var x : bool = true; while x do { x = false } }",
];
#[test]
fn mutable_declarations_assignments_and_loop_bounds_are_checked() {
    let mut sources = MUTABLE_DECLARATIONS.to_vec();
    sources.extend(MUTABLE_POSITIVE);
    let positive = sources.len();
    sources.extend([
        "function wrong_init() -> int = { var x : int = false; x }",
        "function let_escape() -> int = { { let x = 1; () }; x }",
        "function var_escape() -> int = { { var x : int = 1; () }; x }",
        "function wrong_write() -> int = { var x : int = 0; x = false; x }",
        "function constant_write() -> int = { var x = 0; x = 1; x }",
        "function immutable_write() -> int = { let x = 0; x = 0; x }",
        "function shadow_local(x : int) -> int = { var x : int = 0; x }",
        "function shadow_mutable() -> int = { var x : int = 0; var x : int = 1; x }",
        "function shadow_register() -> int = { var R : int = 0; R }",
        "function missing_target() -> unit = missing = 0",
        "function wrong_register() -> unit = R = false",
        "function tuple_write() -> unit = { var (x, y) = (opaque_int(), true); (x, y) = (1, false) }",
        "function bad_bounds() -> unit = foreach (i from 0 to 3) bounded_index(i)",
        "function bad_descending() -> unit = foreach (i from 3 downto 0) bounded_index(i)",
        "function write_iterator() -> unit = foreach (i from 0 to 2) { i = 1 }",
        "function bound_escape() -> unit = { foreach (i from 0 to 2) (); bounded_index(i) }",
        "function wrong_bound_type() -> unit = foreach (i from false to 2) ()",
        "function wrong_body_type() -> unit = foreach (i from 0 to 2) 1",
        "function wrong_condition() -> unit = while 0 do ()",
        "function until_fact(x : int) -> unit = repeat at_least_zero(x) until ge_zero(x)",
        "function stale_fact() -> unit = { var x : bool = true; if x then { x = false; only_true(x) } else () }",
    ]);
    let defs = parse(&sources);
    let program = declare(&defs);
    for (position, (def, it)) in defs.iter().enumerate() {
        let result = program.check_definition(def, it.clone(), position);
        assert_eq!(
            result.is_err(),
            position >= positive,
            "position {position}: {result:?}"
        );
    }
}

#[test]
fn ten_thousand_mutable_and_loop_scopes_check_and_release_iteratively() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut interner = SimpleInterner::new();
            let names: Vec<_> = (0..10_000)
                .map(|i| Name::dummy(interner.intern(&format!("local_{i}"))))
                .collect();
            let loop_var = Name::dummy(interner.intern("index"));
            let definitions = parse(&["function deep() -> unit = ()"]);
            let (mut def, existing) = definitions.into_iter().next().unwrap();
            // Use the parser's symbols for the function and type, and allocate all
            // local symbols in that same interner without copying recursive ASTs.
            let mut merged = (*existing).clone();
            let names: Vec<_> = names
                .iter()
                .map(|name| Name::dummy(merged.intern(interner.resolve(name.symbol).unwrap())))
                .collect();
            let loop_var = Name::dummy(merged.intern(interner.resolve(loop_var.symbol).unwrap()));
            let zero = merged.intern("0");
            let one = merged.intern("1");
            let interner = Arc::new(merged);
            for loops in [false, true] {
                let DefKind::Fundef { funcls, .. } = &mut def.kind else {
                    panic!()
                };
                let PExpKind::PatExp { body, .. } = &mut funcls[0].arm.kind else {
                    panic!()
                };
                let mut nested = Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Unit)));
                for name in &names {
                    nested = Node::dummy(if loops {
                        ExpKind::For {
                            var: loop_var.clone(),
                            from: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Num(
                                zero,
                            ))))),
                            to: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Num(
                                one,
                            ))))),
                            step: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Num(
                                one,
                            ))))),
                            order: Order::Inc,
                            body: Box::new(nested),
                        }
                    } else {
                        ExpKind::Var {
                            lexp: Box::new(Node::dummy(LExpKind::Id(name.clone()))),
                            init: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Unit)))),
                            body: Box::new(nested),
                        }
                    });
                }
                *body = nested;
                let mut program = Program::new();
                program.declare(&def, interner.clone(), 0).unwrap();
                assert_eq!(
                    program.check_definition(&def, interner.clone(), 0).unwrap(),
                    if loops { 40_001 } else { 20_001 }
                );
                let DefKind::Fundef { funcls, .. } = &mut def.kind else {
                    panic!()
                };
                let PExpKind::PatExp { body, .. } = &mut funcls[0].arm.kind else {
                    panic!()
                };
                let old =
                    std::mem::replace(body, Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Unit))));
                sail_ast::storage::drop_definition(Node::dummy(DefKind::Let {
                    pat: Node::dummy(PatKind::Wild),
                    exp: old,
                }));
                drop(program);
            }
            sail_ast::storage::drop_definition(def);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn bit_literals_and_vectors_check_exact_lengths_elements_and_inferred_calls() {
    let mut sources = vec![
        "type bit = bitvector(1)",
        "function one() -> bit = bitone",
        "function zero_bit() -> bit = bitzero",
        "function two() -> bitvector(2) = [bitone, bitzero]",
        "function numbers() -> vector(2, int) = [1, 2]",
        "function nested() -> vector(2, bitvector(2)) = [[bitone, bitzero], [bitzero, bitone]]",
        "function tuples() -> vector(1, (int, bool)) = [(1, true)]",
        "val pass : forall 'n. bitvector('n) -> bitvector('n)",
        "function call() -> bitvector(2) = pass([bitone, bitzero])",
        "val keep : vector(2, int) -> vector(2, int)",
        "function call_vector() -> vector(2, int) = keep([1, 2])",
        "function empty_bits() -> bitvector(0) = []",
        "function empty_vector() -> vector(0, int) = []",
        "val accepts : bit -> unit",
        "function accepts(bitone) = ()",
    ];
    let positive = sources.len();
    sources.extend([
        "function integer() -> int = bitone",
        "function wrong_bits_length() -> bitvector(2) = [bitone]",
        "function wrong_bits_element() -> bitvector(1) = [true]",
        "function wrong_vector_element() -> vector(2, int) = [1, false]",
        "function wrong_vector_length() -> vector(3, int) = [1, 2]",
        "function missing_element() -> bitvector(2) = pass([bitone, absent])",
        "function second_width() -> bitvector(2) = [bitone, 0x12]",
        "function inferred_constants() -> unit = let x = [1, 2] in ()",
    ]);
    let defs = parse(&sources);
    let program = declare(&defs);
    for (position, (def, it)) in defs.iter().enumerate() {
        let result = program.check_definition(def, it.clone(), position);
        assert_eq!(
            result.is_err(),
            position >= positive,
            "position {position}: {result:?}"
        );
    }
}

#[test]
fn ten_thousand_nested_vector_literals_infer_check_and_release_iteratively() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut defs = parse(&["function deep() -> unit = let value = () in ()"]);
            let (mut def, it) = defs.pop().unwrap();
            let DefKind::Fundef { funcls, .. } = &mut def.kind else {
                panic!()
            };
            let PExpKind::PatExp { body, .. } = &mut funcls[0].arm.kind else {
                panic!()
            };
            let ExpKind::Let { bind, .. } = &mut body.kind else {
                panic!()
            };
            let mut nested = Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Unit)));
            for _ in 0..10_000 {
                nested = Node::dummy(ExpKind::Vector(vec![nested]));
            }
            **bind = nested;
            let mut program = Program::new();
            program.declare(&def, it.clone(), 0).unwrap();
            assert_eq!(program.check_definition(&def, it, 0).unwrap(), 10_003);
            sail_ast::storage::drop_definition(def);
            drop(program);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn type_synonyms_compose_type_integer_boolean_and_call_substitutions() {
    let defs = parse(&[
        "type Bits('width : Int) = bitvector('width)",
        "type Integer = int",
        "type Identity('element : Type) = 'element",
        "type Flag('predicate : Bool) = bool('predicate)",
        "type Bigger('size : Int) = Bits('size + 1)",
        "type Pair('element : Type) = (Identity('element), Bits(8))",
        "val forward : forall 'n, 'n >= 0. (implicit('n), Bits('n)) -> Bits('n)",
        "function forward(n, b) = b",
        "function caller() -> Bits(8) = forward(8, 0x12)",
        "val identity : forall 'a. Identity('a) -> Identity('a)",
        "function identity(x) = x",
        "function number() -> Integer = identity(7)",
        "val not_boolean : forall ('p : Bool). bool('p) -> bool(not('p))",
        "val flip : forall ('p : Bool). Flag('p) -> Flag(not('p))",
        "function flip(p) = not_boolean(p)",
        "function truth() -> Flag(false) = flip(true)",
        "val bigger : forall 'n. (int('n), Bigger('n)) -> unit",
        "function width() -> unit = bigger(7, 0x12)",
        "val unpack : Pair(int) -> int",
        "function unpack(x, bits) = x",
        "function wrong_width() -> unit = bigger(8, 0x12)",
        "function wrong_flag() -> Flag(true) = flip(true)",
        "function wrong_argument() -> Bits(8) = forward(true, 0x12)",
    ]);
    let program = declare(&defs);
    for (position, (def, it)) in defs.iter().enumerate() {
        let result = program.check_definition(def, it.clone(), position);
        assert_eq!(
            result.is_err(),
            position >= 20,
            "position {position}: {result:?}"
        );
    }
}

#[test]
fn ten_thousand_parametric_aliases_expand_and_release_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut interner = SimpleInterner::new();
            let parameter = interner.intern("element");
            let list = Name::dummy(interner.intern("list"));
            let names: Vec<_> = (0..10_000)
                .map(|i| Name::dummy(interner.intern(&format!("Alias_{i}"))))
                .collect();
            let interner = Arc::new(interner);
            let mut program = Program::new();
            for (position, name) in names.iter().enumerate() {
                let target = if position == 0 {
                    list.clone()
                } else {
                    names[position - 1].clone()
                };
                let def = Node::dummy(DefKind::Type(Node::dummy(TypeDefKind::Abbrev {
                    name: name.clone(),
                    quant: vec![QuantItem::Id(KOpt {
                        kind: Kind::Type,
                        kid: parameter,
                    })],
                    arg: Node::dummy(TypArgKind::Typ(Node::dummy(TypKind::App {
                        name: target,
                        args: vec![Node::dummy(TypArgKind::Typ(Node::dummy(TypKind::Var(
                            parameter,
                        ))))],
                    }))),
                })));
                program.declare(&def, interner.clone(), position).unwrap();
                sail_ast::storage::drop_definition(def);
            }
            let defs = parse(&[
                "type Alias_9999('a : Type) = 'a",
                "val f : Alias_9999(int) -> list(int)",
                "function f(x) = x",
            ]);
            program
                .declare(&defs[1].0, defs[1].1.clone(), 10_000)
                .unwrap();
            program
                .check_definition(&defs[2].0, defs[2].1.clone(), 10_001)
                .unwrap();
            drop(program);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn explicit_implicit_parameters_have_integer_singleton_semantics() {
    let defs = parse(&[
        "val forward : forall 'n, 'n >= 0. implicit('n) -> int('n)",
        "function forward(n) = n",
        "val consume : forall 'n. int('n) -> int('n)",
        "val relay : forall 'n. implicit('n) -> int('n)",
        "function relay(n) = consume(n)",
        "function literal() -> int(7) = forward(7)",
        "val receive : forall 'n. int('n) -> implicit('n)",
        "function receive(n) = n",
        "function wrong() -> int(8) = forward(7)",
        "function uninferred() -> int = forward(())",
    ]);
    let program = declare(&defs);
    for (position, (def, it)) in defs.iter().enumerate() {
        let result = program.check_definition(def, it.clone(), position);
        assert_eq!(
            result.is_err(),
            position >= 7,
            "position {position}: {result:?}"
        );
    }
}

#[test]
fn conditional_and_guard_facts_prove_constraints_with_borrowed_call_substitutions() {
    let mut sources = FLOW_DECLARATIONS.to_vec();
    sources.extend([
        "function shift(n : int) -> int = if ge(n, 0) then nonnegative(n) else nonnegative(neg(n))",
        "function nested(n : int) -> int = if ge(n, 0) then (if lt(n, 5) then bounded(n) else 0) else 0",
        "function alias(n : int) -> int = if ge(n, 0) then let m = n in nonnegative(m) else 0",
        "function same(n : int) -> int = if eq(n, 0) then zero(n) else 0",
        "function boolean(p : bool) -> unit = if p then only_true(p) else only_true(not_boolean(p))",
        "function checked_condition() -> unit = if (if true then true else false) then () else ()",
        "val guarded : int -> int",
        "function guarded(n if ge(n, 0)) = nonnegative(n)",
        "function impossible() -> int = if false then nonnegative(-1) else 0",
    ]);
    let defs = parse(&sources);
    let program = declare(&defs);
    for (position, (def, it)) in defs.iter().enumerate() {
        program.check_definition(def, it.clone(), position).unwrap();
    }
}

#[test]
fn conditional_facts_never_leak_to_siblings_outer_scopes_or_other_clauses() {
    for source in [
        "function wrong(n : int) -> int = if ge(n, 0) then nonnegative(n) else nonnegative(n)",
        "function wrong(n : int, m : int) -> int = if ge(m, 0) then nonnegative(n) else 0",
        "function wrong(n : int) -> int = { if ge(n, 0) then () else (); nonnegative(n) }",
        "function wrong(n : int, p : bool) -> int = if p then nonnegative(n) else 0",
        "function wrong(p : bool) -> int = if ge(p, 0) then 0 else 0",
        "function wrong(n if ge(n, 0)) = nonnegative(n) and wrong(n) = nonnegative(n)",
    ] {
        let mut sources = FLOW_DECLARATIONS.to_vec();
        sources.extend(["val wrong : int -> int", source]);
        // Functions with inline annotations need their own argument signature.
        if source.contains("->") {
            sources.remove(FLOW_DECLARATIONS.len());
        }
        let defs = parse(&sources);
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
fn ten_thousand_conditional_fact_scopes_check_and_release_without_recursion() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut defs = parse(&["function deep() -> unit = ()"]);
            let (mut def, it) = defs.pop().unwrap();
            let DefKind::Fundef { funcls, .. } = &mut def.kind else {
                panic!()
            };
            let PExpKind::PatExp { body, .. } = &mut funcls[0].arm.kind else {
                panic!()
            };
            let mut nested = Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Unit)));
            for _ in 0..10_000 {
                nested = Node::dummy(ExpKind::If {
                    cond: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(LitKind::True)))),
                    then_branch: Box::new(nested),
                    else_branch: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Unit)))),
                });
            }
            *body = nested;
            let mut program = Program::new();
            program.declare(&def, it.clone(), 0).unwrap();
            assert_eq!(program.check_definition(&def, it, 0).unwrap(), 30_001);
            sail_ast::storage::drop_definition(def);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn overloads_check_all_arguments_results_and_dependent_constraints() {
    let defs = parse(&[
        "val integer : int -> int",
        "val boolean : bool -> bool",
        "overload choose = {integer, boolean}",
        "function pick_bool(x : bool) -> bool = choose(x)",
        "function pick_int(x : int) -> int = choose(x)",
        "val wrong_pair : (int, bool) -> int",
        "val right_pair : (int, int) -> int",
        "overload pair = {wrong_pair, right_pair}",
        "function pair_call(x : int) -> int = pair(x, x)",
        "val returns_bool : int -> bool",
        "overload result = {returns_bool, integer}",
        "function result_call(x : int) -> int = result(x)",
        "val positive : forall 'n, 'n > 0. int('n) -> int('n)",
        "val any_integer : forall 'n. int('n) -> int('n)",
        "overload constrained = {positive, any_integer}",
        "function nonpositive() -> int(0) = constrained(0)",
        "function positive_call() -> int(1) = constrained(1)",
        "overload nested = {choose}",
        "function nested_call(x : bool) -> bool = nested(x)",
        "function bad_arg() -> int = choose(())",
        "function bad_result(x : bool) -> int = choose(x)",
        "function bad_pair() -> int = pair(true, 0)",
    ]);
    let program = declare(&defs);
    for (position, (def, it)) in defs.iter().enumerate() {
        let result = program.check_definition(def, it.clone(), position);
        assert_eq!(
            result.is_err(),
            position >= 19,
            "position {position}: {result:?}"
        );
    }
}

#[test]
fn overload_extensions_preserve_source_position_and_candidate_order() {
    let defs = parse(&[
        "val first : int -> int(1)",
        "val second : int -> int(2)",
        "overload select = {first}",
        "function before() -> int(1) = let x = select(0) in x",
        "overload select = {second}",
        "function after() -> int(1) = let x = select(0) in x",
        "function wrong() -> int(2) = let x = select(0) in x",
        "function too_early() -> bool = select(true)",
        "val boolean : bool -> bool",
        "overload select = {boolean}",
        "function later() -> bool = select(true)",
    ]);
    let program = declare(&defs);
    for position in [3, 5, 10] {
        program
            .check_definition(&defs[position].0, defs[position].1.clone(), position)
            .unwrap();
    }
    assert!(program
        .check_definition(&defs[6].0, defs[6].1.clone(), 6)
        .is_err());
    assert!(program
        .check_definition(&defs[7].0, defs[7].1.clone(), 7)
        .is_err());
}

#[test]
fn overload_declarations_reject_missing_candidates_conflicts_and_cycles() {
    for sources in [
        vec!["overload choose = {missing}"],
        vec!["val choose : int -> int", "overload choose = {choose}"],
    ] {
        let defs = parse(&sources);
        let mut program = Program::new();
        for (position, (def, it)) in defs.iter().enumerate() {
            let result = program.declare(def, it.clone(), position);
            assert_eq!(result.is_err(), position + 1 == defs.len());
        }
    }
    let defs = parse(&[
        "val integer : int -> int",
        "overload choose = {integer}",
        "overload nested = {choose}",
        "overload choose = {nested}",
        "function cyclic(x : int) -> int = choose(x)",
    ]);
    let program = declare(&defs);
    assert!(program
        .check_definition(&defs[4].0, defs[4].1.clone(), 4)
        .is_err());
}

#[test]
fn inline_return_annotations_synthesize_source_ordered_recursive_signatures() {
    let defs = parse(&[
        "function identity(x : int) -> int = x",
        "function truth() -> bool(true) = true",
        "function swap(x : int, y : bool) -> (bool, int) = (y, x)",
        "function recursive(x : int) -> int = recursive(x)",
        "function generic forall 'a. (x : 'a) -> 'a = x",
        "function width forall 'n. (x : bitvector('n)) -> unit = ()",
        "function flag forall 'p. (x : bool('p)) -> unit = ()",
        "val call : int -> int",
        "function call(x) = identity(x)",
        "val identity : bool -> bool",
        "function fixed_number(7) -> int(7) = 7",
    ]);
    let program = declare(&defs);
    for (position, (def, it)) in defs.iter().enumerate() {
        program.check_definition(def, it.clone(), position).unwrap();
    }
    assert!(program.value("identity", 0).is_some());
    assert!(program.value("truth", 0).is_none());
    for (name, kind) in [
        ("width", Kind::Int),
        ("flag", Kind::Bool),
        ("generic", Kind::Type),
    ] {
        assert!(
            matches!(&program.value(name,10).unwrap().typschm.quant[0],QuantItem::Id(k) if k.kind==kind)
        );
    }
}

#[test]
fn inline_signatures_validate_existing_vals_and_reject_inconsistent_headers() {
    let defs = parse(&[
        "val identity : forall 'a. 'a -> 'a",
        "function identity(x : 'a) -> 'a = x",
        "val wrong : int -> bool",
        "function wrong(x : int) -> int = x",
        "val duplicate : forall 'a. 'a -> 'a",
        "function duplicate forall 'a. (x : 'a) -> 'a = x",
    ]);
    let mut program = Program::new();
    for (position, (def, it)) in defs.iter().enumerate() {
        if [3, 5].contains(&position) {
            assert!(program.declare(def, it.clone(), position).is_err());
            assert!(program.check_definition(def, it.clone(), position).is_err());
        } else {
            program.declare(def, it.clone(), position).unwrap();
            program.check_definition(def, it.clone(), position).unwrap();
        }
    }
    for source in [
        "function unknown(x) -> int = 0",
        "function unknown(x : Missing) -> int = 0",
        "function ambiguous(x : int) -> int = x and ambiguous(y : int) = y",
    ] {
        let defs = parse(&[source]);
        let mut program = Program::new();
        assert!(
            program.declare(&defs[0].0, defs[0].1.clone(), 0).is_err(),
            "{source}"
        );
        assert!(program.value("unknown", 0).is_none());
        assert!(program.value("ambiguous", 0).is_none());
    }
    let defs = parse(&[
        "val f : int -> int",
        "val g : int -> int",
        "function f(x) = x and g(y) = y",
    ]);
    let program = declare(&defs);
    assert!(program
        .check_definition(&defs[2].0, defs[2].1.clone(), 2)
        .is_err());
}

#[test]
fn inline_pattern_inference_and_header_cleanup_handle_deep_tuples_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            for bad in [false, true] {
                let mut defs = parse(&["function deep() -> unit = ()"]);
                let (mut def, it) = defs.pop().unwrap();
                let DefKind::Fundef { funcls, .. } = &mut def.kind else {
                    panic!()
                };
                let PExpKind::PatExp { pat, .. } = &mut funcls[0].arm.kind else {
                    panic!()
                };
                let mut nested = Node::dummy(PatKind::Lit(Node::dummy(LitKind::Unit)));
                for _ in 0..10_000 {
                    nested = Node::dummy(PatKind::Tuple(vec![nested]));
                }
                *pat = if bad {
                    Node::dummy(PatKind::Tuple(vec![nested, Node::dummy(PatKind::Wild)]))
                } else {
                    nested
                };
                let mut program = Program::new();
                let result = program.declare(&def, it, 0);
                assert_eq!(result.is_err(), bad);
                assert_eq!(program.value("deep", 0).is_none(), bad);
                sail_ast::storage::drop_definition(def);
                drop(program);
            }
        })
        .unwrap()
        .join()
        .unwrap();
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

// Vektor-Sektion (Lane-06): Normalisierung der internen `vector_*#`-Primitive
// fuer den ARM-Gate-Blocker `to_bytes_le` (vector.sail). Bestehende Tests
// oberhalb bleiben unveraendert.
// ---------------------------------------------------------------------------

#[test]
fn vector_prims_normalize_only_the_four_hash_primitives() {
    use sail_ty::vector_prims::{
        VectorPrim, allows_bitfield_fallback, classify, expected_arity, is_vector_prim,
        normalize, normalized_name, raw_name,
    };

    // Exakte Klassifikation der vier internen Namen.
    assert_eq!(classify("vector_access#"), Some(VectorPrim::Access));
    assert_eq!(classify("vector_subrange#"), Some(VectorPrim::Subrange));
    assert_eq!(classify("vector_update#"), Some(VectorPrim::Update));
    assert_eq!(
        classify("vector_update_subrange#"),
        Some(VectorPrim::UpdateSubrange)
    );

    // Kein generelles `#`-Strippen: fremde Namen werden abgelehnt.
    for other in [
        "",
        "#",
        "vector_access",
        "vector_subrange",
        "vector_update",
        "vector_update_subrange",
        "slice",
        "foo#",
        "vector_access##",
        "Vector_access#",
        "vector_update_subrange #",
    ] {
        assert_eq!(classify(other), None, "{other:?}");
        assert!(!is_vector_prim(other), "{other:?}");
        assert_eq!(normalize(other, 2), None, "{other:?}");
        assert_eq!(normalize(other, 3), None, "{other:?}");
        assert_eq!(normalize(other, 4), None, "{other:?}");
    }

    // Roundtrip raw -> normalisiert je Primitiv.
    for prim in [
        VectorPrim::Access,
        VectorPrim::Subrange,
        VectorPrim::Update,
        VectorPrim::UpdateSubrange,
    ] {
        assert!(is_vector_prim(raw_name(prim)));
        assert_eq!(classify(raw_name(prim)), Some(prim));
        assert_eq!(
            normalize(raw_name(prim), expected_arity(prim)),
            Some(normalized_name(prim))
        );
        assert!(!normalized_name(prim).ends_with('#'));
    }

    // Erwartete Abbildung und Stelligkeit (Parser-Lowering).
    assert_eq!(normalized_name(VectorPrim::Access), "vector_access");
    assert_eq!(normalized_name(VectorPrim::Subrange), "vector_subrange");
    assert_eq!(normalized_name(VectorPrim::Update), "vector_update");
    assert_eq!(
        normalized_name(VectorPrim::UpdateSubrange),
        "vector_update_subrange"
    );
    assert_eq!(expected_arity(VectorPrim::Access), 2);
    assert_eq!(expected_arity(VectorPrim::Subrange), 3);
    assert_eq!(expected_arity(VectorPrim::Update), 3);
    assert_eq!(expected_arity(VectorPrim::UpdateSubrange), 4);

    // Falsche Stelligkeit wird nicht normalisiert.
    assert_eq!(normalize("vector_access#", 3), None);
    assert_eq!(normalize("vector_subrange#", 2), None);
    assert_eq!(normalize("vector_update#", 4), None);
    assert_eq!(normalize("vector_update_subrange#", 3), None);

    // Bitfield-Fallback nur dort, wo OCaml ihn vorsieht (type_check.ml:
    // `vector_access#`-Fallback und `infer_vector_update`-Verzweigung).
    assert!(allows_bitfield_fallback(VectorPrim::Access));
    assert!(allows_bitfield_fallback(VectorPrim::Update));
    assert!(!allows_bitfield_fallback(VectorPrim::Subrange));
    assert!(!allows_bitfield_fallback(VectorPrim::UpdateSubrange));
}

#[test]
fn vector_prims_cover_the_internal_calls_of_to_bytes_le() {
    use sail_ty::vector_prims::normalize;

    // `to_bytes_le` (vector.sail, `_DEFAULT_DEC`):
    //   res[i] = b[(8 * i + 7) .. (8 * i)]
    // Die rechte Seite ist ein interner Subrange-Lesezugriff mit genau drei
    // Argumenten (Vektor, obere und untere Grenze) und normalisiert zum
    // `vector_subrange`-Overload.
    assert_eq!(normalize("vector_subrange#", 3), Some("vector_subrange"));
    // Die linke Seite `res[i]` ist ein L-Wert (`LExpKind::Vector`), kein
    // App-Aufruf: Sie laeuft nicht durch `normalize`, sondern faellt unter die
    // `infer_lexp`-Boundsregeln (Wurzel mutable/Register, Index voll geprueft,
    // `0 <= index < length` per Solver bewiesen). Hier wird festgehalten, dass
    // der korrespondierende funktionale Update-Aufruf die Form
    // `vector_update#` mit drei Argumenten hat.
    assert_eq!(normalize("vector_update#", 3), Some("vector_update"));
    // Vollstaendigkeit der internen Familie fuer Vektor-/Slice-Schreibfaelle
    // (`from_bytes_le` nutzt die Subrange-Schreibform).
    assert_eq!(normalize("vector_access#", 2), Some("vector_access"));
    assert_eq!(
        normalize("vector_update_subrange#", 4),
        Some("vector_update_subrange")
    );
}

fn vector_prims_resolve_app_serves_the_app_arm_call_site() {
    use sail_ty::vector_prims::{resolve_app, AppResolution, VectorPrim};

    // Erfolgsfaelle: exakter interner Name + passende Stelligkeit liefern den
    // Overload-Namen und das Bitfield-Fallback-Bit aus type_check.ml.
    assert_eq!(
        resolve_app("vector_access#", 2),
        Some(AppResolution {
            prim: VectorPrim::Access,
            overload: "vector_access",
            bitfield_fallback: true,
        })
    );
    assert_eq!(
        resolve_app("vector_subrange#", 3),
        Some(AppResolution {
            prim: VectorPrim::Subrange,
            overload: "vector_subrange",
            bitfield_fallback: false,
        })
    );
    assert_eq!(
        resolve_app("vector_update#", 3),
        Some(AppResolution {
            prim: VectorPrim::Update,
            overload: "vector_update",
            bitfield_fallback: true,
        })
    );
    assert_eq!(
        resolve_app("vector_update_subrange#", 4),
        Some(AppResolution {
            prim: VectorPrim::UpdateSubrange,
            overload: "vector_update_subrange",
            bitfield_fallback: false,
        })
    );

    // Miss-Faelle: der App-Arm faehrt mit der bisherigen Kandidatensuche
    // fort (kein stilles Umschreiben).
    for (name, argc) in [
        ("vector_access", 2),
        ("slice", 3),
        ("foo#", 2),
        ("vector_access#", 3),
        ("vector_subrange#", 2),
        ("vector_update#", 4),
        ("vector_update_subrange#", 3),
        ("", 0),
    ] {
        assert_eq!(resolve_app(name, argc), None, "{name:?}/{argc}");
    }
}

// ---------------------------------------------------------------------------
// Lane 20 (Audit O-6): Existential-/Range-/arithmetische Inferenz-Reste.
// ---------------------------------------------------------------------------
//
// Diese Pins dokumentieren, was die kanonische Schicht bewusst sound-offen
// laesst (Regel `Z-UnknownSound`): abhaengige Aufrufparameter ohne exakte
// Bindung scheitern mit `cannot infer dependent call parameter`, statt zu
// raten. Die lokale Schicht (`sail_ty::subsume::match_scalar_arg`)
// loest dafuer bereits exakte lineare Faelle (`'n + 1 = 5` -> `'n := 4`);
// die kanonische Verdrahtung bleibt Folgearbeit (Bericht Lane-20).

/// Quantifizierte Skalare ohne Bindung aus Argument oder Resultat bleiben
/// ein sound-offener Fehler — fuer `K_int`- wie `K_bool`-Binder.
#[test]
fn audit_o6_unbound_dependent_parameters_stay_sound_errors() {
    let defs = parse(&[
        "val pos : forall 'n, 'n > 0. int -> int",
        "function g() -> int = pos(5)",
        "val gated : forall ('p : Bool), 'p. int -> int",
        "function h() -> int = gated(3)",
    ]);
    let program = declare(&defs);
    for position in [1, 3] {
        let error = program
            .check_definition(&defs[position].0, defs[position].1.clone(), position)
            .unwrap_err();
        let message = error.to_string();
        assert!(
            matches!(error.kind, TypeErrorKind::Unsupported(_))
                && message.contains("cannot infer dependent call parameter"),
            "position {position}: {error:?}"
        );
    }
}

/// Arithmetische und Potenz-Formale raten keine Bindung: `int('n + 1)`
/// gegen `5` und `bitvector(2 ^ 'n)` gegen `0xFF` bleiben sound-offen,
/// obwohl die lineare Gleichung lokal exakt loesbar waere.
#[test]
fn audit_o6_arithmetic_and_power_formals_do_not_guess() {
    let defs = parse(&[
        "val succ_arg : forall 'n. int('n + 1) -> int",
        "function g() -> int = succ_arg(5)",
        "val pow_arg : forall 'n. bitvector(2 ^ 'n) -> int",
        "function h() -> int = pow_arg(0xFF)",
    ]);
    let program = declare(&defs);
    for position in [1, 3] {
        let error = program
            .check_definition(&defs[position].0, defs[position].1.clone(), position)
            .unwrap_err();
        let message = error.to_string();
        assert!(
            matches!(error.kind, TypeErrorKind::Unsupported(_))
                && message.contains("cannot infer dependent call parameter"),
            "position {position}: {error:?}"
        );
    }
}

/// Aufgeloeste Synonyme vergleichen per Kopf: ein schlichter `int` gegen
/// `Bigger('n)` scheitert als praeziser Mismatch, nicht als erfundene
/// Instantiierung.
#[test]
fn audit_o6_synonym_head_mismatch_is_a_mismatch_not_a_guess() {
    let defs = parse(&[
        "type Bigger('size : Int) = bitvector('size + 1)",
        "val f : forall 'n. Bigger('n) -> int",
        "function g(x : int) -> int = f(x)",
    ]);
    let program = declare(&defs);
    let error = program
        .check_definition(&defs[2].0, defs[2].1.clone(), 2)
        .unwrap_err();
    assert!(
        matches!(
            error.kind,
            TypeErrorKind::Mismatch { .. }
        ),
        "{error:?}"
    );
}

#[test]
fn outcome_outer_binders_scope_over_inner_val_schemes() {
    // B1 (evidence/2026-10-13/lane-31/REPORT.md + fixture-b1-outcome.sail):
    // an inner `val` sees the outer `with`-binders and the header
    // quantifiers, mirroring OCaml `check_outcome` (`type_check.ml:5379ff`).
    // Prelude-free mirrors of the fixture's outcome blocks (`bits` is a
    // prelude abbreviation for `bitvector`, which is checked here directly).
    let defs = parse(&[
        "outcome o : ('a, int) -> unit with 'a : Type = { val to_bits : 'a -> int }",
        "outcome o2 : forall 'n, 'n > 0. ('a, bitvector('n)) -> unit with 'a : Type = { val to_bits : 'a -> bitvector(8) }",
        "outcome o3 : int -> int = { impl emulator(x) = x }",
        "outcome o4 : int -> int with 'a : Type = { val shadow : forall 'a. 'a -> 'a }",
    ]);
    let mut program = Program::new();
    for (position, (def, it)) in defs.iter().enumerate() {
        program.declare(def, it.clone(), position).unwrap();
    }
    // Unbound inner variable: OCaml rejects, Rust must reject too.
    let defs = parse(&[
        "outcome bad : ('a, int) -> unit with 'a : Type = { val to_bits : 'b -> int }",
    ]);
    let mut program = Program::new();
    assert!(matches!(
        program
            .declare(&defs[0].0, defs[0].1.clone(), 0)
            .unwrap_err()
            .kind,
        TypeErrorKind::UnboundVar(_)
    ));
    // Undeclared type constructor inside an outcome member.
    let defs = parse(&[
        "outcome bad : ('a, int) -> unit with 'a : Type = { val to_bits : 'a -> Missing }",
    ]);
    let mut program = Program::new();
    assert!(matches!(
        program
            .declare(&defs[0].0, defs[0].1.clone(), 0)
            .unwrap_err()
            .kind,
        TypeErrorKind::UnboundVar(_)
    ));
    // Kind mismatch: outer `'a : Type` used where `Int` is required.
    let defs = parse(&[
        "outcome bad3 : int -> int with 'a : Type = { val to_bits : bitvector('a) -> int }",
    ]);
    let mut program = Program::new();
    assert!(program
        .declare(&defs[0].0, defs[0].1.clone(), 0)
        .is_err());
    // Forbidden members mirror OCaml `valid_outcome_def`: only val/impl.
    for source in [
        "outcome bad2 : int -> int = { function f(x) = x }",
        "outcome bad2b : int -> int = { type Local = int }",
    ] {
        let defs = parse(&[source]);
        let mut program = Program::new();
        assert!(
            program.declare(&defs[0].0, defs[0].1.clone(), 0).is_err(),
            "{source}"
        );
    }
}
