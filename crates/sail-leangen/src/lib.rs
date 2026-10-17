#![forbid(unsafe_code)]
//! Lean backend for the pure-Rust Sail compiler: a port of
//! `sail_lean_backend/pretty_print_lean.ml` and of the Lean rewrite list in
//! `sail_plugin_lean.ml` over the canonical `sail_ast`.

pub mod analysis;
pub mod ctx;
pub mod doc;
pub mod info;
pub mod names;
pub mod pp_def;
pub mod pp_exp;
pub mod pp_pat;
pub mod pp_typ;
pub mod remap;
pub mod rw;
pub mod sailpp;
pub mod synth;

/// Static Lean support files copied into generated projects.
pub mod lean_support {
    pub const SPECIALIZATION_V1: &str = include_str!("../lean/SpecializationV1.lean");
    pub const SPECIALIZATION_ARCHSEM: &str = include_str!("../lean/SpecializationArchSem.lean");
    pub const FAKE_REAL: &str = include_str!("../lean/FakeReal.lean");
    pub const REAL: &str = include_str!("../lean/Real.lean");
}
