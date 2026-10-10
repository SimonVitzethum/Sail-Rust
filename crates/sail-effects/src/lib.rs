//! Lane 08: callgraph + effect analysis.
//!
//! Pure-Rust, `#![forbid(unsafe_code)]`, low-RAM + multithreaded friendly:
//! - everything is a pure function over immutable borrows,
//! - the callgraph uses dense index maps (`u32` ids + `Vec` adjacency),
//! - all types are `Send + Sync` so callers (Lane 10) can analyse
//!   functions in parallel with `std::thread` / rayon.

#![forbid(unsafe_code)]

mod callgraph;
mod effects;
mod ir;

pub use callgraph::CallGraph;
pub use effects::{EffectLabel, effect_label, function_is_pure, has_early_return, has_effect, has_loop};
pub use ir::{Binop, Block, Expr, FunDef, Stmt};
