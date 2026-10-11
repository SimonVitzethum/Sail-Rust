#![forbid(unsafe_code)]
//! Central driver file table: path + source text + line resolution (Lane 72).
//!
//! Lane-61 rest item (ARCHITECTURE.md #3, #25): spans are bare byte ranges
//! (`FileId` + `[start, end)`), while paths lived on
//! `sail-parse::ParsedFile` and every diagnostic site recomputed line/column
//! itself (e.g. `sail-diag::SailError::render`, the `sail-sve-sme` line
//! table). This table is the single owner of that mapping:
//!
//! - [`FileTable::add`] hands out the [`FileId`]; the `n`-th added file is
//!   `FileId(n)`, so the Lane-63 single-file bridge invariant
//!   (`FileId(0)` == first file) holds by construction.
//! - [`FileTable::line_col`] / [`FileTable::span_lines`] resolve byte
//!   offsets to 1-based `(line, col)` with byte columns (matches the
//!   `Span` byte-range contract; never panics, `None` when unknown/OOB).
//! - [`FileTable::line_text`] / [`FileTable::snippet`] render source excerpts
//!   so diagnostics need no ad-hoc line splitting anymore (future consumer:
//!   `sail-diag::SailError::render`; that crate stays dependency-free, so the
//!   table lives here next to the `FileId` authority, not there).
//!
//! Pure data (no parsing, no IO): fits the `sail-ast` contract. All types are
//! `Clone + Send + Sync` and `serde`-serializable.

use serde::{Deserialize, Serialize};

use crate::span::{FileId, Span};

fn sat_u32(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// Byte offsets where each line starts (`[0, ..]`; entry `i` starts line
/// `i + 1`). Always non-empty, even for empty sources.
fn compute_line_starts(source: &str) -> Vec<u32> {
    let bytes = source.as_bytes();
    let mut starts = Vec::with_capacity(8);
    starts.push(0u32);
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' && i + 1 < bytes.len() {
            starts.push(sat_u32(i + 1));
        }
    }
    starts
}

/// One source file owned by a [`FileTable`]: path + full text + line index.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInfo {
    /// Display path (as passed to [`FileTable::add`]; never read from disk).
    pub path: String,
    /// Full source text; [`Span`] byte offsets index into this.
    pub source: String,
    line_starts: Vec<u32>,
}

impl FileInfo {
    fn new(path: String, source: String) -> Self {
        let line_starts = compute_line_starts(&source);
        FileInfo {
            path,
            source,
            line_starts,
        }
    }

    /// Number of lines (1-based maximum for [`FileTable::line_text`]).
    #[must_use]
    pub fn line_count(&self) -> u32 {
        sat_u32(self.line_starts.len())
    }
}

/// Central file table: `FileId.0` is the insertion index (first added file
/// is `FileId(0)`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTable {
    files: Vec<FileInfo>,
}

impl FileTable {
    /// Empty table (no files; every lookup yields `None`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a file; returns its [`FileId`] (`FileId(self.len())` before
    /// the push, i.e. `FileId(0)` for the first file).
    pub fn add(&mut self, path: impl Into<String>, source: impl Into<String>) -> FileId {
        let id = FileId(sat_u32(self.files.len()));
        self.files.push(FileInfo::new(path.into(), source.into()));
        id
    }

    /// Number of registered files.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether no file is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// File info for `id`, or `None` when out of range.
    #[must_use]
    pub fn get(&self, id: FileId) -> Option<&FileInfo> {
        self.files.get(id.0 as usize)
    }

    /// Path of `id`, or `None` when out of range.
    #[must_use]
    pub fn path(&self, id: FileId) -> Option<&str> {
        self.get(id).map(|f| f.path.as_str())
    }

    /// Source text of `id`, or `None` when out of range.
    #[must_use]
    pub fn source(&self, id: FileId) -> Option<&str> {
        self.get(id).map(|f| f.source.as_str())
    }

    /// 1-based `(line, col)` for byte `offset` in `id` (byte column).
    ///
    /// `None` for unknown files or offsets past the end (`offset ==
    /// source.len()` is the valid EOF position). Never panics.
    #[must_use]
    pub fn line_col(&self, id: FileId, offset: u32) -> Option<(u32, u32)> {
        let file = self.get(id)?;
        let off = offset as usize;
        if off > file.source.len() {
            return None;
        }
        let idx = file.line_starts.partition_point(|s| (*s as usize) <= off);
        let line_idx = idx.saturating_sub(1);
        let line = sat_u32(line_idx + 1);
        let col = sat_u32(off.saturating_sub(file.line_starts[line_idx] as usize) + 1);
        Some((line, col))
    }

    /// 1-based `((start_line, start_col), (end_line, end_col))` for `span`.
    ///
    /// `None` for dummy spans, unknown files, or out-of-range endpoints.
    #[must_use]
    pub fn span_lines(&self, span: Span) -> Option<((u32, u32), (u32, u32))> {
        if span.is_dummy() {
            return None;
        }
        Some((self.line_col(span.file, span.start)?, self.line_col(span.file, span.end)?))
    }

