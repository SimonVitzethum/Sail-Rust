#![forbid(unsafe_code)]
//! Minimal source spans for diagnostics.
//!
//! These mirror `sail-ast::{FileId, Span}` and `sail-lex::{FileId, Span}`
//! field-for-field so a later lane can unify them into one shared type
//! without changing call sites: `sail-diag` owns no file table, [`FileId`]
//! is an opaque handle supplied by the caller (driver/frontend lane).

use std::fmt;

/// Handle into a file table owned elsewhere (driver/parser lane).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "file{}", self.0)
    }
}

/// Half-open byte range `[start, end)` inside [`FileId`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// Placeholder for synthesized locations.
    pub const DUMMY: Span = Span {
        file: FileId(u32::MAX),
        start: u32::MAX,
        end: u32::MAX,
    };

    #[must_use]
    pub fn new(file: FileId, start: u32, end: u32) -> Self {
        debug_assert!(start <= end);
        Span { file, start, end }
    }

    #[must_use]
    pub fn dummy() -> Self {
        Span::DUMMY
    }

    #[must_use]
    pub fn is_dummy(self) -> bool {
        self == Span::DUMMY
    }

    /// Smallest span covering both, or `DUMMY` if files differ.
    #[must_use]
    pub fn merge(self, other: Span) -> Span {
        if self.is_dummy() {
            return other;
        }
        if other.is_dummy() {
            return self;
        }
        if self.file != other.file {
            return Span::DUMMY;
        }
        Span {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dummy() {
            write!(f, "<no location>")
        } else {
            write!(f, "{}:{}-{}", self.file, self.start, self.end)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_covers_both_spans() {
        let a = Span::new(FileId(1), 2, 5);
        let b = Span::new(FileId(1), 4, 9);
        assert_eq!(a.merge(b), Span::new(FileId(1), 2, 9));
    }

    #[test]
    fn merge_across_files_is_dummy() {
        let a = Span::new(FileId(1), 0, 1);
        let b = Span::new(FileId(2), 0, 1);
        assert!(a.merge(b).is_dummy());
        assert_eq!(Span::dummy().merge(a), a);
    }

    #[test]
    fn display_dummy_and_ranged() {
        assert_eq!(Span::dummy().to_string(), "<no location>");
        assert_eq!(Span::new(FileId(3), 12, 34).to_string(), "file3:12-34");
    }
}
