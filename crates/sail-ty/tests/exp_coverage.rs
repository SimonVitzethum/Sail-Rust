#![forbid(unsafe_code)]
//! Lane 105 (`exp-coverage`): roundtrip coverage for every non-internal
//! Sail `E_*` form now representable as [`MiniExp`].
//!
//! Written as code only (E3: do NOT execute in this lane — the lane runner
//! executes). Each test lowers a `MiniExp` onto `sail_ast::Exp` and lifts it
//! back; the roundtrip must be the identity. No checker rules are asserted
//! here except that extended forms are *rejected* by `infer`
//! (`extended_forms_rejected_by_infer`) — checking itself stays Lane 02.

use sail_ast::SimpleInterner;
use sail_ty::{
    BidirectionalChecker, CheckLimits, MiniExp, MiniFExp, MiniLexp, MiniLoop, MiniOrder, MiniPExp,
    MiniTyp, NConstraint, Nexp, Span, StubSolver, TypeEnv,
    conv::{miniexp_from_ast, miniexp_to_ast, minilexp_from_ast, minilexp_to_ast},
};
use sail_ty::pat::MiniPat;

fn roundtrip(e: &MiniExp) -> MiniExp {
    let mut it = SimpleInterner::new();
    let ast = miniexp_to_ast(e, &mut it).expect("lowering must succeed");
    miniexp_from_ast(&ast, &it).expect("lifting must succeed")
}

fn int(i: i128) -> MiniExp {
    MiniExp::IntLit(i)
}

fn var(name: &str) -> MiniExp {
    MiniExp::Var(name.to_owned())
}

#[test]
fn block_roundtrip() {
    let e = MiniExp::Block(vec![int(1), var("x"), int(2)]);
    assert_eq!(roundtrip(&e), e);
    let empty = MiniExp::Block(Vec::new());
    assert_eq!(roundtrip(&empty), empty);
}

#[test]
fn loop_roundtrip() {
    let while_measured = MiniExp::Loop {
        kind: MiniLoop::While,
        measure: Some(Box::new(int(7))),
        cond: Box::new(MiniExp::BoolLit(true)),
        body: Box::new(var("b")),
    };
    assert_eq!(roundtrip(&while_measured), while_measured);
    let until_plain = MiniExp::Loop {
        kind: MiniLoop::Until,
        measure: None,
        cond: Box::new(var("c")),
        body: Box::new(MiniExp::Block(vec![var("x")])),
    };
    assert_eq!(roundtrip(&until_plain), until_plain);
}

#[test]
fn for_roundtrip() {
    for order in [MiniOrder::Inc, MiniOrder::Dec] {
        let e = MiniExp::For {
            var: "i".to_owned(),
            from: Box::new(int(0)),
            to: Box::new(int(10)),
            step: Box::new(int(1)),
            order,
            body: Box::new(var("i")),
        };
        assert_eq!(roundtrip(&e), e);
    }
}

#[test]
fn vector_and_append_roundtrip() {
    let e = MiniExp::Vector(vec![int(1), int(2), int(3)]);
    assert_eq!(roundtrip(&e), e);
    let cat = MiniExp::VectorAppend(Box::new(var("a")), Box::new(var("b")));
    assert_eq!(roundtrip(&cat), cat);
}

#[test]
fn list_and_cons_roundtrip() {
    let e = MiniExp::List(vec![int(1), int(2)]);
    assert_eq!(roundtrip(&e), e);
    let cons = MiniExp::Cons(Box::new(int(0)), Box::new(var("xs")));
    assert_eq!(roundtrip(&cons), cons);
}

fn sample_fields() -> Vec<MiniFExp> {
    vec![
        MiniFExp { field: "f".to_owned(), value: int(1) },
        MiniFExp { field: "g".to_owned(), value: var("y") },
    ]
}

#[test]
fn struct_family_roundtrip() {
    let named = MiniExp::Struct { name: Some("Point".to_owned()), fields: sample_fields() };
    assert_eq!(roundtrip(&named), named);
    let anon = MiniExp::Struct { name: None, fields: sample_fields() };
    assert_eq!(roundtrip(&anon), anon);
    let upd = MiniExp::StructUpdate { record: Box::new(var("r")), fields: sample_fields() };
    assert_eq!(roundtrip(&upd), upd);
    let field = MiniExp::Field { record: Box::new(var("r")), field: "f".to_owned() };
    assert_eq!(roundtrip(&field), field);
}

fn sample_arms() -> Vec<MiniPExp> {
    vec![
        MiniPExp { pat: MiniPat::Var("x".to_owned()), guard: None, body: var("x") },
        MiniPExp {
            pat: MiniPat::IntLit(0),
            guard: Some(MiniExp::BoolLit(true)),
            body: int(0),
        },
    ]
}

