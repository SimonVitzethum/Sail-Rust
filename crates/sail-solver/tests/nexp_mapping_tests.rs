#![forbid(unsafe_code)]
//! Lane-80-Tests (Code, E3: nicht ausfuehren): `from_nconstraint`-Bruecke
//! (kanonische Keys), Bounded-LRU-Pflicht, Stub-/Pool-Verhalten.

use sail_nexp::{NConstraint, NExp, TypArg};
use sail_solver::{
    Constraint, MemoSolver, Solver, SolverCacheConfig, SolverPool, SolverQuery, SolverVerdict,
    StubSolver,
};
use std::time::Duration;

fn v(n: &str) -> NExp {
    NExp::var(n)
}
fn c(i: i128) -> NExp {
    NExp::Constant(i)
}
fn ge(n: &str, bound: i128) -> NConstraint {
    NConstraint::Ge(Box::new(v(n)), Box::new(c(bound)))
}
fn lt(n: &str, bound: i128) -> NConstraint {
    NConstraint::Lt(Box::new(v(n)), Box::new(c(bound)))
}

#[test]
fn from_nconstraint_maps_leaves_to_stable_atoms() {
    assert_eq!(Constraint::from_nconstraint(&NConstraint::True), Constraint::True);
    assert_eq!(Constraint::from_nconstraint(&NConstraint::False), Constraint::False);
    assert_eq!(
        Constraint::from_nconstraint(&ge("n", 0)),
        Constraint::atom("(>= (var n) (const 0))")
    );
    assert_eq!(
        Constraint::from_nconstraint(&NConstraint::Equal(
            Box::new(TypArg::NExp(v("n"))),
            Box::new(TypArg::NExp(c(2)))
        )),
        Constraint::atom("(= (var n) (const 2))")
    );
    assert_eq!(
        Constraint::from_nconstraint(&NConstraint::NotEqual(
            Box::new(TypArg::NExp(v("n"))),
            Box::new(TypArg::NExp(c(2)))
        )),
        Constraint::atom("(!= (var n) (const 2))")
    );
    assert_eq!(
        Constraint::from_nconstraint(&NConstraint::Set(Box::new(v("n")), vec![3, 1, 3, 1])),
        Constraint::atom("(in (var n) 1 3)")
    );
    assert_eq!(
        Constraint::from_nconstraint(&NConstraint::App(
            "ok".to_string(),
            vec![TypArg::NExp(v("n"))]
        )),
        Constraint::atom("(pred ok (var n))")
    );
    assert_eq!(
        Constraint::from_nconstraint(&NConstraint::Var("b".to_string())),
        Constraint::atom("(bvar b)")
    );
    assert_eq!(
        Constraint::from_nconstraint(&NConstraint::Id("p".to_string())),
        Constraint::atom("(bid p)")
    );
}

#[test]
fn from_nconstraint_is_canonical_for_ac_operands() {
    // `And`-Reihenfolge und `Sum`-Reihenfolge aendern den Key nicht.
    let ab = NConstraint::And(Box::new(ge("n", 0)), Box::new(lt("m", 0)));
    let ba = NConstraint::And(Box::new(lt("m", 0)), Box::new(ge("n", 0)));
    assert_eq!(
        Constraint::from_nconstraint(&ab).cache_key(),
        Constraint::from_nconstraint(&ba).cache_key()
    );
    let s1 = NExp::Sum(Box::new(v("b")), Box::new(v("a")));
    let s2 = NExp::Sum(Box::new(v("a")), Box::new(v("b")));
    let c1 = NConstraint::Ge(Box::new(s1), Box::new(c(0)));
    let c2 = NConstraint::Ge(Box::new(s2), Box::new(c(0)));
    assert_eq!(
        Constraint::from_nconstraint(&c1).cache_key(),
        Constraint::from_nconstraint(&c2).cache_key()
    );
    // Struktur bleibt erhalten (kein Atom-Brei).
    let nested = NConstraint::Not(Box::new(ab));
    match Constraint::from_nconstraint(&nested) {
        Constraint::Not(_) => {}
        other => panic!("expected Not, got {other:?}"),
    }
}

