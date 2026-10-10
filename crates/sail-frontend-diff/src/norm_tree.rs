#![forbid(unsafe_code)]
//! Normalized parse-tree model + both-side constructors.
//!
//! [`NormDef`] is the comparison unit: definition kind + optional name +
//! whitespace-collapsed header text. Everything volatile (spans, node IDs,
//! line endings, blank runs, banner lines, shard markers) is removed at
//! construction so equality on [`NormDef`] is meaningful.
//!
//! OCaml side ([`sift_oracle_dump`]): the `--ddump-initial-ast` dump prints
//! one `DEF_*` constructor per top-level definition (cf. `DEF_*` in
//! `sail/src/lib/extraction/Ast.ml`, as referenced by `sail-parse`). The
//! sifter scans each line for the `DEF_<suffix>` token and takes the first
//! quoted string after it as the name candidate. **Heuristic**: exact dump
//! shapes are to verify at the first Rig run; unknown `DEF_` suffixes map
//! to [`NormKind::Other`] with the raw token preserved in `head_norm` so
//! they mismatch loudly instead of vanishing.

use sail_parse::{Def, ParsedFile};

/// Normalized definition kind (mirrors `sail_parse::Def` variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NormKind {
    Val,
    Fundef,
    Mapdef,
    Impl,
    Type,
    Let,
    Constraint,
    Overload,
    Fixity,
    Default,
    Scattered,
    Register,
    Measure,
    Outcome,
    Instantiation,
    Pragma,
    Other,
}

impl NormKind {
    /// Short label used in canonical sequence rendering.
    pub fn label(self) -> &'static str {
        match self {
            Self::Val => "val",
            Self::Fundef => "function",
            Self::Mapdef => "mapping",
            Self::Impl => "impl",
            Self::Type => "type",
            Self::Let => "let",
            Self::Constraint => "constraint",
            Self::Overload => "overload",
            Self::Fixity => "fixity",
            Self::Default => "default",
            Self::Scattered => "scattered",
            Self::Register => "register",
            Self::Measure => "measure",
            Self::Outcome => "outcome",
            Self::Instantiation => "instantiation",
            Self::Pragma => "pragma",
            Self::Other => "other",
        }
    }

    /// Map an OCaml `DEF_<suffix>` token suffix to a kind.
    /// Returns `None` for unknown suffixes (caller maps to `Other`).
    pub fn of_oracle_suffix(suffix: &str) -> Option<Self> {
        Some(match suffix {
            "val" => Self::Val,
            "fundef" => Self::Fundef,
            "mapdef" => Self::Mapdef,
            "impl" => Self::Impl,
            "type" => Self::Type,
            "let" => Self::Let,
            "constraint" => Self::Constraint,
            "overload" => Self::Overload,
            "fixity" => Self::Fixity,
            "default" => Self::Default,
            "scattered" => Self::Scattered,
            "reg" | "register" => Self::Register,
            "measure" => Self::Measure,
            "outcome" => Self::Outcome,
            "instantiation" => Self::Instantiation,
            "pragma" => Self::Pragma,
            _ => return None,
        })
    }
}

/// One normalized top-level definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormDef {
    pub kind: NormKind,
    pub name: Option<String>,
    /// Whitespace-collapsed header text (signature/head/pragma payload).
    /// Bodies are excluded: raw source at the skeleton layer (Lane 02).
    pub head_norm: String,
}

/// One sifted OCaml dump entry (same shape as [`NormDef`] plus provenance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleDef {
    pub kind: NormKind,
    pub name: Option<String>,
    pub head_norm: String,
    /// 1-based dump line number (for diagnostics only, never compared).
    pub dump_line: usize,
}

impl From<OracleDef> for NormDef {
    fn from(o: OracleDef) -> Self {
        NormDef {
            kind: o.kind,
            name: o.name,
            head_norm: o.head_norm,
        }
    }
}

