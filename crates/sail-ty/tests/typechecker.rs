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

#[test]
fn inline_signature_supplements_return_with_arg_patterns() {
    // Signatur-Ergänzungsregel (OCaml `infer_funtyp::typ_from_pat`):
    // literals classify, `(p : t)` yields `t`, tuples map pointwise.
    use sail_ty::MiniPat;
    use sail_ty::check::synthesize_inline_scheme;
    use sail_ty::conv::typ_from_mini_pat;

    assert_eq!(
        typ_from_mini_pat(&MiniPat::IntLit(7), dummy()).unwrap(),
        MiniTyp::Int
    );
    assert_eq!(
        typ_from_mini_pat(&MiniPat::BoolLit(true), dummy()).unwrap(),
        MiniTyp::Bool
    );
    let ann = MiniPat::Annot(Box::new(MiniPat::Var("x".into())), MiniTyp::Int);
    assert_eq!(typ_from_mini_pat(&ann, dummy()).unwrap(), MiniTyp::Int);
    let tup = MiniPat::Tuple(vec![
        MiniPat::Annot(Box::new(MiniPat::Var("x".into())), MiniTyp::Int),
        MiniPat::IntLit(7),
        MiniPat::Tuple(vec![MiniPat::BoolLit(false)]),
    ]);
    assert_eq!(
        typ_from_mini_pat(&tup, dummy()).unwrap(),
        MiniTyp::Tuple(vec![
            MiniTyp::Int,
            MiniTyp::Int,
            MiniTyp::Tuple(vec![MiniTyp::Bool])
        ])
    );
    // Bare binders carry no type information (OCaml + canonical
    // "cannot infer an argument type from an unannotated pattern").
    let err = typ_from_mini_pat(&MiniPat::Var("x".into()), dummy()).unwrap_err();
    assert!(
        err.to_string().contains("cannot infer an argument type"),
        "{err}"
    );
    assert!(typ_from_mini_pat(&MiniPat::Wild, dummy()).is_err());

    // Completed scheme: telescope + return from the header, domain from
    // the patterns.
    let scheme = synthesize_inline_scheme(
        vec![("n".to_string(), Kind::KInt)],
        NConstraint::True,
        &tup,
        &MiniTyp::Bits(Box::new(Nexp::Var("n".into()))),
        dummy(),
    )
    .unwrap();
    assert_eq!(scheme.quants, vec![("n".to_string(), Kind::KInt)]);
    assert_eq!(
        scheme.typ,
        MiniTyp::Fun(
            Box::new(MiniTyp::Tuple(vec![
                MiniTyp::Int,
                MiniTyp::Int,
                MiniTyp::Tuple(vec![MiniTyp::Bool])
            ])),
            Box::new(MiniTyp::Bits(Box::new(Nexp::Var("n".into())))),
        )
    );
}

