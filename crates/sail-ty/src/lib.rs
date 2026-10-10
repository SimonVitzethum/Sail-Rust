#![forbid(unsafe_code)]
//! `sail-ty`: Sail2 typechecker core (Lane 05).
//!
//! - Kind checking (`K_int` / `K_bool` / `K_type`)
//! - Type quantifier (`TypQuant`) management
//! - Bidirectional expression checking (`infer` / `check`)
//! - Register type checking
//! - [`Solver`] trait (`prove(nc) -> ProveResult`) + [`StubSolver`] for tests
//!
//! No Z3 binding here (Lane 06 implements [`Solver`] via the `z3` crate).
//! AST/nexp types are local minimals; see `TY_NEEDED.md` for the mapping to
//! the future `sail-ast` / `sail-nexp` crates.

pub mod check;
pub mod env;
pub mod error;
pub mod kind;
pub mod nexp;
pub mod quant;
pub mod solver;

pub use check::{BidirectionalChecker, CheckLimits, TypedDefSummary};
pub use env::TypeEnv;
pub use error::{Span, TypeError, TypeResult};
pub use kind::Kind;
pub use nexp::{MiniExp, MiniTyp, NConstraint, Nexp, TypQuant};
pub use quant::{QuantScope, QuantVar};
pub use solver::{CachingSolver, CacheStats, ProveResult, Solver, StubSolver};