/// Collapse all whitespace runs to single spaces and trim.
/// `"  val  x :  int "` -> `"val x : int"`.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Kind of a Rust skeleton definition.
pub fn kind_of(def: &Def) -> NormKind {
    match def {
        Def::Val { .. } => NormKind::Val,
        Def::Fundef { .. } => NormKind::Fundef,
        Def::Mapdef { .. } => NormKind::Mapdef,
        Def::Impl { .. } => NormKind::Impl,
        Def::Type { .. } => NormKind::Type,
        Def::Let { .. } => NormKind::Let,
        Def::Constraint { .. } => NormKind::Constraint,
        Def::Overload { .. } => NormKind::Overload,
        Def::Fixity { .. } => NormKind::Fixity,
        Def::Default { .. } => NormKind::Default,
        Def::Scattered { .. } => NormKind::Scattered,
        Def::Register { .. } => NormKind::Register,
        Def::Measure { .. } => NormKind::Measure,
        Def::Outcome { .. } => NormKind::Outcome,
        Def::Instantiation { .. } => NormKind::Instantiation,
        Def::Pragma { .. } => NormKind::Pragma,
        Def::Other { .. } => NormKind::Other,
    }
}

/// Declared name of a Rust skeleton definition, if it has one.
pub fn name_of(def: &Def) -> Option<String> {
    Some(
        match def {
            Def::Val { name, .. }
            | Def::Fundef { name, .. }
            | Def::Mapdef { name, .. }
            | Def::Impl { name, .. }
            | Def::Type { name, .. }
            | Def::Overload { name, .. }
            | Def::Fixity { name, .. }
            | Def::Register { name, .. } => name.clone(),
            Def::Pragma { name, .. } => name.clone(),
            Def::Other { keyword, .. } => keyword.clone(),
            Def::Let { .. }
            | Def::Constraint { .. }
            | Def::Default { .. }
            | Def::Scattered { .. }
            | Def::Measure { .. }
            | Def::Outcome { .. }
            | Def::Instantiation { .. } => return None,
        },
    )
}

/// Raw header text of a Rust skeleton definition (unnormalized).
pub fn head_text(def: &Def) -> String {
    match def {
        Def::Val { sig_src, .. } => sig_src.clone(),
        Def::Fundef { head_src, .. }
        | Def::Mapdef { head_src, .. }
        | Def::Impl { head_src, .. }
        | Def::Scattered { head_src, .. } => head_src.clone(),
        Def::Type { body_src, .. } => body_src.clone(),
        Def::Let { pat_src, .. } => pat_src.clone(),
        Def::Constraint { src }
        | Def::Measure { src }
        | Def::Outcome { src }
        | Def::Instantiation { src } => src.clone(),
        Def::Overload { ids, .. } => ids.join(" "),
        Def::Fixity { prec, level, .. } => format!("{prec} {level}"),
        Def::Default { kind, value } => format!("{kind} {value}"),
        Def::Register {
            type_src,
            init_src,
            ..
        } => match init_src {
            Some(init) => format!("{type_src} = {init}"),
            None => type_src.clone(),
        },
        Def::Pragma { name, arg } => match arg {
            sail_parse::PragmaArg::Line(s) | sail_parse::PragmaArg::Structured(s) => {
                format!("{name} {s}")
            }
        },
        Def::Other { keyword, src } => format!("{keyword} {src}"),
    }
}

/// Normalize a parsed Rust file.
///
/// Drops Lane-11 shard markers (`file_start`/`file_end`/...) — a
/// Rust-harness artifact with no OCaml counterpart. Returns the normalized
/// sequence plus the number of dropped markers (for diagnostics).
pub fn from_rust(parsed: &ParsedFile) -> (Vec<NormDef>, usize) {
    let mut out = Vec::with_capacity(parsed.defs.len());
    let mut dropped_markers = 0usize;
    for sd in &parsed.defs {
        if sd.def.is_shard_marker() {
            dropped_markers += 1;
            continue;
        }
        out.push(NormDef {
            kind: kind_of(&sd.def),
            name: name_of(&sd.def),
            head_norm: collapse_ws(&head_text(&sd.def)),
        });
    }
    (out, dropped_markers)
}

