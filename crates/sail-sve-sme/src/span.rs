//! Source spans and spanned parse errors.
//!
//! Every AST node produced by [`crate::parser`] carries a [`Span`] so that
//! later lanes (name resolution, typecheck, diagnostics) can report errors
//! with file + line + column. Spans are byte offsets plus line/column for
//! display; they never hold references into the source (low-RAM friendly,
//! `Send + Sync`).

use std::fmt;

/// A half-open byte range `[start_byte, end_byte)` in one source file,
/// with 1-based line/column for both ends.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Span {
    /// File path as passed to [`crate::parse_file`] (not canonicalized).
    pub file: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl Span {
    pub fn new(
        file: &str,
        start_byte: usize,
        end_byte: usize,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    ) -> Self {
        Span {
            file: file.to_owned(),
            start_byte,
            end_byte,
            start_line,
            start_col,
            end_line,
            end_col,
        }
    }

    /// Zero-width span at a cursor position (used for EOF errors).
    pub fn point(file: &str, byte: usize, line: u32, col: u32) -> Self {
        Span::new(file, byte, byte, line, col, line, col)
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}-{}:{}",
            self.file, self.start_line, self.start_col, self.end_line, self.end_col
        )
    }
}

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
