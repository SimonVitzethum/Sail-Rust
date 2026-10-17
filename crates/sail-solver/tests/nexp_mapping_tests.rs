#![forbid(unsafe_code)]
//! Lane-80-Tests (Code, E3: nicht ausfuehren): `from_nconstraint`-Bruecke
//! (kanonische Keys), Bounded-LRU-Pflicht, Stub-/Pool-Verhalten.

use sail_ast::{Interner as _, SimpleInterner};
use sail_nexp::{Kid, NConstraint, NExp, TypArg};
use sail_solver::{
    Constraint, MemoSolver, Solver, SolverCacheConfig, SolverPool, SolverQuery, SolverVerdict,
    StubSolver,
};
use std::time::Duration;

fn v(k: Kid) -> NExp {
    NExp::var(k)
}
fn c(i: i128) -> NExp {
    NExp::Constant(i)
}
fn ge(k: Kid, bound: i128) -> NConstraint {
    NConstraint::Ge(Box::new(v(k)), Box::new(c(bound)))
}
fn lt(k: Kid, bound: i128) -> NConstraint {
    NConstraint::Lt(Box::new(v(k)), Box::new(c(bound)))
}

#[test]
fn from_nconstraint_maps_leaves_to_stable_atoms() {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    let b = ix.intern("b");
    assert_eq!(
        Constraint::from_nconstraint(&NConstraint::True, &ix),
        Constraint::True
    );
    assert_eq!(
        Constraint::from_nconstraint(&NConstraint::False, &ix),
        Constraint::False
    );
    assert_eq!(
        Constraint::from_nconstraint(&ge(n, 0), &ix),
        Constraint::atom("(>= (var n) (const 0))")
    );
    assert_eq!(
        Constraint::from_nconstraint(
            &NConstraint::Equal(Box::new(TypArg::NExp(v(n))), Box::new(TypArg::NExp(c(2)))),
            &ix
        ),
        Constraint::atom("(= (var n) (const 2))")
    );
    assert_eq!(
        Constraint::from_nconstraint(
            &NConstraint::NotEqual(
                Box::new(TypArg::NExp(v(n))),
                Box::new(TypArg::NExp(c(2)))
            ),
            &ix
        ),
        Constraint::atom("(!= (var n) (const 2))")
    );
    assert_eq!(
        Constraint::from_nconstraint(
            &NConstraint::Set(Box::new(v(n)), vec![3, 1, 3, 1]),
            &ix
        ),
        Constraint::atom("(in (var n) 1 3)")
    );
    assert_eq!(
        Constraint::from_nconstraint(
            &NConstraint::App("ok".to_string(), vec![TypArg::NExp(v(n))]),
            &ix
        ),
        Constraint::atom("(pred ok (var n))")
    );
    assert_eq!(
        Constraint::from_nconstraint(&NConstraint::Var(b), &ix),
        Constraint::atom("(bvar b)")
    );
    assert_eq!(
        Constraint::from_nconstraint(&NConstraint::Id("p".to_string()), &ix),
        Constraint::atom("(bid p)")
    );
}

#[test]
fn keys_use_names_and_separate_id_var_namespaces() {
    // Lane 124: gleiche Schreibweise als `Id` vs. `Var` gibt
    // verschiedene Keys (keine Id/Var-Verwechslung); gleiche Namen aus
    // verschieden befuellten Tabellen geben gleiche Keys.
    let mut ix = SimpleInterner::new();
    let f = ix.intern("f");
    let id_key = Constraint::from_nconstraint(
        &NConstraint::Ge(Box::new(NExp::Id("f".to_string())), Box::new(c(0))),
        &ix,
    )
    .cache_key();
    let var_key = Constraint::from_nconstraint(
        &NConstraint::Ge(Box::new(v(f)), Box::new(c(0))),
        &ix,
    )
    .cache_key();
    assert_eq!(id_key, "(atom (>= (id f) (const 0)))");
    assert_eq!(var_key, "(atom (>= (var f) (const 0)))");
    assert_ne!(id_key, var_key);
    // Gleiche Formel, andere Tabelle (andere `Symbol`-Ids): gleicher Key.
    let mut other = SimpleInterner::new();
    other.intern("other");
    let g = other.intern("f");
    assert_ne!(f, g);
    let var_key2 = Constraint::from_nconstraint(
        &NConstraint::Ge(Box::new(v(g)), Box::new(c(0))),
        &other,
    )
    .cache_key();
    assert_eq!(var_key, var_key2);
}

