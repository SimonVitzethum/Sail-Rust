#![forbid(unsafe_code)]
//! Pattern mapping: `P_vector` and `P_vector_concat`.
//!
//! Port of `doc_pat` / `doc_vector_concat` in `pretty_print_lean.ml`
//! (lines ~520-586):
//!
//! * `P_vector` of only literals (outside `match_bv`) → `0b101`;
//! * any other `P_vector` → concatenation of the elements rendered with
//!   `in_vector = true` (`x:4` ascriptions for known bitvector widths);
//! * `P_vector_concat` → `[a,b,...]` with **bare-comma** separators
//!   (`separate_map comma`, cf. `match_bv.expected.lean` line 156:
//!   `| [11,111,0,00,opc:2,1,Rm:5,...]`);
//! * `P_vector_concat` parts: literals become raw bits, identifiers need a
//!   known width (`x:4`), `P_typ` unwraps, nested `P_vector` distributes.
//!   Anything else is a span-carrying [`BitvecError`] (OCaml:
//!   `Reporting.unreachable`).

use crate::error::BitvecError;
use crate::lit::Lit;
use crate::span::Span;

/// A Sail pattern relevant to bitvector matching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pat {
    /// `P_wild`.
    Wild {
        span: Span,
    },
    /// `P_lit`.
    Lit {
        lit: Lit,
        span: Span,
    },
    /// `P_id`, with the statically known bitvector width if any.
    Id {
        name: String,
        /// `Some(w)` for `x : bits(w)` / `x : bit` (= 1).
        width: Option<u32>,
        span: Span,
    },
    /// `P_typ` ascription around an inner pattern.
    Typ {
        inner: Box<Pat>,
        /// Width from the ascribed type, if it is `bit`/`bits(w)`.
        width: Option<u32>,
        span: Span,
    },
    /// `P_vector`.
    Vector {
        elems: Vec<Pat>,
        span: Span,
    },
    /// `P_vector_concat` (`p1 @ p2 @ ...`).
    VectorConcat {
        parts: Vec<Pat>,
        span: Span,
    },
}

impl Pat {
    /// Span of the pattern node itself.
    #[must_use]
    pub fn span(&self) -> Span {
        match *self {
            Pat::Wild { span }
            | Pat::Lit { span, .. }
            | Pat::Id { span, .. }
            | Pat::Typ { span, .. }
            | Pat::Vector { span, .. }
            | Pat::VectorConcat { span, .. } => span,
        }
    }

    /// Statically known width in bits, if any.
    #[must_use]
    pub fn width(&self) -> Option<u32> {
        match self {
            Pat::Wild { .. } => None,
            Pat::Lit { lit, .. } => lit.width_bits(),
            Pat::Id { width, .. } | Pat::Typ { width, .. } => *width,
            Pat::Vector { elems, .. } => {
                let mut sum: u64 = 0;
                for elem in elems {
                    sum += u64::from(elem.width()?);
                }
                u32::try_from(sum).ok()
            }
            Pat::VectorConcat { parts, .. } => {
                let mut sum: u64 = 0;
                for part in parts {
                    sum += u64::from(part.concat_part_width()?);
                }
                u32::try_from(sum).ok()
            }
        }
    }

    /// Width contribution of one `P_vector_concat` part, following `doc_part`:
    /// nested `P_vector` distributes over its elements.
    fn concat_part_width(&self) -> Option<u32> {
        match self {
            Pat::Vector { elems, .. } => {
                let mut sum: u64 = 0;
                for elem in elems {
                    sum += u64::from(elem.concat_part_width()?);
                }
                u32::try_from(sum).ok()
            }
            Pat::Typ { inner, .. } => inner.concat_part_width(),
            Pat::Wild { .. } => None,
            Pat::Lit { lit, .. } => lit.width_bits(),
            Pat::Id { width, .. } => *width,
            Pat::VectorConcat { .. } => None,
        }
    }
}

/// `is_bitvector_pattern` in OCaml: drives the `match` vs `match_bv` choice.
#[must_use]
pub fn is_bitvector_pat(pat: &Pat) -> bool {
    match pat {
        Pat::Vector { .. } | Pat::VectorConcat { .. } => true,
        Pat::Typ { inner, .. } => is_bitvector_pat(inner),
        Pat::Wild { .. } | Pat::Lit { .. } | Pat::Id { .. } => false,
    }
}

/// `match_or_match_bv` in OCaml.
#[must_use]
pub fn match_keyword(is_match_bv: bool) -> &'static str {
    if is_match_bv {
        "match_bv "
    } else {
        "match "
    }
}

