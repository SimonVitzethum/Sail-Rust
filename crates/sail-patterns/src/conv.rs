#![forbid(unsafe_code)]
//! Bridge between the local [`Pat`][crate::Pat] and canonical
//! `sail-ast` pattern nodes (Lane 61 unification, MIGRATION.md #32).
//!
//! Mapping: `Wild/Var/Lit/App/Tuple/Vector/VectorConcat/Or/As/Typ/VarTyp/
//! Struct/List/Cons/StringAppend` map onto the same-named `PatKind`
//! variants. Payload notes:
//! - `Lit(String)` (opaque payload) becomes `LitKind::String`; the
//!   coverage logic only needs literal-vs-wildcard distinction plus the
//!   `"true"`/`"false"` spellings, which survive the round trip.
//! - `Var(name)` (binding wildcard) becomes `Var { pat: Wild, typ_pat:
//!   Wild }`; the binder name has no canonical counterpart and maps
//!   back to `"_"`.
//! - `Typ(pat, spelling)` becomes a `Typ` ascription over `Id(spelling)`;
//!   the reverse drops the ascription (coverage of the inner pattern).
//! - `Struct` with an empty name becomes `StructName::Anon`.
//! - Canonical-only `Not`/`VectorSubrange` map back to `None`.
//!
//! All synthesized nodes carry `NodeId::DUMMY`/`Span::DUMMY`. The
//! exhaustiveness analysis itself is untouched.

use sail_ast::{
    Interner, Lit, LitKind, Name, Node, Pat as AstPat, PatKind, StructName, TypKind, TypPat,
    TypPatKind,
};

use crate::Pat;

fn mk_pat(kind: PatKind) -> AstPat {
    Node::dummy(kind)
}

fn mk_lit(kind: LitKind) -> Lit {
    Node::dummy(kind)
}

fn mk_typpat(kind: TypPatKind) -> TypPat {
    Node::dummy(kind)
}

