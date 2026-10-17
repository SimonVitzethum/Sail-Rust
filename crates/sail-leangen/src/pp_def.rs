#![forbid(unsafe_code)]
//! Definitions (`doc_funcl`, `doc_typdef`, `doc_val`, ...).

use sail_ast::{
    Def, DefKind, Exp, ExpKind, Funcl, Kind, NExpKind, PExpKind, Pat, PatKind, QuantItem, RecOpt,
    SimpleInterner, Typ, TypArgKind, TypKind, TypPatKind, TypSchm, TypeDef, TypeDefKind,
};

use crate::analysis::{has_early_return, has_effect, has_loop};
use crate::ctx::{Ctx, Global};
use crate::doc::{
    brk, chr, comma_sp, concat, empty, flow, group_of, hardline, separate, space, string, Doc,
};
use crate::names::{doc_id_ctor, name_text};
use crate::pp_exp::{fn_implicits, ExpPrinter};
use crate::pp_pat::{doc_pat, update_ctx_pat};
use crate::pp_typ::{
    doc_kind, doc_nconstraint, doc_nexp, doc_typ, doc_typ_quant_in_comment, doc_typ_quant_only_vars,
    doc_typ_quant_relevant, Result,
};

fn remove_empties(docs: Vec<Doc>) -> Vec<Doc> {
    docs.into_iter().filter(|d| !d.is_empty()).collect()
}

/// `pat_is_plain_binder ?suffix env pat`: `Some(Some(name))`, `Some(None)` for a wildcard.
fn pat_plain_binder(ctx: &Ctx<'_>, it: &SimpleInterner, suffix: &str, pat: &Pat) -> Option<Option<String>> {
    match &pat.kind {
        PatKind::Id(id) => {
            let text = name_text(id, it);
            if ctx.global.oracle.is_enum_member(&text) {
                Some(Some(format!("id{suffix}")))
            } else {
                Some(Some(text))
            }
        }
        PatKind::Typ { pat: inner, .. } => match &inner.kind {
            PatKind::Id(id) => {
                let text = name_text(id, it);
                if ctx.global.oracle.is_enum_member(&text) {
                    Some(Some(format!("typ{suffix}")))
                } else {
                    Some(Some(text))
                }
            }
            PatKind::Wild => Some(None),
            _ => Some(Some(format!("typ{suffix}"))),
        },
        PatKind::Wild => Some(None),
        PatKind::Var { .. } => Some(Some(format!("var{suffix}"))),
        PatKind::App { .. } => Some(Some(format!("app{suffix}"))),
        PatKind::Vector(_) => Some(Some(format!("vect{suffix}"))),
        PatKind::Tuple(_) => Some(Some(format!("tuple{suffix}"))),
        PatKind::List(_) => Some(Some(format!("list{suffix}"))),
        PatKind::Cons(..) => Some(Some(format!("cons{suffix}"))),
        PatKind::Lit(l) if matches!(l.kind, sail_ast::LitKind::Unit) => Some(Some("_".to_owned())),
        PatKind::Lit(_) => Some(Some(format!("lit{suffix}"))),
        PatKind::Struct { .. } => Some(Some(format!("struct_pat{suffix}"))),
        _ => None,
    }
}

/// A binder introduced by `untuple_args_pat`: the pattern, its type and, when
/// the pattern had to be split, the let-binding to rebuild it.
struct ArgBinder<'p> {
    pat: &'p Pat,
    typ: &'p Typ,
}

