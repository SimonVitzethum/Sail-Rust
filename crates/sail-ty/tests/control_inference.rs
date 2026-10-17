//! Control-flow results guide unannotated bindings without skipping checks.
use sail_ast::*;
use sail_parse::{ParserContext, RealParser};
use sail_ty::{program::Program, TypeResult};
use std::sync::Arc;

fn check(sources: &[&str]) -> TypeResult<usize> {
    let mut context = ParserContext::default();
    let mut program = Program::new();
    let mut count = 0;
    for (position, source) in sources.iter().enumerate() {
        let mut parser = RealParser::with_context(context.clone());
        let def = parser.parse_def(source, Span::DUMMY).unwrap();
        context = parser.context();
        let interner = Arc::new(parser.into_interner());
        program.declare(&def, interner.clone(), position)?;
        count += program.check_definition(&def, interner, position)?;
    }
    Ok(count)
}

#[test]
fn conditionals_and_blocks_infer_binding_types() {
    for body in [
        "let choice = if flag then left else right in choice",
        "let choice = if flag then { left } else { right } in choice",
        "let choice = { (); left } in choice",
    ] {
        check(&[
            "val select : (bool, int, int) -> int",
            &format!("function select(flag, left, right) = {body}"),
        ])
        .unwrap();
    }
    check(&[
        "val select : bool -> bool",
        "function select(flag) = let choice = if flag then true else false in choice",
    ])
    .unwrap();
}

#[test]
fn inference_does_not_skip_prefixes_conditions_or_branch_checks() {
    for body in [
        "let choice = { false; left } in choice",
        "let choice = if 4 then left else right in choice",
        "let choice = if flag then left else false in choice",
        "let choice = if flag then left else (false : int) in choice",
        "let choice = { missing(); left } in choice",
    ] {
        assert!(
            check(&[
                "val select : (bool, int, int) -> int",
                &format!("function select(flag, left, right) = {body}"),
            ])
            .is_err(),
            "{body}"
        );
    }
}

#[test]
fn separate_existential_calls_do_not_share_branch_witnesses() {
    check(&[
        "val choose : unit -> { 'n, 'n >= 0. bitvector('n) }",
        "val select : bool -> unit",
        "function select(flag) = let choice = if flag then choose() else choose() in ()",
    ])
    .unwrap();
}

#[test]
fn deep_control_inference_uses_a_small_host_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut parser = RealParser::new();
            let mut def = parser
                .parse_def(
                    "function select(left : int) -> int = let choice = left in choice",
                    Span::DUMMY,
                )
                .unwrap();
            let mut interner = parser.into_interner();
            let left = interner.intern("left");
            let DefKind::Fundef { funcls, .. } = &mut def.kind else {
                panic!()
            };
            let PExpKind::PatExp { body, .. } = &mut funcls[0].arm.kind else {
                panic!()
            };
            let ExpKind::Let { bind, .. } = &mut body.kind else {
                panic!()
            };
            let mut nested = Node::dummy(ExpKind::Id(Name::dummy(left)));
            for _ in 0..10_000 {
                nested = Node::dummy(ExpKind::If {
                    cond: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(LitKind::True)))),
                    then_branch: Box::new(Node::dummy(ExpKind::Block(vec![nested]))),
                    else_branch: Box::new(Node::dummy(ExpKind::Id(Name::dummy(left)))),
                });
            }
            *bind = Box::new(nested);
            let interner = Arc::new(interner);
            let mut program = Program::new();
            program.declare(&def, interner.clone(), 0).unwrap();
            let count = program.check_definition(&def, interner, 0).unwrap();
            assert!(count >= 40_000, "{count}");
            sail_ast::storage::drop_definition(def);
        })
        .unwrap()
        .join()
        .unwrap();
}
