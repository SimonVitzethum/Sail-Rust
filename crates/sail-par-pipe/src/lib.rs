//! Memory-bounded parallel Parse → Check → Emit with deterministic output.
//!
//! All entry points share `sail_shard::Scheduler::run_streaming` and a single
//! worker pool. `PipeConfig`'s maximum stage worker count sets that pool size.
//! The credit window bounds queued, running and reordered outputs together.
//! Intermediate values are released after each stage; a stage failure skips
//! the downstream stages for that shard. Worker panics and sink errors cancel
//! ingress without leaving blocked producers behind.
//!
//! `run_pipeline_streaming` accepts lazy inputs and an ordered consumer that
//! can spill results to disk. The Vec-returning compatibility entry points
//! retain their inputs and outputs, so their overall memory grows with them.

#![forbid(unsafe_code)]

pub mod merge;
pub mod pipeline;

pub use merge::{concat_ok, manifest, merge_ordered, merge_ordered_par};
pub use pipeline::{
    PipeConfig, ShardOutput, Stage, StageError, run_pipeline, run_pipeline_streaming, run_shard_parallel,
};