/// `untuple_args_pat`: one `(pattern, type)` per declared argument. Patterns
/// binding several arguments at once are not produced by the rewrites that
/// run before printing, so the general `P_id`/`P_as` case is rejected.
fn untuple_args_pat<'p>(typs: &'p [Typ], pat: &'p Pat) -> std::result::Result<Vec<ArgBinder<'p>>, String> {
    match (&pat.kind, typs) {
        (PatKind::Tuple(pats), _) if pats.is_empty() => Ok(vec![ArgBinder { pat, typ: &typs[0] }]),
        (PatKind::Lit(l), _) if matches!(l.kind, sail_ast::LitKind::Unit) => {
            Ok(vec![ArgBinder { pat, typ: &typs[0] }])
        }
        (PatKind::Tuple(_), [typ]) => Ok(vec![ArgBinder { pat, typ }]),
        (PatKind::Tuple(pats), _) if pats.len() == typs.len() => {
            Ok(pats.iter().zip(typs).map(|(pat, typ)| ArgBinder { pat, typ }).collect())
        }
        (PatKind::Wild, _) => Ok(typs.iter().map(|typ| ArgBinder { pat, typ }).collect()),
        (PatKind::Typ { pat: inner, .. }, _) => untuple_args_pat(typs, inner),
        (PatKind::As { .. } | PatKind::Id(_), [_, _, ..]) => {
            Err("a single binder for several arguments is not supported".into())
        }
        (_, [typ]) => Ok(vec![ArgBinder { pat, typ }]),
        _ => Err("Unexpected pattern/type combination".into()),
    }
}

/// `captured_typ_var`: the kid of an `atom('n)`/`implicit('n)` argument type.
fn captured_typ_var(typ: &Typ, it: &SimpleInterner) -> Option<String> {
    if let TypKind::App { name, args } = &typ.kind {
        let n = name_text(name, it);
        if n == "atom" || n == "implicit" {
            if let [a] = args.as_slice() {
                if let TypArgKind::NExp(ne) = &a.kind {
                    if let NExpKind::Var(k) = &ne.kind {
                        return Some(sail_ast::Interner::resolve(it, *k).unwrap_or("?").to_owned());
                    }
                }
            }
        }
    }
    None
}

fn is_implicit_typ(typ: &Typ, it: &SimpleInterner) -> bool {
    if let TypKind::App { name, args } = &typ.kind {
        if name_text(name, it) == "implicit" {
            if let [a] = args.as_slice() {
                return matches!(&a.kind, TypArgKind::NExp(n) if matches!(n.kind, NExpKind::Var(_)));
            }
        }
    }
    false
}

/// `add_path_renamings`.
fn add_path_renamings(
    path: &str,
    ctx: &mut Ctx<'_>,
    pat: &Pat,
    pat_it: &SimpleInterner,
    typ: &Typ,
    typ_it: &SimpleInterner,
) {
    match (&pat.kind, &typ.kind) {
        (PatKind::Tuple(pats), TypKind::Tuple(typs)) if pats.len() == typs.len() => {
            for (i, (p, t)) in pats.iter().zip(typs).enumerate() {
                add_path_renamings(&format!("{path}.{}", i + 1), ctx, p, pat_it, t, typ_it);
            }
        }
        (PatKind::Id(_), _) => {
            if let Some(kid) = captured_typ_var(typ, typ_it) {
                ctx.add_kid_id_rename(path, &kid);
            }
        }
        _ => {}
    }
}

/// Result of `doc_funcl_init`.
pub struct FunclInit<'a> {
    pub comment: Doc,
    pub signature: Doc,
    pub ctx: Ctx<'a>,
    /// Pattern arguments Lean cannot take directly: `(pattern, variable)`.
    pub fixups: Vec<(Pat, String)>,
}

fn pat_binder_fixup(pat: &Pat, var: &str) -> Option<(Pat, String)> {
    match &pat.kind {
        PatKind::Id(_)
        | PatKind::Typ { .. } if matches!(&pat.kind, PatKind::Id(_)) || matches!(&pat.kind, PatKind::Typ { pat: p, .. } if matches!(p.kind, PatKind::Id(_))) => None,
        PatKind::Tuple(p) if p.is_empty() => None,
        PatKind::Lit(_) | PatKind::Wild => None,
        _ => Some((pat.clone(), var.to_owned())),
    }
}

pub struct FunctionPrinter<'g, 'a> {
    pub global: &'g Global<'a>,
    /// Interner of the function definition.
    pub it: &'a SimpleInterner,
}

