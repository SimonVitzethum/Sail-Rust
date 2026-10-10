#![forbid(unsafe_code)]
//! Bridge between the local mapping fragment and canonical `sail-ast`
//! nodes (Lane 61 unification, MIGRATION.md #35–#40).
//!
//! Mapping (all lowerings synthesize `NodeId::DUMMY`/`Span::DUMMY` and
//! intern names through the caller's interner):
//! - `Mpat` lowers through [`pat_of_mpat`][crate::ast::pat_of_mpat] onto
//!   `PatKind` (`StrAppend` → `StringAppend`, `VecAppend` →
//!   `VectorConcat`; `MPat_when` guards become `PatWhen`).
//! - `Pat::{Lit,Var,App,Tuple,StrAppend,VecAppend,Wild}` map onto the
//!   same-named `PatKind` variants; anything else maps back to `None`.
//! - `Exp::{Id,Lit,App,Tuple,Let,Match,Bool}` map onto the `ExpKind`
//!   subset; `Exit` maps to `ExpKind::Exit`; `Other` maps to `None`
//!   (it entfällt, per contract). Expression-side `Wild` (from
//!   [`exp_of_mpat`][crate::ast::exp_of_mpat]) becomes `Undef`.
//! - `MappingClause::{Bidir,Forwards,Backwards}` map onto
//!   `MapclKind::{Bidir,Forwards,Backwards}` with field names unchanged;
//!   bidir arms elaborate to `left-pat → right-exp` / `right-pat →
//!   left-exp` (the `rewrite_ast_realize_mappings` direction split).
//! - `MappingDef` maps onto `Mapdef { name, annot: Typ_bidir, clauses }`
//!   (`typ1/typ2` spellings become `Id` types; full `TypSchm`
//!   elaboration is Lane 02).
//! - `FunDef` maps onto `Funcl` lists, `ValSpec` onto `VS_val_spec`
//!   with a `Fn` scheme; `Arm.is_last` (`mapping_last`) has no
//!   canonical counterpart yet and is dropped (kept locally).
//!
//! `realize_mappings` and the pattern analyses keep running on the
//! local types; this module is the swap seam.

use sail_ast::{
    Exp as AstExp, ExpKind, Extern, Funcl, Interner, Lit, LitKind, Mapcl, MapclKind, Name, Node,
    NodeId, PExp, PExpKind, Pat as AstPat, PatKind, Span, TAnnotOpt, Typ, TypKind, TypSchm,
    ValSpec as AstValSpec,
};

use crate::ast::{Arm, Exp, FunDef, MappingClause, MappingDef, Mpat, Pat, ValSpec};

fn mk_pat(kind: PatKind) -> AstPat {
    Node::dummy(kind)
}

fn mk_exp(kind: ExpKind) -> AstExp {
    Node::dummy(kind)
}

