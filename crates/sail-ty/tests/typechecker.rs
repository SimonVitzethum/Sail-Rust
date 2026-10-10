#![forbid(unsafe_code)]
//! Integration tests for `sail-ty` (Lane 05).

use sail_ty::{
    BidirectionalChecker, CacheStats, CachingSolver, CheckLimits, Kind, MiniExp, MiniTyp,
    NConstraint, Nexp, ProveResult, QuantScope, QuantVar, Solver, Span, StubSolver, TypeEnv,
    TypQuant,
};

fn dummy() -> Span {
    Span::DUMMY
}

fn checker<'s, S: Solver>(
    s: &'s S,
) -> BidirectionalChecker<'s, S> {
    BidirectionalChecker::new(s, CheckLimits::default())
}

#[test]
fn kind_int_bool_type_skeleton() {
    use sail_ty::kind::{KindEnv, check_constraint_kind, check_nexp_kind, check_typ_kind};
    let mut env = KindEnv::new();
    env.bind("n", Kind::KInt);
    // K_int var ok
    assert_eq!(
        check_nexp_kind(&Nexp::Var("n".into()), &env, dummy()).unwrap(),
        Kind::KInt
    );
    // const nat refines to K_nat, subsumed by K_int
    let k = check_nexp_kind(&Nexp::Const(3), &env, dummy()).unwrap();
    assert!(Kind::KInt.subsumes(k));
    // constraint is K_bool
    let nc = NConstraint::Ge(
        Box::new(Nexp::Var("n".into())),
        Box::new(Nexp::Const(0)),
    );
    assert_eq!(
        check_constraint_kind(&nc, &env, dummy()).unwrap(),
        Kind::KBool
    );
    // bits('n) is K_type
    let t = MiniTyp::Bits(Box::new(Nexp::Var("n".into())));
    assert_eq!(check_typ_kind(&t, &env, dummy()).unwrap(), Kind::KType);
    // wrong kind: bool-var used as nexp
    let mut env2 = KindEnv::new();
    env2.bind("b", Kind::KBool);
    assert!(check_nexp_kind(&Nexp::Var("b".into()), &env2, dummy()).is_err());
    // negative bits rejected
    assert!(check_typ_kind(
        &MiniTyp::Bits(Box::new(Nexp::Const(-1))),
        &env,
        dummy()
    )
    .is_err());
}

#[test]
fn stub_solver_proves_trivial_refutes_false() {
    let s = StubSolver::new();
    assert_eq!(s.prove(&NConstraint::True), ProveResult::Proved);
    assert_eq!(s.prove(&NConstraint::False), ProveResult::Refuted);
    // constant folded comparison
    let nc = NConstraint::Ge(Box::new(Nexp::Const(8)), Box::new(Nexp::Const(0)));
    assert_eq!(s.prove(&nc), ProveResult::Proved);
    // syntactic reflexivity
    let n = Nexp::Add(
        Box::new(Nexp::Var("n".into())),
        Box::new(Nexp::Const(1)),
    );
    let eq = NConstraint::Eq(Box::new(n.clone()), Box::new(n.clone()));
    assert_eq!(s.prove(&eq), ProveResult::Proved);
    // unknown for open constraint — but terminates
    let open = NConstraint::Gt(
        Box::new(Nexp::Var("n".into())),
        Box::new(Nexp::Const(0)),
    );
    assert_eq!(s.prove(&open), ProveResult::Unknown);
}

#[test]
fn stub_solver_always_terminates_on_deep_input() {
    let s = StubSolver::new();
    // 500-deep conjunction; stub recursion is linear, must return fast.
    let mut nc = NConstraint::Gt(
        Box::new(Nexp::Var("n".into())),
        Box::new(Nexp::Const(0)),
    );
    for _ in 0..500 {
        nc = NConstraint::And(Box::new(nc), Box::new(NConstraint::True));
    }
    let r = s.prove(&nc);
    assert_eq!(r, ProveResult::Unknown);
}

