//! Lane 32 — ARMv9 SVE/SME instruction shards.
//!
//! Parses `instrs64_sve.sail` + `instrs64_sme.sail` (138k lines total) as
//! independent shards: definition-level split, vector-pattern/loop catalog
//! ([`patterns`]), shard report ([`shard`]). No full build, no typecheck —
//! constructs this stage cannot lower are reported as [`patterns::Fail`]
//! entries with exact spans (see `FAILURES_SVE_SME.md`).
//!
//! The definition-level scanner (`lexer`, `mini_ast`, `parser`, `span`) is
//! **not** vendored here: Lane 115 dedup — one truth, one import. These
//! modules are re-exports of `sail-parse` (Lane 02, reference
//! implementation); the SVE/SME-specific analysis on top ([`patterns`],
//! [`shard`]) is the only new code in this crate.
//!
//! Properties: `#![forbid(unsafe_code)]`, pure safe Rust, low-RAM by design
//! (one left-to-right pass per file, per-definition processing, raw bodies
//! are never copied — analysis slices the input via definition spans).
//! Spans are canonical `sail-ast` byte ranges (`FileId(0)` per
//! single-file parse); line/column resolution owns [`sail_ast::FileTable`].

#![forbid(unsafe_code)]

// Scanner: one truth (`sail-parse`), re-exported for compatible paths.
pub use sail_parse::lexer;
pub use sail_parse::mini_ast;
pub use sail_parse::parser;
pub use sail_parse::span;

pub mod patterns;
pub mod shard;

pub use sail_parse::{
    Def, FileId, ParseError, ParsedFile, PragmaArg, Span, Spanned, SpannedDef, parse_file,
};
pub use patterns::{Fail, FailKind, FileReport, Hit, PatternKind, analyze_file, analyze_files_parallel};
