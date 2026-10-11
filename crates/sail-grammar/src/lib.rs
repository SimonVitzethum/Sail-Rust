//! Sail grammar ported from Menhir (`sail/src/lib/parser.mly`).
//!
//! Pure Rust (`#![forbid(unsafe_code)]`). This crate provides the skeletal
//! AST ([`ast`]), the hand-written lexer ([`lexer`]), and the LALRPOP
//! grammar (`src/sail.lalrpop`, E3c/E4 conflict fixes — see `ARCHITECTURE.md`),
//! compiled to tables by `build.rs` (`lalrpop::process_root()`,
//! `lalrpop` 0.22 build-dep) and wired up via `lalrpop_mod!(sail)` below.
//! The `parse_*` entry points tokenize their input and run the generated
//! `*Parser`s. Rule coverage: `tests/grammar_coverage.rs`; Sail-library
//! corpus: `tests/corpus.rs` (run on the Test-Rig — E3, no local runs here).

#![forbid(unsafe_code)]

use lalrpop_util::lalrpop_mod;

lalrpop_mod!(sail);

pub mod ast;
pub mod lexer;

use lexer::{LexError, Tok};

fn tokenize(src: &str) -> Result<Vec<(usize, Tok, usize)>, String> {
    lexer::tokenize(src).map_err(|e: LexError| e.to_string())
}

fn parse_error(err: impl core::fmt::Debug) -> String {
    format!("parse error: {err:?}")
}

/// Parse a complete Sail file (`file` in `parser.mly`).
pub fn parse_file(src: &str) -> Result<Vec<ast::Def>, String> {
    let toks = tokenize(src)?;
    sail::FileParser::new()
        .parse(toks.into_iter().map(Ok))
        .map_err(parse_error)
}

/// Parse a type (`typ_eof`).
pub fn parse_typ(src: &str) -> Result<ast::Typ, String> {
    let toks = tokenize(src)?;
    sail::TypParser::new()
        .parse(toks.into_iter().map(Ok))
        .map_err(parse_error)
}

/// Parse a type scheme (`typschm_eof`).
pub fn parse_typschm(src: &str) -> Result<ast::TypSchm, String> {
    let toks = tokenize(src)?;
    sail::TypSchmParser::new()
        .parse(toks.into_iter().map(Ok))
        .map_err(parse_error)
}

/// Parse an expression (`exp_eof`).
pub fn parse_exp(src: &str) -> Result<ast::Exp, String> {
    let toks = tokenize(src)?;
    sail::ExpParser::new()
        .parse(toks.into_iter().map(Ok))
        .map_err(parse_error)
}

/// Parse a pattern (`pat` + EOF).
pub fn parse_pat(src: &str) -> Result<ast::Pat, String> {
    let toks = tokenize(src)?;
    sail::PatParser::new()
        .parse(toks.into_iter().map(Ok))
        .map_err(parse_error)
}

/// Parse a single definition (`def_eof`).
pub fn parse_def(src: &str) -> Result<ast::Def, String> {
    let toks = tokenize(src)?;
    sail::DefParser::new()
        .parse(toks.into_iter().map(Ok))
        .map_err(parse_error)
}
