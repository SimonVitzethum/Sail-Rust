#![forbid(unsafe_code)]
//! Canonical-path pipeline over `sail_ast::Ast`.
//!
//! Builds one program exercising every wired typed pass, runs
//! [`TypedPipeline::lean_implemented`] (Lean order), and checks each OCaml
//! `rewrites.ml` behaviour end to end. A second test runs the full
//! [`TypedPipeline::lean_default`] (implemented passes + stubs) to pin the
//! order wiring.

use sail_ast::{
    Ast, Def, DefKind, Exp, ExpKind, Extern, Funcl, Interner, LExpKind, Name, Node, NodeId, PExp,
    PExpKind, PatKind, QuantItem, RecOpt, SimpleInterner, Span, TAnnotOpt, TypKind, TypSchm,
    ValSpec,
};
use sail_rewrite::pipeline::{TypedPipeline, LEAN_REWRITES};
use sail_rewrite::passes::{AddUnspecifiedRec, MinimiseRecursiveFunctions};

fn nm(it: &mut SimpleInterner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn id(it: &mut SimpleInterner, text: &str) -> Exp {
    Node::dummy(ExpKind::Id(nm(it, text)))
}

fn wild_let(exp: Exp) -> Def {
    Def::dummy(DefKind::Let {
        pat: Node::dummy(PatKind::Wild),
        exp,
    })
}

fn fundef(it: &mut SimpleInterner, name: &str, bodies: Vec<Exp>) -> Def {
    let funcls = bodies
        .into_iter()
        .map(|body| Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(it, name),
            arm: PExp {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                kind: PExpKind::PatExp {
                    pat: Node::dummy(PatKind::Wild),
                    body,
                },
            },
        })
        .collect();
    Def::dummy(DefKind::Fundef {
        rec: RecOpt::NonRec,
        annot: TAnnotOpt::None,
        funcls,
    })
}

fn valspec(it: &mut SimpleInterner, name: &str, backend: Option<(&str, &str)>) -> Def {
    Def::dummy(DefKind::Val(ValSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: nm(it, name),
        typschm: TypSchm {
            quant: Vec::<QuantItem>::new(),
            typ: Node::dummy(TypKind::Unknown),
        },
        extern_opt: backend.map(|(k, v)| Extern {
            pure: true,
            bindings: vec![(k.to_owned(), v.to_owned())],
        }),
    }))
}

/// Program covering every wired typed pass:
/// duplicate valspecs, a 2-clause function (merge), a tuple assignment, a
/// multi-statement block, a redundant `let x = e in x`, a string-append arm,
/// plus preserved `Undef` and a plain assignment.
fn covering_program(it: &mut SimpleInterner) -> Ast {
    let tuple_assign = Node::dummy(ExpKind::Assign {
        lexp: Box::new(Node::dummy(LExpKind::Tuple(vec![
            Node::dummy(LExpKind::Id(nm(it, "a"))),
            Node::dummy(LExpKind::Id(nm(it, "b"))),
        ]))),
        value: Box::new(id(it, "t")),
    });
    let block = Node::dummy(ExpKind::Block(vec![id(it, "s1"), id(it, "s2")]));
    let redun = Node::dummy(ExpKind::Let {
        pat: Node::dummy(PatKind::Id(nm(it, "x"))),
        bind: Box::new(id(it, "e")),
        body: Box::new(id(it, "x")),
    });
    let match_exp = Node::dummy(ExpKind::Match {
        scrutinee: Box::new(id(it, "s")),
        arms: vec![PExp {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind: PExpKind::PatExp {
                pat: Node::dummy(PatKind::StringAppend(vec![Node::dummy(PatKind::Wild)])),
                body: Node::dummy(ExpKind::Undef),
            },
        }],
    });
    let f_bodies = vec![id(it, "one"), id(it, "two")];
    Ast::new(vec![
        valspec(it, "dup", Some(("lean", "one"))),
        valspec(it, "dup", Some(("lean", "two"))),
        fundef(it, "f", f_bodies),
        wild_let(tuple_assign),
        wild_let(block),
        wild_let(redun),
        wild_let(match_exp),
        wild_let(Node::dummy(ExpKind::Undef)),
        wild_let(Node::dummy(ExpKind::Assign {
            lexp: Box::new(Node::dummy(LExpKind::Id(nm(it, "r")))),
            value: Box::new(id(it, "r")),
        })),
    ])
}

fn single_let_exp(ast: &Ast, index: usize) -> &Exp {
    let DefKind::Let { exp, .. } = &ast.defs[index].kind else {
        panic!("def {index} is not a toplevel let")
    };
    exp
}

