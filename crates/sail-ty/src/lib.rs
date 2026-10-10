#![forbid(unsafe_code)]
//! `sail-ty`: Sail2 typechecker core (Lanes 05 + 79).
//!
//! - Kind checking (`K_int` / `K_nat` / `K_bool` / `K_type`)
//! - Type quantifier (`TypQuant`) management + freshening/instantiation
//! - Bidirectional expression checking (`infer` / `check`)
//! - Value patterns (`check_pat`)
//! - Register type checking
//! - Subsumption + numeric coercions ([`subsume`])
//! - Rule catalogue ([`rules::RULE_TABLE`], mirrored in `RULES.md`)
//! - [`Solver`] trait (`prove(nc) -> ProveResult`) + [`StubSolver`] for tests
//!
//! No Z3 binding here (Lane 06/80 implement [`Solver`] via their backends;
//! rule `Z-TraitOnly`). Everything solver-dependent is generic over the
//! trait, and `Unknown` answers are always sound (errors at proof sites).
//! AST/nexp types are local minimals; see `TY_NEEDED.md` for the mapping to
//! the future `sail-ast` / `sail-nexp` crates.

pub mod ast_env;
pub mod check;
pub mod conv;
pub mod env;
pub mod error;
pub mod kind;
pub mod nexp;
pub mod pat;
pub mod quant;
pub mod rules;
pub mod solver;
pub mod subsume;

pub use ast_env::{kenv_from_kopts, kenv_to_kopts};
pub use check::{BidirectionalChecker, CheckLimits, TypedDefSummary};
pub use env::TypeEnv;
pub use error::{Span, TypeError, TypeResult};
pub use kind::Kind;
pub use nexp::{MiniExp, MiniFExp, MiniLexp, MiniLoop, MiniOrder, MiniPExp, MiniTyp, NConstraint, Nexp, TypQuant};
pub use pat::MiniPat;
pub use quant::{QuantScope, QuantVar};
pub use rules::{RuleEntry, RuleId, RuleStatus, RULE_TABLE};
pub use solver::{CachingSolver, CacheStats, ProveResult, Solver, StubSolver};
pub use subsume::{Coercion, NexpSubst, Subsumption};
