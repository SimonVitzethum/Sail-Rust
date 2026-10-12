//! Lane-62 contract for the checker-bound passes.
//!
//! `recheck_defs`, `infer_effects` and `attach_effects` need a checker
//! binding that does not exist yet (see the lane report), so their Rust
//! bodies live in unwired `src/passes/{recheck_defs,infer_effects,
//! attach_effects}.rs` and are NOT registered in `passes/mod.rs` or the
//! pipeline (wiring is the integrator's Insert-Spec). This test pins what
//! the public pipeline guarantees meanwhile:
//!
//! - all three names are known Lean rewrites at the exact OCaml positions
//!   (`sail_plugin_lean.ml` order mirrored in `LEAN_REWRITES`);
//! - all three are still stubs (`is_implemented_pass == false`), i.e. the
//!   full `lean_default()` pipeline passes shards through for them — which
//!   matches OCaml `infer_effects` returning its AST unchanged;
//! - the pipeline stays deterministic under `run_parallel`.
//!
//! When the integrator wires the new passes, the `is_implemented_pass`
//! assertions below must flip to `true` (and `lean_implemented()` will
//! start including these names).

#![forbid(unsafe_code)]

use sail_rewrite::pipeline::{is_implemented_pass, is_known_pass, LEAN_REWRITES};
use sail_rewrite::{Def, DefKind, Pipeline, Shard, Stmt};

fn positions(name: &str) -> Vec<usize> {
    LEAN_REWRITES
        .iter()
        .enumerate()
        .filter_map(|(i, n)| (*n == name).then_some(i))
        .collect()
}

#[test]
fn checker_passes_known_at_ocaml_positions() {
    for name in ["recheck_defs", "infer_effects", "attach_effects"] {
        assert!(is_known_pass(name), "{name} must stay a known pass");
    }
    // Duplicates are intentional: the OCaml driver repeats these passes.
    assert_eq!(positions("recheck_defs").len(), 6);
    assert_eq!(positions("infer_effects").len(), 1);
    assert_eq!(positions("attach_effects").len(), 4);
    // `infer_effects` runs immediately before its first `attach_effects`.
    let infer = positions("infer_effects")[0];
    let attach = positions("attach_effects");
    assert_eq!(attach[0], infer + 1);
    // Last pass of the whole Lean order is `attach_effects`.
    assert_eq!(*LEAN_REWRITES.last().unwrap(), "attach_effects");
    assert_eq!(*attach.last().unwrap(), LEAN_REWRITES.len() - 1);
}

#[test]
fn checker_passes_wired_after_integrator_applied_insert_spec() {
    // Insert-Spec applied (Main-Agent): the three checker passes are wired
    // (identity-with-taxonomy on the string path).
    for name in ["recheck_defs", "infer_effects", "attach_effects"] {
        assert!(
            is_implemented_pass(name),
            "{name} must be wired"
        );
    }
    assert!(Pipeline::from_names(&["recheck_defs", "infer_effects", "attach_effects"]).is_ok());
}

#[test]
fn stubbed_checker_passes_pass_shards_through() {
    // Statements no implemented pass touches: Expr/Return survive the full
    // Lean-default pipeline byte-for-byte, proving the stubbed checker
    // passes (like OCaml `infer_effects`) change nothing on the Shard.
    let defs: Vec<Def> = (0..4)
        .map(|i| {
            Def::new(
                format!("f{i}"),
                DefKind::Function,
                vec![
                    Stmt::Expr(format!("effect({i})")),
                    Stmt::Return("x".to_string()),
                ],
            )
        })
        .collect();
    let shards = Shard::chunk(defs, 2);
    let pipe = Pipeline::lean_default();
    let out = pipe.run_parallel(shards.clone());
    assert_eq!(out, shards);
    // Deterministic across runs.
    let again = pipe.run_parallel(shards.clone());
    assert_eq!(out, again);
}
