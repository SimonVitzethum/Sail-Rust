#![forbid(unsafe_code)]
//! `Solver` trait + stub + bounded caching wrapper.
//!
//! Lane-127-Wahrheit: Die kanonische Solver-Schnittstelle ist
//! `sail_solver::Solver` (`sail-solver`, Lane 06: `prove(&SolverQuery) ->
//! SolverVerdict`, `name()`, kanonische `Constraint`-Bruecke aus
//! `sail-nexp` mit Normalisierung, `MemoCache` mit TTL + LRU,
//! `SolverPool` mit 1-Solver-pro-Thread, optionalem Z3-Backend hinter
//! Feature `z3`). Dieser Modul-Trait ([`Solver`]: `prove(&NConstraint)
//! -> ProveResult`, `Send + Sync`) ist die Lane-05-lokale Sicht auf den
//! *gleichen* Vertrag ueber den lokalen Minimaltypen
//! (`crate::nexp::Nexp`/`NConstraint` mit `String`-Vars, siehe
//! `ARCHITECTURE.md`): gleiche Formel = gleiche Antwort, immer
//! terminierend, `Unknown` ist immer sound.
//!
//! Abbildung: `Proved <-> Valid`, `Refuted <-> Invalid`,
//! `Unknown <-> Unknown` (pinnt `tests/solver_parity.rs` als Code,
//! E3: ungeprueft). [`CachingSolver`] ist die Lane-05-lokale
//! Cache-Huelle (kapazitaetsbegrenzt, LRU); kanonisch ist
//! `sail_solver::{MemoCache, MemoSolver}` (zusaetzlich TTL,
//! poolweit geteilt, atomare Zaehler). Vollmigration (Checker generisch
//! ueber `sail_solver::Solver`, Minimaltypen -> `sail-nexp`) ist
//! Owner-Folgearbeit nach Lane 47 (Typvereinheitlichung) — bis dahin
//! wird hier nichts umgebaut.
//!
//! Lane 06 implements this trait with the real Z3 backend. Everything in
//! Lane 05 must work (and terminate) with [`StubSolver`]. Lane 107 adds the
//! real `NC_set`/`NC_app`/`NC_var`/`NC_id` prove paths through this same
//! trait (constant membership, singleton equations, opaque-bool soundness);
//! Z3 still plugs in only via [`Solver`] (rule `Z-TraitOnly`).

use crate::nexp::{NConstraint, Nexp};
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::Mutex;

/// Result of attempting to prove a numeric constraint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProveResult {
    /// Constraint holds in all models.
    Proved,
    /// Constraint is unsatisfiable (its negation holds).
    Refuted,
    /// Solver cannot decide (needs stronger backend / more fuel).
    Unknown,
}

impl ProveResult {
    #[must_use]
    pub fn is_proved(&self) -> bool {
        *self == Self::Proved
    }
}

/// Numeric-constraint solver interface (implemented by Lane 06 with Z3).
///
/// Must be `Send + Sync` so one solver (or pool) can be shared across the
/// multithreaded checking workers.
pub trait Solver: Send + Sync {
    /// Try to prove `nc`. Must always terminate (bounded work).
    fn prove(&self, nc: &NConstraint) -> ProveResult;
}

/// Stub solver for tests and for checking without Z3.
///
/// Decides only what is syntactically obvious:
/// - boolean constant folding (`True`/`False`, folded comparisons,
///   constant `Set` membership, empty `Set`),
/// - syntactic reflexivity (`n == n` is proved, `n != n` refuted,
///   singleton `Set(n, [v])` as `n == v`),
/// - uninterpreted predicates (`App`) and opaque booleans (`Var`/`Id`)
///   stay [`ProveResult::Unknown`] (sound: they never fold, they only
///   propagate through `And`/`Or`/`Not` like any other open formula),
/// - everything else is [`ProveResult::Unknown`].
///
/// Always terminates in O(tree size) time.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubSolver;