fn ast_name(it: &mut impl Interner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn resolve_name(name: &Name, it: &impl Interner) -> Option<String> {
    it.resolve(name.symbol).map(str::to_owned)
}

fn mk_pexp(kind: PExpKind) -> PExp {
    PExp { id: NodeId::DUMMY, span: Span::DUMMY, kind }
}

/// Lower a local arm pattern onto a canonical node.
pub fn pat_to_ast(p: &Pat, it: &mut impl Interner) -> AstPat {
    match p {
        Pat::Lit(s) => mk_pat(PatKind::Lit(Node::dummy(LitKind::String(s.clone())))),
        Pat::Var(_) => mk_pat(PatKind::Var {
            pat: Box::new(mk_pat(PatKind::Wild)),
            typ_pat: Node::dummy(sail_ast::TypPatKind::Wild),
        }),
        Pat::App(f, args) => mk_pat(PatKind::App {
            func: ast_name(it, f),
            args: args.iter().map(|a| pat_to_ast(a, it)).collect(),
        }),
        Pat::Tuple(ps) => mk_pat(PatKind::Tuple(
            ps.iter().map(|a| pat_to_ast(a, it)).collect(),
        )),
        Pat::StrAppend(ps) => mk_pat(PatKind::StringAppend(
            ps.iter().map(|a| pat_to_ast(a, it)).collect(),
        )),
        Pat::VecAppend(ps) => mk_pat(PatKind::VectorConcat(
            ps.iter().map(|a| pat_to_ast(a, it)).collect(),
        )),
        Pat::Wild => mk_pat(PatKind::Wild),
    }
}

/// Lift a canonical node back onto a local arm pattern. `None` for
/// shapes outside the mapping subset.
pub fn pat_from_ast(p: &AstPat, it: &impl Interner) -> Option<Pat> {
    match &p.kind {
        PatKind::Lit(l) => match &l.kind {
            LitKind::String(s) => Some(Pat::Lit(s.clone())),
            _ => None,
        },
        PatKind::Wild => Some(Pat::Wild),
        PatKind::Var { pat, .. } => match &pat.kind {
            PatKind::Wild => Some(Pat::Var("_".to_owned())),
            _ => None,
        },
        PatKind::App { func, args } => {
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                out.push(pat_from_ast(a, it)?);
            }
            Some(Pat::App(resolve_name(func, it)?, out))
        }
        PatKind::Tuple(ps) => {
            let mut out = Vec::with_capacity(ps.len());
            for a in ps {
                out.push(pat_from_ast(a, it)?);
            }
            Some(Pat::Tuple(out))
        }
        PatKind::StringAppend(ps) => {
            let mut out = Vec::with_capacity(ps.len());
            for a in ps {
                out.push(pat_from_ast(a, it)?);
            }
            Some(Pat::StrAppend(out))
        }
        PatKind::VectorConcat(ps) => {
            let mut out = Vec::with_capacity(ps.len());
            for a in ps {
                out.push(pat_from_ast(a, it)?);
            }
            Some(Pat::VecAppend(out))
        }
        _ => None,
    }
}

/// Lower a local expression onto a canonical node. `None` for `Other`.
pub fn exp_to_ast(e: &Exp, it: &mut impl Interner) -> Option<AstExp> {
    match e {
        Exp::Id(v) => Some(mk_exp(ExpKind::Id(ast_name(it, v)))),
        Exp::Lit(s) => Some(mk_exp(ExpKind::Lit(Node::dummy(LitKind::String(s.clone()))))),
        Exp::Bool(true) => Some(mk_exp(ExpKind::Lit(Node::dummy(LitKind::True)))),
        Exp::Bool(false) => Some(mk_exp(ExpKind::Lit(Node::dummy(LitKind::False)))),
        Exp::App(f, args) => {
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                out.push(exp_to_ast(a, it)?);
            }
            Some(mk_exp(ExpKind::App { func: ast_name(it, f), args: out }))
        }
        Exp::Tuple(es) => {
            let mut out = Vec::with_capacity(es.len());
            for a in es {
                out.push(exp_to_ast(a, it)?);
            }
            Some(mk_exp(ExpKind::Tuple(out)))
        }
        Exp::Let { bound, value, body } => Some(mk_exp(ExpKind::Let {
            pat: pat_to_ast(bound, it),
            bind: Box::new(exp_to_ast(value, it)?),
            body: Box::new(exp_to_ast(body, it)?),
        })),
        Exp::Match { scrutinee, arms } => {
            let mut out = Vec::with_capacity(arms.len());
            for a in arms {
                out.push(arm_to_pexp(a, it)?);
            }
            Some(mk_exp(ExpKind::Match {
                scrutinee: Box::new(exp_to_ast(scrutinee, it)?),
                arms: out,
            }))
        }
        Exp::Exit => Some(mk_exp(ExpKind::Exit(Box::new(mk_exp(ExpKind::Lit(Node::dummy(
            LitKind::Unit,
        ))))))),
        Exp::Other(_) => None,
    }
}

/// Lower a local match arm onto a canonical `PExp`.
/// `is_last` is dropped (no canonical counterpart yet).
pub fn arm_to_pexp(a: &Arm, it: &mut impl Interner) -> Option<PExp> {
    let pat = pat_to_ast(&a.pat, it);
    let body = exp_to_ast(&a.body, it)?;
    match &a.guard {
        None => Some(mk_pexp(PExpKind::PatExp { pat, body })),
        Some(g) => Some(mk_pexp(PExpKind::PatWhen {
            pat,
            guard: exp_to_ast(g, it)?,
            body,
        })),
    }
}

