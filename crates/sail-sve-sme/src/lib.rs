//! Lane 32 — ARMv9 SVE/SME instruction shards.
//!
//! Parses `instrs64_sve.sail` + `instrs64_sme.sail` (138k lines total) as
//! independent shards: definition-level split, vector-pattern/loop catalog
//! ([`patterns`]), shard report ([`shard`]). No full build, no typecheck —
//! constructs this stage cannot lower are reported as [`patterns::Fail`]
//! entries with exact [`span::Span`]s (see `FAILURES_SVE_SME.md`).
//!
//! The definition-level scanner (`lexer`, `mini_ast`, `parser`, `span`) is
//! vendored from `sail-parse` (Lane 02, reference implementation, files kept
//! byte-identical for easy diffing); the SVE/SME-specific analysis on top
//! ([`patterns`], [`shard`]) is new code written for this lane.
//!
//! Properties: `#![forbid(unsafe_code)]`, pure safe Rust, low-RAM by design
//! (one left-to-right pass per file, per-definition processing, raw bodies
//! are never copied — analysis slices the input via definition spans).

#![forbid(unsafe_code)]

pub mod lexer;
pub mod mini_ast;
pub mod parser;
pub mod patterns;
pub mod shard;
pub mod span;

pub use mini_ast::{Def, ParsedFile, PragmaArg, SpannedDef};
pub use parser::parse_file;
pub use patterns::{Fail, FailKind, FileReport, Hit, PatternKind, analyze_file, analyze_files_parallel};
pub use span::{ParseError, Span, Spanned};