impl<'g, 'a: 'g> FunctionPrinter<'g, 'a> {
    /// `doc_funcl_init`.
    pub fn funcl_init(&self, funcl: &Funcl, val_spec: &(TypSchm, std::sync::Arc<SimpleInterner>)) -> Result<(Doc, Doc, Ctx<'g>, Vec<(Pat, String)>, bool, bool, bool)>
    where
        'a: 'g,
    {
        let (scheme, sit) = val_spec;
        let id_text = name_text(&funcl.name, self.it);
        let (arg_typs, ret_typ) = match &scheme.typ.kind {
            TypKind::Fn { args, ret } => (args.as_slice(), ret.as_ref()),
            _ => return Err(format!("Function {id_text} does not have function type")),
        };
        let (pat, exp) = match &funcl.arm.kind {
            PExpKind::PatExp { pat, body } => (pat, body),
            PExpKind::PatWhen { pat, body, .. } => (pat, body),
        };
        let binders = untuple_args_pat(arg_typs, pat)?;
        let mut ctx = Ctx::new(self.global, self.it);
        ctx.quant = None;
        let mut binder_docs = Vec::new();
        let mut fixups = Vec::new();
        let mut renames: Vec<(String, String)> = Vec::new();
        for (i, b) in binders.iter().enumerate() {
            let suffix = format!("_{i}");
            let name = match pat_plain_binder(&ctx, self.it, &suffix, b.pat) {
                Some(Some(n)) => n,
                Some(None) => format!("x_{i}"),
                None => "TODO_ARG_PATTERN".to_owned(),
            };
            // doc_binder: kid captured by an atom/implicit argument type.
            if let Some(kid) = captured_typ_var(b.typ, sit) {
                renames.push((name.clone(), kid));
            }
            let id_doc = string(&crate::names::fix_id(&name));
            // Rename before printing the type so `k_n` becomes the argument name.
            let mut tctx: Ctx<'_> = ctx.clone();
            tctx.quant = Some((scheme.quant.as_slice(), &**sit));
            for (id, kid) in &renames {
                tctx.add_kid_id_rename(id, kid);
            }
            let typ_doc = doc_typ(&tctx, sit, b.typ)?;
            let body = flow(&brk(1), [id_doc, chr(':'), typ_doc]);
            let implicit = is_implicit_typ(b.typ, sit);
            binder_docs.push(if implicit { body.braces() } else { body.parens() });
            if let Some(f) = pat_binder_fixup(b.pat, &name) {
                fixups.push(f);
            }
            if let Some(kid) = captured_typ_var(b.typ, sit) {
                ctx.add_kid_id_rename(&name, &kid);
            }
            add_path_renamings(&name, &mut ctx, b.pat, self.it, b.typ, sit);
        }
        let mut ctx_with_quant: Ctx<'_> = ctx.clone();
        ctx_with_quant.quant = Some((scheme.quant.as_slice(), &**sit));
        let comment = doc_typ_quant_in_comment(&ctx_with_quant, sit, &scheme.quant)?;
        let doc_ret_typ_orig = doc_typ(&ctx_with_quant, sit, ret_typ)?;
        let is_monadic = !self.global.oracle.function_is_pure(&id_text);
        let early_return = has_early_return(exp, self.it);
        let has_loop_b = has_loop(exp, self.it);
        let doc_ret_typ = if is_monadic {
            string("SailM ").cat(doc_ret_typ_orig.clone())
        } else {
            doc_ret_typ_orig.clone()
        };
        let mut decl_val = vec![doc_ret_typ, string(":=")];
        match (is_monadic, early_return, has_loop_b) {
            (true, true, _) => decl_val.extend([string("SailME.run"), string("do")]),
            (true, _, _) => decl_val.push(string("do")),
            (false, false, true) => decl_val.extend([string("Id.run"), string("do")]),
            (false, true, _) => decl_val.extend([string("ExceptM.run"), string("do")]),
            _ => {}
        }
        let partiality = if self.global.partial_functions.contains(&id_text) {
            string("partial")
        } else {
            empty()
        };
        let computability = if self.global.noncomputable_functions.contains(&id_text) {
            string("noncomputable")
        } else {
            empty()
        };
        let mut head = remove_empties(vec![partiality, computability, string("def"), doc_id_ctor(&ctx, self.it, &funcl.name, false)]);
        head.extend(binder_docs);
        head.push(chr(':'));
        head.extend(decl_val);
        let signature = flow(&space(), head);
        // Contexts carry the monad flags for the body.
        let mut body_ctx = ctx;
        body_ctx.in_sail_monad = is_monadic;
        body_ctx.in_except_monad = if early_return { Some(doc_ret_typ_orig) } else { None };
        // Re-borrow the quantifier assumptions for `provably_nneg` inside the body.
        Ok((comment, signature, body_ctx, fixups, is_monadic, early_return, has_loop_b))
    }
}

