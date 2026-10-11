//! One literal classification for expression checking and inline signatures.
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
            let digits = value(symbol)?.bytes().filter(|b| *b != b'_').count();
            let width = digits
                .checked_mul(if matches!(lit.kind, LitKind::Hex(_)) {
                    4
                } else {
                    1
                })
                .ok_or_else(|| TypeError {
                    span: lit.span,
                    kind: TypeErrorKind::Unsupported("literal width overflow".into()),
                })?;
            LiteralType::Refined(
                "bitvector",
                TypArgKind::NExp(Node::dummy(NExpKind::Constant(width.to_string()))),
            )
        }
    })
}
