//! Sail grammar ported from Menhir (`sail/src/lib/parser.mly`).
//!
//! Pure Rust (`#![forbid(unsafe_code)]`). This crate provides the skeletal
//! AST ([`ast`]), the hand-written lexer ([`lexer`]), and the LALRPOP
//! grammar (`src/sail.lalrpop`) as documentation artifact (E3c/E4 conflict
//! fixes, see `GRAMMAR.md`). Table GENERATION is parked: `build.rs` is a
//! no-op because the grammar still has unproven corners — wiring
//! `lalrpop_mod!` without generated tables breaks the whole workspace build
//! (Rig 2026-10-10). Follow-up lane re-enables generation WITH rig verification.
//! Until then [`parse_*`] lex their input and report skeleton status honestly.

#![forbid(unsafe_code)]

pub mod ast;
pub mod lexer;

use lexer::{LexError, Tok};

fn tokenize(src: &str) -> Result<Vec<Result<(usize, Tok, usize), LexError>>, String> {
    lexer::tokenize(src)
        .map(|toks| toks.into_iter().map(Ok).collect())
        .map_err(|e: LexError| e.to_string())
}

fn skeleton<T>(what: &str, src: &str) -> Result<T, String> {
    let _ = tokenize(src)?;
    Err(format!("{what}: skeleton parser (LALRPOP tables not generated yet, see GRAMMAR.md)"))
}

/// Parse a complete Sail file (`file` in `parser.mly`).
pub fn parse_file(src: &str) -> Result<Vec<ast::Def>, String> {
    skeleton("parse_file", src)
}

/// Parse a type (`typ_eof`).
pub fn parse_typ(src: &str) -> Result<ast::Typ, String> {
    skeleton("parse_typ", src)
}

/// Parse a type scheme (`typschm_eof`).
pub fn parse_typschm(src: &str) -> Result<ast::TypSchm, String> {
    skeleton("parse_typschm", src)
}

/// Parse an expression (`exp_eof`).
pub fn parse_exp(src: &str) -> Result<ast::Exp, String> {
    skeleton("parse_exp", src)
}

/// Parse a pattern (`pat` + EOF).
pub fn parse_pat(src: &str) -> Result<ast::Pat, String> {
    skeleton("parse_pat", src)
}

/// Parse a single definition (`def_eof`).
pub fn parse_def(src: &str) -> Result<ast::Def, String> {
    skeleton("parse_def", src)
}
