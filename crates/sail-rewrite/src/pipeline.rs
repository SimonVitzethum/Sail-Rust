//! Configurable rewrite pipeline mirroring OCaml `lean_rewrites`.
//!
//! Source: `sail/src/sail_lean_backend/sail_plugin_lean.ml` (`lean_rewrites`,
//! ~line 176). The order below matches that list; per-pass arguments and
//! flag guards (`If_flag opt_disable_matchbv`, ...) are backend options and
//! are *not* modelled here — the pipeline only fixes pass order.
//!
//! Only a subset of passes is implemented in Rust so far; the rest are
//! represented by [`StubPass`], which passes shards through unchanged so the
//! full Lean order can already be configured and tested end to end. Each
//! implemented pass is shard-local, and [`Pipeline::run_parallel`] fans
//! shards out over `rayon` with no shared mutable state.

#![forbid(unsafe_code)]

use std::sync::Arc;

use rayon::prelude::*;

use crate::ast::Shard;
use crate::pass::Pass;
use crate::passes::{SimpleAssignments, TupleAssignments, Undefined};

/// Full Lean rewrite order, taken from OCaml `lean_rewrites`.
///
/// Duplicates (`recheck_defs`, `attach_effects`) are kept intentionally: the
/// OCaml driver runs them repeatedly between other passes.
pub const LEAN_REWRITES: &[&str] = &[
    "move_termination_measures",
    "instantiate_outcomes",
    "realize_mappings",
    "remove_vector_subrange_pats",
    "remove_duplicate_valspecs",
    "toplevel_string_append",
    "pat_string_append",
    "mapping_patterns",
    "add_unspecified_rec",
    "undefined",
    "remove_not_pats",
    "remove_impossible_int_cases",
    "tuple_assignments",
    "vector_concat_assignments",
    "simple_assignments",
    "remove_mixed_bitvector_literal_pats",
    "remove_vector_concat",
    "remove_bitvector_pats",
    "recheck_defs",
    "fun_guarded_pats",
    "guarded_pats",
    "nexp_ids",
    "split",
    "minimise_recursive_functions",
    "remove_bitfield_records",
    "recheck_defs",
    "top_sort_defs",
    "add_register_init_function",
    "const_prop_mutrec",
    "exp_lift_assign",
    "early_return",
    "recheck_defs",
    "pattern_exhaustivity_redundancy",
    "merge_function_clauses",
    "recheck_defs",
    "rewrite_loops_with_escape_effect",
    "recheck_defs",
    "infer_effects",
    "attach_effects",
    "remove_blocks",
    "attach_effects",
    "remove_e_assign",
    "attach_effects",
    "internal_lets",
    "remove_superfluous_letbinds",
    "remove_superfluous_returns",
    "bit_lists_to_lits",
    "toplevel_let_patterns",
    "recheck_defs",
    "attach_effects",
];

/// Whether `name` is part of the known Lean rewrite set.
#[must_use]
pub fn is_known_pass(name: &str) -> bool {
    LEAN_REWRITES.contains(&name)
}

/// Whether `name` has a real Rust implementation (vs. [`StubPass`]).
#[must_use]
pub fn is_implemented_pass(name: &str) -> bool {
    matches!(
        name,
        "undefined" | "tuple_assignments" | "simple_assignments"
    )
}

/// Build the Rust implementation for a known pass name.
///
/// Returns `None` for known-but-unimplemented passes (caller falls back to
/// [`StubPass`]) and for unknown names.
fn make_implemented(name: &'static str) -> Option<Arc<dyn Pass>> {
    match name {
        "undefined" => Some(Arc::new(Undefined)),
        "tuple_assignments" => Some(Arc::new(TupleAssignments)),
        "simple_assignments" => Some(Arc::new(SimpleAssignments)),
        _ => None,
    }
}

/// Placeholder for a Lean rewrite that has no Rust implementation yet.
///
/// Identity transform: returns the shard unchanged. Exists so the full
/// [`LEAN_REWRITES`] order is configurable and runnable end to end while
/// passes land one by one.
#[derive(Clone, Copy, Debug)]
pub struct StubPass {
    name: &'static str,
}

impl StubPass {
    /// Create a stub for the given (known) pass name.
    #[must_use]
    pub fn new(name: &'static str) -> Self {
        Self { name }
    }
}

impl Pass for StubPass {
    fn name(&self) -> &'static str {
        self.name
    }

    fn run(&self, shard: Shard) -> Shard {
        shard
    }
}

/// Error for [`Pipeline::from_names`] on unknown pass names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownPass {
    /// The offending name.
    pub name: String,
}

impl std::fmt::Display for UnknownPass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown rewrite pass: {}", self.name)
    }
}

impl std::error::Error for UnknownPass {}

/// An ordered, configurable list of rewrite passes.
///
/// Cheap to clone (`Arc`-backed); `&Pipeline` is `Send + Sync` so it can be
/// shared across `rayon` threads.
#[derive(Clone)]
pub struct Pipeline {
    passes: Vec<Arc<dyn Pass>>,
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipeline")
            .field("passes", &self.pass_names())
            .finish()
    }
}

