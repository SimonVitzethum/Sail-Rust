//! `sail-parse`: Sail2 definition-level parser (skeleton).
//!
//! Parses one `.sail` file into [`mini_ast::ParsedFile`] — a flat list of
//! definitions with spans and raw bodies. No name resolution, no
//! typecheck (Lane 03 / Lane 05). Pragmas are preserved as definition
//! nodes; every file is wrapped in `file_start`/`file_end` markers for
//! Lane 11 (sharding).
//!
//! Because Lane 01 (`sail-ast`) has not landed, the AST lives in
//! [`mini_ast`]; the expected `sail-ast` interface and migration path are
//! documented in `crates/sail-parse/AST_NEEDED.md`.

#![forbid(unsafe_code)]

pub mod bridge;
pub mod infix;
pub mod lexer;
pub mod mini_ast;
pub mod parser;
pub mod real;
pub mod span;

mod convert;

pub use bridge::{BridgeOut, Unparsed, bridge_file, bridge_parsed};

pub use mini_ast::{Def, ParsedFile, PragmaArg, SpannedDef};
pub use span::{FileId, ParseError, Span, Spanned};

pub use parser::parse_file;
pub use real::{ParseOut, RealParser, parse_def, parse_exp, parse_file_real, parse_pat, parse_typ};

/// Parse files with a bounded shared worker pool, returning input order.
/// Input sources and returned ASTs are retained by this collecting API;
/// lazy, disk-backed callers should use `sail_shard::Scheduler::run_streaming`
/// with `parse_file` and spill each output in its ordered consumer.
pub fn parse_files_parallel(inputs: &[(&str, &str)]) -> Vec<Result<ParsedFile, ParseError>> {
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
        .min(inputs.len().max(1));
    let scheduler = sail_shard::Scheduler::new(sail_shard::SchedulerConfig::new(8, workers));
    let mut out = Vec::with_capacity(inputs.len());
    scheduler.run_streaming(inputs.iter().copied(), |(path, src)| parse_file(path, src), |_, parsed| {
        out.push(parsed);
        Ok::<_, std::convert::Infallible>(())
    }).expect("infallible collector");
    out
}
