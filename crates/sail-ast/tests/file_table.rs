#![forbid(unsafe_code)]
//! Central driver file table as code (Lane 72, Lane-61-Follow-up).
//!
//! E3: reiner Code, hier nicht ausgefuehrt. Prueft die Lane-72-Entscheide:
//! - `FileId(n)` ist der Index der n-ten per [`FileTable::add`] registrierten
//!   Datei (`FileId(0)` == erste Datei: Lane-63-Bruecken-Invariante).
//! - [`FileTable::line_col`] loest Byte-Offsets in 1-basige
//!   `(Zeile, Spalte)` auf (Byte-Spalten, passend zum `Span`-Vertrag).
//! - [`FileTable::snippet`] rendert den `sail-diag`-kompatiblen
//!   Quelltext-Auszug (`>2: …`-Markierung).
//! - `FileTable` ist JSON-stabil (`from_str(to_string(t)) == t`).

use sail_ast::{FileId, FileTable, Span};

fn two_files() -> FileTable {
    let mut t = FileTable::new();
    assert!(t.is_empty());
    t.add("a.sail", "val x : int\n");
    t.add("b.sail", "line one\nline two\nline three\n");
    assert_eq!(t.len(), 2);
    t
}

#[test]
fn file_ids_follow_insertion_order() {
    let t = two_files();
    assert_eq!(t.path(FileId(0)), Some("a.sail"));
    assert_eq!(t.path(FileId(1)), Some("b.sail"));
    assert_eq!(t.path(FileId(2)), None);
    assert_eq!(
        t.source(FileId(0)),
        Some("val x : int\n"),
        "source text stays attached to its id"
    );
}

#[test]
fn line_col_resolves_byte_offsets() {
    let t = two_files();
    // "line one\nline two\nline three\n": line 2 starts at byte 9.
    assert_eq!(t.line_col(FileId(1), 0), Some((1, 1)));
    assert_eq!(t.line_col(FileId(1), 9), Some((2, 1)));
    assert_eq!(t.line_col(FileId(1), 13), Some((2, 5)));
    assert_eq!(t.span_lines(Span::new(FileId(1), 9, 13)), Some(((2, 1), (2, 5))));
}

#[test]
fn unknown_or_dummy_locations_yield_none() {
    let t = two_files();
    assert_eq!(t.line_col(FileId(9), 0), None);
    assert_eq!(t.line_col(FileId(1), u32::MAX), None);
    assert_eq!(t.span_lines(Span::DUMMY), None);
    assert_eq!(t.span_lines(Span::new(FileId(9), 0, 1)), None);
    assert_eq!(t.line_text(FileId(9), 1), None);
    assert_eq!(t.line_text(FileId(1), 99), None);
    assert_eq!(t.snippet(Span::DUMMY, 1), None);
}

#[test]
fn snippet_matches_diag_format() {
    let t = two_files();
    let snip = t
        .snippet(Span::new(FileId(1), 9, 13), 1)
        .expect("snippet for a valid span");
    assert!(snip.contains(">2: line two"), "{snip}");
    assert!(snip.contains(" 1: line one"), "{snip}");
    assert!(snip.contains(" 3: line three"), "{snip}");
}

#[test]
fn file_table_json_roundtrip() {
    let t = two_files();
    let json = serde_json::to_string(&t).expect("FileTable serializes to JSON");
    let back: FileTable =
        serde_json::from_str(&json).expect("FileTable deserializes from JSON");
    assert_eq!(t, back);
    // Line index survives the roundtrip (recomputed data stays attached).
    assert_eq!(back.line_col(FileId(1), 9), Some((2, 1)));
}
