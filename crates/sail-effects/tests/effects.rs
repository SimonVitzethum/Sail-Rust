//! Tests for predicates + callgraph on the small test IR.

#![forbid(unsafe_code)]

use sail_effects::{Binop, Block, CallGraph, EffectLabel, Expr, FunDef, Stmt, effect_label, function_is_pure, has_early_return, has_effect, has_loop};

fn pure_add() -> FunDef {
    FunDef::new(
        "add",
        Block::new(vec![Stmt::Return(Expr::Binop {
            op: Binop::Add,
            lhs: Box::new(Expr::Var("x".to_owned())),
            rhs: Box::new(Expr::Var("y".to_owned())),
        })]),
    )
}

fn reg_writer() -> FunDef {
    FunDef::new(
        "set_x",
        Block::new(vec![Stmt::WriteReg {
            reg: "x".to_owned(),
            expr: Expr::Lit(1),
        }]),
    )
}

fn looper() -> FunDef {
    FunDef::new(
        "spin",
        Block::new(vec![Stmt::While {
            cond: Expr::Lit(1),
            body: Block::new(vec![Stmt::Assign {
                name: "i".to_owned(),
                expr: Expr::Lit(0),
            }]),
        }]),
    )
}

fn early_ret() -> FunDef {
    FunDef::new(
        "guard",
        Block::new(vec![
            Stmt::If {
                cond: Expr::Var("c".to_owned()),
                then_branch: Block::new(vec![Stmt::Return(Expr::Lit(1))]),
                else_branch: Block::empty(),
            },
            Stmt::Return(Expr::Lit(0)),
        ]),
    )
}

#[test]
fn pure_function_labels() {
    let f = pure_add();
    assert!(function_is_pure(&f));
    assert!(!has_effect(&f));
    assert!(!has_loop(&f));
    assert!(!has_early_return(&f));
    assert_eq!(effect_label(&f), EffectLabel::Pure);
}

#[test]
fn reg_write_is_sailm() {
    let f = reg_writer();
    assert!(!function_is_pure(&f));
    assert!(has_effect(&f));
    assert_eq!(effect_label(&f), EffectLabel::SailM);
}

#[test]
fn reg_read_and_mem_and_nondet_are_effects() {
    for expr in [
        Expr::ReadReg("x".to_owned()),
        Expr::ReadMem(Box::new(Expr::Lit(0))),
        Expr::Nondet,
    ] {
        let f = FunDef::new("g", Block::new(vec![Stmt::Return(expr)]));
        assert!(has_effect(&f), "expected effect");
        assert_eq!(effect_label(&f), EffectLabel::SailM);
    }
}

#[test]
fn loops_detected_including_nested() {
    assert!(has_loop(&looper()));
    let nested = FunDef::new(
        "n",
        Block::new(vec![Stmt::If {
            cond: Expr::Lit(1),
            then_branch: Block::new(vec![Stmt::For {
                var: "i".to_owned(),
                lo: Expr::Lit(0),
                hi: Expr::Lit(9),
                body: Block::empty(),
            }]),
            else_branch: Block::empty(),
        }]),
    );
    assert!(has_loop(&nested));
    assert!(!has_loop(&pure_add()));
}

#[test]
fn early_return_throw_assert() {
    assert!(has_early_return(&early_ret()));
    assert_eq!(effect_label(&early_ret()), EffectLabel::ExceptM);
    // Tail return alone is NOT early.
    assert!(!has_early_return(&pure_add()));
    // Throw is early + impure (ExceptM, not SailM).
    let t = FunDef::new("t", Block::new(vec![Stmt::Throw(Expr::Lit(0))]));
    assert!(has_early_return(&t));
    assert!(!function_is_pure(&t));
    assert!(!has_effect(&t));
    assert_eq!(effect_label(&t), EffectLabel::ExceptM);
    // Assert counts as early (compiled via ExceptM).
    let a = FunDef::new("a", Block::new(vec![Stmt::Assert(Expr::Lit(1))]));
    assert!(has_early_return(&a));
    assert_eq!(effect_label(&a), EffectLabel::ExceptM);
}

#[test]
fn sailm_dominates_exceptm() {
    // Both a loop/throw AND a reg write -> SailM wins.
    let f = FunDef::new(
        "both",
        Block::new(vec![
            Stmt::Throw(Expr::Lit(0)),
            Stmt::WriteReg {
                reg: "x".to_owned(),
                expr: Expr::Lit(1),
            },
        ]),
    );
    assert_eq!(effect_label(&f), EffectLabel::SailM);
}

#[test]
fn callgraph_edges_and_reverse() {
    let prog = vec![
        FunDef::new(
            "main",
            Block::new(vec![
                Stmt::ExprStmt(Expr::Call {
                    func: "helper".to_owned(),
                    args: vec![],
                }),
                Stmt::ExprStmt(Expr::Call {
                    func: "helper".to_owned(), // duplicate -> deduped
                    args: vec![],
                }),
            ]),
        ),
        FunDef::new(
            "helper",
            Block::new(vec![Stmt::ExprStmt(Expr::Call {
                func: "leaf".to_owned(),
                args: vec![],
            })]),
        ),
        FunDef::new("leaf", Block::empty()),
    ];
    let g = CallGraph::build(&prog);
    assert_eq!(g.len(), 3);
    assert!(g.has_edge("main", "helper"));
    assert!(g.has_edge("helper", "leaf"));
    assert!(!g.has_edge("main", "leaf"));
    assert_eq!(g.callees_of("main"), vec!["helper"]);
    assert_eq!(g.callers_of("helper"), vec!["main"]);
    assert_eq!(g.callers_of("leaf"), vec!["helper"]);
    // Transitive reachability.
    let reach = g.reachable_from("main");
    assert!(reach.contains(&"helper") && reach.contains(&"leaf"));
    assert!(g.reachable_from("leaf").is_empty());
    assert!(g.reachable_from("unknown").is_empty());
}

#[test]
fn callgraph_extern_and_recursion() {
    let prog = vec![FunDef::new(
        "rec",
        Block::new(vec![Stmt::ExprStmt(Expr::Call {
            func: "rec".to_owned(), // self-loop
            args: vec![],
        })]),
    )];
    let g = CallGraph::build(&prog);
    assert!(g.has_edge("rec", "rec"));
    assert_eq!(g.callees_of("rec"), vec!["rec"]);

    // Unknown callee becomes an extern node.
    let prog2 = vec![FunDef::new(
        "a",
        Block::new(vec![Stmt::ExprStmt(Expr::Call {
            func: "extern_fn".to_owned(),
            args: vec![],
        })]),
    )];
    let g2 = CallGraph::build(&prog2);
    assert!(g2.has_edge("a", "extern_fn"));
    assert!(g2.id_of("extern_fn").is_some());
    assert!(g2.callees_of("extern_fn").is_empty());
}

#[test]
fn predicates_are_thread_safe() {
    // Pure borrows + Send+Sync: analyse functions in parallel.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<FunDef>();
    assert_send_sync::<CallGraph>();
    let prog = vec![pure_add(), reg_writer(), looper(), early_ret()];
    let handles: Vec<_> = prog
        .into_iter()
        .map(|f| std::thread::spawn(move || (function_is_pure(&f), has_loop(&f), effect_label(&f))))
        .collect();
    let mut saw_pure = false;
    let mut saw_loop = false;
    for h in handles {
        let (pure, looping, _) = h.join().expect("thread");
        saw_pure |= pure;
        saw_loop |= looping;
    }
    assert!(saw_pure && saw_loop);
}