#[test]
fn match_roundtrip() {
    let e = MiniExp::Match { scrutinee: Box::new(var("s")), arms: sample_arms() };
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn try_roundtrip() {
    let e = MiniExp::Try { exp: Box::new(var("risky")), arms: sample_arms() };
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn assign_roundtrip() {
    let e = MiniExp::Assign { lexp: Box::new(MiniLexp::Id("x".to_owned())), value: Box::new(int(5)) };
    assert_eq!(roundtrip(&e), e);
    let field = MiniExp::Assign {
        lexp: Box::new(MiniLexp::Field {
            lexp: Box::new(MiniLexp::Id("r".to_owned())),
            field: "f".to_owned(),
        }),
        value: Box::new(int(6)),
    };
    assert_eq!(roundtrip(&field), field);
}

#[test]
fn lexp_shapes_roundtrip() {
    let mut it = SimpleInterner::new();
    let shapes = vec![
        MiniLexp::Id("x".to_owned()),
        MiniLexp::Deref(Box::new(var("p"))),
        MiniLexp::Typed { typ: MiniTyp::Int, id: "y".to_owned() },
        MiniLexp::Tuple(vec![MiniLexp::Id("a".to_owned()), MiniLexp::Id("b".to_owned())]),
        MiniLexp::VectorConcat(vec![MiniLexp::Id("u".to_owned()), MiniLexp::Id("v".to_owned())]),
        MiniLexp::Vector { lexp: Box::new(MiniLexp::Id("vec".to_owned())), index: Box::new(int(2)) },
        MiniLexp::VectorRange {
            lexp: Box::new(MiniLexp::Id("vec".to_owned())),
            lo: Box::new(int(0)),
            hi: Box::new(int(7)),
        },
        MiniLexp::Field { lexp: Box::new(MiniLexp::Id("r".to_owned())), field: "f".to_owned() },
    ];
    for l in &shapes {
        let ast = minilexp_to_ast(l, &mut it).expect("lexp lowering must succeed");
        assert_eq!(minilexp_from_ast(&ast, &it).expect("lexp lifting must succeed"), *l);
    }
}

#[test]
fn sizeof_roundtrip() {
    let e = MiniExp::Sizeof(Nexp::Const(8));
    assert_eq!(roundtrip(&e), e);
    let sym = MiniExp::Sizeof(Nexp::Var("n".to_owned()));
    assert_eq!(roundtrip(&sym), sym);
}

#[test]
fn control_and_effect_roundtrip() {
    for e in [
        MiniExp::Return(Box::new(int(1))),
        MiniExp::Exit(Box::new(int(2))),
        MiniExp::Throw(Box::new(var("exn"))),
        MiniExp::Ref("cell".to_owned()),
        MiniExp::Undef,
        MiniExp::Config(vec!["sail".to_owned(), "arch".to_owned()]),
    ] {
        assert_eq!(roundtrip(&e), e);
    }
}

#[test]
fn assert_roundtrip() {
    let e = MiniExp::Assert { cond: Box::new(var("ok")), msg: Box::new(var("why")) };
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn var_decl_roundtrip() {
    let e = MiniExp::Var {
        lexp: Box::new(MiniLexp::Id("tmp".to_owned())),
        init: Box::new(int(0)),
        body: Box::new(var("tmp")),
    };
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn constraint_exp_roundtrip() {
    let e = MiniExp::Constraint(NConstraint::Ge(
        Box::new(Nexp::Var("n".to_owned())),
        Box::new(Nexp::Const(0)),
    ));
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn pat_shapes_roundtrip() {
    let mut it = SimpleInterner::new();
    let pats = vec![
        MiniPat::Wild,
        MiniPat::Var("x".to_owned()),
        MiniPat::IntLit(-3),
        MiniPat::BoolLit(false),
        MiniPat::Tuple(vec![MiniPat::Wild, MiniPat::Var("y".to_owned())]),
        MiniPat::Annot(Box::new(MiniPat::Var("z".to_owned())), MiniTyp::Int),
    ];
    for p in &pats {
        let ast = sail_ty::conv::minipat_to_ast(p, &mut it).expect("pat lowering must succeed");
        assert_eq!(
            sail_ty::conv::minipat_from_ast(&ast, &it).expect("pat lifting must succeed"),
            *p
        );
    }
}

#[test]
fn extended_forms_rejected_by_infer() {
    let solver = StubSolver::new();
    let checker = BidirectionalChecker::new(&solver, CheckLimits::default());
    let env = TypeEnv::new();
    // Every extended form must fail synthesis (no silent success); the
    // Lane-79 subset is untouched (see typechecker.rs / typechecker_full.rs).
    for e in [
        MiniExp::Block(vec![int(1)]),
        MiniExp::Loop {
            kind: MiniLoop::While,
            measure: None,
            cond: Box::new(MiniExp::BoolLit(true)),
            body: Box::new(int(1)),
        },
        MiniExp::Vector(vec![int(1)]),
        MiniExp::Match { scrutinee: Box::new(int(0)), arms: sample_arms() },
        MiniExp::Undef,
        MiniExp::Sizeof(Nexp::Const(1)),
        MiniExp::Config(Vec::new()),
    ] {
        assert!(checker.infer(&e, &env, Span::DUMMY).is_err(), "must reject {e:?}");
    }
}
