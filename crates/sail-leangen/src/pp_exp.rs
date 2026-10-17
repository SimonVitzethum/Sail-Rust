#![forbid(unsafe_code)]
//! Expressions (`doc_exp`, `doc_loop`, `doc_match_clause`, ...), a port of
//! `pretty_print_lean.ml`.

use sail_ast::{
    Exp, ExpKind, FExp, LExp, LExpKind, LitKind, Order, PExp, PExpKind, Pat, PatKind, SimpleInterner,
    TypArgKind, TypKind,
};

use crate::analysis::{has_early_return, has_effect, has_loop};
use crate::ctx::Ctx;
use crate::doc::{brk, chr, comma, comma_sp, empty, flow, hardline, prefix, separate, space, string, Doc};
use crate::names::{doc_id_ctor, doc_lit, name_text};
use crate::pp_pat::{
    doc_pat, doc_pat_typ_ascription, is_anonymous_pat, is_bitvector_pattern, update_ctx_pat,
};
use crate::pp_typ::{doc_typ, Result};

fn leftarrow() -> Doc {
    string("←")
}

fn leftarrowdo() -> Doc {
    string("← do")
}

fn coloneq() -> Doc {
    string(":=")
}

/// `wrap_with_pure`.
fn wrap_with_pure(needs_return: bool, with_parens: bool, d: Doc) -> Doc {
    if needs_return {
        let d = if with_parens { d.parens() } else { d };
        flow(&space(), [string("pure"), d]).nest(2).parens()
    } else {
        d
    }
}

/// `wrap_with_left_arrow`.
fn wrap_with_left_arrow(needs: bool, d: Doc) -> Doc {
    if needs {
        flow(&space(), [leftarrow(), d]).nest(2).parens()
    } else {
        d
    }
}

/// `wrap_with_do`.
fn wrap_with_do(with_arrow: bool, needs_return: bool, d: Doc) -> Doc {
    let ar_do = if with_arrow { string("← do") } else { string("do") };
    if needs_return {
        flow(&hardline(), [ar_do, d]).nest(2).parens()
    } else {
        d
    }
}

fn is_lit(e: &Exp, f: impl Fn(&LitKind) -> bool) -> bool {
    matches!(&e.kind, ExpKind::Lit(l) if f(&l.kind))
}

/// Printer state for one function body.
pub struct ExpPrinter<'c, 'a> {
    pub ctx: &'c Ctx<'a>,
    pub it: &'a SimpleInterner,
}

impl<'c, 'a> ExpPrinter<'c, 'a> {
    fn with(&self, ctx: &'c Ctx<'a>) -> ExpPrinter<'c, 'a> {
        ExpPrinter { ctx, it: self.it }
    }

    fn has_effect(&self, e: &Exp) -> bool {
        has_effect(e, self.it, self.ctx.global.oracle)
    }

    fn effectful(&self, e: &Exp) -> bool {
        crate::analysis::effectful(e, self.it, self.ctx.global.oracle, true)
    }

    fn id(&self, name: &sail_ast::Name, ren_ctor: bool) -> Doc {
        doc_id_ctor(self.ctx, self.it, name, ren_ctor)
    }

    /// `d_of_arg ?with_arrow ctx arg`.
    fn d_of_arg(&self, ctx: &Ctx<'a>, with_arrow: bool, arg: &Exp) -> Result<Doc> {
        let p = self.with_ctx(ctx);
        let (wrap, arg_monadic): (Box<dyn Fn(Doc) -> Doc>, bool) = match &arg.kind {
            ExpKind::Typ { exp, .. } if self.has_effect(exp) => {
                (Box::new(move |x| wrap_with_do(with_arrow, true, x)), true)
            }
            ExpKind::Typ { exp, .. } if has_early_return(exp, self.it) => (Box::new(|x: Doc| x.parens()), false),
            ExpKind::Let { .. }
            | ExpKind::InternalPlet { .. }
            | ExpKind::If { .. }
            | ExpKind::Match { .. }
            | ExpKind::Var { .. }
            | ExpKind::Block(_) => {
                if self.has_effect(arg) {
                    (Box::new(move |x| wrap_with_do(with_arrow, true, x)), true)
                } else {
                    (Box::new(|x: Doc| x.parens()), false)
                }
            }
            _ if has_loop(arg, self.it) => (Box::new(move |x| wrap_with_do(with_arrow, true, x)), true),
            _ => (Box::new(|x| x), !with_arrow),
        };
        Ok(wrap(p.doc_exp(arg_monadic, arg)?))
    }