/// Render one `doc_part` of `doc_vector_concat`.
///
/// * literals → raw bit characters (`0b11` becomes `11`);
/// * identifiers → `name:width`, erroring on unclear width;
/// * `P_typ` → unwrap; nested `P_vector` → parts joined with `,`.
fn render_concat_part(part: &Pat) -> Result<String, BitvecError> {
    match part {
        Pat::Lit { lit, .. } => lit.raw_bits(part.span()),
        Pat::Id { name, width, .. } => match width {
            Some(w) => Ok(format!("{name}:{w}")),
            None => Err(BitvecError::unclear_width(
                part.span(),
                format!("identifier {name:?} without known width"),
            )),
        },
        Pat::Typ { inner, .. } => render_concat_part(inner),
        Pat::Vector { elems, .. } => {
            let mut rendered = Vec::with_capacity(elems.len());
            for elem in elems {
                rendered.push(render_concat_part(elem)?);
            }
            Ok(rendered.join(","))
        }
        Pat::Wild { .. } | Pat::VectorConcat { .. } => Err(BitvecError::unexpected_pattern(
            part.span(),
            "only literals, identifiers, ascriptions and flat vectors fit in match_bv concat",
        )),
    }
}

/// `doc_vector_concat` equivalent: `[part,part,...]` with bare commas.
pub fn render_vector_concat(parts: &[Pat], _span: Span) -> Result<String, BitvecError> {
    let mut rendered = Vec::with_capacity(parts.len());
    for part in parts {
        rendered.push(render_concat_part(part)?);
    }
    Ok(format!("[{}]", rendered.join(",")))
}

/// Require concat parts to total exactly `expected_width` bits.
///
/// Unknown part widths are [`ErrorKind::UnclearWidth`](crate::error::ErrorKind),
/// known-but-wrong sums are width mismatches — never silent truncations.
pub fn check_concat_width(parts: &[Pat], expected_width: u32, span: Span) -> Result<(), BitvecError> {
    let mut sum: u64 = 0;
    for part in parts {
        match part.concat_part_width() {
            Some(w) => sum += u64::from(w),
            None => {
                return Err(BitvecError::unclear_width(
                    part.span(),
                    "cannot total concat width with unclear part width",
                ));
            }
        }
    }
    let found = u32::try_from(sum).unwrap_or(u32::MAX);
    if sum == u64::from(expected_width) {
        Ok(())
    } else {
        Err(BitvecError::width_mismatch(
            expected_width,
            found,
            span,
            format!("vector_concat totals {found} bits"),
        ))
    }
}

