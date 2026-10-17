#![forbid(unsafe_code)]
//! Patterns (`doc_pat`, `doc_vector_concat`, ...).

use sail_ast::{LitKind, Pat, PatKind, SimpleInterner};

use crate::ctx::Ctx;
use crate::doc::{chr, concat, empty, separate, space, string, Doc};
use crate::names::{doc_id_ctor, doc_lit, fix_id, name_text};
use crate::pp_typ::{doc_typ, Result};

/// `fixup_match_id`.
#[must_use]
pub fn fixup_match_id(text: &str) -> String {
    match text {
        "Some" => "some".to_owned(),
        "None" => "none".to_owned(),
        "early_return" => "throw".to_owned(),
        other => fix_id(other),
    }
}

fn doc_fixed_id(ctx: &Ctx<'_>, it: &SimpleInterner, id: &sail_ast::Name) -> Doc {
    let text = name_text(id, it);
    let fixed = sail_ast::Name {
        symbol: id.symbol,
        ..id.clone()
    };
    let _ = fixed;
    if id.is_operator || text == "and_bool" || text == "or_bool" {
        return doc_id_ctor(ctx, it, id, false);
    }
    string(&fixup_match_id(&text))
}

/// Binds `'n` patterns (`update_ctx_pat`): `let 'n = e` renames the kid.
pub fn update_ctx_pat(ctx: &mut Ctx<'_>, it: &SimpleInterner, pat: &Pat) {
    match &pat.kind {
        PatKind::Var { pat: inner, typ_pat } => {
            if let (PatKind::Id(id), sail_ast::TypPatKind::Var(kid)) = (&inner.kind, &typ_pat.kind) {
                let kid_text = sail_ast::Interner::resolve(it, *kid).unwrap_or("?").to_owned();
                ctx.add_kid_id_rename(&name_text(id, it), &kid_text);
                return;
            }
            update_ctx_pat(ctx, it, inner);
        }
        PatKind::Typ { pat: inner, .. } | PatKind::As { pat: inner, .. } => update_ctx_pat(ctx, it, inner),
        PatKind::App { args: pats, .. }
        | PatKind::Vector(pats)
        | PatKind::VectorConcat(pats)
        | PatKind::Tuple(pats)
        | PatKind::List(pats)
        | PatKind::StringAppend(pats) => {
            for p in pats {
                update_ctx_pat(ctx, it, p);
            }
        }
        _ => {}
    }
}

/// `doc_pat ?need_parens ?in_vector ctx in_match_bv pat`.
pub fn doc_pat(
    ctx: &Ctx<'_>,
    it: &SimpleInterner,
    need_parens: bool,
    in_vector: bool,
    in_match_bv: bool,
    pat: &Pat,
) -> Result<Doc> {
    let opt_parens = |d: Doc| if need_parens { d.parens() } else { d };
    let sub = |p: &Pat| doc_pat(ctx, it, false, false, in_match_bv, p);
    match &pat.kind {
        PatKind::Wild => Ok(chr('_')),
        PatKind::Lit(l) => Ok(doc_lit(it, l, false)),
        PatKind::Typ { typ, pat: inner } => {
            if in_vector {
                if let sail_ast::TypKind::Id(id) = &typ.kind {
                    if name_text(id, it) == "bit" {
                        return Ok(doc_pat(ctx, it, false, false, in_match_bv, inner)?.cat(string(":1")));
                    }
                }
                if let sail_ast::TypKind::App { name, args } = &typ.kind {
                    let n = name_text(name, it);
                    if (n == "bits" || n == "bitvector") && args.len() == 1 {
                        if let sail_ast::TypArgKind::NExp(ne) = &args[0].kind {
                            if let sail_ast::NExpKind::Constant(c) = &ne.kind {
                                return Ok(doc_pat(ctx, it, false, false, in_match_bv, inner)?
                                    .cat(string(":"))
                                    .cat(string(c)));
                            }
                        }
                    }
                }
                return Ok(doc_pat(ctx, it, false, false, in_match_bv, inner)?
                    .cat(string(":"))
                    .cat(doc_typ(ctx, it, typ)?));
            }
            doc_pat(ctx, it, false, false, in_match_bv, inner)
        }
        PatKind::Id(id) => {
            if in_vector {
                if let Some(width) = crate::analysis::pat_const_bits(pat, it) {
                    return Ok(doc_fixed_id(ctx, it, id).cat(string(":")).cat(string(&width.to_string())));
                }
            }
            let text = name_text(id, it);
            let prefix = if ctx.global.oracle.is_enum_member(&text) {
                string(".")
            } else {
                empty()
            };
            Ok(prefix.cat(doc_fixed_id(ctx, it, id)))
        }
        PatKind::Tuple(pats) => {
            let docs: Vec<Doc> = pats.iter().map(sub).collect::<Result<_>>()?;
            Ok(separate(&string(", "), docs).parens())
        }
        PatKind::List(pats) => {
            let docs: Vec<Doc> = pats.iter().map(sub).collect::<Result<_>>()?;
            Ok(separate(&string(", "), docs).brackets())
        }
        PatKind::Vector(pats)
            if pats.iter().all(|p| matches!(p.kind, PatKind::Lit(_))) && !in_match_bv =>
        {
            let docs: Vec<Doc> = pats
                .iter()
                .map(|p| doc_pat(ctx, it, false, true, in_match_bv, p))
                .collect::<Result<_>>()?;
            Ok(string("0b").cat(concat(docs)))
        }
        PatKind::Vector(pats) => {
            let docs: Vec<Doc> = pats
                .iter()
                .map(|p| doc_pat(ctx, it, false, true, in_match_bv, p))
                .collect::<Result<_>>()?;
            Ok(concat(docs))
        }
        PatKind::VectorConcat(pats) => doc_vector_concat(ctx, it, pats),
        PatKind::App { func, args } => {
            if name_text(func, it) == "None" {
                return Ok(string("none"));
            }
            let docs: Vec<Doc> = args
                .iter()
                .map(|p| doc_pat(ctx, it, true, false, in_match_bv, p))
                .collect::<Result<_>>()?;
            Ok(opt_parens(
                string(".").cat(doc_fixed_id(ctx, it, func)).cat(space()).cat(separate(&string(", "), docs)),
            ))
        }
        PatKind::Var { pat: inner, .. } => sub(inner),
        PatKind::As { pat: inner, .. } => sub(inner),
        PatKind::Struct { fields, .. } => {
            let docs: Vec<Doc> = fields
                .iter()
                .map(|(id, p)| {
                    Ok(separate(
                        &space(),
                        [doc_id_ctor(ctx, it, id, false), string(":="), sub(p)?],
                    ))
                })
                .collect::<Result<_>>()?;
            Ok(space().cat(separate(&comma_space(), docs)).cat(space()).braces())
        }
        PatKind::Cons(h, t) => Ok(separate(&space(), [sub(h)?, string("::"), sub(t)?]).parens()),
        other => Err(format!("Doc Pattern {} not translatable yet", pat_con(other))),
    }
}