#[test]
fn bidirectional_infer_and_check() {
    let s = StubSolver::new();
    let c = checker(&s);
    let env = TypeEnv::new();
    // infer int literal
    assert_eq!(
        c.infer(&MiniExp::IntLit(42), &env, dummy()).unwrap(),
        MiniTyp::Int
    );
    // check int <= int ok
    c.check(&MiniExp::IntLit(1), &MiniTyp::Int, &env, dummy())
        .unwrap();
    // check int <= bool fails
    assert!(c
        .check(&MiniExp::IntLit(1), &MiniTyp::Bool, &env, dummy())
        .is_err());
    // if-then-else
    let e = MiniExp::If(
        Box::new(MiniExp::BoolLit(true)),
        Box::new(MiniExp::IntLit(1)),
        Box::new(MiniExp::IntLit(2)),
    );
    assert_eq!(c.infer(&e, &env, dummy()).unwrap(), MiniTyp::Int);
    // let-binding with annotation
    let e = MiniExp::Let {
        var: "x".into(),
        annot: Some(MiniTyp::Int),
        bound: Box::new(MiniExp::IntLit(3)),
        body: Box::new(MiniExp::Var("x".into())),
    };
    assert_eq!(c.infer(&e, &env, dummy()).unwrap(), MiniTyp::Int);
    // range literal check, constant bounds
    let r = MiniTyp::Range(Box::new(Nexp::Const(0)), Box::new(Nexp::Const(10)));
    c.check(&MiniExp::IntLit(5), &r, &env, dummy()).unwrap();
    assert!(c.check(&MiniExp::IntLit(99), &r, &env, dummy()).is_err());
}

#[test]
fn quant_scope_bind_shadow_lookup() {
    let mut scope = QuantScope::new();
    scope
        .bind(QuantVar::new("n", Kind::KInt), dummy())
        .unwrap();
    assert_eq!(scope.lookup("n"), Some(Kind::KInt));
    // duplicate in same scope errors
    assert!(scope.bind(QuantVar::new("n", Kind::KInt), dummy()).is_err());
    // inner scope may shadow
    scope.push_scope();
    scope.bind(QuantVar::new("n", Kind::KNat), dummy()).unwrap();
    assert_eq!(scope.lookup("n"), Some(Kind::KNat));
    scope.pop_scope();
    assert_eq!(scope.lookup("n"), Some(Kind::KInt));
    // bind_all telescope
    let mut s2 = QuantScope::new();
    s2.bind_all(
        &[QuantVar::new("n", Kind::KInt), QuantVar::new("b", Kind::KBool)],
        dummy(),
    )
    .unwrap();
    assert_eq!(s2.lookup("b"), Some(Kind::KBool));
}

#[test]
fn scheme_rejects_free_var_and_accepts_closed() {
    let s = StubSolver::new();
    let c = checker(&s);
    // open: 'm free
    let bad = TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::Ge(
            Box::new(Nexp::Var("m".into())),
            Box::new(Nexp::Const(0)),
        ),
        typ: MiniTyp::Bits(Box::new(Nexp::Var("n".into()))),
    };
    assert!(c.check_scheme("bad", &bad, dummy()).is_err());
    // closed: ok
    let good = TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::Ge(
            Box::new(Nexp::Var("n".into())),
            Box::new(Nexp::Const(0)),
        ),
        typ: MiniTyp::Bits(Box::new(Nexp::Var("n".into()))),
    };
    c.check_scheme("good", &good, dummy()).unwrap();
}

#[test]
fn register_types_check() {
    let s = StubSolver::new();
    let c = checker(&s);
    // monomorphic register: xlen : bits(64)
    let mono = TypQuant::monomorphic(MiniTyp::Bits(Box::new(Nexp::Const(64))));
    let sum = c.check_register("xlen", &mono, dummy()).unwrap();
    assert_eq!(sum.name, "xlen");
    // polymorphic register: mem : forall 'n. 'n >= 0 => bits('n)
    let poly = TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::Ge(
            Box::new(Nexp::Var("n".into())),
            Box::new(Nexp::Const(0)),
        ),
        typ: MiniTyp::Bits(Box::new(Nexp::Var("n".into()))),
    };
    let sum = c.check_register("mem", &poly, dummy()).unwrap();
    assert_eq!(sum.name, "mem");
    // ill-kinded register rejected
    let bad = TypQuant::monomorphic(MiniTyp::Bits(Box::new(Nexp::Const(-8))));
    assert!(c.check_register("bad", &bad, dummy()).is_err());
}

#[test]
fn val_def_with_stub_solver() {
    let s = StubSolver::new();
    let c = checker(&s);
    let mut env = TypeEnv::new();
    env.add_val("f", TypQuant::monomorphic(MiniTyp::Fun(
        Box::new(MiniTyp::Int),
        Box::new(MiniTyp::Int),
    )));
    // monomorphic val def: body checked
    let scheme = TypQuant::monomorphic(MiniTyp::Int);
    let body = MiniExp::IntLit(7);
    let sum = c.check_val_def("x", &scheme, &body, &env, dummy()).unwrap();
    assert_eq!(sum.name, "x");
    // body mismatch errors
    let scheme_bool = TypQuant::monomorphic(MiniTyp::Bool);
    assert!(c.check_val_def("y", &scheme_bool, &body, &env, dummy()).is_err());
    // refuted precondition errors even before body check
    let impossible = TypQuant {
        quants: vec![],
        constraint: NConstraint::False,
        typ: MiniTyp::Int,
    };
    assert!(c.check_val_def("z", &impossible, &body, &env, dummy()).is_err());
}

