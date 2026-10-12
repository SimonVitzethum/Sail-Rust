//! One literal classification for expression checking and inline signatures.
//!
//! Plus the literal-pattern rules (`P-Lit`, OCaml `bind_pat` `P_lit` cases
//! plus the `==`-guard fallback): every [`LitKind`] maps to either a
//! checkable scrutinee obligation or an explicit deferral — real literals
//! need guard synthesis the canonical checker does not have yet, so they
//! stay a loud [`TypeErrorKind::Unsupported`](crate::error::TypeErrorKind)
//! instead of being silently accepted or rejected.
use crate::error::{TypeError, TypeErrorKind, TypeResult};
use sail_ast::*;

pub(crate) enum LiteralType {
    Primitive(&'static str),
    Refined(&'static str, TypArgKind),
}
impl LiteralType {
    pub(crate) fn into_type(self, interner: &mut SimpleInterner) -> Typ {
        Node::dummy(match self {
            Self::Primitive(name) => TypKind::Id(Name::dummy(interner.intern(name))),
            Self::Refined(name, arg) => TypKind::App {
                name: Name::dummy(interner.intern(name)),
                args: vec![Node::dummy(arg)],
            },
        })
    }
}
pub(crate) fn classify(lit: &Lit, interner: &SimpleInterner) -> TypeResult<LiteralType> {
    let value = |symbol| {
        interner
            .resolve(symbol)
            .ok_or_else(|| TypeError::unbound_var(lit.span, "literal symbol".into()))
    };
    Ok(match lit.kind {
        LitKind::Unit => LiteralType::Primitive("unit"),
        LitKind::Real(_) => LiteralType::Primitive("real"),
        LitKind::String(_) => LiteralType::Primitive("string"),
        LitKind::True | LitKind::False => LiteralType::Refined(
            "atom_bool",
            TypArgKind::Bool(Box::new(Node::dummy(
                if matches!(lit.kind, LitKind::True) {
                    ConstraintKind::True
                } else {
                    ConstraintKind::False
                },
            ))),
        ),
        LitKind::Num(symbol) => LiteralType::Refined(
            "atom",
            TypArgKind::NExp(Node::dummy(NExpKind::Constant(value(symbol)?.to_owned()))),
        ),
        LitKind::Bin(symbol) | LitKind::Hex(symbol) => {
            let width = bit_width(
                lit.span,
                matches!(lit.kind, LitKind::Hex(_)),
                value(symbol)?,
            )?;
            LiteralType::Refined(
                "bitvector",
                TypArgKind::NExp(Node::dummy(NExpKind::Constant(width.to_string()))),
            )
        }
    })
}

/// Bitvector-literal width in bits: hex digits count times four, binary
/// digits one-to-one (`_` separators ignored, like OCaml). Shared by
/// expression classification and the `P-Lit` width obligation below.
pub(crate) fn bit_width(span: Span, hex: bool, digits: &str) -> TypeResult<usize> {
    digits
        .bytes()
        .filter(|b| *b != b'_')
        .count()
        .checked_mul(if hex { 4 } else { 1 })
        .ok_or_else(|| TypeError {
            span,
            kind: TypeErrorKind::Unsupported("literal width overflow".into()),
        })
}

/// Obligation a literal pattern imposes on the scrutinee type.
///
/// Mirrors the OCaml `bind_pat` `P_lit` special cases: a decimal literal
/// against `atom('n)` adds the `nexp == value` constraint, `true`/`false`
/// against `atom_bool` add the boolean constraint, and hex/binary literals
/// require a provable width equality against `bitvector(n)`. String, unit
/// and real literals have no atom special case: strings only match
/// `string`/`string_literal`, `()` only `unit`, and reals fall through to
/// the `==`-guard fallback — guard synthesis is not implemented in the
/// canonical checker, so that case stays an explicit deferral.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LitPatRule {
    /// Decimal literal pattern (OCaml `P_lit (L_num n)` against `atom`:
    /// adds `nexp == n`; against `int`/`range`: plain value check).
    Numeric { value: String },
    /// `true`/`false` pattern (OCaml against `atom_bool`: adds `nc` /
    /// `not nc`; otherwise plain `bool` check).
    Bool { value: bool },
    /// Hex/binary pattern: the scrutinee width must provably equal this.
    Bitvector { width: usize },
    /// String literal pattern: only against `string`/`string_literal`
    /// (OCaml `infer_pat` gives `string`, never `string_literal` alone).
    Str,
    /// `()` pattern: only against `unit`.
    Unit,
    /// Real literal pattern: needs the `==`-guard fallback
    /// (`bind_pat` catches the failed subtyping with a fresh variable plus
    /// guard), which the canonical checker cannot synthesize yet.
    GuardedEquality { reason: &'static str },
}

/// Classify a literal in pattern position: every [`LitKind`] yields either
/// a checkable obligation or an explicit error — never silent acceptance.
pub(crate) fn pattern_rule(lit: &Lit, interner: &SimpleInterner) -> TypeResult<LitPatRule> {
    let value = |symbol| {
        interner
            .resolve(symbol)
            .ok_or_else(|| TypeError::unbound_var(lit.span, "literal symbol".into()))
    };
    Ok(match lit.kind {
        LitKind::Num(symbol) => LitPatRule::Numeric {
            value: value(symbol)?.to_owned(),
        },
        LitKind::True => LitPatRule::Bool { value: true },
        LitKind::False => LitPatRule::Bool { value: false },
        LitKind::Bin(symbol) | LitKind::Hex(symbol) => LitPatRule::Bitvector {
            width: bit_width(
                lit.span,
                matches!(lit.kind, LitKind::Hex(_)),
                value(symbol)?,
            )?,
        },
        LitKind::String(_) => LitPatRule::Str,
        LitKind::Unit => LitPatRule::Unit,
        LitKind::Real(_) => LitPatRule::GuardedEquality {
            reason: "real literal patterns need ==-guard synthesis",
        },
    })
}
