#![forbid(unsafe_code)]
//! Termination measures (`termination_by`, `decreasing_trivial`).
//!
//! Pure-Rust port of the relevant parts of the OCaml Sail→Lean backend:
//!
//! * `Rec_measure` evaluation: `termination_by (<measure>).toNat` generation
//!   (cf. `pretty_print_lean.ml`, `doc_termination`).
//! * Fuel variants for loops: `whileFuelM` / `untilFuelM` signatures
//!   (cf. `Sail/Sail.lean`, `doc_loop` with `` `WhileFuel`` / `` `UntilFuel```).
//! * Sketch of the `move_termination_measures` rewrite pass
//!   (cf. `rewrites.ml`): moves out-of-line `termination_measure f ...`
//!   declarations into the function definitions and drops the `DEF_measure`
//!   nodes.

pub mod fuel;
pub mod pass;
pub mod rec_measure;

pub use fuel::{FuelLoop, LoopKind, fuel_signatures};
pub use pass::{FunDef, LoopMeasure, MeasureDirective, MoveMeasuresError, TerminationPass, parse_measure_directives};
pub use rec_measure::{RecKind, RecMeasure, render_termination_by};
