//! Lane-56 acceptance tests: `remove_e_assign` + `internal_lets`.
//!
//! The two pass files live in `src/passes/` but are not wired into
//! `passes/mod.rs` yet (wiring is the Main-Agent's Insert-Spec job, see the
//! module docs). To still compile and verify them under the gated
//! `cargo test -p sail-rewrite`, they are included here via `#[path]`.

#![forbid(unsafe_code)]

use sail_rewrite::passes::{InternalLets, RemoveEAssign};

use sail_ast::{
    Ast, DecSpec, Def, DefKind, Exp, ExpKind, Funcl, Interner, LExpKind, LitKind, Name, Node,
    NodeId, PExp, PExpKind, PatKind, SimpleInterner, Span, Typ, TypKind, TypSchm, ValSpec,
};

fn nm(it: &mut SimpleInterner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn id(it: &mut SimpleInterner, text: &str) -> Exp {
    Node::dummy(ExpKind::Id(nm(it, text)))
}

fn wild_let(exp: Exp) -> Def {
    Node::dummy(DefKind::Let {
        pat: Node::dummy(PatKind::Wild),
        exp,
    })
}

fn early_return_app(it: &mut SimpleInterner, arg: Exp) -> Exp {
    Node::dummy(ExpKind::App {
        func: nm(it, "early_return"),
        args: vec![arg],
    })
}

fn early_return_of(it: &mut SimpleInterner, arg_text: &str) -> Exp {
    let arg = id(it, arg_text);
    early_return_app(it, arg)
}

fn val_names(ast: &Ast, it: &SimpleInterner) -> Vec<String> {
    ast.defs
        .iter()
        .filter_map(|d| match &d.kind {
            DefKind::Val(vs) => it.resolve(vs.name.symbol).map(str::to_owned),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Insert-Spec pinning
// ---------------------------------------------------------------------------

#[test]
fn pass_names_match_lean_rewrites() {
    assert_eq!(RemoveEAssign::NAME, "remove_e_assign");
    assert_eq!(RemoveEAssign.name(), "remove_e_assign");
    assert_eq!(InternalLets::NAME, "internal_lets");
    assert_eq!(InternalLets.name(), "internal_lets");
}

// ---------------------------------------------------------------------------
// remove_e_assign: loop-spec preamble
// ---------------------------------------------------------------------------

fn scheme_arity(ast: &Ast, index: usize) -> (usize, usize) {
    let DefKind::Val(vs) = &ast.defs[index].kind else {
        panic!("expected valspec at {index}");
    };
    let TypKind::Fn { args, .. } = &vs.typschm.typ.kind else {
        panic!("expected fn scheme at {index}");
    };
    (vs.typschm.quant.len(), args.len())
}

#[test]
fn preamble_inserted_in_lean_order_with_schemes() {
    let mut it = SimpleInterner::new();
    let mut ast = Ast::new(vec![]);
    RemoveEAssign.run_typed(&mut ast, &mut it);
    assert_eq!(
        val_names(&ast, &it),
        ["foreach#", "while#", "until#", "while#t", "until#t"]
    );
    // (quantifier count, argument count) per OCaml `gen_vs` strings.
    assert_eq!(scheme_arity(&ast, 0), (2, 6)); // foreach#
    assert_eq!(scheme_arity(&ast, 1), (2, 3)); // while#
    assert_eq!(scheme_arity(&ast, 2), (2, 3)); // until#
    assert_eq!(scheme_arity(&ast, 3), (2, 4)); // while#t
    assert_eq!(scheme_arity(&ast, 4), (2, 4)); // until#t

    // `foreach#`: (int, int, int, bool, vars_in, vars_out) -> vars_out.
    let DefKind::Val(vs) = &ast.defs[0].kind else {
        panic!("expected valspec");
    };
    let TypKind::Fn { args, ret } = &vs.typschm.typ.kind else {
        panic!("expected fn scheme");
    };
    let text = |t: &Typ| match &t.kind {
        TypKind::Id(n) => it.resolve(n.symbol).unwrap_or("?").to_owned(),
        TypKind::Var(s) => it.resolve(*s).unwrap_or("?").to_owned(),
        _ => "?".to_owned(),
    };
    let arg_texts: Vec<String> = args.iter().map(text).collect();
    assert_eq!(
        arg_texts,
        ["int", "int", "int", "bool", "vars_in", "vars_out"]
    );
    assert_eq!(text(ret), "vars_out");
}

#[test]
fn preamble_skips_existing_and_is_idempotent() {
    let mut it = SimpleInterner::new();
    // Pre-existing `while#` with a foreign scheme must survive untouched.
    let existing: Def = Node::dummy(DefKind::Val(ValSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: nm(&mut it, "while#"),
        typschm: TypSchm {
            quant: vec![],
            typ: Node::dummy(TypKind::Unknown),
        },
        extern_opt: None,
    }));
    let body = wild_let(id(&mut it, "x"));
    let mut ast = Ast::new(vec![existing, body]);
    RemoveEAssign.run_typed(&mut ast, &mut it);
    assert_eq!(
        val_names(&ast, &it),
        ["foreach#", "until#", "while#t", "until#t", "while#"]
    );
    // The user's own `while#` keeps its scheme.
    let DefKind::Val(kept) = &ast.defs[4].kind else {
        panic!("expected kept valspec");
    };
    assert!(matches!(kept.typschm.typ.kind, TypKind::Unknown));
    // Second run changes nothing.
    let snapshot = ast.clone();
    RemoveEAssign.run_typed(&mut ast, &mut it);
    assert_eq!(ast, snapshot);
}

// ---------------------------------------------------------------------------
// remove_e_assign: reference-type stripping
// ---------------------------------------------------------------------------

fn reg_of(it: &mut SimpleInterner, inner: Typ) -> Typ {
    Node::dummy(TypKind::App {
        name: nm(it, "reg"),
        args: vec![Node::dummy(sail_ast::TypArgKind::Typ(inner))],
    })
}

fn int_typ(it: &mut SimpleInterner) -> Typ {
    Node::dummy(TypKind::Id(nm(it, "int")))
}

#[test]
fn reg_stripped_in_type_positions() {
    let mut it = SimpleInterner::new();
    let inner_int = int_typ(&mut it);
    let reg_int = reg_of(&mut it, inner_int);
    let bool_id = nm(&mut it, "bool");
    let bool_typ: Typ = Node::dummy(TypKind::Id(bool_id));
    let bool_arg = Node::dummy(sail_ast::TypArgKind::Typ(bool_typ));
    let reg_name = nm(&mut it, "reg");
    let reg_bool: Typ = Node::dummy(TypKind::App {
        name: reg_name,
        args: vec![bool_arg],
    });
    let reg_reg_bool = reg_of(&mut it, reg_bool);
    let reg_name_r = nm(&mut it, "R");
    let register: Def = Node::dummy(DefKind::Register(DecSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        typ: reg_int,
        name: reg_name_r,
        init: None,
    }));
    let pat_id = nm(&mut it, "v");
    let exp_int = int_typ(&mut it);
    let exp_reg = reg_of(&mut it, exp_int);
    let exp_id = id(&mut it, "v");
    let coerced: Def = Node::dummy(DefKind::Let {
        pat: Node::dummy(PatKind::Typ {
            typ: reg_reg_bool,
            pat: Box::new(Node::dummy(PatKind::Id(pat_id))),
        }),
        exp: Node::dummy(ExpKind::Typ {
            typ: exp_reg,
            exp: Box::new(exp_id),
        }),
    });
    let mut ast = Ast::new(vec![register, coerced]);
    RemoveEAssign.run_typed(&mut ast, &mut it);
    // 5 loop specs + 2 originals.
    assert_eq!(ast.defs.len(), 7);
    let DefKind::Register(spec) = &ast.defs[5].kind else {
        panic!("expected register");
    };
    assert!(matches!(&spec.typ.kind, TypKind::Id(_)));
    let DefKind::Let { pat, exp } = &ast.defs[6].kind else {
        panic!("expected let");
    };
    let PatKind::Typ { typ, .. } = &pat.kind else {
        panic!("expected typ pat");
    };
    assert!(matches!(&typ.kind, TypKind::Id(_)), "reg(reg(bool)) -> bool");
    let ExpKind::Typ { typ, .. } = &exp.kind else {
        panic!("expected typ exp");
    };
    assert!(matches!(&typ.kind, TypKind::Id(_)));
}

#[test]
fn non_reg_applications_survive() {
    let mut it = SimpleInterner::new();
    let bits: Typ = Node::dummy(TypKind::App {
        name: nm(&mut it, "bits"),
        args: vec![Node::dummy(sail_ast::TypArgKind::NExp(Node::dummy(
            sail_ast::NExpKind::Constant("8".to_owned()),
        )))],
    });
    let register: Def = Node::dummy(DefKind::Register(DecSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        typ: bits.clone(),
        name: nm(&mut it, "R"),
        init: None,
    }));
    let mut ast = Ast::new(vec![register]);
    RemoveEAssign.run_typed(&mut ast, &mut it);
    let DefKind::Register(spec) = &ast.defs[5].kind else {
        panic!("expected register");
    };
    assert_eq!(spec.typ, bits);
}

#[test]
fn loops_and_assigns_preserved_without_type_env() {
    let mut it = SimpleInterner::new();
    let body = Node::dummy(ExpKind::Loop {
        loop_kind: sail_ast::Loop::While,
        measure: None,
        cond: Box::new(id(&mut it, "c")),
        body: Box::new(Node::dummy(ExpKind::Assign {
            lexp: Box::new(Node::dummy(LExpKind::Id(nm(&mut it, "x")))),
            value: Box::new(id(&mut it, "v")),
        })),
    });
    let original = body.clone();
    let mut ast = Ast::new(vec![wild_let(body)]);
    RemoveEAssign.run_typed(&mut ast, &mut it);
    let DefKind::Let { exp, .. } = &ast.defs[5].kind else {
        panic!("expected let");
    };
    assert_eq!(*exp, original);
}

// ---------------------------------------------------------------------------
// internal_lets: early-return classification
// ---------------------------------------------------------------------------

fn run_one(it: &mut SimpleInterner, exp: Exp) -> Exp {
    let mut ast = Ast::new(vec![wild_let(exp)]);
    InternalLets.run_typed(&mut ast, it);
    let DefKind::Let { exp, .. } = ast.defs.swap_remove(0).kind else {
        panic!("def shape");
    };
    exp
}

#[test]
fn let_with_early_return_bind_becomes_plet() {
    let mut it = SimpleInterner::new();
    let bind = early_return_of(&mut it, "x");
    let exp = Node::dummy(ExpKind::Let {
        pat: Node::dummy(PatKind::Id(nm(&mut it, "r"))),
        bind: Box::new(bind),
        body: Box::new(id(&mut it, "r")),
    });
    let out = run_one(&mut it, exp);
    assert!(
        matches!(out.kind, ExpKind::InternalPlet { .. }),
        "got {:?}",
        out.kind
    );

    // Bare `return` counts as early return as well.
    let exp = Node::dummy(ExpKind::Let {
        pat: Node::dummy(PatKind::Wild),
        bind: Box::new(Node::dummy(ExpKind::Return(Box::new(id(&mut it, "v"))))),
        body: Box::new(id(&mut it, "w")),
    });
    let out = run_one(&mut it, exp);
    assert!(matches!(out.kind, ExpKind::InternalPlet { .. }));
}

#[test]
fn pure_lets_and_vars_are_preserved() {
    let mut it = SimpleInterner::new();
    let exp = Node::dummy(ExpKind::Let {
        pat: Node::dummy(PatKind::Id(nm(&mut it, "x"))),
        bind: Box::new(id(&mut it, "e")),
        body: Box::new(id(&mut it, "x")),
    });
    let out = run_one(&mut it, exp.clone());
    assert_eq!(out, exp, "pure let must stay E_let");

    let var = Node::dummy(ExpKind::Var {
        lexp: Box::new(Node::dummy(LExpKind::Id(nm(&mut it, "x")))),
        init: Box::new(id(&mut it, "e")),
        body: Box::new(id(&mut it, "x")),
    });
    let out = run_one(&mut it, var.clone());
    assert_eq!(out, var, "pure var must stay E_var");
}

#[test]
fn var_with_early_return_becomes_plet_for_var_lexps() {
    let mut it = SimpleInterner::new();
    // `LE_id` -> `P_id`.
    let var = Node::dummy(ExpKind::Var {
        lexp: Box::new(Node::dummy(LExpKind::Id(nm(&mut it, "x")))),
        init: Box::new(early_return_of(&mut it, "e")),
        body: Box::new(id(&mut it, "x")),
    });
    let out = run_one(&mut it, var);
    let ExpKind::InternalPlet { pat, .. } = &out.kind else {
        panic!("expected plet, got {:?}", out.kind);
    };
    assert!(matches!(&pat.kind, PatKind::Id(_)));

    // `LE_typ` -> `P_typ` wrapper.
    let var = Node::dummy(ExpKind::Var {
        lexp: Box::new(Node::dummy(LExpKind::Typ {
            typ: int_typ(&mut it),
            id: nm(&mut it, "y"),
        })),
        init: Box::new(Node::dummy(ExpKind::Return(Box::new(id(&mut it, "e"))))),
        body: Box::new(id(&mut it, "y")),
    });
    let out = run_one(&mut it, var);
    let ExpKind::InternalPlet { pat, .. } = &out.kind else {
        panic!("expected plet, got {:?}", out.kind);
    };
    assert!(matches!(&pat.kind, PatKind::Typ { .. }));

    // `LE_tuple` of var lexps -> `P_tuple`.
    let var = Node::dummy(ExpKind::Var {
        lexp: Box::new(Node::dummy(LExpKind::Tuple(vec![
            Node::dummy(LExpKind::Id(nm(&mut it, "a"))),
            Node::dummy(LExpKind::Id(nm(&mut it, "b"))),
        ]))),
        init: Box::new(early_return_of(&mut it, "e")),
        body: Box::new(id(&mut it, "a")),
    });
    let out = run_one(&mut it, var);
    let ExpKind::InternalPlet { pat, .. } = &out.kind else {
        panic!("expected plet, got {:?}", out.kind);
    };
    assert!(matches!(&pat.kind, PatKind::Tuple(_)));
}

#[test]
fn var_with_complex_lexp_survives_despite_early_return() {
    let mut it = SimpleInterner::new();
    // OCaml `failwith`s here; we preserve the node (documented).
    let var = Node::dummy(ExpKind::Var {
        lexp: Box::new(Node::dummy(LExpKind::Vector {
            lexp: Box::new(Node::dummy(LExpKind::Id(nm(&mut it, "v")))),
            index: Box::new(id(&mut it, "i")),
        })),
        init: Box::new(early_return_of(&mut it, "e")),
        body: Box::new(id(&mut it, "v")),
    });
    let out = run_one(&mut it, var.clone());
    assert_eq!(out, var);
}

#[test]
fn unresolvable_calls_are_not_early_return() {
    // A call target that resolves to nothing counts as pure (conservative).
    let mut it = SimpleInterner::new();
    let unresolved = Node::dummy(ExpKind::App {
        func: Name::dummy(sail_ast::Symbol::DUMMY),
        args: vec![],
    });
    let exp = Node::dummy(ExpKind::Let {
        pat: Node::dummy(PatKind::Wild),
        bind: Box::new(unresolved),
        body: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Unit)))),
    });
    let snapshot = exp.clone();
    let out = run_one(&mut it, exp);
    assert_eq!(out, snapshot);
}

