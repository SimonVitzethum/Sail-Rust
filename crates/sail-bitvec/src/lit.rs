#![forbid(unsafe_code)]
//! Bitvector literals (`L_bin` / `L_hex`).
//!
//! Port of `doc_lit` in `pretty_print_lean.ml` (lines ~397-416):
//!
//! * expression position (`width = true`): `0xFF#8`, `0b101#3`, single bits
//!   as `0#1` / `1#1`, empty as `BitVec.nil`;
//! * pattern position (`width = false`): same shapes without the `#w` suffix
//!   (`0xFF`, `0b101`, `0` / `1`);
//! * inside `P_vector_concat` (`doc_part`): raw bit characters, no prefix.
//!
//! Width agreement with the context is checked explicitly
//! ([`check_lit_width`]); mismatches are [`BitvecError`]s, never silent
//! truncations.

use crate::error::{BitvecError, ErrorKind};
use crate::span::Span;

/// A single bit of a binary literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bit {
    B0,
    B1,
}

impl Bit {
    #[must_use]
    pub fn as_char(self) -> char {
        match self {
            Bit::B0 => '0',
            Bit::B1 => '1',
        }
    }
}

/// A Sail bitvector literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lit {
    /// `0b...` with at least one bit.
    Bin(Vec<Bit>),
    /// `0x...` with at least one digit (stored uppercase, no prefix).
    Hex(String),
    /// Empty `0x[]` / `0b[]` literal.
    Empty,
    /// Non-bitvector literals passed through by the printer.
    Unit,
    Bool(bool),
    Num(String),
    Str(String),
}

impl Lit {
    /// Parse the body of a `0b...` literal (no prefix, `_` separators allowed).
    pub fn parse_bin(digits: &str, span: Span) -> Result<Self, BitvecError> {
        let mut bits = Vec::new();
        for ch in digits.chars().filter(|c| *c != '_') {
            match ch {
                '0' => bits.push(Bit::B0),
                '1' => bits.push(Bit::B1),
                _ => {
                    return Err(BitvecError::new(
                        ErrorKind::InvalidDigit,
                        span,
                        format!("invalid bit {ch:?} in binary literal"),
                    ));
                }
            }
        }
        if bits.is_empty() {
            Ok(Lit::Empty)
        } else {
            Ok(Lit::Bin(bits))
        }
    }

    /// Parse the body of a `0x...` literal (no prefix, `_` separators allowed).
    pub fn parse_hex(digits: &str, span: Span) -> Result<Self, BitvecError> {
        let mut out = String::new();
        for ch in digits.chars().filter(|c| *c != '_') {
            if ch.is_ascii_hexdigit() {
                out.push(ch.to_ascii_uppercase());
            } else {
                return Err(BitvecError::new(
                    ErrorKind::InvalidDigit,
                    span,
                    format!("invalid digit {ch:?} in hex literal"),
                ));
            }
        }
        if out.is_empty() {
            Ok(Lit::Empty)
        } else {
            Ok(Lit::Hex(out))
        }
    }

    /// Width in bits, or `None` for non-bitvector literals.
    #[must_use]
    pub fn width_bits(&self) -> Option<u32> {
        match self {
            Lit::Bin(bits) => u32::try_from(bits.len()).ok(),
            Lit::Hex(digits) => u32::try_from(digits.len()).ok().and_then(|n| n.checked_mul(4)),
            Lit::Empty => Some(0),
            Lit::Unit | Lit::Bool(_) | Lit::Num(_) | Lit::Str(_) => None,
        }
    }

    /// Raw bit characters (`doc_part` in `doc_vector_concat`): no prefix.
    pub fn raw_bits(&self, span: Span) -> Result<String, BitvecError> {
        match self {
            Lit::Bin(bits) => Ok(bits.iter().copied().map(Bit::as_char).collect()),
            Lit::Hex(digits) => {
                let mut out = String::with_capacity(digits.len() * 4);
                for ch in digits.chars() {
                    let nibble = ch.to_digit(16).ok_or_else(|| {
                        BitvecError::new(
                            ErrorKind::InvalidDigit,
                            span,
                            format!("invalid hex digit {ch:?}"),
                        )
                    })?;
                    for shift in (0..4).rev() {
                        out.push(if (nibble >> shift) & 1 == 1 { '1' } else { '0' });
                    }
                }
                Ok(out)
            }
            Lit::Empty => Ok(String::new()),
            Lit::Unit | Lit::Bool(_) | Lit::Num(_) | Lit::Str(_) => Err(
                BitvecError::unexpected_pattern(span, "non-bit literal in bitvector pattern"),
            ),
        }
    }