fn comma_space() -> Doc {
    crate::doc::comma().cat(space())
}

fn pat_con(kind: &PatKind) -> &'static str {
    match kind {
        PatKind::Or(..) => "P_or",
        PatKind::Not(..) => "P_not",
        PatKind::VectorSubrange { .. } => "P_vector_subrange",
        PatKind::StringAppend(..) => "P_string_append",
        _ => "pattern",
    }
}

fn doc_vector_concat(ctx: &Ctx<'_>, it: &SimpleInterner, pats: &[Pat]) -> Result<Doc> {
    fn part(ctx: &Ctx<'_>, it: &SimpleInterner, pat: &Pat) -> Result<Doc> {
        match &pat.kind {
            PatKind::Lit(l) => match &l.kind {
                LitKind::Bin(b) => Ok(string(sail_ast::Interner::resolve(it, *b).unwrap_or(""))),
                LitKind::Hex(h) => {
                    let digits = sail_ast::Interner::resolve(it, *h).unwrap_or("");
                    let bits: String = digits
                        .chars()
                        .filter_map(|c| c.to_digit(16))
                        .map(|d| format!("{d:04b}"))
                        .collect();
                    Ok(string(&bits))
                }
                _ => Err("Unexpected literal in match_bv vector_concat pattern".into()),
            },
            PatKind::Id(id) => match crate::analysis::pat_const_bits(pat, it) {
                Some(n) => Ok(doc_fixed_id(ctx, it, id).cat(chr(':')).cat(string(&n.to_string()))),
                None => Err("Found subpattern with unclear width in bitvector pattern".into()),
            },
            PatKind::Typ { pat: inner, .. } => part(ctx, it, inner),
            PatKind::Vector(items) => {
                let docs: Vec<Doc> = items.iter().map(|p| part(ctx, it, p)).collect::<Result<_>>()?;
                Ok(separate(&crate::doc::comma(), docs))
            }
            _ => Err("Unexpected pattern in match_bv vector_concat pattern".into()),
        }
    }
    let docs: Vec<Doc> = pats.iter().map(|p| part(ctx, it, p)).collect::<Result<_>>()?;
    Ok(separate(&crate::doc::comma(), docs).brackets())
}

/// `doc_pat_typ_ascription`.
pub fn doc_pat_typ_ascription(ctx: &Ctx<'_>, it: &SimpleInterner, pat: &Pat) -> Result<Option<Doc>> {
    match &pat.kind {
        PatKind::Typ { typ, .. } => Ok(Some(doc_typ(ctx, it, typ)?)),
        _ => Ok(None),
    }
}

/// `is_anonymous_pat`.
#[must_use]
pub fn is_anonymous_pat(it: &SimpleInterner, pat: &Pat) -> bool {
    match &pat.kind {
        PatKind::Wild | PatKind::Lit(_) => true,
        PatKind::Id(id) => name_text(id, it).starts_with('_'),
        PatKind::Typ { pat: inner, .. } => is_anonymous_pat(it, inner),
        _ => false,
    }
}

/// `is_bitvector_pattern`.
#[must_use]
pub fn is_bitvector_pattern(pat: &Pat) -> bool {
    match &pat.kind {
        PatKind::Vector(_) | PatKind::VectorConcat(_) => true,
        PatKind::As { pat, .. } => is_bitvector_pattern(pat),
        _ => false,
    }
}