/// Print `exp` after the pattern fixups (`let pat := var` lines).
fn print_with_fixups(p: &ExpPrinter<'_, '_>, as_monadic: bool, fixups: &[(Pat, String)], exp: &Exp) -> Result<Doc> {
    let mut docs = Vec::new();
    let mut ctx = p.ctx.clone();
    for (pat, var) in fixups {
        let id_typ = doc_pat(&ctx, p.it, false, false, false, pat)?;
        update_ctx_pat(&mut ctx, p.it, pat);
        let line = flow(
            &brk(1),
            [
                separate(&space(), [string("let"), id_typ, string(":=")]),
                string(&crate::names::fix_id(var)),
            ],
        )
        .nest(2)
        .group();
        docs.push(line.cat(hardline()));
    }
    let inner = ExpPrinter { ctx: &ctx, it: p.it };
    Ok(concat(docs).cat(inner.doc_exp(as_monadic, exp)?))
}

/// `doc_funcl`.
pub fn doc_funcl<'g, 'a: 'g>(
    global: &'g Global<'a>,
    it: &'a SimpleInterner,
    rec: &RecOpt,
    funcl: &Funcl,
    val_spec: &(TypSchm, std::sync::Arc<SimpleInterner>),
) -> Result<Doc> {
    let printer = FunctionPrinter { global, it };
    let (comment, signature, ctx, fixups, is_monadic, _early, _loop) = printer.funcl_init(funcl, val_spec)?;
    let id_text = name_text(&funcl.name, it);
    let exp = match &funcl.arm.kind {
        PExpKind::PatExp { body, .. } | PExpKind::PatWhen { body, .. } => body,
    };
    let quant_ctx = Ctx {
        quant: Some((&val_spec.0.quant, &*val_spec.1)),
        ..ctx.clone()
    };
    let p = ExpPrinter { ctx: &quant_ctx, it };
    let monadic_body = has_effect(exp, it, global.oracle) || !global.oracle.function_is_pure(&id_text);
    let body = if untranslatable_mapping(&id_text, exp, it) {
        string("throw Error.Exit")
    } else {
        print_with_fixups(&p, monadic_body, &fixups, exp)?
    };
    let termination = doc_termination(&quant_ctx, it, rec, funcl, &fixups)?;
    let _ = is_monadic;
    Ok(comment.cat(signature.cat(hardline().cat(body)).nest(2)).cat(termination))
}

fn untranslatable_mapping(id: &str, exp: &Exp, it: &SimpleInterner) -> bool {
    let regex_match = id.ends_with("_forwards")
        || id.ends_with("_backwards")
        || id.ends_with("_forwards_matches")
        || id.ends_with("_backwards_matches");
    if regex_match {
        return false;
    }
    fn exp_disc(e: &Exp, it: &SimpleInterner) -> bool {
        match &e.kind {
            ExpKind::Let { body, .. } => exp_disc(body, it),
            ExpKind::App { args, .. } => {
                matches!(args.as_slice(), [a] if matches!(a.kind, ExpKind::Exit(_)))
            }
            _ => false,
        }
    }
    fn exp_match(e: &Exp, it: &SimpleInterner) -> bool {
        match &e.kind {
            ExpKind::Let { body, .. } => exp_match(body, it),
            ExpKind::Match { scrutinee, .. } => exp_disc(scrutinee, it),
            _ => false,
        }
    }
    exp_match(exp, it)
}

