//! Early-return lifting, matching `rewrite_ast_early_return` in
//! `sail/src/lib/rewrites.ml`.
//!
//! OCaml pulls `return`s out of blocks, ifs, matches, lets, vars and
//! applications as far as possible (`e_block`, `e_if`, `e_case`, `e_let`,
//! `e_var`, `e_app` over `add_final_return`), drops the return when the whole
//! clause body is one (`if is_return exp' then get_return exp'`), and rewrites
//! every remaining inner `return v` into an effectful call
//! `early_return(v)` (`e_aux`), prepending the builtin spec
//! `early_return : forall ('a : Type) ('b : Type). 'a -> 'b` once per program
//! (`early_ret_spec`, `gen_vs ~pure:true`).
//!
//! Implemented now (all type/effect-free, hence exact):
//! - per-clause gating on a syntactic `contains_return`, as in OCaml
//!   (`rewrite_funcl_early_return` only fires when the body contains one);
//! - `add_final_return`: trailing leaves (block tails, if/match branches,
//!   let/var bodies) are wrapped in `Return` so the pull-out below sees a
//!   uniform shape; `E_typ` look-through is preserved;
//! - pull-out `e_block` (if-return folding with `Util.last`-style annotation
//!   reuse, singleton-block unwrapping, trailing-return hoisting), `e_if`
//!   (both branches return), `e_case` (all arms return), `e_let`/`e_var`
//!   (returning bodies), `e_app` (first returning argument wins, as in OCaml);
//! - whole-body strip: a body that is a single `return` loses it;
//! - remaining inner returns become `App { func: early_return, .. }`;
//! - the `early_return` valspec is prepended once per program (duplicate runs
//!   are no-ops).
//!
//! Deliberately deferred, with the OCaml reference for each gap:
//! - TAnnot gates: OCaml only converts `E_return` nodes with a non-empty type
//!   annotation (`not (is_empty_tannot tannot)`, `mk_tannot env typ`) and
//!   re-annotates. [`sail_ast::Annotation`] only carries source attributes /
//!   docs, never typechecker output, so the conversion applies uniformly; the
//!   gate must be restored once this runs post-typecheck.
//! - Declared return type: OCaml wraps the payload with `add_e_typ` when
//!   `Env.get_ret_typ` yields one. Approximated with the clause's
//!   [`sail_ast::TAnnotOpt`] declared type when present, bare payload
//!   otherwise.
//! - `effect_info`: `rewrite_ast_early_return` takes it and returns it
//!   *unchanged* — this pass neither needs nor modifies effect info. Its
//!   outputs are the markers downstream keys on: OCaml `has_early_return` /
//!   `needs_monad` (`rewrites.ml`, `E_app (early_return, _)` arms) and,
//!   conceptually, `sail_effects::has_early_return` (same "non-trailing
//!   return" notion, different IR — `sail_effects::ir`, not `sail_ast` —
//!   so not a direct consumer).
//! - Scope: only `Fundef` / `InternalMutrec` clauses are rewritten. OCaml's
//!   `rewrite_fun_early_return` matches `FD_function` only, so mappings
//!   (`Mapdef`), `Impl`, top-level `Let` and register initialisers are
//!   untouched here too. Clause guards are excluded (`destruct_pexp`
//!   rewrites the body only); guards nested inside the body are traversed,
//!   exactly like OCaml's `fold_exp`.
//! - Synthesized nodes use dummy ids/spans (the `typed.rs` convention for
//!   desugared output); nodes OCaml re-annotates from input keep the input
//!   id/span via `rewrap`.
//!
//! Insert-Spec (vom Main-Agent beim Merge anzuwenden, diese Datei nicht
//! aendern):
//! - Pass-Name: `"early_return"`
//! - Position in `LEAN_REWRITES`
//!   (`crates/sail-rewrite/src/pipeline.rs`): Index 30, zwischen
//!   `"exp_lift_assign"` (29) und `"recheck_defs"` (31).
//! - Trait-Impl-Name: `EarlyReturn` (diese Datei).
//!   `passes/mod.rs`: `pub mod early_return;` +
//!   `pub use early_return::EarlyReturn;` ergaenzen.
//!   `pipeline.rs`: `"early_return" => Some(Arc::new(EarlyReturn))`
//!   in `make_typed_implemented` und den Namen in
//!   `is_typed_implemented_pass` aufnehmen.

#![forbid(unsafe_code)]

use sail_ast::{
    Annotation, DefKind, Exp, ExpKind, Funcl, LExp, LExpKind, LitKind, Name, Node, NodeId, PExp,
    PExpKind, Pat, Span, TAnnotOpt, Typ,
};

use crate::pass::TypedPass;

use super::typed::{mk_exp, mk_name};

#[derive(Clone, Copy, Debug, Default)]
pub struct EarlyReturn;

impl TypedPass for EarlyReturn {
    fn name(&self) -> &'static str {
        "early_return"
    }

    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        let ret = mk_name(interner.intern("early_return"));
        for def in &mut ast.defs {
            match &mut def.kind {
                DefKind::Fundef { funcls, annot, .. } => {
                    let ret_typ = declared_return(annot);
                    for funcl in funcls {
                        rewrite_funcl(funcl, ret_typ.as_ref(), &ret);
                    }
                }
                DefKind::InternalMutrec(sets) => {
                    for set in sets {
                        let ret_typ = declared_return(&set.annot);
                        for funcl in &mut set.funcls {
                            rewrite_funcl(funcl, ret_typ.as_ref(), &ret);
                        }
                    }
                }
                _ => {}
            }
        }
        ensure_spec(ast, interner);
    }
}

/// Declared clause return type, the untyped stand-in for OCaml
/// `Env.get_ret_typ` in `e_aux`.
fn declared_return(annot: &TAnnotOpt) -> Option<Typ> {
    match annot {
        TAnnotOpt::Some { typ, .. } => Some(typ.clone()),
        TAnnotOpt::None => None,
    }
}

