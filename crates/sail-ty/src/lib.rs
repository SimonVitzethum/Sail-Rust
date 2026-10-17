#![forbid(unsafe_code)]
//! `sail-ty`: Sail2 typechecker core (Lanes 05 + 79).
//!
//! - Kind checking (`K_int` / `K_nat` / `K_bool` / `K_type`)
//! - Type quantifier (`TypQuant`) management + freshening/instantiation
//! - Bidirectional expression checking (`infer` / `check`)
//! - Value patterns (`check_pat`)
//! - Register type checking
//! - Subsumption + numeric coercions ([`subsume`])
//! - Rule catalogue ([`rules::RULE_TABLE`], mirrored in `ARCHITECTURE.md`)
//! - [`Solver`] trait (`prove(nc) -> ProveResult`) + [`StubSolver`] for tests
//! - [`prove`]: solver context (`NC_set`/`NC_app` paths, kind environments,
//!   `val`/`register` schemes and `scattered` clauses as assumptions —
//!   Z3 only behind the trait, Lane 06/80)
//!
//! No Z3 binding here (Lane 06/80 implement [`Solver`] via their backends;
//! rule `Z-TraitOnly`). Everything solver-dependent is generic over the
//! trait, and `Unknown` answers are always sound (errors at proof sites).
//! AST/nexp types are local minimals; see `ARCHITECTURE.md` for the mapping to
//! the future `sail-ast` / `sail-nexp` crates.

pub mod advanced;
pub mod ast_env;
pub mod check;
pub mod conv;
pub mod env;
pub mod error;
pub mod exp_arms;
pub mod exp_match;
pub mod exp_misc;
pub mod exp_struct;
pub mod kind;
pub mod lexp_bounds;
pub mod mapping_expand;
pub mod measure_check;
pub mod nexp;
pub mod outcome_elab;
pub mod pat;
pub mod prove;
pub mod program;
mod canonical_body;
mod canonical_eq;
mod canonical_literal;
pub mod constraint_synonym;
pub mod quant;
pub mod recursion;
pub mod rules;
pub mod scattered_elab;
pub mod solver;
pub mod subsume;
pub mod typed_def;
pub mod vector_lexp;
pub mod vector_lexp_bridge;
pub mod vector_prims;

pub use advanced::{
    AdvTyp, AdvTypArg, ARM_BITVEC_WIDTHS, adv_arg_to_ast, adv_from_ast, adv_to_ast,
    arm_exist_bits, arm_width_obligation, check_adv_kind, check_bidir_sub, erase_to_mini,
    freshen_exist, is_arm_exist_bits, open_exist, prove_exist_open, subst_adv_typ,
};
pub use ast_env::{kenv_from_kopts, kenv_to_kopts};
pub use check::{BidirectionalChecker, CheckLimits, ScatteredKind, TypedDefSummary};
pub use env::TypeEnv;
pub use error::{Span, TypeError, TypeResult};
pub use kind::Kind;
pub use nexp::{MiniExp, MiniFExp, MiniLexp, MiniLoop, MiniOrder, MiniPExp, MiniTyp, NConstraint, Nexp, TypQuant, gensym};
pub use pat::MiniPat;
pub use prove::{
    ScatteredClauses, SolverContext, assumption_from_register, assumption_from_val,
    implication, kenv_of_quants, prove_in_context,
};
pub use quant::{QuantScope, QuantVar};
pub use rules::{RuleEntry, RuleId, RuleStatus, RULE_TABLE};
pub use solver::{CachingSolver, CacheStats, ProveResult, Solver, StubSolver};
pub use subsume::{Coercion, NexpSubst, Subsumption};
pub use typed_def::{TypedDef, TypedDefKind};