#[test]
fn caching_solver_bounded_and_thread_safe() {
    use std::sync::Arc;
    use std::thread;
    let inner = StubSolver::new();
    let cache = Arc::new(CachingSolver::new(inner, 4));
    let nc_true = NConstraint::True;
    assert_eq!(cache.prove(&nc_true), ProveResult::Proved);
    assert_eq!(cache.prove(&nc_true), ProveResult::Proved);
    let st: CacheStats = cache.stats();
    assert_eq!((st.hits, st.misses), (1, 1));
    // fill beyond capacity => evictions bounded, len <= capacity
    for i in 0..10 {
        let nc = NConstraint::Ge(
            Box::new(Nexp::Const(i)),
            Box::new(Nexp::Const(0)),
        );
        cache.prove(&nc);
    }
    assert!(cache.len() <= 4);
    assert!(cache.stats().evictions > 0);
    // shared across threads (multithreaded checking workers)
    let mut handles = Vec::new();
    for _ in 0..8 {
        let cc = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                cc.prove(&NConstraint::True);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn caching_solver_promotes_hits_lru() {
    // Recency: a cache hit promotes the entry to most-recently-used, so a
    // hot constraint survives eviction while the cold one goes. (The old
    // FIFO order evicted the hot entry first.)
    let cache = CachingSolver::new(StubSolver::new(), 2);
    let a = NConstraint::True;
    let b = NConstraint::False;
    let c = NConstraint::And(Box::new(NConstraint::True), Box::new(NConstraint::True));
    assert_eq!(cache.prove(&a), ProveResult::Proved); // miss
    assert_eq!(cache.prove(&b), ProveResult::Refuted); // miss
    assert_eq!(cache.prove(&a), ProveResult::Proved); // hit: promotes `a`
    assert_eq!(cache.prove(&c), ProveResult::Proved); // miss: evicts LRU (`b`)
    // `a` survived (hot), `b` is gone (cold) — FIFO would answer the
    // reverse here (`a` miss, `b` hit).
    assert_eq!(cache.prove(&a), ProveResult::Proved);
    let st: CacheStats = cache.stats();
    assert_eq!((st.hits, st.misses), (2, 3));
}

#[test]
fn quant_scope_freshen_is_gensym_unique() {
    // Gensym (OCaml `Type_env.fresh_kid`): freshened names carry `#fv`,
    // never collide with user binders, and never repeat across calls.
    let mut scope = QuantScope::new();
    scope.bind(QuantVar::new("n", Kind::KInt), dummy()).unwrap();
    scope.bind(QuantVar::new("f0", Kind::KNat), dummy()).unwrap();
    let m1 = scope.freshen("f");
    let m2 = scope.freshen("f");
    assert_eq!(m1.len(), 2);
    for (orig, fresh) in &m1 {
        assert_ne!(orig, fresh);
        assert!(fresh.contains("#fv"), "gensym shape, got {fresh}");
    }
    assert_ne!(m1["n"], m1["f0"]);
    assert_ne!(m1["n"], m2["n"]);
}

#[test]
fn fuel_guarantees_termination_without_solver() {
    let s = StubSolver::new();
    let tiny = CheckLimits {
        max_steps: 3,
        max_depth: 4,
    };
    let c = BidirectionalChecker::new(&s, tiny);
    let env = TypeEnv::new();
    // deeply nested lets exhaust fuel instead of hanging
    let mut e = MiniExp::IntLit(0);
    for i in 0..20 {
        e = MiniExp::Let {
            var: format!("x{i}"),
            annot: None,
            bound: Box::new(MiniExp::IntLit(i)),
            body: Box::new(e),
        };
    }
    let r = c.infer(&e, &env, dummy());
    assert!(r.is_err());
    // deep nexp in expected type exhausts fuel
    let mut n = Nexp::Const(1);
    for _ in 0..50 {
        n = Nexp::Add(Box::new(n), Box::new(Nexp::Const(1)));
    }
    let t = MiniTyp::Bits(Box::new(n));
    let r = c.check(&MiniExp::IntLit(0), &t, &env, dummy());
    assert!(r.is_err());
}

#[test]
fn types_are_send_sync_for_workers() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MiniTyp>();
    assert_send_sync::<Nexp>();
    assert_send_sync::<NConstraint>();
    assert_send_sync::<TypQuant>();
    assert_send_sync::<TypeEnv>();
    assert_send_sync::<StubSolver>();
    assert_send_sync::<CachingSolver<StubSolver>>();
}