/// Rewrite one clause body: gate on `contains_return`, wrap trailing leaves,
/// pull returns out, strip a whole-body return, convert the rest.
fn rewrite_funcl(funcl: &mut Funcl, ret_typ: Option<&Typ>, ret: &Name) {
    let body = match &mut funcl.arm.kind {
        PExpKind::PatExp { body, .. } => body,
        PExpKind::PatWhen { body, .. } => body,
    };
    let taken = std::mem::replace(body, mk_exp(ExpKind::Undef));
    *body = rewrite_body(taken, ret_typ, ret);
}

fn rewrite_body(body: Exp, ret_typ: Option<&Typ>, ret: &Name) -> Exp {
    if !contains_return(&body) {
        return body;
    }
    let pulled = pull_returns(add_final_return(body, false));
    let stripped = if is_return(&pulled) {
        get_return(pulled)
    } else {
        pulled
    };
    convert_remaining(stripped, ret_typ, ret)
}

/// OCaml `is_return`: a return, looking through `E_typ` casts.
fn is_return(exp: &Exp) -> bool {
    match &exp.kind {
        ExpKind::Return(_) => true,
        ExpKind::Typ { exp: inner, .. } => is_return(inner),
        _ => false,
    }
}

/// OCaml `get_return`: unwrap one return, keeping casts around the payload.
fn get_return(exp: Exp) -> Exp {
    let Node {
        id,
        span,
        annotation,
        kind,
    } = exp;
    match kind {
        ExpKind::Return(payload) => *payload,
        ExpKind::Typ { typ, exp: inner } => Exp {
            id,
            span,
            annotation,
            kind: ExpKind::Typ {
                typ,
                exp: Box::new(get_return(*inner)),
            },
        },
        kind => Exp {
            id,
            span,
            annotation,
            kind,
        },
    }
}

/// OCaml `is_unit`: the `()` literal.
fn is_unit(exp: &Exp) -> bool {
    matches!(&exp.kind, ExpKind::Lit(lit) if matches!(lit.kind, LitKind::Unit))
}

