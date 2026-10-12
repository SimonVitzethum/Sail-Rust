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

/// A rewrite pass over the canonical [`sail_ast::Ast`].
///
/// This is the typed/canonical path: passes see real `sail_ast` definitions
/// (`DefKind`, `ExpKind`, `LExpKind`, `PatKind`) instead of the string-based
/// [`Shard`] surface used by [`Pass`]. Source: `sail/src/lib/rewrites.ml`
/// (`all_rewriters`) in Lean order ([`crate::pipeline::LEAN_REWRITES`]).
///
/// Contract (mirrors [`Pass`]):
/// - `run` rewrites `ast` in place; the caller owns the interner.
/// - Fresh names are interned through `interner` and must not capture user
///   names (collect-then-generate via
///   [`crate::passes::typed::Fresh`]).
/// - No other shared mutable state; `&dyn Pass` objects stay `Send + Sync`.
/// - Passes must be deterministic and idempotent where OCaml reaches a
///   fixpoint (e.g. `tuple_assignments` loops until no change).
/// - Passes that need type/effect environments unavailable on the untyped
///   AST (`undefined` needs `undefined_of_typ`, `simple_assignments` needs
///   `rewrite_lexp_to_rhs`) traverse but preserve the input and document
///   the gap instead of guessing semantics.
pub trait TypedPass: Send + Sync {
    /// Pass name, matching the OCaml rewrite name (e.g. `"remove_blocks"`).
    fn name(&self) -> &'static str;

    /// Run the pass on the whole canonical AST.
    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner);
}
