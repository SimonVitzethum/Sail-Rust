#![forbid(unsafe_code)]
//! Positional comparison of normalized definition sequences.
//!
//! Both frontends elaborate one `.sail` file in source order, so the
//! comparison is positional: entry `i` of the Rust sequence must match entry
//! `i` of the OCaml sequence in kind, name, and header text. Length tails
//! become `MissingInActual` / `ExtraInActual`. The full-text diff itself is
//! rendered from the canonical sequences via `sail-oracle`'s unified diff
//! (see [`render_seq_diff`]).

use crate::norm_tree::{NormDef, render_seq};

/// Verdict for one sequence position (or length tail).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefVerdict {
    Match,
    KindMismatch {
        index: usize,
        expected: &'static str,
        actual: &'static str,
    },
    NameMismatch {
        index: usize,
        expected: Option<String>,
        actual: Option<String>,
    },
    HeadMismatch {
        index: usize,
        name: Option<String>,
    },
    /// Oracle has entries the Rust side did not produce.
    MissingInActual { index: usize, expected: String },
    /// Rust produced entries the oracle dump has no counterpart for.
    ExtraInActual { index: usize, actual: String },
}

impl DefVerdict {
    pub fn is_match(&self) -> bool {
        matches!(self, Self::Match)
    }

    pub fn reason(&self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::KindMismatch { .. } => "def-kind mismatch",
            Self::NameMismatch { .. } => "def-name mismatch",
            Self::HeadMismatch { .. } => "def-header mismatch",
            Self::MissingInActual { .. } => "missing in Rust output",
            Self::ExtraInActual { .. } => "extra in Rust output",
        }
    }
}

/// Full comparison outcome for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeReport {
    pub verdicts: Vec<DefVerdict>,
    pub expected_len: usize,
    pub actual_len: usize,
}

impl TreeReport {
    pub fn is_clean(&self) -> bool {
        self.verdicts.iter().all(|v| v.is_match())
    }

    pub fn mismatch_count(&self) -> usize {
        self.verdicts.iter().filter(|v| !v.is_match()).count()
    }

    pub fn matched_count(&self) -> usize {
        self.verdicts.iter().filter(|v| v.is_match()).count()
    }
}

fn one_line(d: &NormDef) -> String {
    let mut s = String::from(d.kind.label());
    if let Some(n) = &d.name {
        s.push(':');
        s.push_str(n);
    }
    s
}

/// Compare `expected` (oracle) vs `actual` (Rust) positionally.
pub fn compare(expected: &[NormDef], actual: &[NormDef]) -> TreeReport {
    let mut verdicts = Vec::with_capacity(expected.len().max(actual.len()));
    let shared = expected.len().min(actual.len());
    for i in 0..shared {
        let e = &expected[i];
        let a = &actual[i];
        if e.kind != a.kind {
            verdicts.push(DefVerdict::KindMismatch {
                index: i,
                expected: e.kind.label(),
                actual: a.kind.label(),
            });
        } else if e.name != a.name {
            verdicts.push(DefVerdict::NameMismatch {
                index: i,
                expected: e.name.clone(),
                actual: a.name.clone(),
            });
        } else if e.head_norm != a.head_norm {
            verdicts.push(DefVerdict::HeadMismatch {
                index: i,
                name: e.name.clone(),
            });
        } else {
            verdicts.push(DefVerdict::Match);
        }
    }
    for (i, e) in expected.iter().enumerate().skip(shared) {
        verdicts.push(DefVerdict::MissingInActual {
            index: i,
            expected: one_line(e),
        });
    }
    for (i, a) in actual.iter().enumerate().skip(shared) {
        verdicts.push(DefVerdict::ExtraInActual {
            index: i,
            actual: one_line(a),
        });
    }
    TreeReport {
        verdicts,
        expected_len: expected.len(),
        actual_len: actual.len(),
    }
}

/// Short human-readable summary, e.g. `12/14 defs match (2 mismatches)`.
pub fn summarize(report: &TreeReport) -> String {
    format!(
        "{}/{} defs match ({} mismatches; oracle {}, rust {})",
        report.matched_count(),
        report.verdicts.len(),
        report.mismatch_count(),
        report.expected_len,
        report.actual_len,
    )
}

/// Unified diff over the canonical sequences (3 lines context).
/// Empty string when the sequences render identically.
pub fn render_seq_diff(expected: &[NormDef], actual: &[NormDef]) -> String {
    sail_oracle::diff::unified_diff(&render_seq(expected), &render_seq(actual), 3)
}