fn doc_termination(
    ctx: &Ctx<'_>,
    it: &SimpleInterner,
    rec: &RecOpt,
    funcl: &Funcl,
    fixups: &[(Pat, String)],
) -> Result<Doc> {
    match rec {
        RecOpt::NonRec | RecOpt::Rec => Ok(empty()),
        RecOpt::Measure { pat, exp } => {
            let fnpat = match &funcl.arm.kind {
                PExpKind::PatExp { pat, .. } | PExpKind::PatWhen { pat, .. } => pat,
            };
            let p = ExpPrinter { ctx, it };
            let rhs = pat_to_exp_doc(ctx, it, fnpat)?;
            let id_typ = doc_pat(ctx, it, false, false, false, pat)?;
            let mut inner_ctx = ctx.clone();
            update_ctx_pat(&mut inner_ctx, it, pat);
            let p2 = ExpPrinter { ctx: &inner_ctx, it };
            let let_line = flow(
                &brk(1),
                [separate(&space(), [string("let"), id_typ, string(":=")]), rhs],
            )
            .nest(2)
            .group();
            let term = let_line.cat(hardline()).cat(p2.doc_exp(false, exp)?);
            let term = if fixups.is_empty() { term } else { term };
            let _ = p;
            Ok(hardline().cat(string("termination_by ").cat(term.parens()).cat(string(".toNat"))))
        }
    }
}

/// `Rewrites.pat_to_exp` rendered as a document.
fn pat_to_exp_doc(ctx: &Ctx<'_>, it: &SimpleInterner, pat: &Pat) -> Result<Doc> {
    match &pat.kind {
        PatKind::Id(id) => Ok(doc_id_ctor(ctx, it, id, true)),
        PatKind::Typ { pat, .. } | PatKind::Var { pat, .. } | PatKind::As { pat, .. } => pat_to_exp_doc(ctx, it, pat),
        PatKind::Tuple(pats) => {
            let docs: Vec<Doc> = pats.iter().map(|p| pat_to_exp_doc(ctx, it, p)).collect::<Result<_>>()?;
            Ok(separate(&comma_sp(), docs).parens())
        }
        PatKind::Lit(l) => Ok(crate::names::doc_lit(it, l, true)),
        PatKind::Wild => Ok(string("()")),
        _ => Err("pattern in termination measure is not convertible to an expression".into()),
    }
}

/// `doc_type_union`.
fn doc_type_union(ctx: &Ctx<'_>, it: &SimpleInterner, union: &sail_ast::TypeUnion) -> Result<Doc> {
    Ok(flow(
        &space(),
        [
            chr('|'),
            doc_id_ctor(ctx, it, &union.ctor, false),
            flow(&space(), [chr('_'), chr(':'), doc_typ(ctx, it, &union.typ)?]).parens(),
        ],
    )
    .nest(2))
}

fn derivers_doc(items: Vec<&str>) -> Doc {
    separate(&comma_sp(), items.into_iter().map(string))
}

