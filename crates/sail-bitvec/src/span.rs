#![forbid(unsafe_code)]
//! Source spans for [`crate`] diagnostics.
//!
//! Minimal copy of the lane-01 [`Span`] shape (file handle plus half-open
//! byte range) so this crate stays dependency-free and self-contained.
//! File-table ownership stays with a later lane (parser/driver).

/// Handle into a file table owned elsewhere (later lane).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

/// Half-open byte range `[start, end)` inside [`FileId`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// Placeholder for synthesized nodes.
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

impl core::fmt::Display for Span {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_dummy() {
            write!(f, "<synthetic>")
        } else {
            write!(f, "file{}:{}-{}", self.file.0, self.start, self.end)
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
    }
}
