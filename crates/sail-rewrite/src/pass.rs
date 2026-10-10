//! The [`Pass`] trait: every rewrite is shard-local.

#![forbid(unsafe_code)]

use crate::ast::Shard;

/// A single rewrite pass: pure, shard-local, thread-safe.
///
/// Contract:
/// - `run` takes a [`Shard`] by value and returns the rewritten [`Shard`].
/// - No global / shared mutable state (passes must be usable from `rayon`
///   threads concurrently).
/// - Passes must be deterministic: same input shard yields same output shard.
pub trait Pass: Send + Sync {
    /// Pass name, matching the OCaml rewrite name (e.g. `"undefined"`).
    fn name(&self) -> &'static str;

    /// Run the pass on one shard, returning the rewritten shard.
    fn run(&self, shard: Shard) -> Shard;
}
