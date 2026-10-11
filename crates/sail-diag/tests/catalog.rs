#![forbid(unsafe_code)]
//! Catalog completeness: the `--explain` table must cover every current
//! `failwith`/`unreachable` site in `pretty_print_lean.ml`, and every
//! [`DiagCode`] variant must appear in [`ALL`].
//!
//! An optional drift check pins each row's OCaml line against the reference
//! Sail checkout (`SAIL_CHECKOUT`, else `<sail-rs>/../../sail/...` like the
//! `sail-lex` corpus test). Missing checkout → loud skip so `cargo test`
//! stays portable.

use sail_diag::{explain_table, format_explain, DiagCode, ALL};
use std::collections::HashSet;
use std::path::PathBuf;

/// Match every variant: adding a `DiagCode` without a table row (or vice
/// versa) breaks this function at compile time *and* the count assert below.
fn all_codes() -> Vec<DiagCode> {
    vec![
        DiagCode::NexpNotTranslatable,
        DiagCode::TypNotTranslatable,
        DiagCode::DocPatternNotTranslatable,
        DiagCode::BitvectorPatternWidthUnknown,
        DiagCode::BitvectorPatternUnexpected,
        DiagCode::LoopVarBadExpression,
        DiagCode::WhenClauseUnsupported,
        DiagCode::LoopCombinatorArity,
        DiagCode::LoopVarNotFound,
        DiagCode::ForeachCombinatorArity,
        DiagCode::AssignNotImplemented,
        DiagCode::ExpressionNotTranslatable,
        DiagCode::FunctionWithoutFunctionType,
        DiagCode::FundefEmpty,
        DiagCode::FundefMultiClause,
        DiagCode::TypedefNotTranslatable,
        DiagCode::ValPatternNotTranslatable,
        DiagCode::PatternTypeCombinationUnexpected,
        DiagCode::DefsRecUnreachable,
        DiagCode::CollectImportsUnreachable,
        DiagCode::ImportFilesStackUnderflowEnd,
        DiagCode::ImportFilesStackUnderflowDef,
        DiagCode::InstantiationDepsNotSingle,
    ]
}

#[test]
fn catalog_covers_every_variant_exactly_once() {
    let codes = all_codes();
    // Pinned: 23 failwith/unreachable sites in pretty_print_lean.ml.
    assert_eq!(codes.len(), 23, "update catalog + this test together");
    assert_eq!(ALL.len(), codes.len(), "ALL must hold one row per variant");
    let entries: HashSet<&str> = ALL.iter().map(|e| e.code).collect();
    assert_eq!(entries.len(), ALL.len(), "duplicate codes in ALL");
    for code in codes {
        assert!(
            entries.contains(code.code_str()),
            "variant {code:?} missing from ALL"
        );
    }
}

#[test]
fn every_row_is_explain_ready() {
    for e in ALL {
        assert!(!e.title.is_empty(), "{} needs a title", e.code);
        assert!(!e.trigger.is_empty(), "{} needs a trigger", e.code);
        assert!(!e.workaround.is_empty(), "{} needs a workaround", e.code);
        assert!(
            e.ocaml_site.starts_with("pretty_print_lean.ml:"),
            "{} needs an OCaml site, got {}",
            e.code,
            e.ocaml_site
        );
        let document = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(e.lane);
        assert!(
            e.lane == "WORK_STATE.md" && document.is_file(),
            "{} needs the current work-state document, got {}",
            e.code,
            e.lane
        );
        let text = format_explain(e);
        assert!(text.contains(e.code) && text.contains(e.workaround) && text.contains(e.lane));
    }
    let table = explain_table();
    for e in ALL {
        assert!(table.contains(e.code), "index table hides {}", e.code);
    }
}

fn ocaml_file() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SAIL_CHECKOUT") {
        let p = PathBuf::from(dir).join("src/sail_lean_backend/pretty_print_lean.ml");
        if p.is_file() {
            return Some(p);
        }
        return None;
    }
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..5 {
        p.pop();
    }
    p.push("sail");
    p.push("src");
    p.push("sail_lean_backend");
    p.push("pretty_print_lean.ml");
    p.is_file().then_some(p)
}

fn site_line(site: &str) -> Option<usize> {
    site.strip_prefix("pretty_print_lean.ml:")?
        .split_once(' ')?
        .0
        .parse()
        .ok()
}

#[test]
fn catalog_lines_match_ocaml_failwith_sites() {
    let Some(path) = ocaml_file() else {
        eprintln!("SKIP: no Sail checkout found (set SAIL_CHECKOUT); drift check skipped");
        return;
    };
    let src = std::fs::read_to_string(&path).expect("read pretty_print_lean.ml");
    let lines: Vec<&str> = src.lines().collect();
    for e in ALL {
        let n = site_line(e.ocaml_site)
            .unwrap_or_else(|| panic!("{} has unparsable site {}", e.code, e.ocaml_site));
        let line = lines
            .get(n - 1)
            .unwrap_or_else(|| panic!("{}: line {n} out of range", e.code));
        assert!(
            line.contains("failwith")
                || line.contains("unreachable")
                || line.contains("err_unreachable")
                || line.contains("raise"),
            "{}: {}:{n} no longer a failure site: {line}",
            e.code,
            e.ocaml_site
        );
    }
}