/// OCaml `contains_return`: any `Return` node in the subtree, including
/// guards, lexp index expressions and match arms.
fn contains_return(exp: &Exp) -> bool {
    match &exp.kind {
        ExpKind::Return(_) => true,
        ExpKind::Block(es) | ExpKind::Tuple(es) | ExpKind::Vector(es) | ExpKind::List(es) => {
            es.iter().any(contains_return)
        }
        ExpKind::Typ { exp: e, .. } => contains_return(e),
        ExpKind::App { args, .. } => args.iter().any(contains_return),
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => contains_return(cond) || contains_return(then_branch) || contains_return(else_branch),
        ExpKind::Loop {
            measure,
            cond,
            body,
            ..
        } => {
            measure.as_deref().is_some_and(contains_return)
                || contains_return(cond)
                || contains_return(body)
        }
        ExpKind::For {
            from,
            to,
            step,
            body,
            ..
        } => {
            contains_return(from)
                || contains_return(to)
                || contains_return(step)
                || contains_return(body)
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            contains_return(a) || contains_return(b)
        }
        ExpKind::Struct { fields, .. } => fields.iter().any(|f| contains_return(&f.value)),
        ExpKind::StructUpdate { record, fields } => {
            contains_return(record) || fields.iter().any(|f| contains_return(&f.value))
        }
        ExpKind::Field { record, .. } => contains_return(record),
        ExpKind::Match { scrutinee, arms } => {
            contains_return(scrutinee) || arms.iter().any(contains_return_arm)
        }
        ExpKind::Let { bind, body, .. } => contains_return(bind) || contains_return(body),
        ExpKind::Assign { lexp, value } => contains_return_lexp(lexp) || contains_return(value),
        ExpKind::Exit(e) | ExpKind::Throw(e) | ExpKind::InternalReturn(e) => contains_return(e),
        ExpKind::Try { exp: e, arms } => contains_return(e) || arms.iter().any(contains_return_arm),
        ExpKind::Assert { cond, msg } => contains_return(cond) || contains_return(msg),
        ExpKind::Var { lexp, init, body } => {
            contains_return_lexp(lexp) || contains_return(init) || contains_return(body)
        }
        ExpKind::InternalPlet { bind, body, .. } => contains_return(bind) || contains_return(body),
        ExpKind::InternalAssume { exp: e, .. } => contains_return(e),
        ExpKind::Id(_)
        | ExpKind::Lit(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Config(_)
        | ExpKind::Ref(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Constraint(_) => false,
    }
}

fn contains_return_lexp(lexp: &LExp) -> bool {
    match &lexp.kind {
        LExpKind::Deref(e) => contains_return(e),
        LExpKind::Vector { lexp: l, index } => contains_return_lexp(l) || contains_return(index),
        LExpKind::VectorRange { lexp: l, lo, hi } => {
            contains_return_lexp(l) || contains_return(lo) || contains_return(hi)
        }
        LExpKind::Field { lexp: l, .. } => contains_return_lexp(l),
        LExpKind::Tuple(parts) | LExpKind::VectorConcat(parts) => {
            parts.iter().any(contains_return_lexp)
        }
        LExpKind::Id(_) | LExpKind::Typ { .. } => false,
    }
}

fn contains_return_arm(arm: &PExp) -> bool {
    match &arm.kind {
        PExpKind::PatExp { body, .. } => contains_return(body),
        PExpKind::PatWhen { guard, body, .. } => contains_return(guard) || contains_return(body),
    }
}

/// OCaml `add_final_return`: wrap trailing leaves in `Return` so the pull-out
/// below sees every exit uniformly. `nested` is false at the clause top.
fn add_final_return(exp: Exp, nested: bool) -> Exp {
    let Node {
        id,
        span,
        annotation,
        kind,
    } = exp;
    // `id`/`span` are `Copy`; `annotation` is cloned only when a second
    // synthesized node is built.
    let rewrap = |annotation: Option<Box<Annotation>>, kind| Exp {
        id,
        span,
        annotation,
        kind,
    };
    let kind = match kind {
        ExpKind::Return(inner) => return rewrap(annotation, ExpKind::Return(inner)),
        ExpKind::Typ { typ, exp: inner } => {
            let inner = add_final_return(*inner, nested);
            if is_return(&inner) {
                let payload = get_return(inner);
                let typed = rewrap(
                    annotation.clone(),
                    ExpKind::Typ {
                        typ,
                        exp: Box::new(payload),
                    },
                );
                ExpKind::Return(Box::new(typed))
            } else {
                ExpKind::Typ {
                    typ,
                    exp: Box::new(inner),
                }
            }
        }
        ExpKind::Block(mut es) => {
            if let Some(last) = es.pop() {
                es.push(add_final_return(last, true));
            }
            ExpKind::Block(es)
        }
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => ExpKind::If {
            cond,
            then_branch: Box::new(add_final_return(*then_branch, true)),
            else_branch: Box::new(add_final_return(*else_branch, true)),
        },
        ExpKind::Match { scrutinee, arms } => ExpKind::Match {
            scrutinee,
            arms: arms
                .into_iter()
                .map(|arm| add_final_return_arm(arm, true))
                .collect(),
        },
        ExpKind::Let { pat, bind, body } => ExpKind::Let {
            pat,
            bind,
            body: Box::new(add_final_return(*body, true)),
        },
        ExpKind::Var { lexp, init, body } => ExpKind::Var {
            lexp,
            init,
            body: Box::new(add_final_return(*body, true)),
        },
        other => {
            if nested && !contains_return(&rewrap(annotation.clone(), other.clone())) {
                ExpKind::Return(Box::new(rewrap(annotation.clone(), other)))
            } else {
                other
            }
        }
    };
    rewrap(annotation, kind)
}

/// `add_final_return` for match arms: bodies only, guards are left alone
/// (OCaml maps over `Pat_exp` / `Pat_when` bodies).
fn add_final_return_arm(arm: PExp, nested: bool) -> PExp {
    let PExp { id, span, kind } = arm;
    let kind = match kind {
        PExpKind::PatExp { pat, body } => PExpKind::PatExp {
            pat,
            body: add_final_return(body, nested),
        },
        PExpKind::PatWhen { pat, guard, body } => PExpKind::PatWhen {
            pat,
            guard,
            body: add_final_return(body, nested),
        },
    };
    PExp { id, span, kind }
}

/// Bottom-up pull-out: children first (OCaml `fold_exp` order), then the
/// node-local `e_*` rule.
fn pull_returns(exp: Exp) -> Exp {
    pull_node(map_exp_children(exp, &mut pull_returns))
}

/// Bottom-up conversion of remaining returns (OCaml's second fold, `e_aux`).
fn convert_remaining(exp: Exp, ret_typ: Option<&Typ>, ret: &Name) -> Exp {
    let node = map_exp_children(exp, &mut |child| convert_remaining(child, ret_typ, ret));
    let Node {
        id,
        span,
        annotation,
        kind,
    } = node;
    match kind {
        ExpKind::Return(payload) => {
            // OCaml `add_e_typ`: the payload is wrapped in a fresh type
            // node when the function declares a return type; the wrapper is
            // synthesized, hence `Node::dummy` (typed.rs convention).
            let wrapped = match ret_typ {
                Some(typ) => mk_exp(ExpKind::Typ {
                    typ: typ.clone(),
                    exp: payload,
                }),
                None => *payload,
            };
            Exp {
                id,
                span,
                annotation,
                kind: ExpKind::App {
                    func: ret.clone(),
                    args: vec![wrapped],
                },
            }
        }
        kind => Exp {
            id,
            span,
            annotation,
            kind,
        },
    }
}

/// One bottom-up child traversal shared by [`pull_returns`] and
/// [`convert_remaining`]: rebuild the node with `f` applied to every child
/// expression (guards, lexp index expressions and match arms included, as
/// OCaml's `fold_exp` does). Node identity is preserved.
fn map_exp_children(exp: Exp, f: &mut impl FnMut(Exp) -> Exp) -> Exp {
    let Node {
        id,
        span,
        annotation,
        kind,
    } = exp;
    let kind = match kind {
        ExpKind::Block(es) => ExpKind::Block(es.into_iter().map(f).collect()),
        ExpKind::Typ { typ, exp: e } => ExpKind::Typ {
            typ,
            exp: Box::new(f(*e)),
        },
        ExpKind::App { func, args } => ExpKind::App {
            func,
            args: args.into_iter().map(f).collect(),
        },
        ExpKind::Tuple(es) => ExpKind::Tuple(es.into_iter().map(f).collect()),
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => ExpKind::If {
            cond: Box::new(f(*cond)),
            then_branch: Box::new(f(*then_branch)),
            else_branch: Box::new(f(*else_branch)),
        },
        ExpKind::Loop {
            loop_kind,
            measure,
            cond,
            body,
        } => ExpKind::Loop {
            loop_kind,
            measure: measure.map(|m| Box::new(f(*m))),
            cond: Box::new(f(*cond)),
            body: Box::new(f(*body)),
        },
        ExpKind::For {
            var,
            from,
            to,
            step,
            order,
            body,
        } => ExpKind::For {
            var,
            from: Box::new(f(*from)),
            to: Box::new(f(*to)),
            step: Box::new(f(*step)),
            order,
            body: Box::new(f(*body)),
        },
        ExpKind::Vector(es) => ExpKind::Vector(es.into_iter().map(f).collect()),
        ExpKind::VectorAppend(a, b) => ExpKind::VectorAppend(Box::new(f(*a)), Box::new(f(*b))),
        ExpKind::List(es) => ExpKind::List(es.into_iter().map(f).collect()),
        ExpKind::Cons(a, b) => ExpKind::Cons(Box::new(f(*a)), Box::new(f(*b))),
        ExpKind::Struct { name, fields } => ExpKind::Struct {
            name,
            fields: map_fields(fields, f),
        },
        ExpKind::StructUpdate { record, fields } => ExpKind::StructUpdate {
            record: Box::new(f(*record)),
            fields: map_fields(fields, f),
        },
        ExpKind::Field { record, field } => ExpKind::Field {
            record: Box::new(f(*record)),
            field,
        },
        ExpKind::Match { scrutinee, arms } => ExpKind::Match {
            scrutinee: Box::new(f(*scrutinee)),
            arms: arms.into_iter().map(|arm| map_arm(arm, f)).collect(),
        },
        ExpKind::Let { pat, bind, body } => ExpKind::Let {
            pat,
            bind: Box::new(f(*bind)),
            body: Box::new(f(*body)),
        },
        ExpKind::Assign { lexp, value } => ExpKind::Assign {
            lexp: Box::new(map_lexp(*lexp, f)),
            value: Box::new(f(*value)),
        },
        ExpKind::Return(e) => ExpKind::Return(Box::new(f(*e))),
        ExpKind::Exit(e) => ExpKind::Exit(Box::new(f(*e))),
        ExpKind::Throw(e) => ExpKind::Throw(Box::new(f(*e))),
        ExpKind::InternalReturn(e) => ExpKind::InternalReturn(Box::new(f(*e))),
        ExpKind::Try { exp: e, arms } => ExpKind::Try {
            exp: Box::new(f(*e)),
            arms: arms.into_iter().map(|arm| map_arm(arm, f)).collect(),
        },
        ExpKind::Assert { cond, msg } => ExpKind::Assert {
            cond: Box::new(f(*cond)),
            msg: Box::new(f(*msg)),
        },
        ExpKind::Var { lexp, init, body } => ExpKind::Var {
            lexp: Box::new(map_lexp(*lexp, f)),
            init: Box::new(f(*init)),
            body: Box::new(f(*body)),
        },
        ExpKind::InternalPlet { pat, bind, body } => ExpKind::InternalPlet {
            pat,
            bind: Box::new(f(*bind)),
            body: Box::new(f(*body)),
        },
        ExpKind::InternalAssume { constraint, exp: e } => ExpKind::InternalAssume {
            constraint,
            exp: Box::new(f(*e)),
        },
        kind @ (ExpKind::Id(_)
        | ExpKind::Lit(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Config(_)
        | ExpKind::Ref(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Constraint(_)) => kind,
    };
    Exp {
        id,
        span,
        annotation,
        kind,
    }
}

fn map_arm(arm: PExp, f: &mut impl FnMut(Exp) -> Exp) -> PExp {
    let PExp { id, span, kind } = arm;
    let kind = match kind {
        PExpKind::PatExp { pat, body } => PExpKind::PatExp { pat, body: f(body) },
        PExpKind::PatWhen { pat, guard, body } => PExpKind::PatWhen {
            pat,
            guard: f(guard),
            body: f(body),
        },
    };
    PExp { id, span, kind }
}

fn map_lexp(lexp: LExp, f: &mut impl FnMut(Exp) -> Exp) -> LExp {
    let Node {
        id,
        span,
        annotation,
        kind,
    } = lexp;
    let kind = match kind {
        LExpKind::Deref(e) => LExpKind::Deref(Box::new(f(*e))),
        LExpKind::Vector { lexp: l, index } => LExpKind::Vector {
            lexp: Box::new(map_lexp(*l, f)),
            index: Box::new(f(*index)),
        },
        LExpKind::VectorRange { lexp: l, lo, hi } => LExpKind::VectorRange {
            lexp: Box::new(map_lexp(*l, f)),
            lo: Box::new(f(*lo)),
            hi: Box::new(f(*hi)),
        },
        LExpKind::Field { lexp: l, field } => LExpKind::Field {
            lexp: Box::new(map_lexp(*l, f)),
            field,
        },
        LExpKind::Tuple(parts) => {
            LExpKind::Tuple(parts.into_iter().map(|p| map_lexp(p, f)).collect())
        }
        LExpKind::VectorConcat(parts) => {
            LExpKind::VectorConcat(parts.into_iter().map(|p| map_lexp(p, f)).collect())
        }
        kind @ (LExpKind::Id(_) | LExpKind::Typ { .. }) => kind,
    };
    LExp {
        id,
        span,
        annotation,
        kind,
    }
}

fn map_fields(fields: Vec<sail_ast::FExp>, f: &mut impl FnMut(Exp) -> Exp) -> Vec<sail_ast::FExp> {
    fields
        .into_iter()
        .map(|mut field| {
            field.value = f(field.value);
            field
        })
        .collect()
}

/// Node-local pull-out rules (`e_block`, `e_if`, `e_case`, `e_let`, `e_var`,
/// `e_app`); every other shape passes through.
fn pull_node(exp: Exp) -> Exp {
    let Node {
        id,
        span,
        annotation,
        kind,
    } = exp;
    match kind {
        ExpKind::Block(es) => e_block(id, span, annotation, es),
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => e_if(id, span, annotation, *cond, *then_branch, *else_branch),
        ExpKind::Match { scrutinee, arms } => e_case(id, span, annotation, *scrutinee, arms),
        ExpKind::Let { pat, bind, body } => e_let(id, span, annotation, pat, *bind, *body),
        ExpKind::Var { lexp, init, body } => e_var(id, span, annotation, lexp, *init, *body),
        ExpKind::App { func, args } => e_app(id, span, annotation, func, args),
        kind => Exp {
            id,
            span,
            annotation,
            kind,
        },
    }
}

type Ann = Option<Box<Annotation>>;

/// OCaml `e_block`: fold if-returns right-to-left, unwrap singleton blocks,
/// hoist a trailing return out of the block.
fn e_block(id: NodeId, span: Span, annotation: Ann, es: Vec<Exp>) -> Exp {
    let folded = es
        .into_iter()
        .rev()
        .fold(Vec::new(), |block, exp| fold_if_return(exp, block));
    if folded.len() == 1 {
        // `| [e] -> e`: drop the wrapper, keep the statement's own node.
        return folded.into_iter().next().expect("len == 1");
    }
    if folded.last().is_some_and(is_return) {
        let mut es = folded;
        let payload = get_return(es.pop().expect("last is a return"));
        let (pid, pspan, pann) = (payload.id, payload.span, payload.annotation.clone());
        es.push(payload);
        let inner = Exp {
            id: pid,
            span: pspan,
            annotation: pann,
            kind: ExpKind::Block(es),
        };
        Exp {
            id,
            span,
            annotation,
            kind: ExpKind::Return(Box::new(inner)),
        }
    } else {
        Exp {
            id,
            span,
            annotation,
            kind: ExpKind::Block(folded),
        }
    }
}

/// One right-fold step of `e_block` (`fold_if_return`): an if whose branch is
/// an early return absorbs the statements after it into the other branch.
fn fold_if_return(exp: Exp, block: Vec<Exp>) -> Vec<Exp> {
    let Node {
        id,
        span,
        annotation,
        kind,
    } = exp;
    match kind {
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } if is_return(&then_branch) => {
            let (mid, mspan, mann) = block.last().map_or_else(
                || {
                    (
                        else_branch.id,
                        else_branch.span,
                        else_branch.annotation.clone(),
                    )
                },
                |last| (last.id, last.span, last.annotation.clone()),
            );
            let mut inner = block;
            if !is_unit(&else_branch) {
                inner.insert(0, *else_branch);
            }
            let body = e_block(mid, mspan, mann.clone(), inner);
            vec![Exp {
                id,
                span,
                annotation,
                kind: e_if_kind(*cond, *then_branch, body, mid, mspan, mann),
            }]
        }
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } if is_return(&else_branch) => {
            let (mid, mspan, mann) = block.last().map_or_else(
                || {
                    (
                        then_branch.id,
                        then_branch.span,
                        then_branch.annotation.clone(),
                    )
                },
                |last| (last.id, last.span, last.annotation.clone()),
            );
            let mut inner = block;
            if !is_unit(&then_branch) {
                inner.insert(0, *then_branch);
            }
            let body = e_block(mid, mspan, mann.clone(), inner);
            vec![Exp {
                id,
                span,
                annotation,
                kind: e_if_kind(*cond, body, *else_branch, mid, mspan, mann),
            }]
        }
        kind => {
            let mut out = Vec::with_capacity(block.len() + 1);
            out.push(Exp {
                id,
                span,
                annotation,
                kind,
            });
            out.extend(block);
            out
        }
    }
}

/// OCaml `e_if`, with the inner `if` node metadata passed in: `pull_node`
/// uses the unwrapped then-payload's node, `fold_if_return` the absorbed
/// block's tail node.
fn e_if(
    id: NodeId,
    span: Span,
    annotation: Ann,
    cond: Exp,
    then_branch: Exp,
    else_branch: Exp,
) -> Exp {
    let kind = if is_return(&then_branch) && is_return(&else_branch) {
        let unwrapped = get_return(then_branch);
        let (iid, ispan, iann) = (unwrapped.id, unwrapped.span, unwrapped.annotation.clone());
        e_if_kind(cond, unwrapped, get_return(else_branch), iid, ispan, iann)
    } else {
        ExpKind::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch: Box::new(else_branch),
        }
    };
    Exp {
        id,
        span,
        annotation,
        kind,
    }
}

fn e_if_kind(
    cond: Exp,
    then_branch: Exp,
    else_branch: Exp,
    inner_id: NodeId,
    inner_span: Span,
    inner_ann: Ann,
) -> ExpKind {
    if is_return(&then_branch) && is_return(&else_branch) {
        ExpKind::Return(Box::new(Exp {
            id: inner_id,
            span: inner_span,
            annotation: inner_ann,
            kind: ExpKind::If {
                cond: Box::new(cond),
                then_branch: Box::new(get_return(then_branch)),
                else_branch: Box::new(get_return(else_branch)),
            },
        }))
    } else {
        ExpKind::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch: Box::new(else_branch),
        }
    }
}

