#![forbid(unsafe_code)]
//! Bitfield declarations (`bitfield.sail`).
//!
//! A `bitfield cr_type : bits(8)` packs named ranges into one bitvector.
//! The Lean backend lowers each field to `Sail.BitVec.extractLsb` (getter)
//! and `Sail.BitVec.updateSubrange` (setter); see `bitfield.expected.lean`:
//!
//! ```lean
//! def _get_cr_type_CR3 (v : (BitVec 8)) : (BitVec 2) :=
//!   (Sail.BitVec.extractLsb v 1 0)
//! def _update_cr_type_CR3 (v : (BitVec 8)) (x : (BitVec 2)) : (BitVec 8) :=
//!   (Sail.BitVec.updateSubrange v 1 0 x)
//! ```
//!
//! Ranges are validated: `hi >= lo`, `lo >= 0`, `hi < total_width`.
//! Violations are span-carrying [`BitvecError`]s.

use crate::error::{BitvecError, ErrorKind};
use crate::Span;

/// One named range `NAME : hi .. lo` (or `NAME : b` for a single bit).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub hi: u32,
    pub lo: u32,
    pub span: Span,
}

impl Field {
    #[must_use]
    pub fn new(name: impl Into<String>, hi: u32, lo: u32, span: Span) -> Self {
        Field {
            name: name.into(),
            hi,
            lo,
            span,
        }
    }

    /// Single-bit field `NAME : b`.
    #[must_use]
    pub fn single(name: impl Into<String>, bit: u32, span: Span) -> Self {
        let name = name.into();
        Field {
            name,
            hi: bit,
            lo: bit,
            span,
        }
    }

    /// Width in bits (`hi - lo + 1`).
    #[must_use]
    pub fn width(&self) -> u32 {
        self.hi.saturating_sub(self.lo).saturating_add(1)
    }

    fn check(&self, total_width: u32) -> Result<(), BitvecError> {
        if self.hi < self.lo || self.hi >= total_width {
            return Err(BitvecError::new(
                ErrorKind::InvalidRange,
                self.span,
                format!(
                    "field {:?} range {}..{} escapes {} bits",
                    self.name, self.hi, self.lo, total_width
                ),
            ));
        }
        Ok(())
    }
}

/// A `bitfield name : bits(w)` declaration with its fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitfieldDecl {
    pub name: String,
    pub total_width: u32,
    pub fields: Vec<Field>,
    pub span: Span,
}

impl BitfieldDecl {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        total_width: u32,
        fields: Vec<Field>,
        span: Span,
    ) -> Self {
        BitfieldDecl {
            name: name.into(),
            total_width,
            fields,
            span,
        }
    }

    /// Validate every field range against the parent width.
    pub fn check(&self) -> Result<(), BitvecError> {
        if self.total_width == 0 {
            return Err(BitvecError::new(
                ErrorKind::EmptyVector,
                self.span,
                format!("bitfield {:?} has zero width", self.name),
            ));
        }
        for field in &self.fields {
            field.check(self.total_width)?;
        }
        Ok(())
    }

    /// Look up a field by name.
    pub fn field(&self, name: &str, span: Span) -> Result<&Field, BitvecError> {
        self.fields.iter().find(|f| f.name == name).ok_or_else(|| {
            BitvecError::unexpected_pattern(span, format!("unknown field {name:?}"))
        })
    }

    /// `(Sail.BitVec.extractLsb v hi lo)` for a field.
    pub fn render_get(&self, holder: &str, field_name: &str, span: Span) -> Result<String, BitvecError> {
        let field = self.field(field_name, span)?;
        field.check(self.total_width)?;
        Ok(format!(
            "(Sail.BitVec.extractLsb {holder} {} {})",
            field.hi, field.lo
        ))
    }

    /// `(Sail.BitVec.updateSubrange v hi lo x)` for a field.
    pub fn render_set(
        &self,
        holder: &str,
        value: &str,
        field_name: &str,
        span: Span,
    ) -> Result<String, BitvecError> {
        let field = self.field(field_name, span)?;
        field.check(self.total_width)?;
        Ok(format!(
            "(Sail.BitVec.updateSubrange {holder} {} {} {value})",
            field.hi, field.lo
        ))
    }

    /// Lean getter signature name, e.g. `_get_cr_type_CR3`.
    #[must_use]
    pub fn getter_name(&self, field_name: &str) -> String {
        format!("_get_{}_{field_name}", self.name)
    }

    /// Lean setter name, e.g. `_update_cr_type_CR3`.
    #[must_use]
    pub fn setter_name(&self, field_name: &str) -> String {
        format!("_update_{}_{field_name}", self.name)
    }
}

/// The `cr_type` declaration from `bitfield.sail`, as a coded fixture:
/// `CR0: 7..4, LT: 7, GT: 6, CR1: 3..2, CR3: 1..0` over `bits(8)`.
#[must_use]
pub fn cr_type_fixture(span: Span) -> BitfieldDecl {
    BitfieldDecl::new(
        "cr_type",
        8,
        vec![
            Field::new("CR0", 7, 4, span),
            Field::single("LT", 7, span),
            Field::single("GT", 6, span),
            Field::new("CR1", 3, 2, span),
            Field::new("CR3", 1, 0, span),
        ],
        span,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileId;

    fn span() -> Span {
        Span::new(FileId(0), 0, 13)
    }

    #[test]
    fn cr_type_accessors_match_expected_lean() {
        let decl = cr_type_fixture(span());
        decl.check().unwrap();
        // bitfield.expected.lean getters/setters
        assert_eq!(
            decl.render_get("v", "CR3", span()).unwrap(),
            "(Sail.BitVec.extractLsb v 1 0)"
        );
        assert_eq!(
            decl.render_set("v", "x", "CR3", span()).unwrap(),
            "(Sail.BitVec.updateSubrange v 1 0 x)"
        );
        assert_eq!(
            decl.render_get("v", "LT", span()).unwrap(),
            "(Sail.BitVec.extractLsb v 7 7)"
        );
        assert_eq!(
            decl.render_get("v", "GT", span()).unwrap(),
            "(Sail.BitVec.extractLsb v 6 6)"
        );
        assert_eq!(decl.getter_name("CR3"), "_get_cr_type_CR3");
        assert_eq!(decl.setter_name("GT"), "_update_cr_type_GT");
        assert_eq!(decl.field("CR0", span()).unwrap().width(), 4);
        assert_eq!(decl.field("CR1", span()).unwrap().width(), 2);
    }

    #[test]
    fn inverted_or_escaping_range_errors_with_span() {
        let bad = BitfieldDecl::new("bad", 8, vec![Field::new("X", 2, 5, span())], span());
        let err = bad.check().unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidRange);
        assert_eq!(err.span, span());
        let escaping =
            BitfieldDecl::new("bad", 8, vec![Field::new("Y", 9, 8, span())], span());
        assert_eq!(escaping.check().unwrap_err().kind, ErrorKind::InvalidRange);
    }

    #[test]
    fn unknown_field_errors() {
        let decl = cr_type_fixture(span());
        assert!(decl.render_get("v", "NOPE", span()).is_err());
    }
}
