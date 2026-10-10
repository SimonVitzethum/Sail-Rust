#![forbid(unsafe_code)]
//! Standalone Sail2 lexer (Lane 16): bytes in, token stream plus errors out.
//!
//! - [`lex`] turns source text into a [`LexResult`] (tokens + recovery errors).
//! - [`Span`]/[`FileId`] are re-exports of the canonical `sail-ast` types
//!   (Lane 61 unification of MIGRATION.md #1/#2; previously a field-identical
//!   local copy in `src/span.rs`, removed).
//! - The lexer never fails: see the error policy in [`lexer`].
//!
//! No grammar lives here (Lane 17); this crate only produces the token stream.

mod lexer;
mod token;

pub use lexer::{lex, LexError, LexResult};
pub use sail_ast::{FileId, Span};
pub use token::{Fixity, Token, TokenKind};