/// OCaml `e_case`: when every arm returns, hoist into `return match ..`.
/// The inner `match` reuses the first unwrapped arm body's node.
fn e_case(id: NodeId, span: Span, annotation: Ann, scrutinee: Exp, arms: Vec<PExp>) -> Exp {
    if arms.iter().all(arm_is_return) {
        let mut out = Vec::with_capacity(arms.len());
        let mut inner_meta: Option<(NodeId, Span, Ann)> = None;
        for arm in arms {
            let PExp {
                id: aid,
                span: aspan,
                kind,
            } = arm;
            match kind {
                PExpKind::PatExp { pat, body } => {
                    let body = get_return(body);
                    if inner_meta.is_none() {
                        inner_meta = Some((body.id, body.span, body.annotation.clone()));
                    }
                    out.push(PExp {
                        id: aid,
                        span: aspan,
                        kind: PExpKind::PatExp { pat, body },
                    });
                }
                PExpKind::PatWhen { pat, guard, body } => {
                    let body = get_return(body);
                    if inner_meta.is_none() {
                        inner_meta = Some((body.id, body.span, body.annotation.clone()));
                    }
                    out.push(PExp {
                        id: aid,
                        span: aspan,
                        kind: PExpKind::PatWhen { pat, guard, body },
                    });
                }
            }
        }
        let (iid, ispan, iann) = inner_meta.unwrap_or((NodeId::DUMMY, Span::DUMMY, None));
        let inner = Exp {
            id: iid,
            span: ispan,
            annotation: iann,
            kind: ExpKind::Match {
                scrutinee: Box::new(scrutinee),
                arms: out,
            },
        };
        Exp {
            id,
            span,
            annotation,
            kind: ExpKind::Return(Box::new(inner)),
        }
    } else {
        Exp {
            id,
            span,
            annotation,
            kind: ExpKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
        }
    }
}

