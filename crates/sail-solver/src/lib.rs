#![forbid(unsafe_code)]
//! Solver-Schicht des Sail-Compilers (Lane 06).
//!
//! - [`Solver`]: Trait aus Lane 05 (`prove(nc) -> Valid/Invalid/Unknown`).
//! - [`query::Constraint::from_nconstraint`]: kanonische Bruecke aus
//!   `sail_nexp` (gleiche Formel = gleicher Cache-Key).
//! - [`MemoCache`]: begrenzter LRU-Cache (**Pflicht**: kein unbegrenzter
//!   Cache; `capacity >= 1` erzwungen, LRU-Eviction, TTL) mit
//!   Hit/Miss-Zaehlern, `Send + Sync` teilbar.
//! - [`MemoSolver`]: kombiniert einen [`Solver`] mit [`MemoCache`].
//! - [`SolverPool`]: 1 Solver pro Thread per exklusivem Checkout (kein Sharing).
//! - [`StubSolver`]: Default ohne C-Dep, terminiert immer (reiner Stub).
//! - `z3_backend` (nur mit Feature `z3`): Anbindung via `z3`-Crate 0.21
//!   (impliziter Thread-Kontext, Timeout via `Params`), inkl.
//!   vollstaendigem `sail_nexp`-Mapping (`Z3Solver::prove_nconstraint`,
//!   `nexp_to_int`, `nconstraint_to_bool`). `unsafe` steckt
//!   ausschliesslich in der Dep, eigener Code bleibt
//!   `#![forbid(unsafe_code)]`.
//!
//! Operator-Tabelle (`eval`/`simplify`/`provably_*`/Lean/Z3 je Operator):
//! `ARCHITECTURE.md` (Lane 80).

mod cache;
pub mod canonical;
mod pool;
pub mod quota;
mod query;
mod smtlib_backend;
mod solver;
mod stub;
#[cfg(feature = "z3")]
pub mod z3_backend;

pub use cache::{CacheStats, MemoCache, MemoSolver, SolverCacheConfig};
pub use pool::{PooledSolver, SolverPool};
pub use quota::{
    QuotaConfig, QuotaPermit, QuotaRecommendation, QuotaStats, SolverQuota, BYTES_PER_SOLVER,
    MAX_LIVE_SOLVERS, MIN_CACHE_BYTES,
};
pub use query::{Constraint, SolverQuery};
pub use smtlib_backend::SmtLibSolver;
pub use solver::{Solver, SolverVerdict};
pub use stub::StubSolver;
#[cfg(feature = "z3")]
pub use z3_backend::{nconstraint_to_bool, nexp_to_int, Z3Solver};