    /// Run `f` with another context (same interner).
    fn with_ctx<'d>(&self, ctx: &'d Ctx<'a>) -> ExpPrinter<'d, 'a> {
        ExpPrinter { ctx, it: self.it }
    }

    fn wrap_exp(&self, as_monadic: bool, e: &Exp) -> Result<Doc> {
        let with_arrow = !as_monadic;
        let d = self.doc_exp(as_monadic, e)?;
        Ok(match &e.kind {
            ExpKind::Typ { exp, .. } if self.has_effect(exp) => wrap_with_do(with_arrow, true, d),
            ExpKind::Typ { exp, .. } if has_early_return(exp, self.it) => d.parens(),
            ExpKind::Let { .. }
            | ExpKind::InternalPlet { .. }
            | ExpKind::If { .. }
            | ExpKind::Match { .. }
            | ExpKind::Var { .. }
            | ExpKind::Block(_) => {
                if self.has_effect(e) {
                    wrap_with_do(with_arrow, true, d)
                } else {
                    d.parens()
                }
            }
            _ if has_loop(e, self.it) => wrap_with_do(with_arrow, true, d),
            _ => d,
        })
    }

    fn doc_match_clause(&self, is_bv: bool, as_monadic: bool, arm: &PExp) -> Result<Doc> {
        let (pat, guard, branch) = match &arm.kind {
            PExpKind::PatExp { pat, body } => (pat, None, body),
            PExpKind::PatWhen { pat, guard, body } => (pat, Some(guard), body),
        };
        let do_suffix = if is_bv && as_monadic { " do" } else { "" };
        match guard {
            None => Ok(string("| ")
                .cat(doc_pat(self.ctx, self.it, false, false, is_bv, pat)?)
                .cat(string(" =>"))
                .cat(string(do_suffix))
                .cat(brk(1))
                .cat(self.wrap_exp(as_monadic, branch)?)
                .nest(2)
                .group()),
            Some(when) if is_bv => Ok(string("| ")
                .cat(doc_pat(self.ctx, self.it, false, false, is_bv, pat)?)
                .cat(string(" if "))
                .cat(self.doc_exp(false, when)?)
                .cat(string(" =>"))
                .cat(string(do_suffix))
                .cat(brk(1))
                .cat(self.wrap_exp(as_monadic, branch)?)
                .nest(2)
                .group()),
            Some(_) => Err("The Lean backend does not support 'when' clauses in patterns".into()),
        }
    }

    fn doc_loop_var(&self, e: &Exp) -> Result<(Doc, Doc)> {
        match &e.kind {
            ExpKind::Id(id) => {
                let d = self.id(id, false);
                Ok((d.clone(), d))
            }
            ExpKind::Lit(l) if matches!(l.kind, LitKind::Unit) => Ok((string("()"), chr('_'))),
            _ => Err("Bad expression for variable in loop".into()),
        }
    }

    fn make_loop_vars(&self, extra_binders: &[Doc], vars: &Exp) -> Result<(Doc, Doc)> {
        let lambda = |inner: Doc| {
            let mut parts = vec![string("λ")];
            parts.extend(extra_binders.iter().cloned());
            parts.push(inner);
            parts.push(string("=>"));
            separate(&space(), parts)
        };
        match &vars.kind {
            ExpKind::Tuple(vs) => {
                let pairs: Vec<(Doc, Doc)> = vs.iter().map(|v| self.doc_loop_var(v)).collect::<Result<_>>()?;
                let tup = separate(&string(", "), pairs.iter().map(|p| p.0.clone()));
                let mat = separate(&string(", "), pairs.iter().map(|p| p.1.clone()));
                Ok((tup.parens(), lambda(mat.parens())))
            }
            _ => {
                let (exp_pp, match_pp) = self.doc_loop_var(vars)?;
                Ok((exp_pp, lambda(match_pp)))
            }
        }
    }

    fn name_loop_vars(&self) -> (Doc, u32) {
        let level = self.ctx.loop_level.get();
        self.ctx.loop_level.set(level + 1);
        match level {
            0 => (string("loop_vars"), level),
            _ => (string(&format!("loop_vars_{level}")), level),
        }
    }

    fn doc_loop(&self, as_monadic: bool, kind: LoopKind, args: &[Exp]) -> Result<Doc> {
        let lambda = |effects: bool, lambda_pp: Doc, d: Doc| -> Doc {
            let lambda_pp = if effects { lambda_pp.cat(string(" do")) } else { lambda_pp };
            prefix(2, 1, lambda_pp.group(), d).parens()
        };
        let (cond, varstuple, body, measure) = match args {
            [c, v, b] => (c, v, b, None),
            [c, v, b, m] => (c, v, b, Some(m)),
            _ => return Err("Unexpected number of arguments for loop combinator".into()),
        };
        // Drop the `loop dummy assert` binding added by `remove_e_assign`.
        let body = match &body.kind {
            ExpKind::InternalPlet { pat, bind, body: inner }
                if matches!(&pat.kind, PatKind::Wild)
                    || matches!(&pat.kind, PatKind::Typ { pat: p, .. } if matches!(p.kind, PatKind::Wild)) =>
            {
                match &bind.kind {
                    ExpKind::Assert { cond, msg }
                        if is_lit(cond, |l| matches!(l, LitKind::True))
                            && matches!(&msg.kind, ExpKind::Lit(l) if matches!(l.kind, LitKind::String(s) if sail_ast::Interner::resolve(self.it, s) == Some("loop dummy assert"))) =>
                    {
                        inner.as_ref()
                    }
                    _ => body,
                }
            }
            _ => body,
        };
        let body_effects = self.has_effect(body);
        let cond_effects = self.has_effect(cond);
        let (vartuple_pp, base_lambda) = self.make_loop_vars(&[], varstuple)?;
        let (vars_pp, level) = self.name_loop_vars();
        let _ = level;
        let body_pp = self.doc_exp(body_effects, body)?;
        let vars_dec_pp = string("let mut ").cat(vars_pp.clone()).cat(string(" := ")).cat(vartuple_pp.clone());
        let cond_pp = self.doc_exp(cond_effects, cond)?;
        let cond_pp = lambda(cond_effects, base_lambda, cond_pp);
        let loop_cond = wrap_with_left_arrow(cond_effects, prefix(2, 1, cond_pp, vars_pp.clone()));
        let measure_pp = match measure {
            Some(m) => Some(string("fuel :=").cat(self.doc_exp(false, m)?).parens()),
            None => None,
        };
        match kind {
            LoopKind::While => {
                let loop_head = prefix(2, 1, string("while ").cat(loop_cond), string("do"));
                let arrow = if body_effects { leftarrowdo() } else { coloneq() };
                let loop_body_1 = string("let ")
                    .cat(vartuple_pp.clone())
                    .cat(space())
                    .cat(coloneq())
                    .cat(space())
                    .cat(vars_pp.clone());
                let loop_body = loop_body_1
                    .cat(hardline())
                    .cat(prefix(2, 1, vars_pp.clone().cat(space()).cat(arrow), body_pp));
                let full_loop = prefix(2, 1, loop_head, loop_body);
                Ok(separate(
                    &hardline(),
                    [vars_dec_pp, full_loop, wrap_with_pure(as_monadic, false, vars_pp)],
                ))
            }
            LoopKind::WhileFuel | LoopKind::UntilFuel => {
                let measure = measure_pp.ok_or("fuel loop without measure")?;
                let cond_pp = string("fun ")
                    .cat(vartuple_pp.clone())
                    .cat(string(" => "))
                    .cat(self.doc_exp(true, cond)?)
                    .parens();
                let init = self.doc_exp(false, varstuple)?;
                let loop_fn = if matches!(kind, LoopKind::WhileFuel) { "whileFuelM" } else { "untilFuelM" };
                let loop_head = string(loop_fn)
                    .cat(space())
                    .cat(measure)
                    .cat(space())
                    .cat(cond_pp)
                    .cat(space())
                    .cat(init);
                let fun_header = string("fun ").cat(vartuple_pp.clone()).cat(space()).cat(string("=> do"));
                let body_pp = self.doc_exp(true, body)?;
                let loop_body = prefix(2, 1, fun_header, body_pp);
                let full_loop = prefix(2, 1, loop_head, loop_body);
                let full_loop = string("let")
                    .cat(space())
                    .cat(vars_pp.clone())
                    .cat(space())
                    .cat(leftarrow())
                    .cat(space())
                    .cat(full_loop);
                Ok(separate(&hardline(), [full_loop, wrap_with_pure(as_monadic, false, vars_pp)]))
            }
            LoopKind::Until => {
                let loop_head = string("repeat");
                let loop_footer = flow(&brk(1), [string("until"), loop_cond]);
                let arrow = if body_effects { leftarrowdo() } else { coloneq() };
                let loop_body_1 = string("let ")
                    .cat(vartuple_pp)
                    .cat(space())
                    .cat(coloneq())
                    .cat(space())
                    .cat(vars_pp.clone());
                let loop_body = loop_body_1
                    .cat(hardline())
                    .cat(prefix(2, 1, vars_pp.clone().cat(space()).cat(arrow), body_pp));
                let full_loop = prefix(2, 1, loop_head, loop_body);
                Ok(separate(
                    &hardline(),
                    [vars_dec_pp, full_loop, loop_footer, wrap_with_pure(as_monadic, false, vars_pp)],
                ))
            }
        }
    }

    fn doc_fexp(&self, with_arrow: bool, f: &FExp) -> Result<Doc> {
        let arrow = if with_arrow { leftarrow().cat(space()) } else { empty() };
        Ok(self
            .id(&f.field, false)
            .cat(string(" := "))
            .cat(arrow)
            .cat(self.doc_exp(with_arrow, &f.value)?.nest(2)))
    }

    /// `doc_exp as_monadic ctx exp`.
    pub fn doc_exp(&self, as_monadic: bool, exp: &Exp) -> Result<Doc> {
        let ctx = self.ctx;
        let oracle = ctx.global.oracle;
        match &exp.kind {
            ExpKind::Id(id) => {
                let text = name_text(id, self.it);
                if oracle.is_register(&text) {
                    Ok(wrap_with_left_arrow(
                        !as_monadic,
                        string("readReg ").cat(self.id(id, false)),
                    ))
                } else {
                    Ok(wrap_with_pure(as_monadic, false, self.id(id, true)))
                }
            }
            ExpKind::Lit(l) => Ok(wrap_with_pure(as_monadic, false, doc_lit(self.it, l, true))),
            ExpKind::App { func, args } => self.doc_app(as_monadic, exp, func, args),
            ExpKind::Vector(vals) => {
                let bitvector = crate::analysis::exp_is_bitvector(exp, self.it, oracle)
                    .unwrap_or_else(|| vals.iter().all(|v| matches!(&v.kind, ExpKind::Lit(l) if matches!(l.kind, LitKind::Bin(_) | LitKind::Hex(_)))) || vals.iter().all(|v| self.is_bit_exp(v)));
                if bitvector {
                    let docs: Vec<Doc> = vals.iter().map(|v| self.d_of_arg(ctx, true, v)).collect::<Result<_>>()?;
                    Ok(wrap_with_pure(
                        as_monadic,
                        false,
                        flow(&space(), [string("BitVec.join1"), separate(&comma_sp(), docs).brackets()]).parens(),
                    )
                    .nest(2))
                } else {
                    let docs: Vec<Doc> = vals
                        .iter()
                        .rev()
                        .map(|v| self.d_of_arg(ctx, true, v))
                        .collect::<Result<_>>()?;
                    Ok(string("#v").cat(wrap_with_pure(
                        as_monadic,
                        false,
                        separate(&comma_sp(), docs).nest(2).brackets(),
                    )))
                }
            }
            ExpKind::Typ { typ, exp: inner } => {
                if self.has_effect(inner) {
                    self.doc_exp(as_monadic, inner)
                } else {
                    Ok(wrap_with_pure(
                        as_monadic,
                        false,
                        separate(
                            &space(),
                            [self.doc_exp(false, inner)?, chr(':'), doc_typ(ctx, self.it, typ)?],
                        )
                        .parens(),
                    ))
                }
            }
            ExpKind::Tuple(es) => {
                let docs: Vec<Doc> = es.iter().map(|e| self.d_of_arg(ctx, true, e)).collect::<Result<_>>()?;
                Ok(wrap_with_pure(as_monadic, false, separate(&comma().cat(space()), docs).parens()))
            }
            ExpKind::Let { pat, bind, body } | ExpKind::InternalPlet { pat, bind, body } => {
                self.doc_let(as_monadic, exp, pat, bind, body)
            }
            ExpKind::InternalReturn(e) => self.doc_exp(false, e),
            ExpKind::Struct { fields, .. } => {
                let args: Vec<Doc> = fields
                    .iter()
                    .map(|f| self.doc_fexp(self.has_effect(&f.value), f))
                    .collect::<Result<_>>()?;
                Ok(wrap_with_pure(
                    as_monadic,
                    false,
                    space().cat(separate(&hardline(), args).align()).cat(space()).braces(),
                ))
            }
            ExpKind::Field { record, field } => Ok(wrap_with_pure(
                as_monadic,
                false,
                self.doc_exp(false, record)?.cat(chr('.')).cat(self.id(field, false)),
            )),
            ExpKind::StructUpdate { record, fields } => {
                let args: Vec<Doc> = fields
                    .iter()
                    .map(|f| self.doc_fexp(self.has_effect(&f.value), f))
                    .collect::<Result<_>>()?;
                Ok(wrap_with_pure(
                    as_monadic,
                    false,
                    space()
                        .cat(self.doc_exp(false, record)?)
                        .cat(string(" with "))
                        .cat(separate(&comma().cat(space()), args))
                        .cat(space())
                        .braces(),
                ))
            }
            ExpKind::Match { scrutinee, arms } => {
                let is_bv = arms.iter().any(|a| {
                    let pat = match &a.kind {
                        PExpKind::PatExp { pat, .. } | PExpKind::PatWhen { pat, .. } => pat,
                    };
                    is_bitvector_pattern(pat)
                });
                let as_monadic2 = arms.iter().any(|a| match &a.kind {
                    PExpKind::PatExp { body, .. } | PExpKind::PatWhen { body, .. } => self.effectful(body),
                }) || as_monadic;
                let cases: Vec<Doc> = arms
                    .iter()
                    .map(|a| self.doc_match_clause(is_bv, as_monadic2, a))
                    .collect::<Result<_>>()?;
                Ok(string(if is_bv { "match_bv " } else { "match " })
                    .cat(self.d_of_arg(ctx, true, scrutinee)?)
                    .cat(string(" with"))
                    .cat(hardline())
                    .cat(separate(&hardline(), cases)))
            }
            ExpKind::Assign { lexp, value } => {
                let inner = match &lexp.kind {
                    LExpKind::Id(id) | LExpKind::Typ { id, .. } => string("writeReg ")
                        .cat(self.id(id, false))
                        .cat(space())
                        .cat(self.d_of_arg(ctx, true, value)?),
                    LExpKind::Deref(e) => string("writeRegRef ")
                        .cat(self.d_of_arg(ctx, true, e)?)
                        .cat(space())
                        .cat(self.d_of_arg(ctx, true, value)?),
                    _ => return Err("assign lexp not implemented yet".into()),
                };
                Ok(wrap_with_left_arrow(!as_monadic, inner))
            }
            ExpKind::If { cond, then_branch, else_branch } => {
                let statements_monadic =
                    as_monadic || self.has_effect(then_branch) || self.has_effect(else_branch);
                let head = string("if (")
                    .cat(self.d_of_arg(ctx, true, cond)?.nest(1))
                    .cat(string(" : Bool)"))
                    .nest(2);
                let doc = head
                    .cat(hardline())
                    .cat(prefix(2, 1, string("then"), self.wrap_exp(statements_monadic, then_branch)?))
                    .cat(hardline())
                    .cat(prefix(2, 1, string("else"), self.wrap_exp(statements_monadic, else_branch)?));
                Ok(wrap_with_left_arrow(statements_monadic && !as_monadic, doc))
            }
            ExpKind::Ref(id) => Ok(string(".Reg ").cat(self.id(id, false)).parens()),
            ExpKind::Exit(_) => Ok(string("throw Error.Exit")),
            ExpKind::Throw(e) => {
                let arrow = if as_monadic { empty() } else { leftarrow() };
                Ok(arrow.cat(string("sailThrow ")).cat(self.doc_exp(false, e)?.parens()))
            }
            ExpKind::Try { exp: tried, arms } => {
                let scrutinee = Exp::new(
                    sail_ast::NodeId::DUMMY,
                    sail_ast::Span::DUMMY,
                    ExpKind::Id(sail_ast::Name {
                        id: sail_ast::NodeId::DUMMY,
                        span: sail_ast::Span::DUMMY,
                        symbol: self.the_exception_symbol()?,
                        is_operator: false,
                    }),
                );
                let _ = scrutinee;
                // `E_match (the_exception, cases)` printed in monadic position.
                let is_bv = false;
                let cases: Vec<Doc> = arms
                    .iter()
                    .map(|a| self.doc_match_clause(is_bv, true, a))
                    .collect::<Result<_>>()?;
                let match_doc = string("match the_exception with")
                    .cat(hardline())
                    .cat(separate(&hardline(), cases))
                    .nest(2);
                let try_catch = if has_early_return(tried, self.it) {
                    string("sailTryCatchE ")
                } else {
                    string("sailTryCatch ")
                };
                let arrow = if as_monadic { empty() } else { leftarrow() };
                Ok(arrow
                    .cat(try_catch)
                    .cat(self.d_of_arg(ctx, false, tried)?.parens())
                    .cat(space())
                    .cat(string("fun the_exception => ").cat(hardline()).cat(match_doc).parens())
                    .nest(2))
            }
            ExpKind::Assert { cond, msg } => Ok(string("assert ")
                .cat(self.d_of_arg(ctx, true, cond)?)
                .cat(space())
                .cat(self.d_of_arg(ctx, true, msg)?)),
            ExpKind::List(es) => {
                let docs: Vec<Doc> = es
                    .iter()
                    .map(|e| self.doc_exp(as_monadic, e))
                    .collect::<Result<_>>()?;
                Ok(separate(&comma_sp(), docs).brackets())
            }
            ExpKind::Cons(h, t) => Ok(separate(
                &space(),
                [self.doc_exp(false, h)?, string("::"), self.doc_exp(false, t)?],
            )
            .parens()),
            ExpKind::For { var, from, to, step, order, body } => {
                let combinator = ();
                let _ = combinator;
                let from_pp = self.doc_exp(false, from)?;
                let to_pp = self.doc_exp(false, to)?;
                let step_pp = self.doc_exp(false, step)?;
                let step_pp = match order {
                    Order::Inc => step_pp,
                    Order::Dec => chr('-').cat(step_pp),
                };
                let bracket = separate(&chr(':'), [from_pp, to_pp, step_pp]).brackets().cat(string("i"));
                let loopvar = self.id(var, false);
                let body_effect = self.has_effect(body);
                let enter_monad = if body_effect { empty() } else { string("Id.run") };
                let mut parts = Vec::new();
                if !enter_monad.is_empty() {
                    parts.push(enter_monad);
                }
                parts.extend([string("for"), loopvar, string("in"), bracket, string("do")]);
                let loop_head = flow(&brk(1), parts);
                let loop_body = self.doc_exp(body_effect, body)?;
                Ok(prefix(2, 1, loop_head, loop_body))
            }
            other => Err(format!("Expression {} not translatable yet", exp_con(other))),
        }
    }

    fn the_exception_symbol(&self) -> Result<sail_ast::Symbol> {
        Ok(sail_ast::Symbol(0))
    }

    fn is_bit_exp(&self, v: &Exp) -> bool {
        matches!(&v.kind, ExpKind::Id(id) if matches!(name_text(id, self.it).as_str(), "bitzero" | "bitone"))
    }

    fn doc_let(&self, as_monadic: bool, exp: &Exp, lpat: &Pat, lexp: &Exp, rest: &Exp) -> Result<Doc> {
        let ctx = self.ctx;
        let has_loop_l = has_loop(lexp, self.it);
        let is_arrow_do = !(matches!(exp.kind, ExpKind::Let { .. }) && !has_loop_l);
        let id_typ = doc_pat(ctx, self.it, false, false, false, lpat)?;
        let typ_ascription = doc_pat_typ_ascription(ctx, self.it, lpat)?;
        let mut ctx2 = ctx.clone();
        update_ctx_pat(&mut ctx2, self.it, lpat);
        let p2 = ExpPrinter { ctx: &ctx2, it: self.it };
        let pp_let_line_f = |l: Vec<Doc>| flow(&brk(1), l).nest(2).group();
        let effectful_l = self.has_effect(lexp) || has_loop_l;
        let pp_let_line = if effectful_l {
            let unit_l = match crate::analysis::exp_is_unit(lexp, self.it, ctx.global.oracle) {
                Some(b) => b,
                None => matches!(&lexp.kind, ExpKind::Lit(l) if matches!(l.kind, LitKind::Unit)),
            };
            if unit_l && is_anonymous_pat(self.it, lpat) {
                p2.doc_exp(true, lexp)?
            } else {
                match (is_arrow_do, typ_ascription) {
                    (true, None) => pp_let_line_f(vec![
                        separate(&space(), [string("let"), id_typ, leftarrowdo()]),
                        p2.doc_exp(true, lexp)?,
                    ]),
                    (false, None) => pp_let_line_f(vec![
                        separate(&space(), [string("let"), id_typ, leftarrow()]),
                        p2.doc_exp(true, lexp)?,
                    ]),
                    (true, Some(asc)) => {
                        let mut parts = vec![
                            separate(
                                &space(),
                                [string("let"), id_typ, leftarrow(), string("(("), string("do")],
                            ),
                            p2.doc_exp(true, lexp)?,
                            string(")"),
                            chr(':'),
                        ];
                        parts.extend(self.prepend_monad(asc));
                        parts.push(string(")"));
                        pp_let_line_f(parts)
                    }
                    (false, Some(asc)) => {
                        let mut parts = vec![
                            separate(&space(), [string("let"), id_typ, leftarrow(), string("((")]),
                            p2.doc_exp(true, lexp)?,
                            string(")"),
                            chr(':'),
                        ];
                        parts.extend(self.prepend_monad(asc));
                        parts.push(string(")"));
                        pp_let_line_f(parts)
                    }
                }
            }
        } else {
            match typ_ascription {
                Some(asc) => pp_let_line_f(vec![
                    separate(&space(), [string("let"), id_typ, chr(':'), asc, coloneq()]),
                    p2.doc_exp(false, lexp)?,
                ]),
                None => pp_let_line_f(vec![
                    separate(&space(), [string("let"), id_typ, coloneq()]),
                    p2.doc_exp(false, lexp)?,
                ]),
            }
        };
        Ok(pp_let_line.cat(hardline()).cat(p2.doc_exp(as_monadic, rest)?))
    }

    /// `prepend_monad ctx exp doc`.
    fn prepend_monad(&self, doc: Doc) -> Vec<Doc> {
        match (self.ctx.in_sail_monad, &self.ctx.in_except_monad) {
            (true, Some(ty)) => vec![string("SailME"), ty.clone(), doc],
            (true, None) => vec![string("SailM"), doc],
            (false, Some(ty)) => vec![string("ExceptM"), ty.clone(), doc],
            (false, None) => vec![string("Id"), doc],
        }
    }

    fn doc_app(&self, as_monadic: bool, exp: &Exp, func: &sail_ast::Name, args: &[Exp]) -> Result<Doc> {
        let ctx = self.ctx;
        let oracle = ctx.global.oracle;
        let fname = name_text(func, self.it);
        match fname.as_str() {
            "None" => return Ok(wrap_with_pure(as_monadic, false, string("none"))),
            "Some" => {
                let d_args: Vec<Doc> = args.iter().map(|a| self.d_of_arg(ctx, true, a)).collect::<Result<_>>()?;
                let mut parts = vec![string("some")];
                parts.extend(d_args);
                return Ok(wrap_with_pure(as_monadic, false, flow(&brk(1), parts).nest(2).parens()));
            }
            "__id" if args.len() == 1 => return self.doc_exp(as_monadic, &args[0]),
            "while#" => return self.doc_loop(as_monadic, LoopKind::While, args),
            "while#t" => return self.doc_loop(as_monadic, LoopKind::WhileFuel, args),
            "until#" => return self.doc_loop(as_monadic, LoopKind::Until, args),
            "until#t" => return self.doc_loop(as_monadic, LoopKind::UntilFuel, args),
            "foreach#" => return self.doc_foreach(as_monadic, args),
            "early_return" if args.len() == 1 => {
                let throw = if ctx.in_sail_monad { "SailME.throw " } else { "throw " };
                return Ok(string(throw).cat(self.d_of_arg(ctx, true, &args[0])?).nest(2));
            }
            _ => {}
        }
        let _ = exp;
        let (implicits, spec_known) = match oracle.val_spec(&fname) {
            Some((scheme, sit)) => (fn_implicits(&scheme.typ, &sit), true),
            None => (Vec::new(), false),
        };
        let _ = spec_known;
        let arg_names = oracle.fun_args(&fname).unwrap_or_default();
        let extern_id = oracle.lean_extern(&fname);
        let d_id = match &extern_id {
            Some(e) => string(e),
            None => self.id(func, true),
        };
        let d_args: Vec<Doc> = args.iter().map(|a| self.d_of_arg(ctx, true, a)).collect::<Result<_>>()?;
        let d_imargs = doc_implicit_args(&arg_names, &implicits, &d_args);
        let d_args: Vec<Doc> = if implicits.len() == d_args.len() {
            implicits
                .iter()
                .zip(d_args)
                .filter(|(im, _)| !**im)
                .map(|(_, d)| d)
                .collect()
        } else {
            d_args
        };
        let fn_monadic = !oracle.function_is_pure(&fname);
        match lean_op(extern_id.as_deref()) {
            LeanOp::Binop(op) => {
                let e1 = d_args.first().cloned().ok_or("binop without arguments")?;
                let e2 = d_args.get(1).cloned().ok_or("binop with one argument")?;
                let res = e1.cat(space()).cat(string(op)).cat(space()).cat(e2);
                Ok(wrap_with_pure(as_monadic, false, res.parens()).nest(2))
            }
            LeanOp::Unop(op) => {
                let e = d_args.first().cloned().ok_or("unop without arguments")?;
                let res = string(op).cat(space()).cat(e);
                Ok(wrap_with_pure(as_monadic, false, res.parens()).nest(2))
            }
            LeanOp::NotOp => {
                let mut parts = vec![d_id];
                parts.extend(d_imargs);
                parts.extend(d_args);
                Ok(wrap_with_left_arrow(
                    !as_monadic && fn_monadic,
                    wrap_with_pure(as_monadic && !fn_monadic, false, flow(&brk(1), parts).parens()),
                )
                .nest(2))
            }
        }
    }

    fn doc_foreach(&self, as_monadic: bool, args: &[Exp]) -> Result<Doc> {
        let [from_exp, to_exp, step_exp, ord_exp, vartuple, body] = args else {
            return Err("Unexpected number of arguments for loop combinator".into());
        };
        // `if _ then let <var> = _ in body`, as produced by `remove_e_assign`.
        let (loopvar, body) = match &body.kind {
            ExpKind::If { then_branch, .. } => match &then_branch.kind {
                ExpKind::Let { pat, body, .. } => match loop_pat_id(pat) {
                    Some(id) => (id, body.as_ref()),
                    None => return Err("Unable to find loop variable".into()),
                },
                _ => return Err("Unable to find loop variable".into()),
            },
            _ => return Err("Unable to find loop variable".into()),
        };
        let from_pp = self.doc_exp(false, from_exp)?;
        let to_pp = self.doc_exp(false, to_exp)?;
        let step_pp = self.doc_exp(false, step_exp)?;
        let step_pp = if is_lit(ord_exp, |l| matches!(l, LitKind::True)) { step_pp } else { chr('-').cat(step_pp) };
        let loopvar_pp = self.id(loopvar, false);
        let effects = self.has_effect(body);
        let (vartuple_pp, body_lambda) = self.make_loop_vars(&[loopvar_pp.clone()], vartuple)?;
        let _ = if effects { body_lambda.cat(string(" do")) } else { body_lambda };
        let (vars_pp, _) = self.name_loop_vars();
        let body_pp = self.doc_exp(as_monadic && effects, body)?;
        let vars_dec_pp = string("let mut ").cat(vars_pp.clone()).cat(string(" := ")).cat(vartuple_pp.clone());
        let bracket = separate(&chr(':'), [from_pp, to_pp, step_pp]).brackets().cat(string("i"));
        let loop_head = flow(&brk(1), [string("for"), loopvar_pp, string("in"), bracket, string("do")]);
        let arrow = if effects { leftarrowdo() } else { coloneq() };
        let loop_body_1 = string("let ").cat(vartuple_pp).cat(space()).cat(coloneq()).cat(space()).cat(vars_pp.clone());
        let loop_body = loop_body_1
            .cat(hardline())
            .cat(prefix(2, 1, vars_pp.clone().cat(space()).cat(arrow), body_pp));
        let full_loop = prefix(2, 1, loop_head, loop_body);
        Ok(separate(
            &hardline(),
            [vars_dec_pp, full_loop, wrap_with_pure(as_monadic, false, vars_pp)],
        ))
    }
}

