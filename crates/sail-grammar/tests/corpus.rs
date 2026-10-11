//! Corpus test over the Sail standard libraries (Lane 59 task (c)).
//!
//! NOTE (E3): this test is written as code but NOT executed in this lane —
//! no local `cargo` runs. Verification happens later via the Test-Rig
//! (`TESTING.md`, e.g. `scripts/limited-run.sh`), which runs
//! `cargo test -p sail-grammar`. The test parses every `.sail` file in
//! `crates/sail-prelude/sail-lib` with [`sail_grammar::parse_file`] and
//! fails listing per-file errors, so Rig output directly shows:
//! - E3c/E4 acceptance deviations (open-`if` tails, bare `let`/`var`
//!   outside blocks, `{}`),
//! - D1/D2 gaps (`Typ` index-range vs tuple, paren vs tuple — those rules
//!   are intentionally unchanged in this lane and evaluated here).
//!
//! Workspace fixtures outside `sail-lib` (e.g. `sail-parse` fixtures) are
//! deliberately NOT included: they target the Sail2 front end
//! (`sail-parse`), not this Menhir (`parser.mly`) port.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

fn sail_lib_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sail-prelude/sail-lib")
}

fn collect_sail(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_sail(&path, out);
        } else if path.extension().map(|e| e == "sail").unwrap_or(false) {
            out.push(path);
        }
    }
}

#[test]
fn corpus_sail_lib_parses() {
    let mut files = Vec::new();
    collect_sail(&sail_lib_dir(), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no .sail corpus files found");
    let mut failures = Vec::new();
    for path in &files {
        let src =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        if let Err(e) = sail_grammar::parse_file(&src) {
            failures.push(format!("{}: {}", path.display(), e));
        }
    }
    assert!(
        failures.is_empty(),
        "corpus failures ({} files, {} failed):\n{}",
        files.len(),
        failures.len(),
        failures.join("\n")
    );
}
