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

use crate::nexp::{NConstraint, Nexp, linear_form};
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
/// Decides only what is syntactically obvious, after Lane-10
/// canonical-path normalization ([`crate::nexp::normalize_constraint`]):
/// - boolean constant folding (`True`/`False`, folded comparisons,
///   constant `Set` membership, empty `Set`),
/// - ground SMT powers (`2^c` with constant `c` fold; symbolic powers
///   stay [`ProveResult::Unknown`] — the canonical SMT emitter needs
///   elaboration for those, see `sail-solver/src/canonical.rs`),
/// - syntactic reflexivity (`n == n` is proved, `n != n` refuted,
///   singleton `Set(n, [v])` as `n == v`),
/// - general numeric/constraint applications (`Nexp::Div`/`Mod`/`Abs`
///   fold when ground; uninterpreted `App` predicates) and opaque
///   booleans (`Var`/`Id`) stay [`ProveResult::Unknown`] (sound: they
///   never fold, they only propagate through `And`/`Or`/`Not` like any
///   other open formula; stronger backends decide via the trait),
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
        // Canonical path: normalize identities/ground folds first so
        // `Eq`-reflexivity fires on `n+0 == n`-style goals (OCaml
        // `nexp_simp`/`constraint_simp`); symbolic powers and general
        // apps survive normalization and stay soundly `Unknown`.
        let owned = crate::nexp::normalize_constraint(nc);
        self.prove_normalized(&owned)
    }
}

impl StubSolver {
    fn prove_normalized(&self, nc: &NConstraint) -> ProveResult {
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
            NConstraint::And(a, b) => match (self.prove_normalized(a), self.prove_normalized(b)) {
                (ProveResult::Proved, ProveResult::Proved) => ProveResult::Proved,
                (ProveResult::Refuted, _) | (_, ProveResult::Refuted) => ProveResult::Refuted,
                _ => ProveResult::Unknown,
            },
            NConstraint::Or(a, b) => match (self.prove_normalized(a), self.prove_normalized(b)) {
                (ProveResult::Proved, _) | (_, ProveResult::Proved) => ProveResult::Proved,
                (ProveResult::Refuted, ProveResult::Refuted) => ProveResult::Refuted,
                _ => ProveResult::Unknown,
            },
            NConstraint::Not(a) => match self.prove_normalized(a) {
                ProveResult::Proved => ProveResult::Refuted,
                ProveResult::Refuted => ProveResult::Proved,
                ProveResult::Unknown => ProveResult::Unknown,
            },
            // `NC_set`: a singleton is that equation (recurses into the
            // reflexivity/folding paths above); anything else non-foldable
            // is open. (Constants and `[]` never reach here: `as_bool`
            // folds them on the fast path.)
            NConstraint::Set(n, vs) if vs.len() == 1 => {
                self.prove_normalized(&NConstraint::Eq(n.clone(), Box::new(Nexp::Const(vs[0]))))
            }
            // Symbolic SMT powers (`2^n`), general numeric applications
            // (`Div`/`Mod`/`Abs` beyond ground folds) and general
            // constraint applications / opaque booleans (`App`/`Var`/`Id`):
            // sound `Unknown`, never a fabricated proof (the canonical SMT
            // emitter likewise needs elaboration/synonym expansion for
            // these; stronger backends decide via the trait).
            NConstraint::Set(_, _) | NConstraint::App(_, _) | NConstraint::Var(_) | NConstraint::Id(_) => {
                ProveResult::Unknown
            }
            _ => ProveResult::Unknown,
        }
    }
}

/// Exact-linear normalizing layer over any [`Solver`] (Lane 20, Audit O-6).
///
/// Falls back to normalization only when the inner solver answers
/// [`ProveResult::Unknown`], so it never weakens a backend verdict —
/// it only closes goals the inner solver leaves open:
/// - the [`NConstraint::simplify`] normal form (integer identities such as
///   `n + 0 == n`, `True`/`False` elimination, double negation),
/// - exact linear integer arithmetic: a comparison whose normalized
///   difference `lhs - rhs` is constant is decided by the constant's sign
///   (`'n + 1 > 'n`, `2 * 'n == 'n + 'n`, `'n + 1 == 'n` refuted — all
///   through checked `i128` arithmetic, overflow stays `Unknown`).
///
/// Sound-`Unknown` catalogue (never guessed, pinned by
/// `tests/solver_parity.rs`): symbolic powers (`2 ^ 'n`, which need
/// elaboration before any SMT proof), nonlinear multiplication
/// (`'n * 'm`), symbolic division/modulo, undecided `if`/absolute value,
/// general constraint applications, opaque booleans, and comparisons with
/// a genuinely open (non-constant) linear difference such as `'n > 0`.
///
/// Total like the inner solver (normalization is one structural pass;
/// the linear fallback is O(tree size)); `Send + Sync` whenever the
/// inner solver is.
pub struct NormalizingSolver<S: Solver> {
    inner: S,
}

