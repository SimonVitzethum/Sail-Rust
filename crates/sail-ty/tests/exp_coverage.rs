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
use sail_ast::{
    Interner as _, Lit, LitKind, Name, Node, NodeId, PExp, PExpKind, Pat, PatKind, StructName,
    TypPatKind,
};
use sail_ty::pat::MiniPat;
use sail_ty::{
    conv::{miniexp_from_ast, miniexp_to_ast, minilexp_from_ast, minilexp_to_ast},
    BidirectionalChecker, CheckLimits, MiniExp, MiniFExp, MiniLexp, MiniLoop, MiniOrder, MiniPExp,
    MiniTyp, NConstraint, Nexp, Span, StubSolver, TypeEnv,
};

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
        MiniFExp {
            field: "f".to_owned(),
            value: int(1),
        },
        MiniFExp {
            field: "g".to_owned(),
            value: var("y"),
        },
    ]
}

#[test]
fn struct_family_roundtrip() {
    let named = MiniExp::Struct {
        name: Some("Point".to_owned()),
        fields: sample_fields(),
    };
    assert_eq!(roundtrip(&named), named);
    let anon = MiniExp::Struct {
        name: None,
        fields: sample_fields(),
    };
    assert_eq!(roundtrip(&anon), anon);
    let upd = MiniExp::StructUpdate {
        record: Box::new(var("r")),
        fields: sample_fields(),
    };
    assert_eq!(roundtrip(&upd), upd);
    let field = MiniExp::Field {
        record: Box::new(var("r")),
        field: "f".to_owned(),
    };
    assert_eq!(roundtrip(&field), field);
}

fn sample_arms() -> Vec<MiniPExp> {
    vec![
        MiniPExp {
            pat: MiniPat::Var("x".to_owned()),
            guard: None,
            body: var("x"),
        },
        MiniPExp {
            pat: MiniPat::IntLit(0),
            guard: Some(MiniExp::BoolLit(true)),
            body: int(0),
        },
    ]
}