#[test]
fn typed_implemented_rewrites_covering_program() {
    let mut it = SimpleInterner::new();
    let mut ast = covering_program(&mut it);
    TypedPipeline::lean_implemented().run(&mut ast, &mut it);

    // Duplicate valspec: move_termination_measures (index 0) drops the later
    // duplicate like OCaml `called_output`, so the first spec wins here.
    // remove_e_assign prepends loop-spec preamble defs; locate the fixture
    // by content instead of fixed indices.
    let base = ast
        .defs
        .iter()
        .position(|def| {
            matches!(&def.kind, DefKind::Val(v) if it.resolve(v.name.symbol) == Some("dup"))
        })
        .expect("dup valspec present");
    let DefKind::Val(first) = &ast.defs[base].kind else {
        panic!("dup shape")
    };
    assert_eq!(it.resolve(first.name.symbol), Some("dup"));
    assert_eq!(
        first.extern_opt.as_ref().expect("extern").bindings,
        vec![("lean".to_owned(), "one".to_owned())]
    );
    assert_eq!(ast.defs.len(), base + 8, "one duplicate valspec must go");

    // Merged clauses: one clause matching on merge#var.
    let DefKind::Fundef { funcls, .. } = &ast.defs[base + 1].kind else {
        panic!("def 1 shape")
    };
    assert_eq!(funcls.len(), 1);
    let PExpKind::PatExp { body, .. } = &funcls[0].arm.kind else {
        panic!("arm shape")
    };
    assert!(matches!(&body.kind, ExpKind::Match { arms, .. } if arms.len() == 2));

    // Tuple assignment bound once; the later `remove_blocks` stage flattens
    // the generated two-statement block into a nested let, as in OCaml.
    let ExpKind::Let { pat, bind, body } = &single_let_exp(&ast, base + 2).kind else {
        panic!("tuple shape: {:?}", single_let_exp(&ast, base + 2).kind)
    };
    assert!(matches!(&pat.kind, PatKind::Tuple(ps) if ps.len() == 2));
    assert!(matches!(&bind.kind, ExpKind::Id(_)));
    let ExpKind::Let { bind: a1, body: a2, .. } = &body.kind else {
        panic!("flattened block shape: {:?}", body.kind)
    };
    assert!(matches!(&a1.kind, ExpKind::Assign { .. }));
    assert!(matches!(&a2.kind, ExpKind::Assign { .. }));

    // Block flattened to nested lets.
    assert!(matches!(&single_let_exp(&ast, base + 3).kind, ExpKind::Let { .. }));

    // Redundant let collapsed to its bound value.
    assert!(matches!(&single_let_exp(&ast, base + 4).kind, ExpKind::Id(_)));

    // String-append arm became wildcard -> exit.
    let ExpKind::Match { arms, .. } = &single_let_exp(&ast, base + 5).kind else {
        panic!("match shape")
    };
    let PExpKind::PatExp { pat, body } = &arms[0].kind else {
        panic!("arm shape")
    };
    assert!(matches!(pat.kind, PatKind::Wild));
    assert!(matches!(body.kind, ExpKind::Exit(_)));

    // Preserved without type info.
    assert!(matches!(&single_let_exp(&ast, base + 6).kind, ExpKind::Undef));
    assert!(matches!(&single_let_exp(&ast, base + 7).kind, ExpKind::Assign { .. }));

    // Whole pipeline is idempotent.
    let snapshot = ast.clone();
    TypedPipeline::lean_implemented().run(&mut ast, &mut it);
    assert_eq!(ast, snapshot, "typed pipeline must be idempotent");
}

#[test]
fn typed_default_keeps_lean_order_with_stubs() {
    let mut it = SimpleInterner::new();
    let mut ast = covering_program(&mut it);
    let pipe = TypedPipeline::lean_default();
    assert_eq!(pipe.pass_names(), LEAN_REWRITES);
    pipe.run(&mut ast, &mut it);
    // Same observable outcome as the implemented-only pipeline: stubs are
    // identity, so tuple lowering still happened exactly once. The tuple
    // let is located by content (preamble shifts fixed indices).
    let tuple_idx = ast
        .defs
        .iter()
        .position(|def| {
            matches!(&def.kind, DefKind::Let { exp, .. } if matches!(&exp.kind, ExpKind::Let { pat, .. } if matches!(&pat.kind, PatKind::Tuple(ps) if ps.len() == 2)))
        })
        .expect("tuple let present");
    let ExpKind::Let { pat, .. } = &single_let_exp(&ast, tuple_idx).kind else {
        panic!("tuple shape")
    };
    assert!(matches!(&pat.kind, PatKind::Tuple(ps) if ps.len() == 2));
}

// Lane-18: Rekursions-Paesse (O-3).

fn call(it: &mut SimpleInterner, func: &str) -> Exp {
    Node::dummy(ExpKind::App {
        func: nm(it, func),
        args: Vec::new(),
    })
}

fn unit() -> Exp {
    Node::dummy(ExpKind::Lit(Node::dummy(sail_ast::LitKind::Unit)))
}

