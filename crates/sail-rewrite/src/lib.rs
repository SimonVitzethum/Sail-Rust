//! sail-rewrite: shard-local Lean rewrite passes.
//!
//! Mirrors the OCaml `lean_rewrites` list from
//! `sail/src/sail_lean_backend/sail_plugin_lean.ml` as a configurable,
//! parallel pipeline. Each pass is shard-local (no global mutables) and the
//! pipeline fans shards out over `rayon`.
//!
//! Shards are small definition chunks so peak RAM stays low even for large
//! Sail inputs: passes never see the whole program at once.

#![forbid(unsafe_code)]

pub mod ast;
pub mod pass;
pub mod passes;
pub mod pipeline;

pub use ast::{Def, DefKind, Shard, Stmt};
pub use pass::Pass;
pub use pipeline::Pipeline;
