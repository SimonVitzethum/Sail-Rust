//! Implemented rewrite passes.
//!
//! Each pass is shard-local: it only looks at one [`crate::ast::Shard`] at a
//! time and holds no shared mutable state, so the pipeline can run shards in
//! parallel via `rayon`.
//!
//! The `typed` module plus the `TypedPass` implementations in each pass file
//! form the canonical path over `sail_ast::Ast` (see `pass::TypedPass` and
//! `pipeline::TypedPipeline`).

#![forbid(unsafe_code)]

pub mod add_unspecified_rec;
pub mod merge_function_clauses;
pub mod minimise_recursive_functions;
pub mod pat_string_append;
pub mod remove_blocks;
pub mod remove_duplicate_valspecs;
pub mod remove_superfluous_letbinds;
pub mod realize_mappings;
pub mod mapping_patterns;
pub mod instantiate_outcomes;
pub mod simple_assignments;
pub mod recheck_defs;
pub mod infer_effects;
pub mod attach_effects;
pub mod remove_e_assign;
pub mod internal_lets;
pub mod early_return;
pub mod exp_lift_assign;
pub mod bit_lists_to_lits;
pub mod remove_vector_subrange_pats;
pub mod vector_concat_assignments;
pub mod remove_superfluous_returns;
pub mod split;
pub mod const_prop_mutrec;
pub mod move_termination_measures;
pub mod toplevel_let_patterns;
pub mod top_sort_defs;
pub mod nexp_ids;
pub mod add_register_init_function;
pub mod tuple_assignments;
pub mod typed;
pub mod undefined;

pub use add_unspecified_rec::AddUnspecifiedRec;
pub use merge_function_clauses::MergeFunctionClauses;
pub use minimise_recursive_functions::MinimiseRecursiveFunctions;
pub use pat_string_append::PatStringAppend;
pub use remove_blocks::RemoveBlocks;
pub use remove_duplicate_valspecs::RemoveDuplicateValspecs;
pub use remove_superfluous_letbinds::RemoveSuperfluousLetbinds;
pub use realize_mappings::RealizeMappings;
pub use mapping_patterns::MappingPatterns;
pub use instantiate_outcomes::InstantiateOutcomes;
pub use simple_assignments::SimpleAssignments;
pub use recheck_defs::RecheckDefs;
pub use infer_effects::InferEffects;
pub use attach_effects::AttachEffects;
pub use remove_e_assign::RemoveEAssign;
pub use internal_lets::InternalLets;
pub use early_return::EarlyReturn;
pub use exp_lift_assign::ExpLiftAssign;
pub use bit_lists_to_lits::BitListsToLits;
pub use remove_vector_subrange_pats::RemoveVectorSubrangePats;
pub use vector_concat_assignments::{ConcatLengths, VectorConcatAssignments};
pub use remove_superfluous_returns::RemoveSuperfluousReturns;
pub use split::Split;
pub use const_prop_mutrec::ConstPropMutrec;
pub use move_termination_measures::MoveTerminationMeasures;
pub use toplevel_let_patterns::ToplevelLetPatterns;
pub use top_sort_defs::TopSortDefs;
pub use nexp_ids::NexpIds;
pub use add_register_init_function::AddRegisterInitFunction;
pub use tuple_assignments::TupleAssignments;
pub use undefined::Undefined;