fn rec_fundef(it: &mut SimpleInterner, name: &str, rec: RecOpt, bodies: Vec<Exp>) -> Def {
    let funcls = bodies
        .into_iter()
        .map(|body| Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(it, name),
            arm: PExp {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                kind: PExpKind::PatExp {
                    pat: Node::dummy(PatKind::Wild),
                    body,
                },
            },
        })
        .collect();
    Def::dummy(DefKind::Fundef {
        rec,
        annot: TAnnotOpt::None,
        funcls,
    })
}

fn rec_of(ast: &Ast, index: usize) -> RecOpt {
    let DefKind::Fundef { rec, .. } = &ast.defs[index].kind else {
        panic!("def {index} is not a fundef")
    };
    rec.clone()
}

fn is_nonrec(ast: &Ast, index: usize) -> bool {
    matches!(rec_of(ast, index), RecOpt::NonRec)
}

fn is_rec(ast: &Ast, index: usize) -> bool {
    matches!(rec_of(ast, index), RecOpt::Rec)
}

#[test]
fn lean_order_positions_pin_insert_spec() {
    // Insert-Spec-Anker: die beiden neuen Paesse stehen in LEAN_REWRITES an
    // diesen Positionen (Nachbar-Paesse als Kontext).
    let pos = |name: &str| LEAN_REWRITES.iter().position(|n| *n == name).expect(name);
    assert_eq!(pos("mapping_patterns") + 1, pos("add_unspecified_rec"));
    assert_eq!(pos("add_unspecified_rec"), 8);
    assert_eq!(pos("add_unspecified_rec") + 1, pos("undefined"));
    assert_eq!(pos("split") + 1, pos("minimise_recursive_functions"));
    assert_eq!(pos("minimise_recursive_functions"), 23);
    assert_eq!(
        pos("minimise_recursive_functions") + 1,
        pos("remove_bitfield_records")
    );
}

#[test]
fn add_unspecified_rec_marks_self_call() {
    let mut it = SimpleInterner::new();
    let body = call(&mut it, "f");
    let mut ast = Ast::new(vec![rec_fundef(&mut it, "f", RecOpt::NonRec, vec![body])]);
    AddUnspecifiedRec.run(&mut ast, &mut it);
    assert!(is_rec(&ast, 0));
}

#[test]
fn minimise_demotes_spurious_rec() {
    let mut it = SimpleInterner::new();
    let mut ast = Ast::new(vec![rec_fundef(&mut it, "f", RecOpt::Rec, vec![unit()])]);
    MinimiseRecursiveFunctions.run(&mut ast, &mut it);
    assert!(is_nonrec(&ast, 0));
}

#[test]
fn round_trip_is_stable_and_complementary() {
    // f ruft sich selbst: add markiert Rec, minimise behaelt Rec.
    // g ist nicht rekursiv: add laesst NonRec, minimise demotiert Rec.
    let mut it = SimpleInterner::new();
    let f_body = call(&mut it, "f");
    let f = rec_fundef(&mut it, "f", RecOpt::NonRec, vec![f_body]);
    let g = rec_fundef(&mut it, "g", RecOpt::Rec, vec![unit()]);
    let mut ast = Ast::new(vec![f, g]);
    AddUnspecifiedRec.run(&mut ast, &mut it);
    assert!(is_rec(&ast, 0));
    assert!(is_rec(&ast, 1));
    MinimiseRecursiveFunctions.run(&mut ast, &mut it);
    assert!(is_rec(&ast, 0), "echte Rekursion bleibt Rec");
    assert!(is_nonrec(&ast, 1), "scheinbares Rec wird NonRec");
    // Fixpunkt: weitere Laeufe aendern nichts mehr.
    let snapshot = ast.clone();
    AddUnspecifiedRec.run(&mut ast, &mut it);
    MinimiseRecursiveFunctions.run(&mut ast, &mut it);
    assert_eq!(ast, snapshot);
}

#[test]
fn mutual_recursion_marks_both_and_keeps_both() {
    let mut it = SimpleInterner::new();
    let p_body = call(&mut it, "q");
    let q_body = call(&mut it, "p");
    let mut ast = Ast::new(vec![
        rec_fundef(&mut it, "p", RecOpt::NonRec, vec![p_body]),
        rec_fundef(&mut it, "q", RecOpt::NonRec, vec![q_body]),
    ]);
    AddUnspecifiedRec.run(&mut ast, &mut it);
    assert!(is_rec(&ast, 0));
    assert!(is_rec(&ast, 1));
    MinimiseRecursiveFunctions.run(&mut ast, &mut it);
    assert!(is_rec(&ast, 0));
    assert!(is_rec(&ast, 1));
}

#[test]
fn pass_names_match_ocaml() {
    assert_eq!(AddUnspecifiedRec.name(), "add_unspecified_rec");
    assert_eq!(
        MinimiseRecursiveFunctions.name(),
        "minimise_recursive_functions"
    );
}