    /// Text of 1-based `line` in `id` (without the trailing `\n`).
    ///
    /// `None` for unknown files or out-of-range lines. Never panics, also
    /// with multi-byte characters (line boundaries are `\n`-aligned).
    #[must_use]
    pub fn line_text(&self, id: FileId, line: u32) -> Option<&str> {
        let file = self.get(id)?;
        let idx = line.checked_sub(1)? as usize;
        let start = *file.line_starts.get(idx)? as usize;
        let end = file
            .line_starts
            .get(idx + 1)
            .map(|s| (*s as usize).saturating_sub(1))
            .unwrap_or(file.source.len());
        let text = file.source.get(start..end)?;
        Some(text.strip_suffix('\n').unwrap_or(text))
    }

    /// Source excerpt around the span start: the containing line plus
    /// `context` lines on each side (`>` marks the span line, ` ` the rest).
    ///
    /// `None` for dummy spans, unknown files, or out-of-range offsets.
    /// Format mirrors `sail-diag::SailError::render` (`>2: line two`) so that
    /// call site can migrate without output churn.
    #[must_use]
    pub fn snippet(&self, span: Span, context: u32) -> Option<String> {
        if span.is_dummy() {
            return None;
        }
        let file = self.get(span.file)?;
        let (line, _) = self.line_col(span.file, span.start)?;
        let total = file.line_count();
        let from = line.saturating_sub(context + 1).max(1);
        let to = line.saturating_add(context).min(total).max(from);
        let mut out = String::new();
        for l in from..=to {
            let marker = if l == line { ">" } else { " " };
            out.push_str(&format!("{marker}{l}: {}\n", self.line_text(span.file, l)?));
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FileTable {
        let mut t = FileTable::new();
        t.add("a.sail", "line one\nline two\nline three\n");
        t
    }

    #[test]
    fn first_file_is_id_zero() {
        let mut t = FileTable::new();
        assert_eq!(t.add("a.sail", "x"), FileId(0));
        assert_eq!(t.add("b.sail", "y"), FileId(1));
        assert_eq!(t.path(FileId(0)), Some("a.sail"));
        assert_eq!(t.path(FileId(7)), None);
    }

    #[test]
    fn line_col_is_one_based_byte_columns() {
        let t = sample();
        assert_eq!(t.line_col(FileId(0), 0), Some((1, 1)));
        assert_eq!(t.line_col(FileId(0), 9), Some((2, 1)));
        assert_eq!(t.line_col(FileId(0), 12), Some((2, 4)));
        // EOF position is valid (last line, column past its end); past-EOF is not.
        assert_eq!(t.line_col(FileId(0), 29), Some((3, 12)));
        assert_eq!(t.line_col(FileId(0), 30), None);
        assert_eq!(t.line_col(FileId(9), 0), None);
    }

    #[test]
    fn line_col_survives_multibyte_chars() {
        let mut t = FileTable::new();
        t.add("u.sail", "caf\u{e9}\nx\n");
        // `é` is 2 bytes: second line starts at byte 6, columns stay byte-based.
        assert_eq!(t.line_col(FileId(0), 6), Some((2, 1)));
        assert_eq!(t.line_text(FileId(0), 1), Some("caf\u{e9}"));
        assert_eq!(t.line_text(FileId(0), 2), Some("x"));
        assert_eq!(t.line_text(FileId(0), 3), None);
    }

    #[test]
    fn span_lines_rejects_dummy_and_oob() {
        let t = sample();
        let span = Span::new(FileId(0), 9, 13);
        assert_eq!(t.span_lines(span), Some(((2, 1), (2, 5))));
        assert_eq!(t.span_lines(Span::DUMMY), None);
        assert_eq!(t.span_lines(Span::new(FileId(0), 999, 1000)), None);
        assert_eq!(t.span_lines(Span::new(FileId(4), 0, 1)), None);
    }

    #[test]
    fn snippet_marks_span_line_with_context() {
        let t = sample();
        let span = Span::new(FileId(0), 9, 13); // inside "line two"
        let snip = t.snippet(span, 1).expect("snippet");
        assert!(snip.contains(">2: line two"), "{snip}");
        assert!(snip.contains(" 1: line one"), "{snip}");
        assert!(snip.contains(" 3: line three"), "{snip}");
        assert_eq!(t.snippet(Span::DUMMY, 1), None);
        assert_eq!(t.snippet(Span::new(FileId(4), 0, 1), 1), None);
    }

    #[test]
    fn empty_source_has_one_empty_line() {
        let mut t = FileTable::new();
        t.add("e.sail", "");
        assert_eq!(t.line_col(FileId(0), 0), Some((1, 1)));
        assert_eq!(t.line_text(FileId(0), 1), Some(""));
    }
}
