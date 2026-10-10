#![forbid(unsafe_code)]
//! Lane-107-prove-Pfade (Code, E3: nicht ausfuehren): echte `NC_set`/
//! `NC_app`-Pfade (Kind-Env, Faltung, Stub-Beweise, `conv`-Bruecke),
//! Solver-Kontext (`val`/`register`/`scattered`-Annahmen als Implikation)
//! und `scattered`-Klauselpruefung — alles nur via [`Solver`]-Trait
//! (kein Z3-Import; Lane 06/80 haengt dahinter).

use sail_ast::{ConstraintKind, Interner as _, NExpKind, Name, Node, SimpleInterner, TypArgKind, TypKind};
use sail_ty::conv::{constraint_from_ast, constraint_to_ast};
use sail_ty::kind::{KindEnv, check_constraint_kind};
use sail_ty::{
    BidirectionalChecker, CheckLimits, Kind, MiniExp, MiniTyp, NConstraint, Nexp, ProveResult,
    ScatteredClauses, Solver, SolverContext, Span, StubSolver, TypeEnv, TypQuant,
    assumption_from_register, assumption_from_val, implication, kenv_of_quants, prove_in_context,
};

fn dummy() -> Span {
    Span::DUMMY
}

fn checker<'s, S: Solver>(s: &'s S) -> BidirectionalChecker<'s, S> {
    BidirectionalChecker::new(s, CheckLimits::default())
}

fn var(name: &str) -> Nexp {
    Nexp::Var(name.to_string())
}

fn const_(c: i128) -> Nexp {
    Nexp::Const(c)
}

/// Staerkerer Trait-Backend als Z3-Platzhalter (Lane 06/80): versteht die
/// [`implication`]-Kodierung (`A -> A` per Excluded-Middle) und faellt
/// sonst auf den Stub zurueck. Erreicht den Checker nur via `Solver`.
#[derive(Clone, Copy, Debug, Default)]
struct CtxSolver;

impl Solver for CtxSolver {
    fn prove(&self, nc: &NConstraint) -> ProveResult {
        if let NConstraint::Or(a, b) = nc {
            if let NConstraint::Not(inner) = &**a {
                if **inner == **b {
                    return ProveResult::Proved;
                }
            }
        }
        StubSolver::new().prove(nc)
    }
}

// ---------------------------------------------------------------------------
// CK-SetApp: Kind-Regeln + Faltung + Stub-Pfade + conv-Brücke
// ---------------------------------------------------------------------------