fn ast_name(it: &mut impl Interner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn resolve_name(name: &Name, it: &impl Interner) -> Option<String> {
    it.resolve(name.symbol).map(str::to_owned)
}

/// Lower a local pattern onto a canonical node.
pub fn pat_to_ast(p: &Pat, it: &mut impl Interner) -> AstPat {
    match p {
        Pat::Wild => mk_pat(PatKind::Wild),
        Pat::Var(_) => mk_pat(PatKind::Var {
            pat: Box::new(mk_pat(PatKind::Wild)),
            typ_pat: mk_typpat(TypPatKind::Wild),
        }),
        Pat::Lit(s) => mk_pat(PatKind::Lit(mk_lit(LitKind::String(s.clone())))),
        Pat::App { name, args } => mk_pat(PatKind::App {
            func: ast_name(it, name),
            args: args.iter().map(|a| pat_to_ast(a, it)).collect(),
        }),
        Pat::Tuple(ps) => mk_pat(PatKind::Tuple(
            ps.iter().map(|a| pat_to_ast(a, it)).collect(),
        )),
        Pat::Vector(ps) => mk_pat(PatKind::Vector(
            ps.iter().map(|a| pat_to_ast(a, it)).collect(),
        )),
        Pat::VectorConcat(ps) => mk_pat(PatKind::VectorConcat(
            ps.iter().map(|a| pat_to_ast(a, it)).collect(),
        )),
        Pat::Or(a, b) => mk_pat(PatKind::Or(
            Box::new(pat_to_ast(a, it)),
            Box::new(pat_to_ast(b, it)),
        )),
        Pat::As(inner, x) => mk_pat(PatKind::As {
            pat: Box::new(pat_to_ast(inner, it)),
            id: ast_name(it, x),
        }),
        Pat::Typ(inner, spelling) => mk_pat(PatKind::Typ {
            typ: Node::dummy(TypKind::Id(ast_name(it, spelling))),
            pat: Box::new(pat_to_ast(inner, it)),
        }),
        Pat::VarTyp(inner) => mk_pat(PatKind::Var {
            pat: Box::new(pat_to_ast(inner, it)),
            typ_pat: mk_typpat(TypPatKind::Wild),
        }),
        Pat::Struct { name, fields, wildcard } => mk_pat(PatKind::Struct {
            name: if name.is_empty() {
                StructName::Anon
            } else {
                StructName::Id(ast_name(it, name))
            },
            fields: fields
                .iter()
                .map(|(n, q)| (ast_name(it, n), pat_to_ast(q, it)))
                .collect(),
            wildcard: *wildcard,
        }),
        Pat::List(ps) => mk_pat(PatKind::List(
            ps.iter().map(|a| pat_to_ast(a, it)).collect(),
        )),
        Pat::Cons(a, b) => mk_pat(PatKind::Cons(
            Box::new(pat_to_ast(a, it)),
            Box::new(pat_to_ast(b, it)),
        )),
        Pat::StringAppend(ps) => mk_pat(PatKind::StringAppend(
            ps.iter().map(|a| pat_to_ast(a, it)).collect(),
        )),
    }
}

fn lit_payload(lit: &Lit) -> String {
    match &lit.kind {
        LitKind::Unit => "()".to_owned(),
        LitKind::True => "true".to_owned(),
        LitKind::False => "false".to_owned(),
        LitKind::Num(s)
        | LitKind::Hex(s)
        | LitKind::Bin(s)
        | LitKind::String(s)
        | LitKind::Real(s) => s.clone(),
    }
}

/// Lift a canonical node back onto a local pattern. `None` for shapes
/// with no local counterpart (`Not`, `VectorSubrange`); binder names
/// without a canonical slot map to `"_"`.
pub fn pat_from_ast(p: &AstPat, it: &impl Interner) -> Option<Pat> {
    match &p.kind {
        PatKind::Lit(l) => Some(Pat::Lit(lit_payload(l))),
        PatKind::Wild => Some(Pat::Wild),
        PatKind::Or(a, b) => Some(Pat::Or(
            Box::new(pat_from_ast(a, it)?),
            Box::new(pat_from_ast(b, it)?),
        )),
        PatKind::Not(_) | PatKind::VectorSubrange { .. } => None,
        PatKind::As { pat, id } => Some(Pat::As(
            Box::new(pat_from_ast(pat, it)?),
            resolve_name(id, it)?,
        )),
        PatKind::Typ { pat, .. } => pat_from_ast(pat, it),
        PatKind::Id(name) => Some(Pat::Var(resolve_name(name, it)?)),
        PatKind::Var { pat, .. } => {
            let inner = pat_from_ast(pat, it)?;
            if matches!(inner, Pat::Wild) {
                Some(Pat::Var("_".to_owned()))
            } else {
                Some(Pat::VarTyp(Box::new(inner)))
            }
        }
        PatKind::App { func, args } => {
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                out.push(pat_from_ast(a, it)?);
            }
            Some(Pat::App { name: resolve_name(func, it)?, args: out })
        }
        PatKind::Vector(ps) => {
            let mut out = Vec::with_capacity(ps.len());
            for a in ps {
                out.push(pat_from_ast(a, it)?);
            }
            Some(Pat::Vector(out))
        }
        PatKind::VectorConcat(ps) => {
            let mut out = Vec::with_capacity(ps.len());
            for a in ps {
                out.push(pat_from_ast(a, it)?);
            }
            Some(Pat::VectorConcat(out))
        }
        PatKind::Tuple(ps) => {
            let mut out = Vec::with_capacity(ps.len());
            for a in ps {
                out.push(pat_from_ast(a, it)?);
            }
            Some(Pat::Tuple(out))
        }
        PatKind::List(ps) => {
            let mut out = Vec::with_capacity(ps.len());
            for a in ps {
                out.push(pat_from_ast(a, it)?);
            }
            Some(Pat::List(out))
        }
        PatKind::Cons(a, b) => Some(Pat::Cons(
            Box::new(pat_from_ast(a, it)?),
            Box::new(pat_from_ast(b, it)?),
        )),
        PatKind::StringAppend(ps) => {
            let mut out = Vec::with_capacity(ps.len());
            for a in ps {
                out.push(pat_from_ast(a, it)?);
            }
            Some(Pat::StringAppend(out))
        }
        PatKind::Struct { name, fields, wildcard } => {
            let n = match name {
                StructName::Id(id) => resolve_name(id, it)?,
                StructName::Anon => String::new(),
            };
            let mut out = Vec::with_capacity(fields.len());
            for (fname, fpat) in fields {
                out.push((resolve_name(fname, it)?, pat_from_ast(fpat, it)?));
            }
            Some(Pat::Struct { name: n, fields: out, wildcard: *wildcard })
        }
    }
}
