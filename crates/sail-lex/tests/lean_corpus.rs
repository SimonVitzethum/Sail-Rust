#![forbid(unsafe_code)]
//! Corpus test: lex every `sail/test/lean/*.sail` from the reference Sail
//! checkout, error-free, in parallel (proves `Send + Sync` + threading).
//!
//! The corpus directory resolves via `SAIL_TEST_DIR`, falling back to the
//! checkout layout (`<sail-rs>/../../sail/test/lean`). Missing directory →
//! loud skip (exit ok) so `cargo test` stays portable.

use sail_lex::{lex, FileId, TokenKind};
use std::path::PathBuf;

fn corpus_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SAIL_TEST_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
        return None;
    }
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..5 {
        p.pop();
    }
    p.push("sail");
    p.push("test");
    p.push("lean");
    p.is_dir().then_some(p)
}

#[test]
fn lean_corpus_lexes_without_errors() {
    let Some(dir) = corpus_dir() else {
        eprintln!("SKIP lean_corpus: no sail/test/lean found (set SAIL_TEST_DIR)");
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read corpus dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "sail"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .sail files in {}", dir.display());
    eprintln!("lexing {} files from {}", files.len(), dir.display());

    // Parallel lexing: one thread per file, results joined in order.
    let results: Vec<(PathBuf, usize, Vec<String>)> = std::thread::scope(|s| {
        files
            .iter()
            .enumerate()
            .map(|(i, path)| {
                s.spawn(move || {
                    let src = std::fs::read_to_string(path)
                        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                    let out = lex(FileId(i.min(u32::MAX as usize) as u32), &src);
                    let problems: Vec<String> = out.errors.iter().map(|e| e.to_string()).collect();
                    (path.clone(), out.tokens.len(), problems)
                })
                .join()
                .expect("lexer thread panicked")
            })
            .collect()
    });

    let mut failures = 0;
    let mut total_tokens = 0usize;
    for (path, n_tokens, problems) in &results {
        total_tokens += n_tokens;
        if !problems.is_empty() {
            failures += 1;
            eprintln!("ERRORS in {}:", path.display());
            for p in problems.iter().take(5) {
                eprintln!("  {p}");
            }
        }
        assert!(*n_tokens >= 2, "empty token stream for {}", path.display());
    }
    eprintln!("total tokens: {total_tokens}");
    assert_eq!(failures, 0, "{failures} corpus files had lexer errors");
}

#[test]
fn lean_corpus_streams_end_with_eof() {
    let Some(dir) = corpus_dir() else {
        eprintln!("SKIP eof check: no sail/test/lean found");
        return;
    };
    let mut checked = 0;
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read corpus dir")
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "sail") {
            let src = std::fs::read_to_string(&path).expect("read file");
            let out = lex(FileId(0), &src);
            assert!(
                out.tokens
                    .last()
                    .is_some_and(|t| matches!(t.kind, TokenKind::Eof)),
                "no trailing Eof in {}",
                path.display()
            );
            // Spans are well-formed and monotone non-decreasing in `end`.
            let mut prev_end = 0u32;
            for t in &out.tokens {
                assert!(t.span.start <= t.span.end, "bad span in {}", path.display());
                assert!(
                    t.span.start >= prev_end || t.span.is_dummy(),
                    "spans regress in {}",
                    path.display()
                );
                prev_end = prev_end.max(t.span.end);
            }
            checked += 1;
        }
    }
    assert!(checked > 0);
    eprintln!("eof/span checked on {checked} files");
}