#[test]
fn prove_set_app() {
    let mut env = KindEnv::new();
    env.bind("n", Kind::KInt);
    env.bind("b", Kind::KBool);
    // CK-Set: int-Stellen ok, bool-Stellen abgelehnt.
    let set = NConstraint::Set(Box::new(var("n")), vec![1, 5]);
    assert_eq!(check_constraint_kind(&set, &env, dummy()), Ok(Kind::KBool));
    assert!(check_constraint_kind(
        &NConstraint::Set(Box::new(var("b")), vec![1]),
        &env,
        dummy()
    )
    .is_err());
    // CK-App: int-Args ok, bool-Args abgelehnt.
    let app = NConstraint::App("ok".to_string(), vec![var("n")]);
    assert_eq!(check_constraint_kind(&app, &env, dummy()), Ok(Kind::KBool));
    assert!(check_constraint_kind(
        &NConstraint::App("ok".to_string(), vec![var("b")]),
        &env,
        dummy()
    )
    .is_err());
    // CK-Var: nur `K_bool`-gebunden; CK-Id: immer `K_bool`.
    assert_eq!(
        check_constraint_kind(&NConstraint::Var("b".to_string()), &env, dummy()),
        Ok(Kind::KBool)
    );
    assert!(check_constraint_kind(&NConstraint::Var("n".to_string()), &env, dummy()).is_err());
    assert!(check_constraint_kind(&NConstraint::Var("free".to_string()), &env, dummy()).is_err());
    assert_eq!(
        check_constraint_kind(&NConstraint::Id("p".to_string()), &env, dummy()),
        Ok(Kind::KBool)
    );
    // Faltung: konstante Mitgliedschaft + leere Menge; App/Var/Id nie.
    assert_eq!(
        NConstraint::Set(Box::new(const_(5)), vec![1, 5]).as_bool(),
        Some(true)
    );
    assert_eq!(
        NConstraint::Set(Box::new(const_(7)), vec![1, 5]).as_bool(),
        Some(false)
    );
    assert_eq!(NConstraint::Set(Box::new(var("n")), vec![]).as_bool(), Some(false));
    assert_eq!(NConstraint::Set(Box::new(var("n")), vec![1]).as_bool(), None);
    assert_eq!(app.as_bool(), None);
    assert_eq!(NConstraint::Var("b".to_string()).as_bool(), None);
    assert_eq!(NConstraint::Id("p".to_string()).as_bool(), None);
    // Stub-Pfade: konstant bewiesen/widerlegt, Singleton als Gleichung,
    // Opaque bleiben sound `Unknown`.
    let stub = StubSolver::new();
    assert_eq!(
        stub.prove(&NConstraint::Set(Box::new(const_(5)), vec![1, 5])),
        ProveResult::Proved
    );
    assert_eq!(
        stub.prove(&NConstraint::Set(Box::new(const_(7)), vec![1, 5])),
        ProveResult::Refuted
    );
    assert_eq!(
        stub.prove(&NConstraint::Set(Box::new(var("n")), vec![])),
        ProveResult::Refuted
    );
    assert_eq!(
        stub.prove(&NConstraint::Set(Box::new(var("n")), vec![3, 1])),
        ProveResult::Unknown
    );
    assert_eq!(stub.prove(&app), ProveResult::Unknown);
    assert_eq!(
        stub.prove(&NConstraint::Var("b".to_string())),
        ProveResult::Unknown
    );
    assert_eq!(stub.prove(&NConstraint::Id("p".to_string())), ProveResult::Unknown);
    // Tiefen-/Variablen-/Display-Pfade terminieren total.
    assert_eq!(set.depth(), 2);
    assert_eq!(NConstraint::Id("p".to_string()).depth(), 1);
    let mut free = Vec::new();
    NConstraint::App("f".to_string(), vec![var("n")]).free_vars(&mut free);
    assert_eq!(free, vec!["n".to_string()]);
    let mut free = Vec::new();
    NConstraint::Var("b".to_string()).free_vars(&mut free);
    assert_eq!(free, vec!["b".to_string()]);
    assert_eq!(format!("{set}"), "('n in {1, 5})");
    assert_eq!(format!("{app}"), "ok('n)");
    // `conv`-Bruecke: Set/App/Id/Var runden durchs Kanonische …
    let mut it = SimpleInterner::new();
    for local in [
        NConstraint::Set(Box::new(var("n")), vec![5, 1, 5]),
        NConstraint::App("ok".to_string(), vec![var("n"), const_(2)]),
        NConstraint::Id("p".to_string()),
        NConstraint::Var("b".to_string()),
    ] {
        let ast = constraint_to_ast(&local, &mut it);
        assert_eq!(constraint_from_ast(&ast, &it), Some(local));
    }
    // … Fremdformen bleiben `None` (residual X-NcSetApp).
    let foreign_app = Node::dummy(ConstraintKind::App {
        func: Name::dummy(it.intern("f")),
        args: vec![Node::dummy(TypArgKind::Typ(Node::dummy(TypKind::Id(Name::dummy(
            it.intern("int"),
        )))))],
    });
    assert_eq!(constraint_from_ast(&foreign_app, &it), None);
    let bad_members = Node::dummy(ConstraintKind::Set {
        nexp: Node::dummy(NExpKind::Constant("1".to_string())),
        members: vec!["abc".to_string()],
    });
    assert_eq!(constraint_from_ast(&bad_members, &it), None);
    let foreign_nexp = Node::dummy(ConstraintKind::Ge(
        Node::dummy(NExpKind::Id(Name::dummy(it.intern("f")))),
        Node::dummy(NExpKind::Constant("0".to_string())),
    ));
    assert_eq!(constraint_from_ast(&foreign_nexp, &it), None);
}