fn body_exp(e: &Exp, it: &mut impl Interner) -> Option<AstExp> {
    // Expression-side wildcard (from `exp_of_mpat`) has no `ExpKind`
    // counterpart; `Undef` marks the untranslatable direction.
    if matches!(e, Exp::Other(s) if s == "_") {
        return Some(mk_exp(ExpKind::Undef));
    }
    exp_to_ast(e, it)
}

fn mpat_arm(
    m: &Mpat,
    guard: &Option<Exp>,
    body: &Exp,
    it: &mut impl Interner,
) -> Option<PExp> {
    let pat = pat_to_ast(&crate::ast::pat_of_mpat(m), it);
    let body = body_exp(body, it)?;
    match guard {
        None => Some(mk_pexp(PExpKind::PatExp { pat, body })),
        Some(g) => Some(mk_pexp(PExpKind::PatWhen {
            pat,
            guard: exp_to_ast(g, it)?,
            body,
        })),
    }
}

fn mk_mapcl(kind: MapclKind) -> Mapcl {
    Mapcl { id: NodeId::DUMMY, span: Span::DUMMY, kind }
}

/// Lower a local mapping clause onto a canonical `Mapcl`.
pub fn clause_to_ast(c: &MappingClause, it: &mut impl Interner) -> Option<Mapcl> {
    match c {
        MappingClause::Bidir { left, left_guard, right, right_guard } => {
            Some(mk_mapcl(MapclKind::Bidir {
                forwards: mpat_arm(left, left_guard, &crate::ast::exp_of_mpat(right), it)?,
                backwards: mpat_arm(right, right_guard, &crate::ast::exp_of_mpat(left), it)?,
            }))
        }
        MappingClause::Forwards { arm } => {
            Some(mk_mapcl(MapclKind::Forwards(arm_to_pexp(arm, it)?)))
        }
        MappingClause::Backwards { arm } => {
            Some(mk_mapcl(MapclKind::Backwards(arm_to_pexp(arm, it)?)))
        }
    }
}

fn spelling_typ(it: &mut impl Interner, spelling: &str) -> Typ {
    Node::dummy(TypKind::Id(ast_name(it, spelling)))
}

/// Lower a local mapping definition onto a canonical annotation plus
/// clause list (`Mapdef { name, annot: Typ_bidir, clauses }`).
pub fn mappingdef_to_ast(d: &MappingDef, it: &mut impl Interner) -> Option<(TAnnotOpt, Vec<Mapcl>)> {
    let mut clauses = Vec::with_capacity(d.clauses.len());
    for c in &d.clauses {
        clauses.push(clause_to_ast(c, it)?);
    }
    let annot = TAnnotOpt::Some {
        quant: Vec::new(),
        typ: Node::dummy(TypKind::Bidir {
            lhs: Box::new(spelling_typ(it, &d.typ1)),
            rhs: Box::new(spelling_typ(it, &d.typ2)),
        }),
    };
    Some((annot, clauses))
}

/// Lower a realized function onto canonical `Funcl` nodes.
pub fn fundef_to_ast(d: &FunDef, it: &mut impl Interner) -> Option<Vec<Funcl>> {
    let name = ast_name(it, &d.name);
    let mut out = Vec::with_capacity(d.arms.len());
    for a in &d.arms {
        out.push(Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: name.clone(),
            arm: arm_to_pexp(a, it)?,
        });
    }
    Some(out)
}

/// Lower a realized value spec onto a canonical `VS_val_spec`.
pub fn valspec_to_ast(v: &ValSpec, it: &mut impl Interner) -> AstValSpec {
    AstValSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: ast_name(it, &v.name),
        typschm: TypSchm {
            quant: Vec::new(),
            typ: Node::dummy(TypKind::Fn {
                args: vec![spelling_typ(it, &v.arg_ty)],
                ret: Box::new(spelling_typ(it, &v.ret_ty)),
            }),
        },
        extern_opt: None::<Extern>,
    }
}
