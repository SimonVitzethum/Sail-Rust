//! Source spans and spanned parse errors.
//!
//! Spans are the canonical `sail-ast` type (Lane 61 unification of
//! MIGRATION.md #3; previously a 7-field struct with a path `String`
//! plus 1-based line/column, removed). The file path stays on
//! [`crate::mini_ast::ParsedFile`]; line/column resolution moves to a
//! file table owned by a later (driver) lane, so spans are byte ranges
//! (`FileId` is `FileId(0)` per `parse_file` call until that table lands).
//!
//! [`Spanned`] and [`ParseError`] are kept here: they belong to Lane
//! error-diagnostics (35), not `sail-ast` (MIGRATION.md §3).
//!
//! Line/column resolution lives in the central [`sail_ast::FileTable`]
//! (Lane 72); the `FileId(0)`-per-`parse_file` convention matches its
//! insertion order (first added file is `FileId(0)`).

use std::fmt;

pub use sail_ast::{FileId, Span};

/// A value with its source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(value: T, span: Span) -> Self {
        Spanned { value, span }
    }
}

/// Parse failure with exact location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub span: Span,
    pub message: String,
}

impl ParseError {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        ParseError { span, message: message.into() }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error at {}: {}", self.span, self.message)
    }
}

impl std::error::Error for ParseError {}
