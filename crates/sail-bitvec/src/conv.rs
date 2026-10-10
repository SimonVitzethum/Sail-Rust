#![forbid(unsafe_code)]
//! Bridge from canonical `sail-ast` pattern nodes onto the local bitvector
//! [`Pat`][crate::pat::Pat] (Lane 61 unification, MIGRATION.md #33).
//!
//! One direction only: canonical nodes lower onto the `Wild/Lit/Id/Typ/
//! Vector/VectorConcat` subset with spans taken from the nodes and widths
//! left `None` (unknown until the `Typ`-ascription + `width` analysis
//! runs, which stays Lane-22 logic). The reverse direction is blocked:
//! widths and per-variant spans have no canonical counterpart, so the
//! local `Pat` is retained (not deleted) with this module as the swap
//! seam. Anything outside the subset maps to `None`.

use sail_ast::{Interner, LitKind as AstLitKind, Pat as AstPat, PatKind};

use crate::lit::Lit;
use crate::pat::Pat;

fn lit_from_ast(lit: &sail_ast::Lit) -> Option<Lit> {
    match &lit.kind {
        AstLitKind::Num(s) => Some(Lit::Num(s.clone())),
        AstLitKind::String(s) => Some(Lit::Str(s.clone())),
        AstLitKind::True => Some(Lit::Bool(true)),
        AstLitKind::False => Some(Lit::Bool(false)),
        AstLitKind::Unit => Some(Lit::Unit),
        AstLitKind::Hex(s) => {
            if !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit()) {
                Some(Lit::Hex(s.to_ascii_uppercase()))
            } else if s.is_empty() {
                Some(Lit::Empty)
            } else {
                None
            }
        }
        AstLitKind::Bin(s) => {
            if s.is_empty() {
                return Some(Lit::Empty);
            }
            let mut bits = Vec::with_capacity(s.len());
            for ch in s.chars() {
                match ch {
                    '0' => bits.push(crate::lit::Bit::B0),
                    '1' => bits.push(crate::lit::Bit::B1),
                    _ => return None,
                }
            }
            Some(Lit::Bin(bits))
        }
        AstLitKind::Real(_) => None,
    }
}

/// Lower a canonical pattern node onto the bitvector subset.
pub fn pat_from_ast(p: &AstPat, it: &impl Interner) -> Option<Pat> {
    let span = p.span;
    match &p.kind {
        PatKind::Wild => Some(Pat::Wild { span }),
        PatKind::Lit(l) => Some(Pat::Lit { lit: lit_from_ast(l)?, span }),
        PatKind::Id(name) => Some(Pat::Id {
            name: it.resolve(name.symbol)?.to_owned(),
            width: None,
            span,
        }),
        PatKind::Typ { inner, .. } => Some(Pat::Typ {
            inner: Box::new(pat_from_ast(inner, it)?),
            width: None,
            span,
        }),
        PatKind::Vector(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for e in elems {
                out.push(pat_from_ast(e, it)?);
            }
            Some(Pat::Vector { elems: out, span })
        }
        PatKind::VectorConcat(parts) => {
            let mut out = Vec::with_capacity(parts.len());
            for e in parts {
                out.push(pat_from_ast(e, it)?);
            }
            Some(Pat::VectorConcat { parts: out, span })
        }
        _ => None,
    }
}