// ---------------------------------------------------------------------------
// Z-ProveCtx: Kind-Env + Annahmen als Implikation, nur via Trait
// ---------------------------------------------------------------------------

#[test]
fn prove_solver_context() {
    let kenv = kenv_of_quants(&[("n".to_string(), Kind::KInt)]);
    assert_eq!(kenv.lookup("n"), Some(Kind::KInt));
    let goal = NConstraint::Ge(Box::new(var("n")), Box::new(const_(0)));
    // Leerer Kontext ≡ direkter Trait-Aufruf (plus Artpruefung).
    let stub = StubSolver::new();
    assert!(SolverContext::new().is_empty());
    assert_eq!(
        prove_in_context(&stub, &kenv, &SolverContext::new(), &goal, dummy()),
        stub.prove(&goal)
    );
    // `implication` ohne Annahmen ist das Ziel selbst …
    assert_eq!(implication(&[], &goal), goal);
    // … mit Annahmen `¬(a1 & a2) | goal`.
    let asm = NConstraint::Ge(Box::new(var("n")), Box::new(const_(8)));
    let imp = implication(&[asm.clone()], &goal);
    assert_eq!(
        imp,
        NConstraint::Or(
            Box::new(NConstraint::Not(Box::new(asm.clone()))),
            Box::new(goal.clone())
        )
    );
    // Kind-Env-Pfad: schlechtgekindetes Ziel ist `Unknown`, selbst wenn
    // das Backend alles beweisen wuerde.
    struct YesSolver;
    impl Solver for YesSolver {
        fn prove(&self, _nc: &NConstraint) -> ProveResult {
            ProveResult::Proved
        }
    }
    let ill = NConstraint::Ge(Box::new(var("free")), Box::new(const_(0)));
    assert_eq!(
        prove_in_context(&YesSolver, &kenv, &SolverContext::new(), &ill, dummy()),
        ProveResult::Unknown
    );
    // Widerspruchs-Annahme beweist alles — schon mit dem Stub.
    let mut ctx = SolverContext::new();
    ctx.assume(NConstraint::False);
    assert_eq!(ctx.len(), 1);
    assert_eq!(ctx.assumptions().to_vec(), vec![NConstraint::False]);
    assert_eq!(
        prove_in_context(&stub, &kenv, &ctx, &goal, dummy()),
        ProveResult::Proved
    );
    // `val`/`register`-Annahmen kommen aus den Env-Schemes (Q-Inst-frisch).
    let mut env = TypeEnv::new();
    env.add_val(
        "f",
        TypQuant {
            quants: vec![("n".to_string(), Kind::KInt)],
            constraint: NConstraint::Ge(Box::new(var("n")), Box::new(const_(8))),
            typ: MiniTyp::Int,
        },
    );
    assert_eq!(
        assumption_from_val(&env, "f"),
        Some(NConstraint::Ge(Box::new(var("inst0")), Box::new(const_(8))))
    );
    assert_eq!(assumption_from_val(&env, "missing"), None);
    env.add_register("r", TypQuant::monomorphic(MiniTyp::Int));
    assert_eq!(assumption_from_register(&env, "r"), Some(NConstraint::True));
    assert_eq!(assumption_from_register(&env, "missing"), None);
    let mut ctx = SolverContext::new();
    assert!(ctx.assume_val(&env, "f"));
    assert!(!ctx.assume_val(&env, "missing"));
    assert!(ctx.assume_register(&env, "r"));
    assert!(!ctx.assume_register(&env, "missing"));
    assert_eq!(ctx.len(), 2);
    // Gleiche Annahme wie Ziel entscheidet der staerkere Trait-Backend.
    let mut kenv2 = KindEnv::new();
    kenv2.bind("n", Kind::KInt);
    let mut ctx2 = SolverContext::new();
    ctx2.assume(goal.clone());
    assert_eq!(
        prove_in_context(&CtxSolver, &kenv2, &ctx2, &goal, dummy()),
        ProveResult::Proved
    );
    assert_eq!(
        prove_in_context(&stub, &kenv2, &ctx2, &goal, dummy()),
        ProveResult::Unknown,
        "Stub bleibt sound Unknown"
    );
    // `check_val_def_in_context`: leere Annahmen wie bisher (Mono ok,
    // Poly abstrakt, refuted ein Fehler) …
    let c = checker(&stub);
    let env = TypeEnv::new();
    let mono = TypQuant::monomorphic(MiniTyp::Int);
    assert!(c
        .check_val_def_in_context("x", &mono, &MiniExp::IntLit(7), &env, &ctx2, dummy())
        .is_ok());
    let impossible = TypQuant { quants: Vec::new(), constraint: NConstraint::False, typ: MiniTyp::Int };
    assert!(c
        .check_val_def_in_context("z", &impossible, &MiniExp::IntLit(7), &env, &ctx2, dummy())
        .is_err());
    // … und `check_register_in_context` entlaedt Register-Schemes echt:
    // `False`-Precondition ist jetzt ein harter Fehler.
    assert!(c
        .check_register_in_context(
            "r",
            &TypQuant { quants: Vec::new(), constraint: NConstraint::False, typ: MiniTyp::Int },
            &SolverContext::new(),
            dummy()
        )
        .is_err());
    assert!(c
        .check_register_in_context("ok", &mono, &SolverContext::new(), dummy())
        .is_ok());
}