#[test]
fn inline_header_alias_expansion_matches_val_signature() {
    // Alias-Fälle (OCaml `expanded_typ_identical`): direct, single,
    // chained, nested, unknown, arity mismatch, cycles, budget.
    use sail_ty::check::check_inline_header;
    use sail_ty::conv::AliasEnv;

    fn val_of(ret: MiniTyp) -> TypQuant {
        TypQuant {
            quants: vec![],
            constraint: NConstraint::True,
            typ: MiniTyp::Fun(Box::new(MiniTyp::Int), Box::new(ret)),
        }
    }
    fn named(name: &str, args: Vec<Nexp>) -> MiniTyp {
        MiniTyp::Named { name: name.to_string(), args }
    }
    let val_span = dummy();
    let inline_span = dummy();

    // No aliases: direct match passes, mismatch fails loudly.
    let aliases = AliasEnv::new();
    assert!(aliases.is_empty());
    check_inline_header(
        &val_of(MiniTyp::Int),
        &[],
        &MiniTyp::Int,
        val_span,
        inline_span,
        &aliases,
    )
    .unwrap();
    assert!(
        check_inline_header(
            &val_of(MiniTyp::Bool),
            &[],
            &MiniTyp::Int,
            val_span,
            inline_span,
            &aliases,
        )
        .is_err()
    );

    // Single expansion: `type Integer = int` matches `int` from either side.
    let mut aliases = AliasEnv::new();
    aliases.insert("Integer".to_string(), vec![], MiniTyp::Int);
    assert_eq!(aliases.len(), 1);
    assert!(aliases.lookup("Integer").is_some());
    assert!(aliases.lookup("Missing").is_none());
    check_inline_header(
        &val_of(MiniTyp::Int),
        &[],
        &named("Integer", vec![]),
        val_span,
        inline_span,
        &aliases,
    )
    .unwrap();
    check_inline_header(
        &val_of(named("Integer", vec![])),
        &[],
        &MiniTyp::Int,
        val_span,
        inline_span,
        &aliases,
    )
    .unwrap();

    // Chained expansion: `A -> B -> int`, compared nested in a tuple.
    let mut aliases = AliasEnv::new();
    aliases.insert("B".to_string(), vec![], MiniTyp::Int);
    aliases.insert("A".to_string(), vec![], named("B", vec![]));
    let nested_val = MiniTyp::Tuple(vec![MiniTyp::Int, named("A", vec![])]);
    let nested_inline = MiniTyp::Tuple(vec![MiniTyp::Int, MiniTyp::Int]);
    check_inline_header(
        &val_of(nested_val),
        &[],
        &nested_inline,
        val_span,
        inline_span,
        &aliases,
    )
    .unwrap();

    // Parametrized: `type Wrap('n) = bits('n)`; substitution on expansion.
    let mut aliases = AliasEnv::new();
    aliases.insert(
        "Wrap".to_string(),
        vec!["n".to_string()],
        MiniTyp::Bits(Box::new(Nexp::Var("n".into()))),
    );
    let bits8 = MiniTyp::Bits(Box::new(Nexp::Const(8)));
    check_inline_header(
        &val_of(bits8.clone()),
        &[],
        &named("Wrap", vec![Nexp::Const(8)]),
        val_span,
        inline_span,
        &aliases,
    )
    .unwrap();
    // Wrong argument still mismatches after substitution.
    assert!(
        check_inline_header(
            &val_of(bits8),
            &[],
            &named("Wrap", vec![Nexp::Const(7)]),
            val_span,
            inline_span,
            &aliases,
        )
        .is_err()
    );

    // Unknown aliases stay unexpanded: same unknown head matches, distinct
    // heads mismatch (never silent success).
    let plain = AliasEnv::new();
    check_inline_header(
        &val_of(named("Missing", vec![])),
        &[],
        &named("Missing", vec![]),
        val_span,
        inline_span,
        &plain,
    )
    .unwrap();
    assert!(
        check_inline_header(
            &val_of(named("Missing", vec![])),
            &[],
            &MiniTyp::Int,
            val_span,
            inline_span,
            &plain,
        )
        .is_err()
    );

    // Arity mismatch stays unexpanded: `Wrap` (1 param) vs `Wrap(8, 9)`.
    assert!(
        check_inline_header(
            &val_of(named("Wrap", vec![Nexp::Const(8)])),
            &[],
            &named("Wrap", vec![Nexp::Const(8), Nexp::Const(9)]),
            val_span,
            inline_span,
            &aliases,
        )
        .is_err()
    );

    // Cycles terminate: `A <-> B`; equal heads still match, distinct heads
    // mismatch without hanging.
    let mut cyclic = AliasEnv::new();
    cyclic.insert("A".to_string(), vec![], named("B", vec![]));
    cyclic.insert("B".to_string(), vec![], named("A", vec![]));
    check_inline_header(
        &val_of(named("A", vec![])),
        &[],
        &named("A", vec![]),
        val_span,
        inline_span,
        &cyclic,
    )
    .unwrap();
    assert!(
        check_inline_header(
            &val_of(named("A", vec![])),
            &[],
            &named("B", vec![]),
            val_span,
            inline_span,
            &cyclic,
        )
        .is_err()
    );

    // Exhausted budget leaves types unexpanded (still terminates).
    let t = named("Integer", vec![]);
    let mut aliases = AliasEnv::new();
    aliases.insert("Integer".to_string(), vec![], MiniTyp::Int);
    assert_eq!(aliases.expand_typ_limited(&t, 0), t);
    assert_eq!(aliases.expand_typ(&t), MiniTyp::Int);

    // Non-arrow `val` body and duplicate quantifiers are hard errors.
    let non_arrow = TypQuant::monomorphic(MiniTyp::Int);
    assert!(
        check_inline_header(
            &non_arrow,
            &[],
            &MiniTyp::Int,
            val_span,
            inline_span,
            &aliases,
        )
        .is_err()
    );
    assert!(
        check_inline_header(
            &val_of(MiniTyp::Int),
            &[("a".to_string(), Kind::KType)],
            &MiniTyp::Int,
            val_span,
            inline_span,
            &aliases,
        )
        .is_err()
    );
}