/// `doc_typdef`.
pub fn doc_typdef(ctx: &Ctx<'_>, it: &SimpleInterner, td: &TypeDef) -> Result<Doc> {
    let g = ctx.global;
    match &td.kind {
        TypeDefKind::Enum { name, members, .. } => {
            let id_text = name_text(name, it);
            let ids: Vec<Doc> = members
                .iter()
                .map(|m| space().cat(chr('|')).cat(space()).cat(doc_id_ctor(ctx, it, m, false)))
                .collect();
            let mut derivers = if members.is_empty() { vec!["Repr"] } else { vec!["Inhabited", "Repr"] };
            if !g.non_beq_types.contains(&id_text) {
                derivers.insert(0, "BEq");
            }
            g.opens.borrow_mut().insert(id_text.clone());
            let id = doc_id_ctor(ctx, it, name, false);
            Ok(flow(&brk(1), [string("inductive"), id.clone(), string("where")])
                .cat(concat(ids))
                .cat(hardline())
                .cat(string("deriving"))
                .cat(space())
                .cat(derivers_doc(derivers))
                .cat(hardline())
                .cat(string("open"))
                .cat(space())
                .cat(id)
                .nest(2))
        }
        TypeDefKind::Record { name, quant, fields, .. } => {
            let id_text = name_text(name, it);
            let field_docs: Vec<Doc> = fields
                .iter()
                .map(|(f, t)| {
                    Ok(flow(&brk(1), [doc_id_ctor(ctx, it, f, false), chr(':'), doc_typ(ctx, it, t)?]))
                })
                .collect::<Result<_>>()?;
            let rectyp: Vec<Doc> = doc_typ_quant_relevant(ctx, it, quant).into_iter().map(Doc::parens).collect();
            let mut derivers = vec!["Inhabited", "Repr"];
            if !g.non_beq_types.contains(&id_text) {
                derivers.insert(0, "BEq");
            }
            let mut head = vec![string("structure"), doc_id_ctor(ctx, it, name, false)];
            if !rectyp.is_empty() {
                head.push(separate(&space(), rectyp));
            }
            head.push(string("where"));
            Ok(doc_typ_quant_in_comment(ctx, it, quant)?.cat(
                flow(&brk(1), remove_empties(head))
                    .cat(hardline())
                    .cat(separate(&hardline(), field_docs))
                    .cat(hardline())
                    .cat(string("deriving"))
                    .cat(space())
                    .cat(derivers_doc(derivers))
                    .nest(2),
            ))
        }
        TypeDefKind::Abbrev { name, quant, arg } => {
            let id = doc_id_ctor(ctx, it, name, false);
            let id_text = name_text(name, it);
            match &arg.kind {
                TypArgKind::Typ(t) => {
                    let is_range = matches!(&t.kind, TypKind::App { name: n, .. } if name_text(n, it) == "range");
                    if id_text == "fp_bits" {
                        return Ok(string(&format!("-- Abbreviation {id_text} skipped")));
                    }
                    let vars: Vec<Doc> = if is_range {
                        doc_typ_quant_relevant(ctx, it, quant).into_iter().map(Doc::parens).collect()
                    } else {
                        doc_typ_quant_only_vars(ctx, it, quant)
                    };
                    let mut parts = vec![string("abbrev"), id];
                    if !vars.is_empty() {
                        parts.push(separate(&space(), vars));
                    }
                    parts.extend([string(":="), doc_typ(ctx, it, t)?]);
                    Ok(flow(&brk(1), parts).nest(2))
                }
                TypArgKind::NExp(ne) => {
                    let vars: Vec<Doc> = doc_typ_quant_relevant(ctx, it, quant).into_iter().map(Doc::parens).collect();
                    let mut parts = vec![string("abbrev"), id];
                    if !vars.is_empty() {
                        parts.push(separate(&space(), vars));
                    }
                    parts.extend([chr(':'), string("Int"), string(":="), doc_nexp(ctx, it, ne)?]);
                    Ok(flow(&brk(1), parts).nest(2))
                }
                TypArgKind::Bool(c) => {
                    let vars: Vec<Doc> = doc_typ_quant_relevant(ctx, it, quant).into_iter().map(Doc::parens).collect();
                    let mut parts = vec![string("abbrev"), id];
                    if !vars.is_empty() {
                        parts.push(separate(&space(), vars));
                    }
                    parts.extend([chr(':'), string("Bool"), string(":="), doc_nconstraint(ctx, it, c)?]);
                    Ok(flow(&brk(1), parts).nest(2))
                }
            }
        }
        TypeDefKind::Variant { name, quant, ctors, .. } => {
            let id_text = name_text(name, it);
            let tus: Vec<Doc> = ctors
                .iter()
                .map(|u| Ok(hardline().cat(doc_type_union(ctx, it, u)?)))
                .collect::<Result<_>>()?;
            let rectyp: Vec<Doc> = doc_typ_quant_relevant(ctx, it, quant).into_iter().map(Doc::parens).collect();
            g.opens.borrow_mut().insert(id_text.clone());
            let mut derivers = vec!["Repr"];
            if !g.non_beq_types.contains(&id_text) {
                derivers.insert(0, "BEq");
            }
            if !ctors.is_empty() {
                derivers.insert(0, "Inhabited");
            }
            let mut head = vec![string("inductive"), doc_id_ctor(ctx, it, name, false)];
            if !rectyp.is_empty() {
                head.push(separate(&space(), rectyp));
            }
            head.push(string("where"));
            let head = flow(&space(), remove_empties(head)).nest(2);
            Ok(doc_typ_quant_in_comment(ctx, it, quant)?.cat(
                head.cat(concat(tus))
                    .cat(hardline())
                    .cat(string("deriving"))
                    .cat(space())
                    .cat(derivers_doc(derivers))
                    .cat(hardline())
                    .cat(string("open"))
                    .cat(space())
                    .cat(doc_id_ctor(ctx, it, name, false))
                    .nest(2),
            ))
        }
        TypeDefKind::Bitfield { .. } | TypeDefKind::Abstract { .. } => {
            Err("Type definition not translatable yet".into())
        }
    }
}

