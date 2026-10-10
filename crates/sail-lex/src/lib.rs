#![forbid(unsafe_code)]
//! Standalone Sail2 lexer (Lane 16): bytes in, token stream plus errors out.
//!
//! - [`lex`] turns source text into a [`LexResult`] (tokens + recovery errors).
//!   The caller owns a [`SimpleInterner`] (or any [`Interner`]) and passes it
//!   in: every [`TokenKind`] payload is interned there (Lane 71
//!   payload-intern), so repeated identifiers/literals share one table entry.
//!   Resolve payloads with [`Interner::resolve`].
//! - [`Span`]/[`FileId`] are re-exports of the canonical `sail-ast` types
//!   (Lane 61 unification of MIGRATION.md #1/#2; previously a field-identical
//!   local copy in `src/span.rs`, removed).
//! - The lexer never fails: see the error policy in [`lexer`].
//!
//! No grammar lives here (Lane 17); this crate only produces the token stream.

mod lexer;
mod token;

pub use lexer::{lex, LexError, LexResult};
pub use sail_ast::{FileId, Interner, SimpleInterner, Span, Symbol};
pub use token::{Fixity, Token, TokenKind};