fn loop_pat_id(pat: &Pat) -> Option<&sail_ast::Name> {
    match &pat.kind {
        PatKind::Typ { pat: p, .. } => match &p.kind {
            PatKind::Var { pat: v, .. } => match &v.kind {
                PatKind::Id(id) => Some(id),
                _ => None,
            },
            _ => None,
        },
        PatKind::Var { pat: v, .. } => match &v.kind {
            PatKind::Id(id) => Some(id),
            _ => None,
        },
        PatKind::Id(id) => Some(id),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum LoopKind {
    While,
    WhileFuel,
    Until,
    UntilFuel,
}

enum LeanOp {
    Binop(&'static str),
    Unop(&'static str),
    NotOp,
}

fn lean_op(extern_id: Option<&str>) -> LeanOp {
    match extern_id {
        Some("_lean_and") => LeanOp::Binop("&&"),
        Some("_lean_or") => LeanOp::Binop("||"),
        Some("_lean_beq") => LeanOp::Binop("=="),
        Some("_lean_bne") => LeanOp::Binop("!="),
        Some("_lean_not") => LeanOp::Unop("!"),
        Some("_lean_add") => LeanOp::Binop("+"),
        Some("_lean_addi") => LeanOp::Binop("+i"),
        Some("_lean_sub") => LeanOp::Binop("-"),
        Some("_lean_subi") => LeanOp::Binop("-i"),
        Some("_lean_mul") => LeanOp::Binop("*"),
        Some("_lean_muli") => LeanOp::Binop("*i"),
        Some("_lean_div") => LeanOp::Binop("/"),
        Some("_lean_app") => LeanOp::Binop("+++"),
        Some("_lean_bvand") => LeanOp::Binop("&&&"),
        Some("_lean_bvor") => LeanOp::Binop("|||"),
        Some("_lean_bvxor") => LeanOp::Binop("^^^"),
        Some("_lean_shiftl") => LeanOp::Binop("<<<"),
        Some("_lean_shiftr") => LeanOp::Binop(">>>"),
        Some("_lean_lt") => LeanOp::Binop("<b"),
        Some("_lean_ge") => LeanOp::Binop("≥b"),
        Some("_lean_le") => LeanOp::Binop("≤b"),
        Some("_lean_gt") => LeanOp::Binop(">b"),
        Some("_lean_pow2") => LeanOp::Unop("2 ^"),
        Some("_lean_pow2i") => LeanOp::Unop("2 ^i"),
        _ => LeanOp::NotOp,
    }
}

/// `get_fn_implicits`.
#[must_use]
pub fn fn_implicits(typ: &sail_ast::Typ, it: &SimpleInterner) -> Vec<bool> {
    match &typ.kind {
        TypKind::Fn { args, .. } => args
            .iter()
            .map(|a| match &a.kind {
                TypKind::App { name, args } => {
                    name_text(name, it) == "implicit"
                        && matches!(args.as_slice(), [x] if matches!(&x.kind, TypArgKind::NExp(n) if matches!(n.kind, sail_ast::NExpKind::Var(_))))
                }
                _ => false,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// `doc_implicit_args`.
fn doc_implicit_args(names: &[String], implicits: &[bool], d_args: &[Doc]) -> Vec<Doc> {
    if names.len() != implicits.len() || implicits.len() != d_args.len() {
        return Vec::new();
    }
    let mut docs = Vec::new();
    for ((n, im), d) in names.iter().zip(implicits).zip(d_args) {
        if *im {
            docs.insert(
                0,
                string(n).cat(space()).cat(coloneq()).cat(space()).cat(d.clone()).parens(),
            );
        }
    }
    docs
}

fn exp_con(kind: &ExpKind) -> &'static str {
    match kind {
        ExpKind::Block(_) => "E_block",
        ExpKind::Loop { .. } => "E_loop",
        ExpKind::VectorAppend(..) => "E_vector_append",
        ExpKind::Sizeof(_) => "E_sizeof",
        ExpKind::Constraint(_) => "E_constraint",
        ExpKind::Return(_) => "E_return",
        ExpKind::Var { .. } => "E_var",
        ExpKind::Undef => "E_undef",
        ExpKind::InternalValue(_) => "E_internal_value",
        ExpKind::InternalAssume { .. } => "E_internal_assume",
        ExpKind::Config(_) => "E_config",
        _ => "expression",
    }
}

#[allow(dead_code)]
fn lexp_unused(_: &LExp) {}
