#![forbid(unsafe_code)]
//! `Rec_measure` evaluation and `termination_by (...).toNat` generation.
//!
//! Mirrors `doc_termination` in `pretty_print_lean.ml`:
//!
//! ```ocaml
//! let doc_termination fixup_binders ctx fnpat (Rec_aux (meas, _)) =
//!   match meas with
//!   | Rec_nonrec | Rec_rec -> empty
//!   | Rec_measure (pat, exp) ->
//!       let term = doc_exp false ctx (fixup_binders
//!         (E_let (pat, Rewrites.pat_to_exp (env_of_pat fnpat) fnpat, exp))) in
//!       let term_by = string "termination_by " ^^ parens term ^^ string ".toNat" in
//!       hardline ^^ term_by
//! ```
//!
//! The key detail: the measure pattern is bound to the *function pattern*
//! converted to an expression (`pat_to_exp`), i.e. the generated Lean is
//!
//! ```lean
//! termination_by (let e := e
//! (measure e)).toNat
//! ```
//!
//! and for multi-argument functions (`termination_measure enabled2(_, x) = …`):
//!
//! ```lean
//! termination_by (let (_, x) := (b, e)
//! (measure2 x)).toNat
//! ```
//!
//! (See `sail/test/lean/termination.expected.lean`, lines 186-187 / 201-202.)

use std::fmt;

/// Recursion annotation of a Sail function definition.
///
/// Mirrors the OCaml `rec` type: `Rec_none`/`Rec_nonrec` (no annotation),
/// `Rec_rec` (plain `rec`), `Rec_measure (pat, exp)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecKind {
    /// Non-recursive function (OCaml `Rec_nonrec`).
    NonRec,
    /// Plain recursive function without a measure (OCaml `Rec_rec`).
    Rec,
    /// Recursive function with an explicit termination measure.
    Measure(RecMeasure),
}

/// A Sail termination measure: `termination_measure f(pat) = exp`.
///
/// After `move_termination_measures` this is attached to the function
/// definition as `Rec_measure (pat, exp)`. `fn_pattern` is the function's
/// own argument pattern (`fnpat` in `doc_termination`), rendered as the
/// right-hand side of the generated `let`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecMeasure {
    /// Measure pattern, e.g. `e` or `(_, x)`.
    pub pattern: String,
    /// Measure expression, already pretty-printed, e.g. `measure e`.
    pub measure_expr: String,
    /// Function argument pattern as an expression, e.g. `e` or `(b, e)`.
    pub fn_pattern: String,
}

impl RecMeasure {
    /// Build a measure from its parts. Empty patterns/expressions are rejected
    /// so malformed input fails early instead of emitting broken Lean.
    pub fn new(
        pattern: impl Into<String>,
        measure_expr: impl Into<String>,
        fn_pattern: impl Into<String>,
    ) -> Result<Self, String> {
        let (pattern, measure_expr, fn_pattern) =
            (pattern.into(), measure_expr.into(), fn_pattern.into());
        if pattern.trim().is_empty() {
            return Err("termination measure pattern must not be empty".to_string());
        }
        if measure_expr.trim().is_empty() {
            return Err("termination measure expression must not be empty".to_string());
        }
        if fn_pattern.trim().is_empty() {
            return Err("function pattern must not be empty".to_string());
        }
        Ok(Self { pattern, measure_expr, fn_pattern })
    }

    /// Render the `let <pat> := <fnpat>\n(<measure>)` body, i.e. the `term`
    /// in `doc_termination` (the `E_let` of measure pattern over the
    /// function pattern converted to an expression).
    pub fn render_term(&self) -> String {
        format!("let {} := {}\n({})", self.pattern, self.fn_pattern, self.measure_expr)
    }
}

impl fmt::Display for RecMeasure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", render_termination_by(self))
    }
}

/// Render the full Lean clause for a recursion annotation.
///
/// * `NonRec` / `Rec` → empty string (OCaml returns `empty`).
/// * `Measure` → `\ntermination_by (<term>).toNat`.
///
/// The leading newline corresponds to the OCaml `hardline ^^ term_by`:
/// the clause always starts on its own line after the function body.
pub fn render_termination_by(measure: &RecMeasure) -> String {
    format!("\ntermination_by ({}).toNat", measure.render_term())
}

/// Render any [`RecKind`]; returns `None` for the cases that emit nothing.
pub fn render_rec_kind(rec: &RecKind) -> Option<String> {
    match rec {
        RecKind::NonRec | RecKind::Rec => None,
        RecKind::Measure(m) => Some(render_termination_by(m)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_arg_matches_expected_lean() {
        // sail/test/lean/termination.expected.lean:186-187
        let m = RecMeasure::new("e", "measure e", "e").unwrap();
        assert_eq!(
            render_termination_by(&m),
            "\ntermination_by (let e := e\n(measure e)).toNat"
        );
    }

    #[test]
    fn multi_arg_matches_expected_lean() {
        // sail/test/lean/termination.expected.lean:201-202
        let m = RecMeasure::new("(_, x)", "measure2 x", "(b, e)").unwrap();
        assert_eq!(
            render_termination_by(&m),
            "\ntermination_by (let (_, x) := (b, e)\n(measure2 x)).toNat"
        );
    }

    #[test]
    fn plain_recursion_emits_nothing() {
        assert_eq!(render_rec_kind(&RecKind::NonRec), None);
        assert_eq!(render_rec_kind(&RecKind::Rec), None);
    }

    #[test]
    fn empty_parts_rejected() {
        assert!(RecMeasure::new("", "m x", "x").is_err());
        assert!(RecMeasure::new("p", "", "x").is_err());
        assert!(RecMeasure::new("p", "m x", "").is_err());
    }
}