/// Render a pattern.
///
/// * `in_vector`: inside `P_vector`/`P_vector_concat` — identifiers with
///   known widths gain `:w` ascriptions;
/// * `in_match_bv`: inside a `match_bv` scrutinee — an all-literal `P_vector`
///   renders as concatenated raw bits instead of `0b...`.
pub fn render_pat(pat: &Pat, in_vector: bool, in_match_bv: bool) -> Result<String, BitvecError> {
    match pat {
        Pat::Wild { .. } => Ok("_".to_owned()),
        Pat::Lit { lit, .. } => Ok(lit.render_lean(false)),
        Pat::Id { name, width, .. } => match (in_vector, width) {
            (true, Some(w)) => Ok(format!("{name}:{w}")),
            _ => Ok(name.clone()),
        },
        Pat::Typ { inner, width, .. } => {
            if in_vector {
                match width {
                    Some(w) => Ok(format!("{}:{w}", render_pat(inner, false, in_match_bv)?)),
                    None => render_pat(inner, false, in_match_bv),
                }
            } else {
                render_pat(inner, false, in_match_bv)
            }
        }
        Pat::Vector { elems, .. } => {
            // Faithful to OCaml: only a literal-only vector outside match_bv
            // takes the `0b...` branch. `P_typ` unwraps to its literal first.
            let mut lits_only = true;
            for elem in elems {
                let mut probe = elem;
                while let Pat::Typ { inner, .. } = probe {
                    probe = inner;
                }
                if !matches!(probe, Pat::Lit { .. }) {
                    lits_only = false;
                    break;
                }
            }
            if lits_only && !in_match_bv {
                let mut body = String::from("0b");
                for elem in elems {
                    let mut probe = elem;
                    while let Pat::Typ { inner, .. } = probe {
                        probe = inner;
                    }
                    match probe {
                        Pat::Lit { lit, .. } => body.push_str(&lit.raw_bits(probe.span())?),
                        _ => unreachable!("checked lits_only above"),
                    }
                }
                Ok(body)
            } else {
                let mut out = String::new();
                for elem in elems {
                    out.push_str(&render_pat(elem, true, in_match_bv)?);
                }
                Ok(out)
            }
        }
        Pat::VectorConcat { parts, span } => render_vector_concat(parts, *span),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::span::FileId;

    fn span(start: u32, end: u32) -> Span {
        Span::new(FileId(0), start, end)
    }

    fn bit_lit(bit: char) -> Pat {
        Pat::Lit {
            lit: Lit::parse_bin(&bit.to_string(), span(0, 1)).unwrap(),
            span: span(0, 1),
        }
    }

    fn id(name: &str, width: Option<u32>) -> Pat {
        Pat::Id {
            name: name.to_owned(),
            width,
            span: span(0, 1),
        }
    }

    #[test]
    fn all_literal_vector_renders_0b() {
        // mapping.sail ln.186: `| 0b00 => BYTE`
        let pat = Pat::Vector {
            elems: vec![bit_lit('0'), bit_lit('0')],
            span: span(0, 4),
        };
        assert_eq!(render_pat(&pat, false, false).unwrap(), "0b00");
    }

    #[test]
    fn decode_clause_matches_expected_lean() {
        // match_bv.sail clause 1, match_bv.expected.lean ln.156:
        // | [11,111,0,00,opc:2,1,Rm:5,option_v:3,S:1,10,Rn:5,Rt:5]
        let lit = |bits: &str| Pat::Lit {
            lit: Lit::parse_bin(bits, span(0, 1)).unwrap(),
            span: span(0, 1),
        };
        let parts = vec![
            lit("11"),
            lit("111"),
            lit("0"),
            lit("00"),
            id("opc", Some(2)),
            lit("1"),
            id("Rm", Some(5)),
            id("option_v", Some(3)),
            id("S", Some(1)),
            lit("10"),
            id("Rn", Some(5)),
            id("Rt", Some(5)),
        ];
        let pat = Pat::VectorConcat {
            parts,
            span: span(0, 32),
        };
        assert_eq!(
            render_pat(&pat, false, true).unwrap(),
            "[11,111,0,00,opc:2,1,Rm:5,option_v:3,S:1,10,Rn:5,Rt:5]"
        );
        assert!(is_bitvector_pat(&pat));
        assert_eq!(match_keyword(true), "match_bv ");
        // 2+3+1+2+2+1+5+3+1+2+5+5 = 32
        check_concat_width(
            match &pat {
                Pat::VectorConcat { parts, .. } => parts,
                _ => unreachable!(),
            },
            32,
            span(0, 32),
        )
        .unwrap();
    }

    #[test]
    fn unclear_width_identifier_errors_with_span() {
        let parts = vec![id("x", None)];
        let err = render_vector_concat(&parts, span(2, 5)).unwrap_err();
        assert_eq!(err.kind, ErrorKind::UnclearWidth);
        assert_eq!(err.span, span(0, 1));
    }

    #[test]
    fn concat_width_mismatch_errors_not_truncates() {
        let lit = |bits: &str| Pat::Lit {
            lit: Lit::parse_bin(bits, span(0, 1)).unwrap(),
            span: span(0, 1),
        };
        let parts = vec![lit("1011000"), id("index", Some(5))];
        // write_CSR total is 12; demanding 16 must fail, not truncate.
        let err = check_concat_width(&parts, 16, span(0, 12)).unwrap_err();
        assert_eq!(
            err.kind,
            ErrorKind::WidthMismatch {
                expected: 16,
                found: 12
            }
        );
        assert_eq!(err.span, span(0, 12));
    }

    #[test]
    fn nested_vector_distributes_and_typ_unwraps() {
        // (0b1011000 : bits(7) @ index : bits(5)) from match_bv.sail write_CSR
        let lit = Pat::Lit {
            lit: Lit::parse_bin("1011000", span(0, 7)).unwrap(),
            span: span(0, 7),
        };
        let ascribed = Pat::Typ {
            inner: Box::new(lit),
            width: Some(7),
            span: span(0, 7),
        };
        let parts = vec![ascribed, id("index", Some(5))];
        assert_eq!(
            render_vector_concat(&parts, span(0, 12)).unwrap(),
            "[1011000,index:5]"
        );
    }

    #[test]
    fn single_bit_id_gets_width_one() {
        // [sf] from match_bv.sail clause 2 renders sf:1
        let parts = vec![
            Pat::Vector {
                elems: vec![id("sf", Some(1))],
                span: span(0, 1),
            },
            id("shift", Some(2)),
        ];
        assert_eq!(
            render_vector_concat(&parts, span(0, 3)).unwrap(),
            "[sf:1,shift:2]"
        );
    }
}