#[test]
fn from_nconstraint_covers_all_nexp_forms() {
    // Jede NExp-Form ergibt einen stabilen (nicht-leeren) Key.
    let forms = [
        NExp::Id("f".to_string()),
        v("n"),
        c(-7),
        NExp::div(v("n"), c(2)),
        NExp::modulo(v("n"), c(2)),
        NExp::abs(v("n")),
        NExp::App("uf".to_string(), vec![v("n"), c(1)]),
        NExp::If(
            Box::new(NConstraint::True),
            Box::new(c(1)),
            Box::new(c(2)),
        ),
        NExp::Times(Box::new(v("n")), Box::new(c(2))),
        NExp::Minus(Box::new(v("n")), Box::new(c(2))),
        NExp::Exp(Box::new(c(3))),
        NExp::Neg(Box::new(v("n"))),
    ];
    for e in &forms {
        let key =
            Constraint::from_nconstraint(&NConstraint::Ge(Box::new(e.clone()), Box::new(c(0))))
                .cache_key();
        assert!(!key.is_empty(), "empty key for {e:?}");
        // Deterministisch: zweimal derselbe Key.
        let again =
            Constraint::from_nconstraint(&NConstraint::Ge(Box::new(e.clone()), Box::new(c(0))))
                .cache_key();
        assert_eq!(key, again);
    }
}

#[test]
fn bounded_lru_is_mandatory() {
    // capacity 0 wird auf 1 geklemmt (nie unbegrenzt, nie 0).
    let tiny = MemoSolver::new(
        StubSolver::new(),
        SolverCacheConfig {
            capacity: 0,
            ttl: Duration::from_secs(60),
        },
    );
    assert_eq!(tiny.stats().capacity, 1);
    // LRU-Eviction begrenzt die Eintraege.
    let solver = MemoSolver::new(
        StubSolver::new(),
        SolverCacheConfig {
            capacity: 2,
            ttl: Duration::from_secs(60),
        },
    );
    let q = |s: &str| SolverQuery::new(Constraint::from_nconstraint(&ge(s, 0)));
    assert_eq!(solver.prove(&q("a")), SolverVerdict::Unknown);
    assert_eq!(solver.prove(&q("a")), SolverVerdict::Unknown); // hit
    assert_eq!(solver.prove(&q("b")), SolverVerdict::Unknown);
    assert_eq!(solver.prove(&q("c")), SolverVerdict::Unknown); // evict a
    let s = solver.stats();
    assert_eq!(s.len, 2);
    assert_eq!((s.hits, s.misses), (1, 3));
    // Memo greift auch fuer strukturierte NConstraint-Queries.
    let valid = SolverQuery::new(Constraint::from_nconstraint(&NConstraint::True));
    assert_eq!(solver.prove(&valid), SolverVerdict::Valid);
    assert_eq!(solver.prove(&valid), SolverVerdict::Valid);
    assert_eq!(solver.stats().hits, 2);
}

#[test]
fn stub_pool_and_query_basics() {
    let pool = SolverPool::with_default_cache(StubSolver::new);
    let s = pool.checkout();
    assert_eq!(s.name(), "stub");
    // Konstanten faltet der Stub, Atome bleiben Unknown (terminiert immer).
    assert_eq!(
        s.prove(&SolverQuery::new(Constraint::True)),
        SolverVerdict::Valid
    );
    assert!(SolverVerdict::Valid.is_valid());
    assert!(!SolverVerdict::Unknown.is_valid());
    assert_eq!(s.query_count(), 1);
    drop(s);
    assert_eq!(pool.idle(), 1);
    // Timeout geht in den Cache-Key ein.
    let a = SolverQuery::new(Constraint::atom("x")).with_timeout(10);
    let b = SolverQuery::new(Constraint::atom("x")).with_timeout(20);
    assert_ne!(a.cache_key(), b.cache_key());
    // Display == Cache-Key.
    assert_eq!(a.to_string(), a.cache_key());
}