fn arm_is_return(arm: &PExp) -> bool {
    match &arm.kind {
        PExpKind::PatExp { body, .. } => is_return(body),
        PExpKind::PatWhen { body, .. } => is_return(body),
    }
}

/// OCaml `e_let`: a returning body is hoisted out of the `let`.
fn e_let(id: NodeId, span: Span, annotation: Ann, pat: Pat, bind: Exp, body: Exp) -> Exp {
    if is_return(&body) {
        let body = get_return(body);
        let (bid, bspan, bann) = (body.id, body.span, body.annotation.clone());
        let inner = Exp {
            id: bid,
            span: bspan,
            annotation: bann,
            kind: ExpKind::Let {
                pat,
                bind: Box::new(bind),
                body: Box::new(body),
            },
        };
        Exp {
            id,
            span,
            annotation,
            kind: ExpKind::Return(Box::new(inner)),
        }
    } else {
        Exp {
            id,
            span,
            annotation,
            kind: ExpKind::Let {
                pat,
                bind: Box::new(bind),
                body: Box::new(body),
            },
        }
    }
}

/// OCaml `e_var`: a returning body is hoisted out of the `var`.
fn e_var(id: NodeId, span: Span, annotation: Ann, lexp: Box<LExp>, init: Exp, body: Exp) -> Exp {
    if is_return(&body) {
        let body = get_return(body);
        let (bid, bspan, bann) = (body.id, body.span, body.annotation.clone());
        let inner = Exp {
            id: bid,
            span: bspan,
            annotation: bann,
            kind: ExpKind::Var {
                lexp,
                init: Box::new(init),
                body: Box::new(body),
            },
        };
        Exp {
            id,
            span,
            annotation,
            kind: ExpKind::Return(Box::new(inner)),
        }
    } else {
        Exp {
            id,
            span,
            annotation,
            kind: ExpKind::Var {
                lexp,
                init: Box::new(init),
                body: Box::new(body),
            },
        }
    }
}