#[test]
fn nested_lets_classified_bottom_up_and_idempotent() {
    let mut it = SimpleInterner::new();
    let inner = Node::dummy(ExpKind::Let {
        pat: Node::dummy(PatKind::Id(nm(&mut it, "y"))),
        bind: Box::new(early_return_of(&mut it, "e")),
        body: Box::new(id(&mut it, "y")),
    });
    let outer = Node::dummy(ExpKind::Let {
        pat: Node::dummy(PatKind::Id(nm(&mut it, "x"))),
        bind: Box::new(early_return_of(&mut it, "f")),
        body: Box::new(inner),
    });
    let out = run_one(&mut it, outer);
    let ExpKind::InternalPlet { body, .. } = &out.kind else {
        panic!("outer must be plet, got {:?}", out.kind);
    };
    assert!(matches!(&body.kind, ExpKind::InternalPlet { .. }));
    // Idempotent: plet nodes are never reclassified.
    let again = run_one(&mut it, out.clone());
    assert_eq!(again, out);
}

#[test]
fn fundef_arms_are_visited() {
    let mut it = SimpleInterner::new();
    let body = Node::dummy(ExpKind::Let {
        pat: Node::dummy(PatKind::Id(nm(&mut it, "x"))),
        bind: Box::new(early_return_of(&mut it, "e")),
        body: Box::new(id(&mut it, "x")),
    });
    let fundef: Def = Node::dummy(DefKind::Fundef {
        rec: sail_ast::RecOpt::NonRec,
        annot: sail_ast::TAnnotOpt::None,
        funcls: vec![Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(&mut it, "f"),
            arm: PExp {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                kind: PExpKind::PatExp {
                    pat: Node::dummy(PatKind::Wild),
                    body,
                },
            },
        }],
    });
    let mut ast = Ast::new(vec![fundef]);
    InternalLets.run_typed(&mut ast, &mut it);
    let DefKind::Fundef { funcls, .. } = &ast.defs[0].kind else {
        panic!("expected fundef");
    };
    let PExpKind::PatExp { body, .. } = &funcls[0].arm.kind else {
        panic!("expected arm");
    };
    assert!(matches!(&body.kind, ExpKind::InternalPlet { .. }));
}