impl<S: Solver> NormalizingSolver<S> {
    /// Wrap `inner`: its verdicts win; normalization only fills `Unknown`.
    pub fn new(inner: S) -> Self {
        Self { inner }
    }

    /// The wrapped solver.
    #[must_use]
    pub fn inner(&self) -> &S {
        &self.inner
    }
}

impl<S: Solver> Solver for NormalizingSolver<S> {
    fn prove(&self, nc: &NConstraint) -> ProveResult {
        match self.inner.prove(nc) {
            ProveResult::Unknown => Self::prove_normalized(nc),
            decided => decided,
        }
    }
}

impl<S: Solver> NormalizingSolver<S> {
    /// Decide `nc` by its exact normalized difference.
    ///
    /// Normalizes first (identities, boolean simplifications, constant
    /// folding), then proves (or refutes) comparisons with a constant
    /// normalized difference: integer linear arithmetic is exact, so the
    /// constant's sign decides soundly — a constant difference is
    /// decidable both ways (`holds` proves, `!holds` refutes; no third
    /// case exists over the integers). Open differences (a variable
    /// survives), nonlinear skeletons, and checked-arithmetic overflow all
    /// yield [`ProveResult::Unknown`].
    fn prove_normalized(nc: &NConstraint) -> ProveResult {
        // Normalize: identities (`n + 0`), boolean simplifications, and
        // constant folding turn proved/refuted goals into `True`/`False`
        // or reflexive form.
        let nc = nc.simplify();
        if let Some(b) = nc.as_bool() {
            return if b { ProveResult::Proved } else { ProveResult::Refuted };
        }
        match nc {
            NConstraint::Eq(a, b) if a == b => ProveResult::Proved,
            NConstraint::Neq(a, b) if a == b => ProveResult::Refuted,
            NConstraint::Ge(a, b) | NConstraint::Le(a, b) if a == b => ProveResult::Proved,
            NConstraint::Gt(a, b) | NConstraint::Lt(a, b) if a == b => ProveResult::Refuted,
            NConstraint::Eq(a, b) => Self::prove_linear(&a, &b, LinearOp::Eq),
            NConstraint::Neq(a, b) => Self::prove_linear(&a, &b, LinearOp::Ne),
            NConstraint::Ge(a, b) => Self::prove_linear(&a, &b, LinearOp::Ge),
            NConstraint::Gt(a, b) => Self::prove_linear(&a, &b, LinearOp::Gt),
            NConstraint::Le(a, b) => Self::prove_linear(&a, &b, LinearOp::Le),
            NConstraint::Lt(a, b) => Self::prove_linear(&a, &b, LinearOp::Lt),
            // Boolean structure composes through the same normalized
            // fallback (the inner solver said `Unknown` on the whole
            // formula, but parts may decide exactly).
            NConstraint::And(a, b) => match (Self::prove_normalized(&a), Self::prove_normalized(&b)) {
                (ProveResult::Proved, ProveResult::Proved) => ProveResult::Proved,
                (ProveResult::Refuted, _) | (_, ProveResult::Refuted) => ProveResult::Refuted,
                _ => ProveResult::Unknown,
            },
            NConstraint::Or(a, b) => match (Self::prove_normalized(&a), Self::prove_normalized(&b)) {
                (ProveResult::Proved, _) | (_, ProveResult::Proved) => ProveResult::Proved,
                (ProveResult::Refuted, ProveResult::Refuted) => ProveResult::Refuted,
                _ => ProveResult::Unknown,
            },
            NConstraint::Not(a) => match Self::prove_normalized(&a) {
                ProveResult::Proved => ProveResult::Refuted,
                ProveResult::Refuted => ProveResult::Proved,
                ProveResult::Unknown => ProveResult::Unknown,
            },
            // Singleton `Set` is its equation; everything else
            // (`True`/`False` are folded above; opaque `Set`/`App`/`Var`/`Id`)
            // has no linear difference to decide by — stays open.
            NConstraint::Set(n, vs) if vs.len() == 1 => Self::prove_normalized(
                &NConstraint::Eq(n, Box::new(Nexp::Const(vs[0]))),
            ),
            _ => ProveResult::Unknown,
        }
    }

