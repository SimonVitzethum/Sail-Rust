//! Differential tests against the reference backend corpus
//! (`sail/test/lean/*.expected.lean`).
//!
//! These tests do not re-run the Sail frontend; they pin the structural
//! invariants this crate's [`sail_typedef`] printer guarantees, so any drift
//! in the reference output (or in our port) turns red:
//!
//! - **E2**: no `sorry` in any expected file.
//! - Every `inductive` block generated from a Sail enum/union (everything
//!   except the `Register` inductive, which belongs to the register lane and
//!   uses `DecidableEq` deriving) is followed by a `deriving … Repr` line and
//!   an `open <Name>` line for the same name.
//! - Every `structure` block carries `deriving` with `BEq`, `Inhabited` and
//!   `Repr`.
//! - `fp_bits` is never emitted as an `abbrev` (documented skip).
//! - `bitfield.expected.lean` shows the desugar assumption this crate's
//!   `TD_bitfield` error documents: `bitfield cr_type` arrives as
//!   `abbrev cr_type := (BitVec 8)`.
//!
//! The corpus directory is located by walking up from `CARGO_MANIFEST_DIR`
//! for an ancestor containing `sail/test/lean`. If no Sail checkout is
//! present the tests pass vacuously with a printed notice.

use std::path::{Path, PathBuf};

fn corpus_dir() -> Option<PathBuf> {
    let mut dir: &Path = Path::new(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = dir.join("sail/test/lean");
        if candidate.is_dir() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

fn expected_files() -> Vec<PathBuf> {
    let Some(dir) = corpus_dir() else {
        println!("SKIP: no sail/test/lean checkout found above CARGO_MANIFEST_DIR");
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("corpus dir readable")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "lean"))
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".expected.lean"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "corpus dir has no *.expected.lean files");
    files
}

/// Defs section: everything before the `XXXXXXXXX` separator line, or `None`
/// for files in the newer single-section format (e.g. `try_catch`).
fn defs_section(text: &str) -> Option<Vec<&str>> {
    let mut section = Vec::new();
    for line in text.lines() {
        if line.trim() == "XXXXXXXXX" {
            return Some(section);
        }
        section.push(line);
    }
    None
}

#[test]
fn corpus_has_no_sorry() {
    for path in expected_files() {
        let text = std::fs::read_to_string(&path).expect("expected file readable");
        assert!(
            !text.contains("sorry"),
            "{}: E2 violation — contains `sorry`",
            path.display()
        );
    }
}

#[test]
fn corpus_never_abbrevs_fp_bits() {
    for path in expected_files() {
        let text = std::fs::read_to_string(&path).expect("expected file readable");
        for line in text.lines() {
            assert!(
                !(line.starts_with("abbrev fp_bits")),
                "{}: fp_bits must be skipped, found {line:?}",
                path.display()
            );
        }
    }
}

#[test]
fn inductive_blocks_have_deriving_and_open() {
    for path in expected_files() {
        let text = std::fs::read_to_string(&path).expect("expected file readable");
        let Some(defs) = defs_section(&text) else {
            continue;
        };
        let mut index = 0;
        while index < defs.len() {
            let line = defs[index];
            let name = line
                .strip_prefix("inductive ")
                .and_then(|rest| rest.split_whitespace().next());
            // `Register` inductives belong to the register lane
            // (`DecidableEq` deriving, unindented `open`); skip them here.
            if let Some(name) = name {
                if name != "Register" {
                    let mut cursor = index + 1;
                    while cursor < defs.len() && !defs[cursor].starts_with("  deriving") {
                        let body = defs[cursor];
                        let is_continuation = body.is_empty()
                            || body.starts_with("  |")
                            || body.starts_with("  (")
                            || body.starts_with("/--");
                        assert!(
                            is_continuation,
                            "{}:{}: unexpected line inside `inductive {name}` block: {body:?}",
                            path.display(),
                            index + 1
                        );
                        cursor += 1;
                    }
                    assert!(
                        cursor < defs.len(),
                        "{}:{}: `inductive {name}` without deriving",
                        path.display(),
                        index + 1
                    );
                    assert!(
                        defs[cursor].contains("Repr"),
                        "{}:{}: `inductive {name}` deriving lacks Repr: {:?}",
                        path.display(),
                        index + 1,
                        defs[cursor]
                    );
                    let mut next = cursor + 1;
                    while next < defs.len() && defs[next].trim().is_empty() {
                        next += 1;
                    }
                    assert!(
                        next < defs.len() && defs[next] == format!("  open {name}"),
                        "{}:{}: `inductive {name}` not followed by `  open {name}`, found {:?}",
                        path.display(),
                        index + 1,
                        defs.get(next)
                    );
                    index = next;
                }
            }
            index += 1;
        }
    }
}

#[test]
fn structure_blocks_derive_beq_inhabited_repr() {
    for path in expected_files() {
        let text = std::fs::read_to_string(&path).expect("expected file readable");
        let Some(defs) = defs_section(&text) else {
            continue;
        };
        let mut index = 0;
        while index < defs.len() {
            if let Some(rest) = defs[index].strip_prefix("structure ") {
                let name = rest.split_whitespace().next().unwrap_or("<anon>");
                let mut cursor = index + 1;
                while cursor < defs.len() && !defs[cursor].starts_with("  deriving") {
                    cursor += 1;
                }
                assert!(
                    cursor < defs.len(),
                    "{}:{}: `structure {name}` without deriving",
                    path.display(),
                    index + 1
                );
                for needed in ["BEq", "Inhabited", "Repr"] {
                    assert!(
                        defs[cursor].contains(needed),
                        "{}:{}: `structure {name}` deriving lacks {needed}: {:?}",
                        path.display(),
                        index + 1,
                        defs[cursor]
                    );
                }
                index = cursor;
            }
            index += 1;
        }
    }
}

#[test]
fn bitfield_desugars_to_abbrev_upstream() {
    let files = expected_files();
    if files.is_empty() {
        return;
    }
    let dir = corpus_dir().expect("corpus present");
    let text = std::fs::read_to_string(dir.join("bitfield.expected.lean"))
        .expect("bitfield.expected.lean readable");
    let Some(defs) = defs_section(&text) else {
        panic!("bitfield.expected.lean has no Defs section");
    };
    assert!(
        defs.contains(&"abbrev cr_type := (BitVec 8)"),
        "bitfield `cr_type` must arrive as an abbrev (desugared before Lean printing)"
    );
}