    /// Lean rendering of a literal.
    ///
    /// `with_width = true` is expression position (`doc_lit ~width:true`),
    /// `false` is pattern position (`doc_lit ~width:false`).
    #[must_use]
    pub fn render_lean(&self, with_width: bool) -> String {
        match self {
            Lit::Empty => "BitVec.nil".to_owned(),
            Lit::Hex(digits) => {
                let width = digits.len() as u32 * 4;
                if with_width {
                    format!("0x{digits}#{width}")
                } else {
                    format!("0x{digits}")
                }
            }
            Lit::Bin(bits) => match bits.as_slice() {
                [Bit::B0] => {
                    if with_width {
                        "0#1".to_owned()
                    } else {
                        "0".to_owned()
                    }
                }
                [Bit::B1] => {
                    if with_width {
                        "1#1".to_owned()
                    } else {
                        "1".to_owned()
                    }
                }
                _ => {
                    let body: String = bits.iter().copied().map(Bit::as_char).collect();
                    if with_width {
                        format!("0b{body}#{}", bits.len())
                    } else {
                        format!("0b{body}")
                    }
                }
            },
            Lit::Unit => "()".to_owned(),
            Lit::Bool(true) => "true".to_owned(),
            Lit::Bool(false) => "false".to_owned(),
            Lit::Num(n) => n.clone(),
            Lit::Str(s) => format!("\"{}\"", s.replace('"', "\\\"")),
        }
    }
}

/// Require a bitvector literal to have exactly `expected` bits.
///
/// Returns the literal's Lean rendering on success; otherwise a
/// [`BitvecError`] carrying `span` (never a silent truncation).
pub fn check_lit_width(lit: &Lit, expected: u32, span: Span) -> Result<String, BitvecError> {
    match lit.width_bits() {
        Some(found) if found == expected => Ok(lit.render_lean(true)),
        Some(found) => Err(BitvecError::width_mismatch(
            expected,
            found,
            span,
            format!("literal {} is {found} bits wide", lit.render_lean(false)),
        )),
        None => Err(BitvecError::unexpected_pattern(
            span,
            "non-bitvector literal where bits were expected",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::FileId;

    fn span() -> Span {
        Span::new(FileId(0), 0, 4)
    }

    #[test]
    fn expr_and_pat_shapes_match_doc_lit() {
        let hex = Lit::parse_hex("ff", span()).unwrap();
        assert_eq!(hex.render_lean(true), "0xFF#8");
        assert_eq!(hex.render_lean(false), "0xFF");
        let bin = Lit::parse_bin("101", span()).unwrap();
        assert_eq!(bin.render_lean(true), "0b101#3");
        assert_eq!(bin.render_lean(false), "0b101");
        assert_eq!(Lit::parse_bin("0", span()).unwrap().render_lean(true), "0#1");
        assert_eq!(Lit::parse_bin("1", span()).unwrap().render_lean(false), "1");
        assert_eq!(Lit::Empty.render_lean(true), "BitVec.nil");
    }

    #[test]
    fn raw_bits_expands_hex_nibbles() {
        let hex = Lit::parse_hex("B0", span()).unwrap();
        assert_eq!(hex.raw_bits(span()).unwrap(), "10110000");
        let bin = Lit::parse_bin("10", span()).unwrap();
        assert_eq!(bin.raw_bits(span()).unwrap(), "10");
    }

    #[test]
    fn width_mismatch_carries_span() {
        let hex = Lit::parse_hex("1FF", span()).unwrap();
        let err = check_lit_width(&hex, 8, span()).unwrap_err();
        assert_eq!(
            err.kind,
            ErrorKind::WidthMismatch {
                expected: 8,
                found: 12
            }
        );
        assert_eq!(err.span, span());
    }

    #[test]
    fn invalid_digits_error() {
        assert!(Lit::parse_bin("102", span()).is_err());
        assert!(Lit::parse_hex("zz", span()).is_err());
    }
}