/// Find the `DEF_<suffix>` token in a dump line; returns the suffix.
fn def_token(line: &str) -> Option<&str> {
    // Token chars: alphanumerics + underscore; `DEF_` prefix, case-sensitive
    // (OCaml constructors are uppercase `DEF_` + lowercase suffix).
    let mut search = line;
    loop {
        let i = search.find("DEF_")?;
        let after = &search[i + 4..];
        let len: usize = after
            .char_indices()
            .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
            .map(|(b, c)| b + c.len_utf8())
            .last()
            .unwrap_or(0);
        if len > 0 {
            return Some(&search[i + 4..i + 4 + len]);
        }
        search = &search[i + 4..];
    }
}

/// First `"quoted"` string after `from` (byte index into `line`).
fn first_quoted_after(line: &str, from: usize) -> Option<String> {
    let rest = line.get(from..)?;
    let open = rest.find('"')?;
    let inner = rest.get(open + 1..)?;
    let close = inner.find('"')?;
    Some(inner[..close].to_string())
}

/// Sift OCaml `--ddump-initial-ast` stdout into normalized entries.
///
/// Rules: volatile banner lines (same predicate as `sail-oracle`
/// normalization) are skipped; lines without a `DEF_` token are skipped
/// (location headers, separators); unknown suffixes become `Other` with the
/// raw token kept in `head_norm` so they mismatch loudly. The collapsed
/// remainder of the line (token included) is kept as `head_norm`.
pub fn sift_oracle_dump(text: &str) -> Vec<OracleDef> {
    let mut out = Vec::new();
    for (idx, raw_line) in text.replace("\r\n", "\n").split('\n').enumerate() {
        let stripped = raw_line.trim_end();
        if stripped.trim().is_empty() {
            continue;
        }
        if sail_oracle::normalize::is_volatile_line(stripped) {
            continue;
        }
        let Some(suffix) = def_token(stripped) else {
            continue;
        };
        let kind = NormKind::of_oracle_suffix(suffix).unwrap_or(NormKind::Other);
        let tok_start = stripped.find("DEF_").unwrap_or(0);
        let name = first_quoted_after(stripped, tok_start);
        // Keep the raw token for unknown suffixes so `Other` entries from
        // different constructors never compare equal by accident.
        let head_norm = if NormKind::of_oracle_suffix(suffix).is_none() {
            collapse_ws(stripped)
        } else {
            // Drop the volatile `l<num>.c<num>`-style location fragments the
            // OCaml pretty-printer interleaves; keep the rest collapsed.
            collapse_ws(&strip_locations(stripped))
        };
        out.push(OracleDef {
            kind,
            name,
            head_norm,
            dump_line: idx + 1,
        });
    }
    out
}

/// Remove `l<num>` / `l<num>.c<num>` / `:l<num>` location fragments from a
/// dump line. Conservative: only touches tokens that are *exactly* a
/// location fragment, never substrings of identifiers.
pub fn strip_locations(line: &str) -> String {
    line.split_whitespace()
        .filter(|tok| !is_location_token(tok))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_location_token(tok: &str) -> bool {
    let t = tok
        .trim_matches(|c: char| matches!(c, '(' | ')' | ',' | ':' | '[' | ']'));
    let t = t.strip_prefix(':').unwrap_or(t);
    let t = t.strip_prefix('l').unwrap_or(t);
    let (line_part, col_part) = match t.split_once(".c") {
        Some((l, c)) => (l, Some(c)),
        None => (t, None),
    };
    if line_part.is_empty() || !line_part.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    match col_part {
        None => true,
        Some(c) => !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()),
    }
}

/// Canonical one-def-per-line rendering: `kind[:name] head`.
/// Used as the unified-diff input on both sides.
pub fn render_seq(defs: &[NormDef]) -> String {
    let mut out = String::new();
    for d in defs {
        out.push_str(d.kind.label());
        if let Some(n) = &d.name {
            out.push(':');
            out.push_str(n);
        }
        if !d.head_norm.is_empty() {
            out.push(' ');
            out.push_str(&d.head_norm);
        }
        out.push('\n');
    }
    out
}