#[test]
fn inline_header_preserves_source_spans() {
    // Quellpositions-Sichtbarkeit: every inline diagnostic carries the
    // inline site; the `val` site is quoted `(declared here: ...)` in the
    // message (OCaml `Hint ("declared here", vs_l, l)`); the lowered
    // signature root carries the header span.
    use sail_ty::MiniPat;
    use sail_ty::check::{check_inline_header, synthesize_inline_scheme};
    use sail_ty::conv::{AliasEnv, inline_signature_to_ast};

    let val_span = Span::new(sail_ast::FileId(0), 10, 20);
    let inline_span = Span::new(sail_ast::FileId(0), 100, 114);
    assert!(!val_span.is_dummy() && !inline_span.is_dummy());
    let val = TypQuant::monomorphic(MiniTyp::Fun(
        Box::new(MiniTyp::Int),
        Box::new(MiniTyp::Int),
    ));

    let err = check_inline_header(
        &val,
        &[("a".to_string(), Kind::KType)],
        &MiniTyp::Int,
        val_span,
        inline_span,
        &AliasEnv::new(),
    )
    .unwrap_err();
    assert_eq!(err.span, inline_span);
    let msg = err.to_string();
    assert!(msg.contains("duplicate quantifier"), "{msg}");
    assert!(msg.contains("declared here"), "{msg}");
    assert!(msg.contains("file0:10-20"), "{msg}");

    let err = check_inline_header(
        &val,
        &[],
        &MiniTyp::Bool,
        val_span,
        inline_span,
        &AliasEnv::new(),
    )
    .unwrap_err();
    assert_eq!(err.span, inline_span);
    assert!(!err.span.is_dummy());
    let msg = err.to_string();
    assert!(msg.contains("do not match"), "{msg}");
    assert!(msg.contains("file0:10-20"), "{msg}");

    let err = synthesize_inline_scheme(
        vec![],
        NConstraint::True,
        &MiniPat::Var("x".into()),
        &MiniTyp::Int,
        inline_span,
    )
    .unwrap_err();
    assert_eq!(err.span, inline_span);

    let mut it = sail_ast::SimpleInterner::new();
    let sig = inline_signature_to_ast(&MiniTyp::Int, &MiniTyp::Int, &mut it, inline_span);
    assert_eq!(sig.span, inline_span);
    assert!(matches!(sig.kind, sail_ast::TypKind::Fn { .. }));
}


#[test]
fn lane10_stub_normalizes_identities_before_proving() {
    // Lane-10 canonical path: the stub normalizes pure arithmetic
    // identities before proving (OCaml `nexp_simp` parity), so
    // `n+0 == n` and `2^3 == 8` prove without a backend …
    use sail_ty::nexp::{normalize_constraint, normalize_nexp, solve_for_var};
    let s = StubSolver::new();
    let ident = NConstraint::Eq(
        Box::new(Nexp::Add(
            Box::new(Nexp::Var("n".into())),
            Box::new(Nexp::Const(0)),
        )),
        Box::new(Nexp::Var("n".into())),
    );
    assert_eq!(s.prove(&ident), ProveResult::Proved);
    assert_eq!(
        normalize_nexp(&Nexp::Exp(Box::new(Nexp::Const(3)))),
        Nexp::Const(8)
    );
    // … backward inference solves omitted implicits from equations …
    assert_eq!(
        solve_for_var(
            "x",
            &Nexp::Add(Box::new(Nexp::Var("x".into())), Box::new(Nexp::Const(1))),
            &Nexp::Const(5),
        ),
        Some(Nexp::Const(4))
    );
    // … and normalizing `true /\ (n == n)` folds fully to `True`,
    // keeping the goal provable …
    let conj = NConstraint::And(
        Box::new(NConstraint::True),
        Box::new(NConstraint::Eq(
            Box::new(Nexp::Var("n".into())),
            Box::new(Nexp::Var("n".into())),
        )),
    );
    assert_eq!(normalize_constraint(&conj), NConstraint::True);
    assert_eq!(s.prove(&conj), ProveResult::Proved);
    // … while open goals stay soundly `Unknown`.
    let open = NConstraint::Eq(
        Box::new(Nexp::Var("n".into())),
        Box::new(Nexp::Var("m".into())),
    );
    assert_eq!(s.prove(&open), ProveResult::Unknown);
}
