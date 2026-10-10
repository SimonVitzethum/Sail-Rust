#![forbid(unsafe_code)]
//! Span-carrying width errors: mismatches are reported, never silently truncated.
//!
//! Port of the `Reporting.unreachable` width failures in
//! `pretty_print_lean.ml` (`doc_vector_concat`, `doc_pat`), turned into
//! recoverable [`BitvecError`] values that always carry the offending [`Span`].

use crate::span::Span;
use core::fmt;

/// Machine-checkable reason for a bitvector error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Declared width and actual width differ (no silent truncation/padding).
    WidthMismatch {
        /// Width demanded by the context (target type, concat sum, ...).
        expected: u32,
        /// Width actually found.
        found: u32,
    },
    /// A sub-pattern/expression has no statically known width
    /// (OCaml: "Found subpattern with unclear width in bitvector pattern").
    UnclearWidth,
    /// A node kind cannot appear inside a bitvector concat pattern
    /// (OCaml: "Unexpected pattern in match_bv vector_concat pattern").
    UnexpectedPattern,
    /// Bitvector literal with zero bits where at least one was required.
    EmptyVector,
    /// A bitfield range is inverted (`hi < lo`) or escapes the parent width.
    InvalidRange,
    /// A literal character is not a valid bit/nibble.
    InvalidDigit,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ErrorKind::WidthMismatch { expected, found } => write!(
                f,
                "width mismatch: expected {expected} bits, found {found} bits"
            ),
            ErrorKind::UnclearWidth => write!(
                f,
                "subpattern with unclear width in bitvector pattern"
            ),
            ErrorKind::UnexpectedPattern => {
                write!(f, "unexpected pattern in match_bv vector_concat pattern")
            }
            ErrorKind::EmptyVector => write!(f, "empty bitvector where bits were required"),
            ErrorKind::InvalidRange => write!(f, "invalid bitfield range"),
            ErrorKind::InvalidDigit => write!(f, "invalid digit in bitvector literal"),
        }
    }
}

/// A bitvector semantic error with source location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitvecError {
    pub kind: ErrorKind,
    pub span: Span,
    /// Human context, e.g. the offending identifier or pattern shape.
    pub message: String,
}

impl BitvecError {
    #[must_use]
    pub fn new(kind: ErrorKind, span: Span, message: impl Into<String>) -> Self {
        BitvecError {
            kind,
            span,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn width_mismatch(expected: u32, found: u32, span: Span, message: impl Into<String>) -> Self {
        BitvecError::new(ErrorKind::WidthMismatch { expected, found }, span, message)
    }

    #[must_use]
    pub fn unclear_width(span: Span, message: impl Into<String>) -> Self {
        BitvecError::new(ErrorKind::UnclearWidth, span, message)
    }

    #[must_use]
    pub fn unexpected_pattern(span: Span, message: impl Into<String>) -> Self {
        BitvecError::new(ErrorKind::UnexpectedPattern, span, message)
    }
}

impl fmt::Display for BitvecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.message.is_empty() {
            write!(f, "{} at {}", self.kind, self.span)
        } else {
            write!(f, "{} at {}: {}", self.kind, self.span, self.message)
        }
    }
}

impl std::error::Error for BitvecError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::FileId;

    #[test]
    fn display_carries_span_and_widths() {
        let span = Span::new(FileId(0), 4, 9);
        let err = BitvecError::width_mismatch(32, 30, span, "decode clause");
        let text = err.to_string();
        assert!(text.contains("32"), "{text}");
        assert!(text.contains("30"), "{text}");
        assert!(text.contains("file0:4-9"), "{text}");
    }
}