    fn prove_linear(a: &Nexp, b: &Nexp, op: LinearOp) -> ProveResult {
        let diff = Nexp::Sub(Box::new(a.clone()), Box::new(b.clone()));
        let form = match linear_form(&diff) {
            Some(form) if form.is_constant() => form,
            _ => return ProveResult::Unknown,
        };
        let c = form.constant;
        let holds = match op {
            LinearOp::Eq => c == 0,
            LinearOp::Ne => c != 0,
            LinearOp::Ge => c >= 0,
            LinearOp::Gt => c > 0,
            LinearOp::Le => c <= 0,
            LinearOp::Lt => c < 0,
        };
        if holds { ProveResult::Proved } else { ProveResult::Refuted }
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

/// Comparison operators for [`NormalizingSolver::prove_linear`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinearOp {
    Eq,
    Ne,
    Ge,
    Gt,
    Le,
    Lt,
}

// `Mutex<CacheState>` is `Send + Sync` given `NConstraint: Send`; assert it.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<CachingSolver<StubSolver>>();
    assert_send_sync::<NormalizingSolver<StubSolver>>();
};

#[cfg(test)]
mod tests {
    use super::*;

    fn var(v: &str) -> Nexp {
        Nexp::Var(v.to_string())
    }

    fn const_(c: i128) -> Nexp {
        Nexp::Const(c)
    }

    fn cmp(
        mk: fn(Box<Nexp>, Box<Nexp>) -> NConstraint,
        lhs: Nexp,
        rhs: Nexp,
    ) -> NConstraint {
        mk(Box::new(lhs), Box::new(rhs))
    }

    #[test]
    fn normalizing_proves_exact_linear_comparisons() {
        let s = NormalizingSolver::new(StubSolver::new());
        let n = || var("n");
        // Tautologies with constant normalized difference.
        assert_eq!(
            s.prove(&cmp(NConstraint::Gt, Nexp::Add(Box::new(n()), Box::new(const_(1))), n())),
            ProveResult::Proved
        );
        assert_eq!(
            s.prove(&cmp(NConstraint::Ge, n(), n())),
            ProveResult::Proved
        );
        assert_eq!(
            s.prove(&cmp(
                NConstraint::Eq,
                Nexp::Mul(Box::new(const_(2)), Box::new(n())),
                Nexp::Add(Box::new(n()), Box::new(n()))
            )),
            ProveResult::Proved
        );
        // Normalization fires first: `n + 0 == n` is reflexive after simplify.
        assert_eq!(
            s.prove(&cmp(
                NConstraint::Eq,
                Nexp::Add(Box::new(n()), Box::new(const_(0))),
                n()
            )),
            ProveResult::Proved
        );
        // Constant nonzero differences refute.
        assert_eq!(
            s.prove(&cmp(
                NConstraint::Eq,
                Nexp::Add(Box::new(n()), Box::new(const_(1))),
                n()
            )),
            ProveResult::Refuted
        );
        assert_eq!(
            s.prove(&cmp(NConstraint::Lt, n(), n())),
            ProveResult::Refuted
        );
        // `!=` decides dually.
        assert_eq!(
            s.prove(&cmp(
                NConstraint::Neq,
                Nexp::Add(Box::new(n()), Box::new(const_(1))),
                n()
            )),
            ProveResult::Proved
        );
    }

