#![forbid(unsafe_code)]
//! Type-judgment sampling: compare Rust type schemes against OCaml
//! `--ddump-tc-ast` excerpts for a deterministic per-file sample.
//!
//! Full type-AST equality is out of scope (Rust `sail-ty` works on local
//! minimals; see `ARCHITECTURE.md`): instead each sampled definition contributes
//! one normalized scheme string, and sampling is deterministic (first `N`
//! eligible `val`/`register`/`function` definitions in source order) so Rig
//! runs are reproducible.
//!
//! OCaml side ([`sift_tc_dump`]): heuristic line sifter over `--ddump-tc-ast`
//! stdout — keeps collapsed lines mentioning typechecked schemes
//! (`typquant` / `TypSchm` / `val_spec`), with the leading identifier as the
//! name candidate. **To verify at the first Rig run** against Sail 0.20.3.

/// Which definition flavor a type sample comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TyDefKind {
    Val,
    Register,
    Fundef,
}

impl TyDefKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Val => "val",
            Self::Register => "register",
            Self::Fundef => "function",
        }
    }
}

/// One normalized type judgment: `def_name` elaborates with `scheme_norm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TySample {
    pub def_name: String,
    pub def_kind: TyDefKind,
    /// [`normalize_scheme`] output.
    pub scheme_norm: String,
}

/// Normalize a scheme rendering: CRLF -> LF, whitespace runs collapsed,
/// surrounding space trimmed. Binder order is preserved (reordering
/// `forall` binders is *not* semantics-preserving in general).
pub fn normalize_scheme(s: &str) -> String {
    crate::norm_tree::collapse_ws(&s.replace("\r\n", "\n").replace('\r', "\n"))
}

/// A sampling request: elaborate `def_name` and render its scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleRequest {
    pub def_name: String,
    pub def_kind: TyDefKind,
}

/// Deterministically pick up to `max_per_file` sampling requests from a
/// normalized definition sequence: eligible kinds in source order
/// (`val` -> [`TyDefKind::Val`], `register` -> `Register`, `function` ->
/// `Fundef`; nameless entries are skipped).
pub fn sample_requests(
    defs: &[crate::norm_tree::NormDef],
    max_per_file: usize,
) -> Vec<SampleRequest> {
    defs.iter()
        .filter_map(|d| {
            let kind = match d.kind {
                crate::norm_tree::NormKind::Val => TyDefKind::Val,
                crate::norm_tree::NormKind::Register => TyDefKind::Register,
                crate::norm_tree::NormKind::Fundef => TyDefKind::Fundef,
                _ => return None,
            };
            Some(SampleRequest {
                def_name: d.name.clone()?,
                def_kind: kind,
            })
        })
        .take(max_per_file)
        .collect()
}

/// Verdict for one sampled definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyVerdictKind {
    Match,
    SchemeMismatch,
    MissingInActual,
    MissingInOracle,
}

/// Verdict plus diffable detail for one sampled definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TyVerdict {
    pub def_name: String,
    pub kind: TyVerdictKind,
    pub detail: String,
}

impl TyVerdict {
    pub fn is_match(&self) -> bool {
        matches!(self.kind, TyVerdictKind::Match)
    }
}

/// Compare oracle samples vs Rust samples by definition name.
/// Both sides are `($name, $normalized_scheme)` pairs; duplicates on either
/// side are reported as mismatches (they indicate a sifter bug, and must
/// not silently pass).
pub fn compare_samples(
    expected: &[(String, String)],
    actual: &[(String, String)],
) -> Vec<TyVerdict> {
    let mut verdicts = Vec::new();
    for (name, scheme) in expected {
        let hits: Vec<&String> = actual
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, s)| s)
            .collect::<Vec<_>>();
        match hits.as_slice() {
            [] => verdicts.push(TyVerdict {
                def_name: name.clone(),
                kind: TyVerdictKind::MissingInActual,
                detail: format!("'{name}' sampled from oracle dump but not from Rust"),
            }),
            [one] => {
                if one == &scheme {
                    verdicts.push(TyVerdict {
                        def_name: name.clone(),
                        kind: TyVerdictKind::Match,
                        detail: String::new(),
                    });
                } else {
                    verdicts.push(TyVerdict {
                        def_name: name.clone(),
                        kind: TyVerdictKind::SchemeMismatch,
                        detail: sail_oracle::diff::unified_diff(scheme, one, 3),
                    });
                }
            }
            _ => verdicts.push(TyVerdict {
                def_name: name.clone(),
                kind: TyVerdictKind::MissingInActual,
                detail: format!(
                    "'{name}' appears {} times on the Rust side (duplicate sample)",
                    hits.len()
                ),
            }),
        }
    }
    for (name, _) in actual {
        if !expected.iter().any(|(n, _)| n == name) {
            verdicts.push(TyVerdict {
                def_name: name.clone(),
                kind: TyVerdictKind::MissingInOracle,
                detail: format!("'{name}' sampled from Rust but not from oracle dump"),
            });
        }
    }
    verdicts
}

/// Sift `--ddump-tc-ast` stdout into `(name, normalized_scheme)` pairs.
///
/// Keeps collapsed non-volatile lines that mention a typechecked-scheme
/// marker (`typquant`, `TypSchm`, `val_spec`, `typq`); the name candidate is
/// the first identifier-like token on the line, the scheme is the whole
/// collapsed line. Heuristic — to verify at the first Rig run.
pub fn sift_tc_dump(text: &str) -> Vec<(String, String)> {
    const MARKERS: &[&str] = &["typquant", "TypSchm", "val_spec", "typq"];
    let mut out = Vec::new();
    for raw_line in text.replace("\r\n", "\n").split('\n') {
        let stripped = raw_line.trim_end();
        if stripped.trim().is_empty() {
            continue;
        }
        if sail_oracle::normalize::is_volatile_line(stripped) {
            continue;
        }
        if !MARKERS.iter().any(|m| stripped.contains(m)) {
            continue;
        }
        let collapsed = crate::norm_tree::collapse_ws(&crate::norm_tree::strip_locations(stripped));
        let Some(name) = leading_ident(&collapsed) else {
            continue;
        };
        out.push((name, normalize_scheme(&collapsed)));
    }
    out
}

/// First identifier-like token (`[A-Za-z_][A-Za-z0-9_']*`) on the line.
fn leading_ident(line: &str) -> Option<String> {
    for tok in line.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\'')) {
        if tok.is_empty() {
            continue;
        }
        let mut chars = tok.chars();
        let first = chars.next()?;
        if (first.is_alphabetic() || first == '_') && first != '\'' {
            return Some(tok.to_string());
        }
    }
    None
}
