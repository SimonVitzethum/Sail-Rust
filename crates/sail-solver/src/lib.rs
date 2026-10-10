#![forbid(unsafe_code)]
//! Solver-Schicht des Sail-Compilers (Lane 06).
//!
//! - [`Solver`]: Trait aus Lane 05 (`prove(nc) -> Valid/Invalid/Unknown`).
//! - [`MemoCache`]: begrenzter LRU-Cache (Groesse/TTL konfigurierbar) mit
//!   Hit/Miss-Zaehlern, `Send + Sync` teilbar.
//! - [`MemoSolver`]: kombiniert einen [`Solver`] mit [`MemoCache`].
//! - [`SolverPool`]: 1 Solver pro Thread per exklusivem Checkout (kein Sharing).
//! - [`StubSolver`]: Default ohne C-Dep, terminiert immer (reiner Stub).
//! - `z3_backend` (nur mit Feature `z3`): Anbindung via `z3`-Crate.
//!   `unsafe` steckt ausschliesslich in der Dep, eigener Code bleibt
//!   `#![forbid(unsafe_code)]`.

mod cache;
mod pool;
mod query;
mod solver;
mod stub;
#[cfg(feature = "z3")]
pub mod z3_backend;

pub use cache::{CacheStats, MemoCache, MemoSolver, SolverCacheConfig};
pub use pool::{PooledSolver, SolverPool};
pub use query::{Constraint, SolverQuery};
pub use solver::{Solver, SolverVerdict};
pub use stub::StubSolver;
#[cfg(feature = "z3")]
pub use z3_backend::Z3Solver;