impl Pipeline {
    /// Build a pipeline from an explicit ordered list of pass names.
    ///
    /// Known-but-unimplemented names become [`StubPass`]es; unknown names
    /// yield [`UnknownPass`].
    pub fn from_names(names: &[&'static str]) -> Result<Self, UnknownPass> {
        let mut passes: Vec<Arc<dyn Pass>> = Vec::with_capacity(names.len());
        for &name in names {
            if !is_known_pass(name) {
                return Err(UnknownPass {
                    name: name.to_string(),
                });
            }
            let pass: Arc<dyn Pass> =
                make_implemented(name).unwrap_or_else(|| Arc::new(StubPass::new(name)));
            passes.push(pass);
        }
        Ok(Self { passes })
    }

    /// The full Lean default order ([`LEAN_REWRITES`]).
    ///
    /// Infallible: every name in [`LEAN_REWRITES`] is known by construction.
    #[must_use]
    pub fn lean_default() -> Self {
        Self::from_names(LEAN_REWRITES).expect("LEAN_REWRITES must be known")
    }

    /// Only the passes that have a real Rust implementation, in Lean order.
    #[must_use]
    pub fn lean_implemented() -> Self {
        let names: Vec<&'static str> = LEAN_REWRITES
            .iter()
            .copied()
            .filter(|n| is_implemented_pass(n))
            .collect();
        Self::from_names(&names).expect("implemented names are known")
    }

    /// Names of the passes in this pipeline, in order.
    #[must_use]
    pub fn pass_names(&self) -> Vec<&'static str> {
        self.passes.iter().map(|p| p.name()).collect()
    }

    /// Number of passes in this pipeline.
    #[must_use]
    pub fn len(&self) -> usize {
        self.passes.len()
    }

    /// Whether this pipeline holds no passes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }

    /// Run the whole pipeline on a single shard (passes in order).
    #[must_use]
    pub fn run(&self, mut shard: Shard) -> Shard {
        for pass in &self.passes {
            shard = pass.run(shard);
        }
        shard
    }

    /// Run the pipeline over many shards in parallel via `rayon`.
    ///
    /// Each shard is processed independently (shard-local passes, no shared
    /// mutable state). Output order matches input order; use `shard.id` for
    /// deterministic reassembly.
    #[must_use]
    pub fn run_parallel(&self, shards: Vec<Shard>) -> Vec<Shard> {
        shards
            .into_par_iter()
            .map(|shard| self.run(shard))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind, Stmt};

    #[test]
    fn lean_default_matches_ocaml_order() {
        let pipe = Pipeline::lean_default();
        assert_eq!(pipe.pass_names(), LEAN_REWRITES);
        // Spot-check anchors from sail_plugin_lean.ml.
        assert_eq!(pipe.pass_names()[0], "move_termination_measures");
        assert_eq!(pipe.pass_names()[9], "undefined");
        assert_eq!(pipe.pass_names()[12], "tuple_assignments");
        assert_eq!(pipe.pass_names()[14], "simple_assignments");
        assert_eq!(*pipe.pass_names().last().unwrap_or(&""), "attach_effects");
    }

    #[test]
    fn stubs_pass_shards_through() {
        let pipe =
            Pipeline::from_names(&["move_termination_measures"]).unwrap_or_else(|e| panic!("{e}"));
        assert!(!is_implemented_pass("move_termination_measures"));
        let shard = Shard::new(
            0,
            vec![Def::new(
                "f",
                DefKind::Function,
                vec![Stmt::Expr("e".to_string())],
            )],
        );
        assert_eq!(pipe.run(shard.clone()), shard);
    }

    #[test]
    fn rejects_unknown_pass() {
        let err = Pipeline::from_names(&["no_such_pass"]).unwrap_err();
        assert_eq!(err.name, "no_such_pass");
    }

    #[test]
    fn implemented_pipeline_rewrites_end_to_end() {
        let pipe = Pipeline::lean_implemented();
        assert_eq!(
            pipe.pass_names(),
            vec!["undefined", "tuple_assignments", "simple_assignments"]
        );
        let shard = Shard::new(
            0,
            vec![Def::new(
                "f",
                DefKind::Function,
                vec![
                    Stmt::Undefined {
                        target: "n".to_string(),
                        ty: "int".to_string(),
                    },
                    Stmt::TupleAssign {
                        targets: vec!["a".to_string(), "b".to_string()],
                        rhs: "t".to_string(),
                    },
                    Stmt::Assign {
                        lhs: "x".to_string(),
                        rhs: "x".to_string(),
                    },
                ],
            )],
        );
        let out = pipe.run(shard);
        assert_eq!(
            out.defs[0].body,
            vec![
                Stmt::Assign {
                    lhs: "n".to_string(),
                    rhs: "0".to_string()
                },
                Stmt::Assign {
                    lhs: "a".to_string(),
                    rhs: "t.0".to_string()
                },
                Stmt::Assign {
                    lhs: "b".to_string(),
                    rhs: "t.1".to_string()
                },
            ]
        );
    }

    #[test]
    fn parallel_matches_sequential_and_keeps_order() {
        let pipe = Pipeline::lean_implemented();
        let shards: Vec<Shard> = (0..32)
            .map(|i| {
                Shard::new(
                    i,
                    vec![Def::new(
                        format!("f{i}"),
                        DefKind::Function,
                        vec![Stmt::Undefined {
                            target: "x".to_string(),
                            ty: "bool".to_string(),
                        }],
                    )],
                )
            })
            .collect();
        let seq: Vec<Shard> = shards.iter().cloned().map(|s| pipe.run(s)).collect();
        let par = pipe.run_parallel(shards);
        assert_eq!(par, seq);
        for (i, shard) in par.iter().enumerate() {
            assert_eq!(shard.id, i);
            assert_eq!(
                shard.defs[0].body,
                vec![Stmt::Assign {
                    lhs: "x".to_string(),
                    rhs: "false".to_string()
                }]
            );
        }
    }
}