impl StubSolver {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Solver for StubSolver {
    fn prove(&self, nc: &NConstraint) -> ProveResult {
        // Fast path: constant folding.
        if let Some(b) = nc.as_bool() {
            return if b {
                ProveResult::Proved
            } else {
                ProveResult::Refuted
            };
        }
        // Syntactic reflexivity for non-foldable sides.
        match nc {
            NConstraint::Eq(a, b) if a == b => ProveResult::Proved,
            NConstraint::Neq(a, b) if a == b => ProveResult::Refuted,
            NConstraint::Ge(a, b) | NConstraint::Le(a, b) if a == b => ProveResult::Proved,
            NConstraint::Gt(a, b) | NConstraint::Lt(a, b) if a == b => ProveResult::Refuted,
            // `And`/`Or`/`Not` combinations over stub-decidable parts.
            NConstraint::And(a, b) => match (self.prove(a), self.prove(b)) {
                (ProveResult::Proved, ProveResult::Proved) => ProveResult::Proved,
                (ProveResult::Refuted, _) | (_, ProveResult::Refuted) => ProveResult::Refuted,
                _ => ProveResult::Unknown,
            },
            NConstraint::Or(a, b) => match (self.prove(a), self.prove(b)) {
                (ProveResult::Proved, _) | (_, ProveResult::Proved) => ProveResult::Proved,
                (ProveResult::Refuted, ProveResult::Refuted) => ProveResult::Refuted,
                _ => ProveResult::Unknown,
            },
            NConstraint::Not(a) => match self.prove(a) {
                ProveResult::Proved => ProveResult::Refuted,
                ProveResult::Refuted => ProveResult::Proved,
                ProveResult::Unknown => ProveResult::Unknown,
            },
            // `NC_set`: a singleton is that equation (recurses into the
            // reflexivity/folding paths above); anything else non-foldable
            // is open. (Constants and `[]` never reach here: `as_bool`
            // folds them on the fast path.)
            NConstraint::Set(n, vs) if vs.len() == 1 => {
                self.prove(&NConstraint::Eq(n.clone(), Box::new(Nexp::Const(vs[0]))))
            }
            // `NC_app`/`NC_var`/`NC_id`: opaque — sound `Unknown`, never a
            // fabricated proof (stronger backends decide via the trait).
            NConstraint::Set(_, _) | NConstraint::App(_, _) | NConstraint::Var(_) | NConstraint::Id(_) => {
                ProveResult::Unknown
            }
            _ => ProveResult::Unknown,
        }
    }
}

/// Cache statistics for [`CachingSolver`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

/// Bounded LRU cache in front of any [`Solver`].
///
/// This is the caching interface Lane 06 reuses; here it wraps the stub or
/// any future backend. Capacity-bounded (default 1024) so RAM stays flat;
/// `Mutex`-guarded for sharing across threads (one entry per solver, no
/// cross-thread solver sharing needed).
pub struct CachingSolver<S: Solver> {
    inner: S,
    capacity: usize,
    state: Mutex<CacheState>,
}

struct CacheState {
    map: HashMap<NConstraint, ProveResult>,
    order: VecDeque<NConstraint>,
    stats: CacheStats,
}

impl<S: Solver> CachingSolver<S> {
    pub fn new(inner: S, capacity: usize) -> Self {
        Self {
            inner,
            capacity: capacity.max(1),
            state: Mutex::new(CacheState {
                map: HashMap::new(),
                order: VecDeque::new(),
                stats: CacheStats::default(),
            }),
        }
    }

    #[must_use]
    pub fn stats(&self) -> CacheStats {
        self.state
            .lock()
            .map(|s| s.stats)
            .unwrap_or_default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.state.lock().map(|s| s.map.len()).unwrap_or(0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<S: Solver> Solver for CachingSolver<S> {
    fn prove(&self, nc: &NConstraint) -> ProveResult {
        // Fast path under lock.
        if let Ok(mut state) = self.state.lock() {
            if let Some(r) = state.map.get(nc).copied() {
                state.stats.hits += 1;
                // True LRU recency: the hit is promoted to the back so
                // hot entries survive eviction. Without this the queue
                // was FIFO — frequently proved constraints were evicted
                // first while cold entries stayed resident.
                if let Some(pos) = state.order.iter().position(|k| k == nc) {
                    state.order.remove(pos);
                }
                state.order.push_back(nc.clone());
                return r;
            }
            state.stats.misses += 1;
            // Compute outside mutation but still under lock is fine: inner
            // solvers here are pure/total. Lane 06 keeps per-thread solvers
            // so no contention pathology.
            let result = self.inner.prove(nc);
            if state.map.len() >= self.capacity {
                if let Some(old) = state.order.pop_front() {
                    state.map.remove(&old);
                    state.stats.evictions += 1;
                }
            }
            state.map.insert(nc.clone(), result);
            state.order.push_back(nc.clone());
            result
        } else {
            // Poisoned lock: fall back to uncached prove (still terminates).
            self.inner.prove(nc)
        }
    }
}

// `Mutex<CacheState>` is `Send + Sync` given `NConstraint: Send`; assert it.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<CachingSolver<StubSolver>>();
};
