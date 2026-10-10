#![forbid(unsafe_code)]
//! Standalone Sail2 lexer (Lane 16): bytes in, token stream plus errors out.
//!
//! - [`lex`] turns source text into a [`LexResult`] (tokens + recovery errors).
//! - [`Span`]/[`FileId`] are minimal local types, field-compatible with
//!   `sail-abstractions` from Lane 01; see `LEXER_NEEDED.md` for the swap.
//! - The lexer never fails: see the error policy in [`lexer`].
//!
//! No grammar lives here (Lane 17); this crate only produces the token stream.

mod lexer;
mod span;
mod token;

pub use lexer::{LexError, LexResult, lex};
pub use span::{FileId, Span};
pub use token::{Fixity, Token, TokenKind};