// ---------------------------------------------------------------------------
// D-Scattered: Klauseln unter akkumuliertem Solver-Kontext
// ---------------------------------------------------------------------------

#[test]
fn scattered_clauses() {
    let stub = StubSolver::new();
    let c = checker(&stub);
    let open = || TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::Ge(Box::new(var("n")), Box::new(const_(0))),
        typ: MiniTyp::Bits(Box::new(var("n"))),
    };
    // Zwei offene Klauseln: beide ok, Namen `<name>#<i>`.
    let sums = c.check_scattered("step", &[open(), open()], dummy()).unwrap();
    assert_eq!(sums.len(), 2);
    assert_eq!(sums[0].name, "step#0");
    assert_eq!(sums[1].name, "step#1");
    // Leere Klauselliste ist ok (keine Summaries).
    assert!(c.check_scattered("empty", &[], dummy()).unwrap().is_empty());
    // `False`-Klausel ist ein harter Fehler (widerlegt, auch im Kontext).
    assert!(c.check_scattered("bad", &[open(), TypQuant::monomorphic(MiniTyp::Int)], dummy()).is_ok());
    let mut bad_second = open();
    bad_second.constraint = NConstraint::False;
    assert!(c.check_scattered("bad", &[open(), bad_second], dummy()).is_err());
    // `ScatteredClauses`: Konjunktion in Elaborationsreihenfolge.
    let mut sc = ScatteredClauses::new("step");
    assert_eq!(sc.conjoin(), None);
    sc.push(NConstraint::True);
    sc.push(open().constraint);
    assert_eq!(
        sc.conjoin(),
        Some(NConstraint::And(
            Box::new(NConstraint::True),
            Box::new(NConstraint::Ge(Box::new(var("n")), Box::new(const_(0))))
        ))
    );
    let mut ctx = SolverContext::new();
    ctx.assume_scattered(&sc);
    assert_eq!(ctx.len(), 2);
}