#[test]
fn match_roundtrip() {
    let e = MiniExp::Match {
        scrutinee: Box::new(var("s")),
        arms: sample_arms(),
    };
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn try_roundtrip() {
    let e = MiniExp::Try {
        exp: Box::new(var("risky")),
        arms: sample_arms(),
    };
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn assign_roundtrip() {
    let e = MiniExp::Assign {
        lexp: Box::new(MiniLexp::Id("x".to_owned())),
        value: Box::new(int(5)),
    };
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
        MiniLexp::Typed {
            typ: MiniTyp::Int,
            id: "y".to_owned(),
        },
        MiniLexp::Tuple(vec![
            MiniLexp::Id("a".to_owned()),
            MiniLexp::Id("b".to_owned()),
        ]),
        MiniLexp::VectorConcat(vec![
            MiniLexp::Id("u".to_owned()),
            MiniLexp::Id("v".to_owned()),
        ]),
        MiniLexp::Vector {
            lexp: Box::new(MiniLexp::Id("vec".to_owned())),
            index: Box::new(int(2)),
        },
        MiniLexp::VectorRange {
            lexp: Box::new(MiniLexp::Id("vec".to_owned())),
            lo: Box::new(int(0)),
            hi: Box::new(int(7)),
        },
        MiniLexp::Field {
            lexp: Box::new(MiniLexp::Id("r".to_owned())),
            field: "f".to_owned(),
        },
    ];
    for l in &shapes {
        let ast = minilexp_to_ast(l, &mut it).expect("lexp lowering must succeed");
        assert_eq!(
            minilexp_from_ast(&ast, &it).expect("lexp lifting must succeed"),
            *l
        );
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
    let e = MiniExp::Assert {
        cond: Box::new(var("ok")),
        msg: Box::new(var("why")),
    };
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn var_decl_roundtrip() {
    let e = MiniExp::VarDecl {
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
        MiniExp::Match {
            scrutinee: Box::new(int(0)),
            arms: sample_arms(),
        },
        MiniExp::Undef,
        MiniExp::Sizeof(Nexp::Const(1)),
        MiniExp::Config(Vec::new()),
    ] {
        assert!(
            checker.infer(&e, &env, Span::DUMMY).is_err(),
            "must reject {e:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Lane 09 canonical pattern coverage (`sail_ty::pat` over `sail_ast::Pat`).
// ---------------------------------------------------------------------------

use sail_ty::pat::{
    arm_overlap, match_arms, match_plan, missing_fields, plan, struct_name_matches, AppForm,
    LitPatRule, MatchArm, PatOverlap, PatPlan, PatRule, StructNamePlan,
};

fn pat_name(it: &mut SimpleInterner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn id_pat(it: &mut SimpleInterner, text: &str) -> Pat {
    Node::dummy(PatKind::Id(pat_name(it, text)))
}

fn num_pat(it: &mut SimpleInterner, value: i128) -> Pat {
    Node::dummy(PatKind::Lit(Node::dummy(LitKind::Num(
        it.intern(&value.to_string()),
    ))))
}

fn wild_pat() -> Pat {
    Node::dummy(PatKind::Wild)
}

fn arm(pat: Pat, body: sail_ast::Exp) -> PExp {
    PExp {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        kind: PExpKind::PatExp { pat, body },
    }
}

fn unit_exp() -> sail_ast::Exp {
    Node::dummy(sail_ast::ExpKind::Lit(Node::dummy(LitKind::Unit)))
}

#[test]
fn pat_plan_covers_every_shape_with_rule_codes_and_gates() {
    let mut it = SimpleInterner::new();
    // Shapes with no environment query: no gate.
    let wild = wild_pat();
    let planned = plan(&wild, &it).unwrap();
    assert_eq!(planned.rule(), PatRule::Wild);
    assert_eq!(planned.rule().as_str(), "P-Wild");
    assert!(planned.gate().is_none());

    let tuple = Node::dummy(PatKind::Tuple(vec![wild_pat(), wild_pat()]));
    let planned = plan(&tuple, &it).unwrap();
    assert_eq!(planned.rule().as_str(), "P-Tuple");
    if let PatPlan::Tuple { items } = planned {
        assert_eq!(items.len(), 2);
    } else {
        panic!("tuple plan");
    }

    let inner = id_pat(&mut it, "x");
    let annot = Node::dummy(PatKind::Typ {
        typ: Node::dummy(sail_ast::TypKind::Id(pat_name(&mut it, "int"))),
        pat: Box::new(inner),
    });
    assert_eq!(plan(&annot, &it).unwrap().rule().as_str(), "P-Annot");

    let as_pat = Node::dummy(PatKind::As {
        pat: Box::new(wild_pat()),
        id: pat_name(&mut it, "whole"),
    });
    let planned = plan(&as_pat, &it).unwrap();
    assert_eq!(planned.rule().as_str(), "P-As");
    assert!(planned.gate().is_none());

    let or_pat = Node::dummy(PatKind::Or(Box::new(wild_pat()), Box::new(wild_pat())));
    assert_eq!(plan(&or_pat, &it).unwrap().rule().as_str(), "P-Or");
    let not_pat = Node::dummy(PatKind::Not(Box::new(wild_pat())));
    assert_eq!(plan(&not_pat, &it).unwrap().rule().as_str(), "P-Not");

    let cons = Node::dummy(PatKind::Cons(Box::new(wild_pat()), Box::new(wild_pat())));
    assert_eq!(plan(&cons, &it).unwrap().rule().as_str(), "P-Cons");
    let list = Node::dummy(PatKind::List(vec![wild_pat()]));
    let planned = plan(&list, &it).unwrap();
    assert_eq!(planned.rule().as_str(), "P-List");
    if let PatPlan::List { items } = planned {
        assert_eq!(items.len(), 1);
    } else {
        panic!("list plan");
    }
    let strapp = Node::dummy(PatKind::StringAppend(vec![wild_pat(), wild_pat()]));
    assert_eq!(
        plan(&strapp, &it).unwrap().rule().as_str(),
        "P-StringAppend"
    );

    // Shapes with an owed environment/solver query: explicit gate.
    let id = id_pat(&mut it, "x");
    let planned = plan(&id, &it).unwrap();
    assert_eq!(planned.rule().as_str(), "P-Id");
    let gate = planned.gate().expect("P-Id owes disambiguation");
    assert_eq!(gate.rule, "P-Id");
    assert!(gate.query.contains('x'));

    let single = Node::dummy(PatKind::App {
        func: pat_name(&mut it, "Wrap"),
        args: vec![wild_pat()],
    });
    let planned = plan(&single, &it).unwrap();
    assert_eq!(planned.rule().as_str(), "P-App");
    if let PatPlan::App { func, form, gate } = planned {
        assert_eq!(func, "Wrap");
        assert!(matches!(form, AppForm::Single(_)));
        assert!(gate.query.contains("Wrap"));
    } else {
        panic!("app plan");
    }
    // Multi-argument constructors normalize to one tuple argument.
    let multi = Node::dummy(PatKind::App {
        func: pat_name(&mut it, "Pair"),
        args: vec![wild_pat(), wild_pat()],
    });
    if let PatPlan::App { form, .. } = plan(&multi, &it).unwrap() {
        assert!(matches!(form, AppForm::Multi(items) if items.len() == 2));
    } else {
        panic!("multi-arg normalization");
    }

    let vector = Node::dummy(PatKind::Vector(vec![wild_pat(), wild_pat()]));
    let planned = plan(&vector, &it).unwrap();
    assert_eq!(planned.rule().as_str(), "P-Vector");
    assert!(planned.gate().is_some());
    let concat = Node::dummy(PatKind::VectorConcat(vec![wild_pat()]));
    assert_eq!(
        plan(&concat, &it).unwrap().rule().as_str(),
        "P-VectorConcat"
    );

    let subrange = Node::dummy(PatKind::VectorSubrange {
        id: pat_name(&mut it, "v"),
        lo: "7".to_owned(),
        hi: "0".to_owned(),
    });
    if let PatPlan::VectorSubrange { id, lo, hi, gate } = plan(&subrange, &it).unwrap() {
        assert_eq!((id.as_str(), lo.as_str(), hi.as_str()), ("v", "7", "0"));
        assert_eq!(gate.rule, "P-VectorSubrange");
    } else {
        panic!("subrange plan");
    }

    // `P_var`: direct binders resolve here, applications gate.
    let var_wild = Node::dummy(PatKind::Var {
        pat: Box::new(wild_pat()),
        typ_pat: Node::dummy(TypPatKind::Wild),
    });
    if let PatPlan::Var { binders, gate, .. } = plan(&var_wild, &it).unwrap() {
        assert!(binders.is_empty() && gate.is_none());
    } else {
        panic!("var plan");
    }
    let var_binder = Node::dummy(PatKind::Var {
        pat: Box::new(wild_pat()),
        typ_pat: Node::dummy(TypPatKind::Var(it.intern("'n"))),
    });
    if let PatPlan::Var { binders, gate, .. } = plan(&var_binder, &it).unwrap() {
        assert_eq!(binders, vec!["'n".to_owned()]);
        assert!(gate.is_none());
    } else {
        panic!("var binder plan");
    }

    // Struct patterns decompose fields, flags and gates.
    let strukt = Node::dummy(PatKind::Struct {
        name: StructName::Id(pat_name(&mut it, "Point")),
        fields: vec![(pat_name(&mut it, "x"), wild_pat())],
        wildcard: true,
    });
    if let PatPlan::Struct {
        name,
        fields,
        wildcard,
        gate,
    } = plan(&strukt, &it).unwrap()
    {
        assert_eq!(name, StructNamePlan::Named("Point".to_owned()));
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "x");
        assert!(wildcard);
        assert_eq!(gate.rule, "P-Struct");
    } else {
        panic!("struct plan");
    }
    let anon = Node::dummy(PatKind::Struct {
        name: StructName::Anon,
        fields: vec![],
        wildcard: false,
    });
    if let PatPlan::Struct { name, .. } = plan(&anon, &it).unwrap() {
        assert_eq!(name, StructNamePlan::Anonymous);
    } else {
        panic!("anon struct plan");
    }
}

#[test]
fn pat_plan_rejects_what_it_cannot_classify() {
    let it = SimpleInterner::new();
    // Empty vector/concat patterns cannot infer element types or widths.
    for (kind, rule) in [
        (PatKind::Vector(Vec::new()), "P-Vector"),
        (PatKind::VectorConcat(Vec::new()), "P-VectorConcat"),
    ] {
        let pat = Node::dummy(kind);
        let deferred = plan(&pat, &it).expect_err("must defer");
        assert_eq!(deferred.rule, rule);
        let error = deferred.into_error();
        assert!(
            matches!(error.kind, sail_ty::error::TypeErrorKind::Unsupported(_)),
            "{error}"
        );
        assert_eq!(error.span, pat.span);
    }
    // Unresolvable symbols are loud, never silent.
    let dangling = Node::dummy(PatKind::Id(Name::dummy(sail_ast::Symbol::DUMMY)));
    assert_eq!(plan(&dangling, &it).expect_err("dangling").rule, "P-Id");
}

fn check_lit(it: &SimpleInterner, kind: LitKind, expected: LitPatRule) {
    let lit: Lit = Node::dummy(kind);
    let pat = Node::dummy(PatKind::Lit(lit));
    if let PatPlan::Lit { rule } = plan(&pat, it).expect("literal plans") {
        assert_eq!(rule, expected);
    } else {
        panic!("lit plan");
    }
}

#[test]
fn lit_pattern_rules_cover_every_literal_kind() {
    let mut it = SimpleInterner::new();
    let num_sym = it.intern("007");
    let hex_sym = it.intern("0F_F");
    let bin_sym = it.intern("101");
    let str_sym = it.intern("s");
    let real_sym = it.intern("1.5");
    check_lit(
        &it,
        LitKind::Num(num_sym),
        LitPatRule::Numeric {
            value: "007".to_owned(),
        },
    );
    check_lit(&it, LitKind::True, LitPatRule::Bool { value: true });
    check_lit(&it, LitKind::False, LitPatRule::Bool { value: false });
    check_lit(
        &it,
        LitKind::Hex(hex_sym),
        LitPatRule::Bitvector { width: 12 },
    );
    check_lit(
        &it,
        LitKind::Bin(bin_sym),
        LitPatRule::Bitvector { width: 3 },
    );
    check_lit(&it, LitKind::String(str_sym), LitPatRule::Str);
    check_lit(&it, LitKind::Unit, LitPatRule::Unit);
    // Reals need `==`-guard synthesis: explicit, never silent.
    if let PatPlan::Lit { rule } = plan(
        &Node::dummy(PatKind::Lit(Node::dummy(LitKind::Real(real_sym)))),
        &it,
    )
    .unwrap()
    {
        assert!(matches!(rule, LitPatRule::GuardedEquality { .. }));
    } else {
        panic!("real plan");
    }
}

#[test]
fn struct_name_and_missing_field_rules() {
    assert!(struct_name_matches(&StructNamePlan::Anonymous, "Anything"));
    assert!(struct_name_matches(
        &StructNamePlan::Named("Point".to_owned()),
        "Point"
    ));
    assert!(!struct_name_matches(
        &StructNamePlan::Named("Point".to_owned()),
        "Other"
    ));
    // Missing fields in declaration order; `..` fills exactly these.
    let declared = ["x", "y", "z"];
    assert!(missing_fields(&declared, &["x", "y", "z"].map(str::to_owned)).is_empty());
    assert_eq!(
        missing_fields(&declared, &["z".to_owned(), "x".to_owned()]),
        vec!["y"]
    );
    let none: Vec<String> = Vec::new();
    assert_eq!(missing_fields(&declared, &none), vec!["x", "y", "z"]);
}

#[test]
fn match_plan_decomposes_arms_guards_and_redundancy() {
    let mut it = SimpleInterner::new();
    // Duplicate numeric arms are provably redundant.
    let arms = vec![
        arm(num_pat(&mut it, 7), unit_exp()),
        arm(num_pat(&mut it, 7), unit_exp()),
        arm(num_pat(&mut it, 8), unit_exp()),
    ];
    let planned = match_plan(&arms, &it).unwrap();
    assert_eq!(planned.arms.len(), 3);
    assert!(planned.arms.iter().all(|a| !a.has_guard));
    assert_eq!(planned.redundant, vec![(0, 1)]);
    // Guarded arms report their guard.
    let guarded = PExp {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        kind: PExpKind::PatWhen {
            pat: wild_pat(),
            guard: unit_exp(),
            body: unit_exp(),
        },
    };
    let guarded_arms = [guarded];
    let planned = match_plan(&guarded_arms, &it).unwrap();
    assert!(planned.arms[0].has_guard);
    assert!(planned.redundant.is_empty());
    // `match_arms` keeps pattern/guard/body triples aligned.
    let triples = match_arms(&arms);
    assert!(matches!(triples[0], MatchArm { guard: None, .. }));
    // Empty matches are hard errors, never vacuous success.
    let deferred = match_plan(&[], &it).expect_err("empty match");
    assert_eq!(deferred.rule, "E-Match");
}

fn overlap_pats(a: &Pat, b: &Pat, it: &SimpleInterner) -> PatOverlap {
    arm_overlap(a, b, it)
}

fn app_pat(it: &mut SimpleInterner, name: &str, value: i128) -> Pat {
    Node::dummy(PatKind::App {
        func: pat_name(it, name),
        args: vec![num_pat(it, value)],
    })
}

#[test]
fn pat_overlap_is_sound_across_shapes() {
    let mut it = SimpleInterner::new();
    // Build phase: every symbol comes from the one test interner.
    let seven_a = num_pat(&mut it, 7);
    let seven_b = num_pat(&mut it, 7);
    let eight = num_pat(&mut it, 8);
    let one = num_pat(&mut it, 1);
    let two = num_pat(&mut it, 2);
    let t = Node::dummy(PatKind::Lit(Node::dummy(LitKind::True)));
    let f = Node::dummy(PatKind::Lit(Node::dummy(LitKind::False)));
    let pair = Node::dummy(PatKind::Tuple(vec![wild_pat(), wild_pat()]));
    let triple = Node::dummy(PatKind::Tuple(vec![wild_pat(), wild_pat(), wild_pat()]));
    let var_x = id_pat(&mut it, "x");
    let wild = wild_pat();
    let as1 = Node::dummy(PatKind::As {
        pat: Box::new(num_pat(&mut it, 1)),
        id: pat_name(&mut it, "whole"),
    });
    let as1_again = Node::dummy(PatKind::As {
        pat: Box::new(num_pat(&mut it, 1)),
        id: pat_name(&mut it, "whole"),
    });
    let as2 = Node::dummy(PatKind::As {
        pat: Box::new(num_pat(&mut it, 2)),
        id: pat_name(&mut it, "whole"),
    });
    let or_same = Node::dummy(PatKind::Or(Box::new(one.clone()), Box::new(two.clone())));
    let or_same_again = Node::dummy(PatKind::Or(Box::new(one.clone()), Box::new(two.clone())));
    let or_swapped = Node::dummy(PatKind::Or(Box::new(two.clone()), Box::new(one.clone())));
    let or_one = Node::dummy(PatKind::Or(Box::new(one.clone()), Box::new(one.clone())));
    let or_mixed = Node::dummy(PatKind::Or(Box::new(one.clone()), Box::new(two.clone())));
    let wrap_a = app_pat(&mut it, "Wrap", 1);
    let wrap_b = app_pat(&mut it, "Wrap", 1);
    let wrap_two = app_pat(&mut it, "Wrap", 2);
    let other = app_pat(&mut it, "Other", 1);
    let strukt = Node::dummy(PatKind::Struct {
        name: StructName::Anon,
        fields: vec![],
        wildcard: true,
    });
    // Literals: same value equal, different values disjoint.
    assert_eq!(overlap_pats(&seven_a, &seven_b, &it), PatOverlap::Equal);
    assert_eq!(overlap_pats(&seven_a, &eight, &it), PatOverlap::Disjoint);
    assert_eq!(overlap_pats(&t, &t, &it), PatOverlap::Equal);
    assert_eq!(overlap_pats(&t, &f, &it), PatOverlap::Disjoint);
    // Arity is structural: different lengths are disjoint, same wildcard
    // shapes share the whole match set.
    assert_eq!(overlap_pats(&pair, &triple, &it), PatOverlap::Disjoint);
    assert_eq!(overlap_pats(&pair, &pair, &it), PatOverlap::Equal);
    // Bare identifiers and cross-shape pairs need the environment.
    assert_eq!(overlap_pats(&var_x, &wild, &it), PatOverlap::Unknown);
    assert_eq!(overlap_pats(&one, &wild, &it), PatOverlap::Unknown);
    assert_eq!(overlap_pats(&one, &pair, &it), PatOverlap::Unknown);
    // `as`/annotations inherit the inner verdict only when disjoint.
    assert_eq!(overlap_pats(&as1, &as2, &it), PatOverlap::Disjoint);
    assert_eq!(overlap_pats(&as1, &as1_again, &it), PatOverlap::Equal);
    // `p1 | p2` agrees only when both sides agree.
    assert_eq!(
        overlap_pats(&or_same, &or_same_again, &it),
        PatOverlap::Equal
    );
    // Swapped unions share the set too — never misread as disjoint.
    assert_eq!(overlap_pats(&or_same, &or_swapped, &it), PatOverlap::Equal);
    assert_eq!(overlap_pats(&or_one, &two, &it), PatOverlap::Disjoint);
    assert_eq!(overlap_pats(&or_mixed, &wild, &it), PatOverlap::Unknown);
    // Constructor shapes without environment knowledge stay unknown unless
    // the whole application (head plus arguments) is syntactically equal.
    assert_eq!(overlap_pats(&wrap_a, &wrap_b, &it), PatOverlap::Equal);
    assert_eq!(overlap_pats(&wrap_a, &other, &it), PatOverlap::Unknown);
    // Disjoint arguments under one head stay Unknown: a non-injective
    // mapping could merge them (injectivity needs the environment).
    assert_eq!(overlap_pats(&wrap_a, &wrap_two, &it), PatOverlap::Unknown);
    // Struct patterns always need record knowledge.
    assert_eq!(overlap_pats(&strukt, &strukt, &it), PatOverlap::Unknown);
}