/// OCaml `e_app`: an application with a returning argument collapses to that
/// argument's payload (a `return` aborts the whole call, so the application
/// never produces a value).
fn e_app(id: NodeId, span: Span, annotation: Ann, func: Name, args: Vec<Exp>) -> Exp {
    if let Some(pos) = args.iter().position(is_return) {
        let mut args = args;
        let found = args.remove(pos);
        Exp {
            id,
            span,
            annotation,
            kind: ExpKind::Return(Box::new(get_return(found))),
        }
    } else {
        Exp {
            id,
            span,
            annotation,
            kind: ExpKind::App { func, args },
        }
    }
}

/// Prepend OCaml's `early_ret_spec`
/// (`gen_vs ~pure:true ("early_return", "forall ('a : Type) ('b : Type). 'a -> 'b")`)
/// unless already present (idempotency guard: OCaml prepends unconditionally
/// because it runs once; the pipeline may run this pass again).
///
/// The guard resolves through the caller's interner, so it only recognizes
/// specs interned with the *same* interner — the `TypedPass` contract (the
/// caller owns one interner per program). Under a fresh interner the symbols
/// no longer resolve and a second spec would be prepended; that rerun mode is
/// outside the contract (it also breaks every other interner-relative
/// comparison, e.g. `Fresh` seeding).
fn ensure_spec(ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
    let present = ast.defs.iter().any(|def| {
        matches!(&def.kind, DefKind::Val(vs) if interner.resolve(vs.name.symbol) == Some("early_return"))
    });
    if present {
        return;
    }
    let mut kid = |text: &str| interner.intern(text);
    let (a, b) = (kid("a"), kid("b"));
    let var = |kid| sail_ast::Typ::dummy(sail_ast::TypKind::Var(kid));
    let spec = sail_ast::ValSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: mk_name(interner.intern("early_return")),
        typschm: sail_ast::TypSchm {
            quant: vec![
                sail_ast::QuantItem::Id(sail_ast::KOpt {
                    kind: sail_ast::Kind::Type,
                    kid: a,
                }),
                sail_ast::QuantItem::Id(sail_ast::KOpt {
                    kind: sail_ast::Kind::Type,
                    kid: b,
                }),
            ],
            typ: sail_ast::Typ::dummy(sail_ast::TypKind::Fn {
                args: vec![var(a)],
                ret: Box::new(var(b)),
            }),
        },
        // `gen_vs` parses a bare spec string: no `extern` block, so the
        // `~pure:true` flag has nowhere to land on the untyped AST.
        extern_opt: None,
    };
    ast.defs.insert(0, sail_ast::Def::dummy(DefKind::Val(spec)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{
        Ast, Interner, NodeId, PExpKind, PatKind, RecOpt, SimpleInterner, Span, TAnnotOpt,
    };

    use crate::passes::typed::mk_pat;

    fn funcl(it: &mut SimpleInterner, body: Exp) -> Funcl {
        Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: mk_name(it.intern("f")),
            arm: PExp {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                kind: PExpKind::PatExp {
                    pat: mk_pat(PatKind::Wild),
                    body,
                },
            },
        }
    }

    fn run_fun(it: &mut SimpleInterner, body: Exp) -> Ast {
        let mut ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Fundef {
            rec: RecOpt::NonRec,
            annot: TAnnotOpt::None,
            funcls: vec![funcl(it, body)],
        })]);
        EarlyReturn.run(&mut ast, it);
        ast
    }

    fn body_of(ast: &Ast) -> &Exp {
        let fun = ast
            .defs
            .iter()
            .find_map(|def| match &def.kind {
                DefKind::Fundef { funcls, .. } => Some(&funcls[0]),
                _ => None,
            })
            .expect("fundef");
        match &fun.arm.kind {
            PExpKind::PatExp { body, .. } => body,
            PExpKind::PatWhen { .. } => panic!("unexpected guard arm"),
        }
    }

    fn id(it: &mut SimpleInterner, name: &str) -> Exp {
        mk_exp(ExpKind::Id(mk_name(it.intern(name))))
    }

    fn is_early_call(body: &Exp, it: &SimpleInterner, payload: &str) -> bool {
        match &body.kind {
            ExpKind::App { func, args } => {
                it.resolve(func.symbol) == Some("early_return")
                    && args.len() == 1
                    && matches!(&args[0].kind, ExpKind::Id(n) if it.resolve(n.symbol) == Some(payload))
            }
            _ => false,
        }
    }

    #[test]
    fn strips_trailing_return_and_leaves_other_bodies_alone() {
        let mut it = SimpleInterner::new();
        let head = id(&mut it, "a");
        let tail = mk_exp(ExpKind::Return(Box::new(id(&mut it, "b"))));
        let ast = run_fun(&mut it, mk_exp(ExpKind::Block(vec![head, tail])));
        let ExpKind::Block(stmts) = &body_of(&ast).kind else {
            panic!("expected block, got {:?}", body_of(&ast).kind)
        };
        assert_eq!(stmts.len(), 2);
        assert!(matches!(&stmts[0].kind, ExpKind::Id(_)));
        assert!(matches!(&stmts[1].kind, ExpKind::Id(n) if it.resolve(n.symbol) == Some("b")));

        // No return anywhere: body (and spec aside) untouched.
        let mut it = SimpleInterner::new();
        let plain = id(&mut it, "a");
        let before = mk_exp(ExpKind::Block(vec![plain]));
        let ast = run_fun(&mut it, before.clone());
        assert_eq!(body_of(&ast), &before);
    }

    #[test]
    fn converts_non_final_returns_into_early_return_calls() {
        let mut it = SimpleInterner::new();
        let early = mk_exp(ExpKind::Return(Box::new(id(&mut it, "a"))));
        let tail = id(&mut it, "b");
        let mut ast = run_fun(&mut it, mk_exp(ExpKind::Block(vec![early, tail])));
        let ExpKind::Block(stmts) = &body_of(&ast).kind else {
            panic!("expected block")
        };
        assert_eq!(stmts.len(), 2);
        assert!(is_early_call(&stmts[0], &it, "a"));
        assert!(matches!(&stmts[1].kind, ExpKind::Id(_)));
        // Idempotent under the shared interner (the pipeline contract):
        // no `Return` is left and the spec guard recognizes its own output.
        let snapshot = ast.clone();
        EarlyReturn.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }

    #[test]
    fn pulls_returns_out_of_ifs_lets_and_applications() {
        let mut it = SimpleInterner::new();
        // Both branches return: the whole body is one return, so the `if`
        // survives with unwrapped branches (OCaml's whole-body strip).
        let cond = id(&mut it, "c");
        let then_ret = mk_exp(ExpKind::Return(Box::new(id(&mut it, "a"))));
        let else_ret = mk_exp(ExpKind::Return(Box::new(id(&mut it, "b"))));
        let ast = run_fun(
            &mut it,
            mk_exp(ExpKind::If {
                cond: Box::new(cond),
                then_branch: Box::new(then_ret),
                else_branch: Box::new(else_ret),
            }),
        );
        let ExpKind::If {
            then_branch,
            else_branch,
            ..
        } = &body_of(&ast).kind
        else {
            panic!("expected stripped if, got {:?}", body_of(&ast).kind)
        };
        assert!(matches!(&then_branch.kind, ExpKind::Id(_)));
        assert!(matches!(&else_branch.kind, ExpKind::Id(_)));

        // Returning let body: the return hoists out and then strips, leaving
        // a bare `let`.
        let mut it = SimpleInterner::new();
        let bind = id(&mut it, "bind");
        let ret_x = mk_exp(ExpKind::Return(Box::new(id(&mut it, "x"))));
        let ast = run_fun(
            &mut it,
            mk_exp(ExpKind::Let {
                pat: mk_pat(PatKind::Wild),
                bind: Box::new(bind),
                body: Box::new(ret_x),
            }),
        );
        let ExpKind::Let { body, .. } = &body_of(&ast).kind else {
            panic!("expected stripped let, got {:?}", body_of(&ast).kind)
        };
        assert!(matches!(&body.kind, ExpKind::Id(_)));

        // Returning call argument collapses the application.
        let mut it = SimpleInterner::new();
        let func = mk_name(it.intern("f"));
        let ret_x = mk_exp(ExpKind::Return(Box::new(id(&mut it, "x"))));
        let ast = run_fun(
            &mut it,
            mk_exp(ExpKind::App {
                func,
                args: vec![ret_x],
            }),
        );
        assert!(matches!(&body_of(&ast).kind, ExpKind::Id(n) if it.resolve(n.symbol) == Some("x")));
    }

    #[test]
    fn hoists_all_returning_match_arms_and_converts_mixed_ones() {
        let mut it = SimpleInterner::new();
        let arm = |body: Exp| PExp {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind: PExpKind::PatExp {
                pat: mk_pat(PatKind::Wild),
                body,
            },
        };
        // All arms return: whole match hoists, then the top return strips.
        let ret_a = mk_exp(ExpKind::Return(Box::new(id(&mut it, "a"))));
        let ret_b = mk_exp(ExpKind::Return(Box::new(id(&mut it, "b"))));
        let scrutinee = id(&mut it, "s");
        let ast = run_fun(
            &mut it,
            mk_exp(ExpKind::Match {
                scrutinee: Box::new(scrutinee),
                arms: vec![arm(ret_a), arm(ret_b)],
            }),
        );
        let ExpKind::Match { arms, .. } = &body_of(&ast).kind else {
            panic!("expected stripped match, got {:?}", body_of(&ast).kind)
        };
        assert_eq!(arms.len(), 2);
        for a in arms {
            let PExpKind::PatExp { body, .. } = &a.kind else {
                panic!("arm shape")
            };
            assert!(matches!(&body.kind, ExpKind::Id(_)));
        }

        // Genuinely mixed arms: one arm keeps a non-hoistable inner return
        // (inside a loop body, which no `e_*` rule pulls out), so the match
        // stays and only that return becomes an `early_return` call. (A plain
        // value arm would be wrapped by `add_final_return`, hoisted with the
        // rest and stripped — exactly like the all-return case above.)
        let mut it = SimpleInterner::new();
        let cond = id(&mut it, "c");
        let loop_ret = mk_exp(ExpKind::Return(Box::new(id(&mut it, "a"))));
        let loopy = mk_exp(ExpKind::Loop {
            loop_kind: sail_ast::Loop::While,
            measure: None,
            cond: Box::new(cond),
            body: Box::new(loop_ret),
        });
        let plain_b = id(&mut it, "b");
        let scrutinee = id(&mut it, "s");
        let ast = run_fun(
            &mut it,
            mk_exp(ExpKind::Match {
                scrutinee: Box::new(scrutinee),
                arms: vec![arm(loopy), arm(plain_b)],
            }),
        );
        let ExpKind::Match { arms, .. } = &body_of(&ast).kind else {
            panic!("expected match")
        };
        let PExpKind::PatExp { body: first, .. } = &arms[0].kind else {
            panic!("arm shape")
        };
        let ExpKind::Loop { body, .. } = &first.kind else {
            panic!("expected loop arm, got {:?}", first.kind)
        };
        assert!(is_early_call(body, &it, "a"));
        let PExpKind::PatExp { body: second, .. } = &arms[1].kind else {
            panic!("arm shape")
        };
        // The plain arm was wrapped by `add_final_return` and, unhoisted,
        // converted like any other remaining return.
        assert!(is_early_call(second, &it, "b"));
    }

    #[test]
    fn looks_through_casts_and_keeps_clause_guards_verbatim() {
        let mut it = SimpleInterner::new();
        // `return` under `E_typ` still counts; the strip keeps the cast
        // around the payload (OCaml `get_return`).
        let cast = sail_ast::Typ::dummy(sail_ast::TypKind::Id(mk_name(it.intern("int"))));
        let ret_x = mk_exp(ExpKind::Return(Box::new(id(&mut it, "x"))));
        let ast = run_fun(
            &mut it,
            mk_exp(ExpKind::Typ {
                typ: cast,
                exp: Box::new(ret_x),
            }),
        );
        let ExpKind::Typ { exp, .. } = &body_of(&ast).kind else {
            panic!("expected cast kept, got {:?}", body_of(&ast).kind)
        };
        assert!(matches!(&exp.kind, ExpKind::Id(n) if it.resolve(n.symbol) == Some("x")));

        // Clause guards are never rewritten, even when they return.
        let mut it = SimpleInterner::new();
        let guard = mk_exp(ExpKind::Return(Box::new(id(&mut it, "g"))));
        let mut ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Fundef {
            rec: RecOpt::NonRec,
            annot: TAnnotOpt::None,
            funcls: vec![Funcl {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                name: mk_name(it.intern("f")),
                arm: PExp {
                    id: NodeId::DUMMY,
                    span: Span::DUMMY,
                    kind: PExpKind::PatWhen {
                        pat: mk_pat(PatKind::Wild),
                        guard: guard.clone(),
                        body: id(&mut it, "b"),
                    },
                },
            }],
        })]);
        EarlyReturn.run(&mut ast, &mut it);
        let DefKind::Fundef { funcls, .. } = &ast.defs[1].kind else {
            panic!("def shape (spec is defs[0])")
        };
        let PExpKind::PatWhen { guard: kept, .. } = &funcls[0].arm.kind else {
            panic!("arm shape")
        };
        assert_eq!(kept, &guard);
    }

    #[test]
    fn wraps_payload_in_declared_return_type_when_present() {
        let mut it = SimpleInterner::new();
        let ret_ty = sail_ast::Typ::dummy(sail_ast::TypKind::Id(mk_name(it.intern("int"))));
        let ret_a = mk_exp(ExpKind::Return(Box::new(id(&mut it, "a"))));
        let plain_b = id(&mut it, "b");
        let mut ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Fundef {
            rec: RecOpt::NonRec,
            annot: TAnnotOpt::Some {
                quant: vec![],
                typ: ret_ty.clone(),
            },
            funcls: vec![funcl(&mut it, mk_exp(ExpKind::Block(vec![ret_a, plain_b])))],
        })]);
        EarlyReturn.run(&mut ast, &mut it);
        let ExpKind::Block(stmts) = &body_of(&ast).kind else {
            panic!("expected block")
        };
        let ExpKind::App { func, args } = &stmts[0].kind else {
            panic!("expected early_return call")
        };
        assert_eq!(it.resolve(func.symbol), Some("early_return"));
        let ExpKind::Typ { typ, exp } = &args[0].kind else {
            panic!("expected type wrapper, got {:?}", args[0].kind)
        };
        assert_eq!(typ, &ret_ty);
        assert!(matches!(&exp.kind, ExpKind::Id(_)));
    }

    #[test]
    fn injects_the_builtin_spec_exactly_once() {
        let mut it = SimpleInterner::new();
        let body = id(&mut it, "b");
        let mut ast = Ast::new(vec![sail_ast::Def::dummy(DefKind::Fundef {
            rec: RecOpt::NonRec,
            annot: TAnnotOpt::None,
            funcls: vec![funcl(&mut it, body)],
        })]);
        EarlyReturn.run(&mut ast, &mut it);
        assert_eq!(ast.defs.len(), 2);
        let DefKind::Val(vs) = &ast.defs[0].kind else {
            panic!("spec first, got {:?}", ast.defs[0].kind)
        };
        assert_eq!(it.resolve(vs.name.symbol), Some("early_return"));
        assert_eq!(vs.typschm.quant.len(), 2);
        assert!(vs.extern_opt.is_none());
        // Second run: no duplicate spec.
        let snapshot = ast.clone();
        EarlyReturn.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }
}