    #[test]
    fn normalizing_leaves_open_differences_and_nonlinear_shapes_unknown() {
        let s = NormalizingSolver::new(StubSolver::new());
        let n = || var("n");
        // Genuinely open: the difference still mentions `'n`.
        assert_eq!(
            s.prove(&cmp(NConstraint::Gt, n(), const_(0))),
            ProveResult::Unknown
        );
        assert_eq!(
            s.prove(&cmp(NConstraint::Le, n(), const_(10))),
            ProveResult::Unknown
        );
        // Symbolic powers: valid goals like `2 ^ 'n >= 1` stay open —
        // powers need elaboration before any proof.
        assert_eq!(
            s.prove(&cmp(
                NConstraint::Eq,
                Nexp::Exp(Box::new(n())),
                const_(8)
            )),
            ProveResult::Unknown
        );
        assert_eq!(
            s.prove(&cmp(NConstraint::Ge, Nexp::Exp(Box::new(n())), const_(1))),
            ProveResult::Unknown
        );
        // Nonlinear multiplication and symbolic division stay open.
        assert_eq!(
            s.prove(&cmp(
                NConstraint::Eq,
                Nexp::Mul(Box::new(n()), Box::new(var("m"))),
                Nexp::Mul(Box::new(var("m")), Box::new(n()))
            )),
            ProveResult::Unknown
        );
        assert_eq!(
            s.prove(&cmp(
                NConstraint::Eq,
                Nexp::Mul(
                    Box::new(Nexp::Div(Box::new(n()), Box::new(const_(2)))),
                    Box::new(const_(2))
                ),
                n()
            )),
            ProveResult::Unknown
        );
        // Opaque predicates and booleans stay open (never fabricated).
        assert_eq!(
            s.prove(&NConstraint::App("p".to_string(), vec![n()])),
            ProveResult::Unknown
        );
        assert_eq!(
            s.prove(&NConstraint::Var("b".to_string())),
            ProveResult::Unknown
        );
        assert_eq!(
            s.prove(&NConstraint::Id("p".to_string())),
            ProveResult::Unknown
        );
    }

    #[test]
    fn normalizing_power_composes_through_boolean_structure() {
        let s = NormalizingSolver::new(StubSolver::new());
        let n = || var("n");
        // `(n + 1 > n) & true` proves; `(n + 1 == n) | false` refutes.
        let taut = cmp(
            NConstraint::Gt,
            Nexp::Add(Box::new(n()), Box::new(const_(1))),
            n(),
        );
        assert_eq!(
            s.prove(&NConstraint::And(Box::new(taut.clone()), Box::new(NConstraint::True))),
            ProveResult::Proved
        );
        let contra = cmp(
            NConstraint::Eq,
            Nexp::Add(Box::new(n()), Box::new(const_(1))),
            n(),
        );
        assert_eq!(
            s.prove(&NConstraint::Or(Box::new(contra), Box::new(NConstraint::False))),
            ProveResult::Refuted
        );
        // Mixed open/closed conjunctions stay open.
        assert_eq!(
            s.prove(&NConstraint::And(
                Box::new(taut),
                Box::new(cmp(NConstraint::Gt, n(), const_(0)))
            )),
            ProveResult::Unknown
        );
    }
}

#[cfg(test)]
mod layering_tests {
    use super::*;

    /// Schicht-Trennung (Audit O-6, Lane-10-Vertrag): der `StubSolver`
    /// normalisiert Identitäten selbst (`n + 0 == n` ist dort `Proved`);
    /// `NormalizingSolver` schließt darüber hinaus die exakte lineare
    /// Lücke, ohne je ein Backend-Urteil zu schwächen.
    #[test]
    fn bare_stub_stays_syntactic_while_normalizing_closes_linear_gaps() {
        let n = Nexp::Var("n".to_string());
        let plus0 = NConstraint::Eq(
            Box::new(Nexp::Add(Box::new(n.clone()), Box::new(Nexp::Const(0)))),
            Box::new(n),
        );
        let stub = StubSolver::new();
        assert_eq!(stub.prove(&plus0), ProveResult::Proved);
        let normalizing = NormalizingSolver::new(StubSolver::new());
        assert_eq!(normalizing.prove(&plus0), ProveResult::Proved);
        // Inner verdicts win: a refuted goal stays refuted, never upgraded.
        let contra = NConstraint::Eq(Box::new(Nexp::Const(1)), Box::new(Nexp::Const(2)));
        assert_eq!(stub.prove(&contra), ProveResult::Refuted);
        assert_eq!(normalizing.prove(&contra), ProveResult::Refuted);
        assert_eq!(normalizing.inner().prove(&contra), ProveResult::Refuted);
    }
}
