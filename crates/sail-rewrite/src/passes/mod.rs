//! Implemented rewrite passes.
//!
//! Each pass is shard-local: it only looks at one [`Shard`] at a time and
//! holds no shared mutable state, so the pipeline can run shards in parallel
//! via `rayon`.

#![forbid(unsafe_code)]

pub mod simple_assignments;
pub mod tuple_assignments;
pub mod undefined;

pub use simple_assignments::SimpleAssignments;
pub use tuple_assignments::TupleAssignments;
pub use undefined::Undefined;