#[test]
fn from_nconstraint_is_canonical_for_ac_operands() {
    // `And`-Reihenfolge und `Sum`-Reihenfolge aendern den Key nicht.
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    let m = ix.intern("m");
    let b = ix.intern("b");
    let a = ix.intern("a");
    let ab = NConstraint::And(Box::new(ge(n, 0)), Box::new(lt(m, 0)));
    let ba = NConstraint::And(Box::new(lt(m, 0)), Box::new(ge(n, 0)));
    assert_eq!(
        Constraint::from_nconstraint(&ab, &ix).cache_key(),
        Constraint::from_nconstraint(&ba, &ix).cache_key()
    );
    let s1 = NExp::Sum(Box::new(v(b)), Box::new(v(a)));
    let s2 = NExp::Sum(Box::new(v(a)), Box::new(v(b)));
    let c1 = NConstraint::Ge(Box::new(s1), Box::new(c(0)));
    let c2 = NConstraint::Ge(Box::new(s2), Box::new(c(0)));
    assert_eq!(
        Constraint::from_nconstraint(&c1, &ix).cache_key(),
        Constraint::from_nconstraint(&c2, &ix).cache_key()
    );
    // Struktur bleibt erhalten (kein Atom-Brei).
    let nested = NConstraint::Not(Box::new(ab));
    match Constraint::from_nconstraint(&nested, &ix) {
        Constraint::Not(_) => {}
        other => panic!("expected Not, got {other:?}"),
    }
}

#[test]
fn from_nconstraint_covers_all_nexp_forms() {
    // Jede NExp-Form ergibt einen stabilen (nicht-leeren) Key.
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    let forms = [
        NExp::Id("f".to_string()),
        v(n),
        c(-7),
        NExp::div(v(n), c(2)),
        NExp::modulo(v(n), c(2)),
        NExp::abs(v(n)),
        NExp::App("uf".to_string(), vec![v(n), c(1)]),
        NExp::If(
            Box::new(NConstraint::True),
            Box::new(c(1)),
            Box::new(c(2)),
        ),
        NExp::Times(Box::new(v(n)), Box::new(c(2))),
        NExp::Minus(Box::new(v(n)), Box::new(c(2))),
        NExp::Exp(Box::new(c(3))),
        NExp::Neg(Box::new(v(n))),
    ];
    for e in &forms {
        let key =
            Constraint::from_nconstraint(&NConstraint::Ge(Box::new(e.clone()), Box::new(c(0))), &ix)
                .cache_key();
        assert!(!key.is_empty(), "empty key for {e:?}");
        // Deterministisch: zweimal derselbe Key.
        let again =
            Constraint::from_nconstraint(&NConstraint::Ge(Box::new(e.clone()), Box::new(c(0))), &ix)
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
    let mut ix = SimpleInterner::new();
    let ka = ix.intern("a");
    let kb = ix.intern("b");
    let kc = ix.intern("c");
    let q = |k: Kid| SolverQuery::new(Constraint::from_nconstraint(&ge(k, 0), &ix));
    // Eviction ist Cache-Eigenschaft (direkt abgelegt, nicht ueber
    // Solver-Verdikte, die seit dem Unknown-Fix nur definitiv
    // gecacht werden).
    for k in [ka, kb, kc] {
        solver.cache().store(&q(k), SolverVerdict::Valid);
    }
    let s = solver.stats();
    assert_eq!(s.len, 2);
    assert_eq!(solver.prove(&q(kb)), SolverVerdict::Valid); // hit
    assert_eq!(solver.prove(&q(kc)), SolverVerdict::Valid); // hit
    let s = solver.stats();
    assert_eq!((s.hits, s.misses), (2, 0));
    // Memo greift auch fuer strukturierte NConstraint-Queries.
    let valid = SolverQuery::new(Constraint::from_nconstraint(&NConstraint::True, &ix));
    assert_eq!(solver.prove(&valid), SolverVerdict::Valid);
    assert_eq!(solver.prove(&valid), SolverVerdict::Valid);
    assert_eq!(solver.stats().hits, 3);
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
    // Display == Cache-Key (SolverQuery hat kein eigenes Display;
    // ohne Timeout faellt der Query-Key mit dem Constraint-Key zusammen,
    // dessen Display der Cache-Key ist).
    let plain = SolverQuery::new(Constraint::atom("x"));
    assert_eq!(plain.constraint.to_string(), plain.cache_key());
}
