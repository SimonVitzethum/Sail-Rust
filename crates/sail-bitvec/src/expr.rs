#![forbid(unsafe_code)]
//! Expression mapping: `E_vector` and `E_vector_append`.
//!
//! Port of the `E_vector` case in `pretty_print_lean.ml` (lines ~965-973):
//!
//! * bitvector target type → `(BitVec.join1 [x, y])` with `, ` separators;
//! * any other vector type → `#v[...]` with elements **reversed**
//!   (`List.rev vals` in OCaml);
//! * `E_vector_append` (`bitvector_concat`, extern `_lean_app`, see
//!   `op_of_id` → `+++`) → `(x +++ y)`.
//!
//! Element widths must add up to the declared target width; anything else is
//! a span-carrying [`BitvecError`], never a silent truncation.

use crate::error::BitvecError;
use crate::lit::Lit;
use crate::span::Span;

/// One already-rendered vector element with its known width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Elem {
    /// Lean rendering of the element (e.g. `x`, `0b101#3`).
    pub lean: String,
    /// Width in bits (`1` for a `bit`).
    pub width: u32,
}

impl Elem {
    #[must_use]
    pub fn new(lean: impl Into<String>, width: u32) -> Self {
        Elem {
            lean: lean.into(),
            width,
        }
    }

    /// A single-bit variable (`x : bit`).
    #[must_use]
    pub fn bit(name: impl Into<String>) -> Self {
        Elem::new(name, 1)
    }
}

/// Render an `E_vector` with a bitvector target type as
/// `(BitVec.join1 [...])`.
///
/// Every element width must be known and the widths must sum to
/// `expected_width` (cf. `bitvector_literal` in `bitvec_operation.sail`:
/// two `bit` elements give `bits(2)`).
pub fn render_e_vector_bitvec(
    elems: &[Elem],
    expected_width: u32,
    span: Span,
) -> Result<String, BitvecError> {
    if elems.is_empty() {
        if expected_width == 0 {
            return Ok("BitVec.nil".to_owned());
        }
        return Err(BitvecError::width_mismatch(
            expected_width,
            0,
            span,
            "empty vector literal for non-empty bitvector",
        ));
    }
    let mut sum: u64 = 0;
    for elem in elems {
        sum += u64::from(elem.width);
    }
    let found = u32::try_from(sum).unwrap_or(u32::MAX);
    if sum != u64::from(expected_width) {
        return Err(BitvecError::width_mismatch(
            expected_width,
            found,
            span,
            format!("E_vector of {} element(s) totals {found} bits", elems.len()),
        ));
    }
    let joined = elems
        .iter()
        .map(|e| e.lean.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!("(BitVec.join1 [{joined}])"))
}

/// Render an `E_vector` with a non-bitvector target type as `#v[...]`.
///
/// Elements are emitted in **reverse** order, matching `List.rev vals` in
/// OCaml (cf. `vector_literal` in `bitvec_operation.sail`, which yields
/// `#v[y, x]` for source `[x, y]`).
#[must_use]
pub fn render_e_vector_generic(elems: &[String]) -> String {
    let reversed: Vec<&str> = elems.iter().rev().map(String::as_str).collect();
    format!("#v[{}]", reversed.join(", "))
}

/// Render an `E_vector_append` (`bitvector_concat`) as `(x +++ y)`.
#[must_use]
pub fn render_e_vector_append(lhs_lean: &str, rhs_lean: &str) -> String {
    format!("({lhs_lean} +++ {rhs_lean})")
}

/// Render an `E_lit` bitvector literal in expression position
/// (`doc_lit ~width:true`).
#[must_use]
pub fn render_e_lit(lit: &Lit) -> String {
    lit.render_lean(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::span::FileId;

    fn span() -> Span {
        Span::new(FileId(0), 0, 8)
    }

    #[test]
    fn join1_idiom_for_two_bits() {
        // bitvector_literal(x: bit, y: bit) -> bits(2) from bitvec_operation.sail
        let elems = [Elem::bit("x"), Elem::bit("y")];
        assert_eq!(
            render_e_vector_bitvec(&elems, 2, span()).unwrap(),
            "(BitVec.join1 [x, y])"
        );
    }

    #[test]
    fn join1_width_mismatch_errors_with_span() {
        let elems = [Elem::bit("x"), Elem::bit("y")];
        let err = render_e_vector_bitvec(&elems, 3, span()).unwrap_err();
        assert_eq!(
            err.kind,
            ErrorKind::WidthMismatch {
                expected: 3,
                found: 2
            }
        );
        assert_eq!(err.span, span());
    }

    #[test]
    fn join1_mixed_widths_must_sum() {
        let elems = [Elem::new("hi", 13), Elem::new("lo", 3)];
        assert_eq!(
            render_e_vector_bitvec(&elems, 16, span()).unwrap(),
            "(BitVec.join1 [hi, lo])"
        );
        assert!(render_e_vector_bitvec(&elems, 8, span()).is_err());
    }

    #[test]
    fn generic_vector_reverses_elements() {
        // vector_literal(x, y) -> #v[y, x] per bitvec_operation.expected.lean
        let elems = ["x".to_owned(), "y".to_owned()];
        assert_eq!(render_e_vector_generic(&elems), "#v[y, x]");
    }

    #[test]
    fn append_renders_triple_plus() {
        // bitvector_append from bitvec_operation.expected.lean
        assert_eq!(render_e_vector_append("x", "y"), "(x +++ y)");
    }

    #[test]
    fn empty_vector_needs_zero_width() {
        assert_eq!(render_e_vector_bitvec(&[], 0, span()).unwrap(), "BitVec.nil");
        assert!(render_e_vector_bitvec(&[], 8, span()).is_err());
    }
}
