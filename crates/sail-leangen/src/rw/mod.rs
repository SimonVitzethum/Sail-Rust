#![forbid(unsafe_code)]
//! Rewrites of the Lean pipeline (`sail_plugin_lean.ml: lean_rewrites`) over
//! the canonical AST. Types come from the elaboration annotations
//! (`Annotation::typ`), which the passes thread through new nodes the way the
//! OCaml passes thread `tannot`.

pub mod early_return;
pub mod simplify;
pub mod typs;
pub mod lower;
pub mod pipeline;
pub mod util;

pub use util::{fold_exp, Rw};
