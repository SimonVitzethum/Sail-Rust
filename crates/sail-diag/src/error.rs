#![forbid(unsafe_code)]
//! The shared Sail error type: every lane reports failures as [`SailError`].
//!
//! A [`SailError`] carries a [`Span`] (where), a [`Pass`] (which stage), a
//! [`DiagCode`] (which known failure mode, see `--explain`), a message
//! (what happened), a hint (what to do), and an optional Sail source
//! snippet. Render one error per line-group via [`fmt::Display`]; use
//! [`SailError::render`] to attach the source text around the span.

use crate::{DiagCode, Pass, Span};
use std::fmt;

/// One compiler diagnostic. All lanes construct this; the driver renders it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SailError {
    /// Source location of the offending construct.
    pub span: Span,
    /// Pipeline stage that raised the error.
    pub pass: Pass,
    /// Known failure mode (`--explain <code>` prints the workaround).
    pub code: DiagCode,
    /// What happened (may include the OCaml message pattern).
    pub message: String,
    /// What to do (usually the catalog workaround).
    pub hint: String,
    /// Sail source lines around [`SailError::span`], if already extracted.
    pub snippet: Option<String>,
}

impl SailError {
    /// Build an error from its parts.
    pub fn new(
        span: Span,
        pass: Pass,
        code: DiagCode,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        SailError {
            span,
            pass,
            code,
            message: message.into(),
            hint: hint.into(),
            snippet: None,
        }
    }

    /// Build an error from a catalog row; message is free detail text, hint
    /// defaults to the catalog workaround.
    pub fn from_entry(span: Span, code: DiagCode, message: impl Into<String>) -> Self {
        let entry = code.entry();
        SailError {
            span,
            pass: entry.pass,
            code,
            message: message.into(),
            hint: entry.workaround.to_string(),
            snippet: None,
        }
    }

    /// Attach a pre-extracted source snippet (builder style).
    #[must_use]
    pub fn with_snippet(mut self, snippet: impl Into<String>) -> Self {
        self.snippet = Some(snippet.into());
        self
    }

    /// Extract the source line(s) around `span` from `source` and attach
    /// them as [`SailError::snippet`]. Byte offsets outside `source` (or a
    /// dummy span) yield no snippet instead of panicking.
    #[must_use]
    pub fn render(mut self, source: &str) -> Self {
        self.snippet = extract_snippet(source, self.span);
        self
    }
}

impl fmt::Display for SailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "error[{}]: {}", self.code.code_str(), self.message)?;
        writeln!(f, "  --> {} (pass: {})", self.span, self.pass)?;
        if let Some(snippet) = &self.snippet {
            for line in snippet.lines() {
                writeln!(f, "   | {line}")?;
            }
        }
        write!(f, "   = hint: {}", self.hint)
    }
}

/// Extract the 1-based line containing the span plus one line of context on
/// each side. Returns `None` for dummy spans or out-of-range offsets.
fn extract_snippet(source: &str, span: Span) -> Option<String> {
    if span.is_dummy() {
        return None;
    }
    let start = span.start as usize;
    if start > source.len() {
        return None;
    }
    // Line number (0-based) of the span start.
    let target_line = source[..start].bytes().filter(|&b| b == b'\n').count();
    let lines: Vec<&str> = source.lines().collect();
    if target_line >= lines.len() {
        return None;
    }
    let from = target_line.saturating_sub(1);
    let to = (target_line + 1).min(lines.len().saturating_sub(1));
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate().take(to + 1).skip(from) {
        let marker = if i == target_line { ">" } else { " " };
        out.push_str(&format!("{marker}{}: {line}\n", i + 1));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_contains_code_span_pass_and_hint() {
        let err = SailError::new(
            Span::dummy(),
            Pass::LeanPrinter,
            DiagCode::NexpNotTranslatable,
            "NExp foo not translatable yet.",
            "specialize it",
        );
        let text = err.to_string();
        assert!(text.contains("E3501"), "{text}");
        assert!(text.contains("lean-printer"), "{text}");
        assert!(text.contains("specialize it"), "{text}");
    }

    #[test]
    fn from_entry_takes_hint_from_catalog() {
        let err = SailError::from_entry(
            Span::dummy(),
            DiagCode::WhenClauseUnsupported,
            "custom detail",
        );
        assert_eq!(err.pass, Pass::LeanPrinter);
        assert_eq!(err.message, "custom detail");
        assert!(!err.hint.is_empty());
        assert_eq!(err.hint, DiagCode::WhenClauseUnsupported.entry().workaround);
    }

    #[test]
    fn render_attaches_source_line() {
        let src = "line one\nline two\nline three\n";
        let span = Span::new(crate::FileId(0), 9, 13); // inside "line two"
        let err = SailError::new(
            span,
            Pass::LeanPrinter,
            DiagCode::ExpressionNotTranslatable,
            "m",
            "h",
        )
        .render(src);
        let snippet = err.snippet.expect("snippet attached");
        assert!(snippet.contains(">2: line two"), "{snippet}");
        assert!(snippet.contains("1: line one"), "{snippet}");
        assert!(snippet.contains("3: line three"), "{snippet}");
    }

    #[test]
    fn render_dummy_or_oob_yields_no_snippet() {
        let mk = |span| {
            SailError::new(
                span,
                Pass::LeanPrinter,
                DiagCode::ExpressionNotTranslatable,
                "m",
                "h",
            )
            .render("short\n")
        };
        assert_eq!(mk(Span::dummy()).snippet, None);
        assert_eq!(mk(Span::new(crate::FileId(0), 999, 1000)).snippet, None);
    }
}