/// `doc_val`: a global `let`. Returns the global kid renames to install.
pub fn doc_val(
    ctx: &Ctx<'_>,
    it: &SimpleInterner,
    pat: &Pat,
    exp: &Exp,
) -> Result<(Option<(String, String)>, Doc)> {
    // Unwrap `P_typ`, `P_var` with the kid named like the id.
    let (typ, inner) = match &pat.kind {
        PatKind::Typ { typ, pat } => (Some(typ), pat.as_ref()),
        _ => (None, pat),
    };
    let (id, rename) = match &inner.kind {
        PatKind::Id(id) => (id, None),
        PatKind::Var { pat: p, typ_pat } => {
            let PatKind::Id(id) = &p.kind else {
                return Err("Pattern not translatable yet".into());
            };
            let kid = match &typ_pat.kind {
                TypPatKind::Var(k) => Some(*k),
                TypPatKind::App { name, args } if name_text(name, it) == "atom" => match args.as_slice() {
                    [a] => match &a.kind {
                        TypPatKind::Var(k) => Some(*k),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            };
            match kid {
                Some(k) => {
                    let kid_text = sail_ast::Interner::resolve(it, k).unwrap_or("?").to_owned();
                    let id_text = name_text(id, it);
                    if format!("'{id_text}") == kid_text {
                        (id, Some((id_text, kid_text)))
                    } else {
                        return Err("Pattern not translatable yet".into());
                    }
                }
                None => return Err("Pattern not translatable yet".into()),
            }
        }
        _ => return Err("Pattern not translatable yet".into()),
    };
    let typpp = match typ {
        None => empty(),
        Some(t) => space().cat(chr(':')).cat(space()).cat(doc_typ(ctx, it, t)?),
    };
    let idpp = doc_id_ctor(ctx, it, id, false);
    let p = ExpPrinter { ctx, it };
    let base = if has_effect(exp, it, ctx.global.oracle) {
        string("unwrapValue").cat(space()).cat(p.doc_exp(true, exp)?.parens())
    } else {
        p.doc_exp(false, exp)?
    };
    let doc = string("def")
        .cat(space())
        .cat(idpp)
        .cat(typpp)
        .cat(space())
        .cat(string(":="))
        .cat(brk(1))
        .cat(base)
        .group()
        .nest(2);
    Ok((rename, doc))
}

#[allow(dead_code)]
fn unused(_: &Def, _: &DefKind, _: Kind, _: &QuantItem) {
    let _ = (group_of, doc_kind);
}
