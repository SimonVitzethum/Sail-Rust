#![forbid(unsafe_code)]
//! `sail-frontend-diff`: differential harness for the *frontend* (Lane 86).
//!
//! Compares OCaml `sail` against the Rust frontend at two levels:
//!
//! 1. **Parse trees, normalized** ([`norm_tree`]): the Rust side
//!    (`sail_parse::ParsedFile`: definition-kind sequence + names +
//!    whitespace-collapsed headers) vs. the OCaml side
//!    (`sail --just-check --ddump-initial-ast` stdout, sifted for `DEF_*`
//!    constructors). Spans, node IDs, shard markers (`file_start`/
//!    `file_end`, a Rust-harness artifact with no OCaml counterpart) and
//!    volatile banner lines are excluded by construction.
//! 2. **Type judgments, sampled** ([`ty_sample`]): deterministic sample of
//!    up to `N` `val`/`register`/`function` definitions per file; the Rust
//!    scheme rendering vs. the OCaml `--ddump-tc-ast` excerpt, both passed
//!    through [`ty_sample::normalize_scheme`].
//!
//! [`catalog`] pins the case list to `sail/test/lean/*.sail` fixtures with
//! tiers (smoke / standard / nightly) and per-case focus. [`harness`] wires
//! everything into runnable cases (oracle invocation under
//! `ulimit`+`timeout`, same convention as `sail-oracle`); the Rig run itself
//! happens later (E3: this lane ships code + plan, no executions).
//!
//! Known limits (see `FRONTEND_DIFF_PLAN.md`):
//! - The OCaml dump sifters are heuristic: the exact `--ddump-initial-ast`
//!   / `--ddump-tc-ast` line shapes are **to verify at the first Rig run**
//!   against Sail 0.20.3 output. Sifter mismatches must surface as
//!   `HarnessError`, never as fake passes.
//! - Rust definition bodies are raw source at the skeleton layer (Lane 02);
//!   comparison covers kind + name + header text only.

pub mod catalog;
pub mod harness;
pub mod norm_tree;
pub mod tree_compare;
pub mod ty_sample;
