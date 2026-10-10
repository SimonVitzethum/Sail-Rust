//! Sail grammar ported from Menhir (`sail/src/lib/parser.mly`).
//!
//! Pure Rust (`#![forbid(unsafe_code)]`). This crate is the skeleton stage of
//! the port: it provides the skeletal AST ([`ast`]), the hand-written lexer
//! ([`lexer`]), and the LALRPOP grammar file (`src/sail.lalrpop`) as a
//! documentation artifact. The generated parser is not wired up yet because
//! the direct Menhir port has LALR(1) conflicts that LALRPOP rejects (Menhir
//! resolves them as shift); see `GRAMMAR.md`. The [`parse_*`] entry points
//! therefore lex their input (so lexical errors still surface) and then
//! report that the skeleton parser is not yet available. Semantic actions
//! are constructors only — no simplification, desugaring, deprecation
//! warnings, or custom errors from the OCaml actions.

#![forbid(unsafe_code)]

pub mod ast;
pub mod lexer;

use lexer::LexError;

/// Error returned by the [`parse_*`] entry points while the crate is a
/// skeleton (lexer + AST only, no generated parser yet).
pub const SKELETON_MSG: &str =
    "sail-grammar skeleton: parser tables not generated yet (see GRAMMAR.md)";

fn tokens(src: &str) -> Result<Vec<(usize, lexer::Tok, usize)>, String> {
    lexer::tokenize(src).map_err(|e: LexError| e.to_string())
}

fn skeleton<T>(src: &str) -> Result<T, String> {
    let _ = tokens(src)?;
    Err(SKELETON_MSG.to_string())
}

/// Parse a complete Sail file (`file` in `parser.mly`).
pub fn parse_file(src: &str) -> Result<Vec<ast::Def>, String> {
    skeleton(src)
}

/// Parse a type (`typ_eof`).
pub fn parse_typ(src: &str) -> Result<ast::Typ, String> {
    skeleton(src)
}

/// Parse a type scheme (`typschm_eof`).
pub fn parse_typschm(src: &str) -> Result<ast::TypSchm, String> {
    skeleton(src)
}

/// Parse an expression (`exp_eof`).
pub fn parse_exp(src: &str) -> Result<ast::Exp, String> {
    skeleton(src)
}

/// Parse a pattern (`pat` + EOF).
pub fn parse_pat(src: &str) -> Result<ast::Pat, String> {
    skeleton(src)
}

/// Parse a single definition (`def_eof`).
pub fn parse_def(src: &str) -> Result<ast::Def, String> {
    skeleton(src)
}
