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

pub mod lexer;
pub mod mini_ast;
pub mod parser;
pub mod span;

pub use mini_ast::{Def, ParsedFile, PragmaArg, SpannedDef};
pub use span::{ParseError, Span, Spanned};

pub use parser::parse_file;

/// Parse many files in parallel (one thread per file, via
/// [`std::thread::scope`]). Results are in input order. The parser holds
/// no shared mutable state, so this scales linearly with file count —
/// the intended driver for the full `arm-v9.4-a/src/*.sail` model.
pub fn parse_files_parallel(inputs: &[(&str, &str)]) -> Vec<Result<ParsedFile, ParseError>> {
    let mut out: Vec<Option<Result<ParsedFile, ParseError>>> = Vec::with_capacity(inputs.len());
    out.resize_with(inputs.len(), || None);
    std::thread::scope(|s| {
        let handles: Vec<_> = inputs
            .iter()
            .enumerate()
            .map(|(i, (path, src))| {
                s.spawn(move || {
                    let r = parse_file(path, src);
                    (i, r)
                })
            })
            .collect();
        for h in handles {
            let (i, r) = h.join().unwrap_or_else(|_| {
                (
                    usize::MAX,
                    Err(ParseError::new(Span::point("?", 0, 1, 1), "parser thread panicked")),
                )
            });
            if i != usize::MAX {
                out[i] = Some(r);
            }
        }
    });
    out.into_iter().map(|o| o.expect("parser thread missing")).collect()
}
