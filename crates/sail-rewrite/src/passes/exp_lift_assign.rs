//! Assignment lifting into `var`, matching `rewrite_exp_lift_assign_intro`
//! and `rewrite_ast_exp_lift_assign` in `sail/src/lib/rewrites.ml`.
//!
//! OCaml turns an assignment to a *locally introduced* lvalue into a `var`
//! binding whose body is the rest of the block: in
//! `[.., x = e, ..rest]` (with `x` local) the tail becomes
//! `E_var (x', e', rest)` where `(x', e' -> ..)` is [`rewrite_lexp_to_rhs`].
//! A standalone `x = e` becomes `E_var (x', e', E_block [])`. Assignments to
//! non-locals (registers, globals) and effectful lvalues are left alone.
//!
//! Implemented now (all type/effect-free, hence exact):
//! - the traversal skeleton: [`visit_all`](super::typed::visit_all)
//!   bottom-up over [`ExpKind::Block`] statements ([`lift_block`]) and
//!   standalone [`ExpKind::Assign`] nodes ([`lift_single`]), with the exact
//!   OCaml `E_var (lhs, update(value), rest)` / `E_var (lhs, update(value),
//!   [])` construction behind the gate;
//! - [`lexp_to_exp`], mirroring `ast_util.ml` `lexp_to_exp` case for case
//!   (`Id`/`Typ` to `Id`, tuples of ids to tuples, `Vector` to
//!   `vector_access#`, `VectorRange` to `vector_subrange#`, `Field` to field
//!   reads, `VectorConcat` to `VectorAppend` chains, `Deref` to `__deref`
//!   calls; non-id tuple elements have no reading and yield `None`, where
//!   OCaml raises "Unsupported sub-lexp");
//! - [`rewrite_lexp_to_rhs`], mirroring the OCaml split case for case: simple
//!   lvalues pass through with the identity update; `Vector` / `VectorRange`
//!   recurse and wrap in `vector_update#` / `vector_update_subrange#` calls
//!   (argument order `[vec; index; value]` / `[vec; lo; hi; value]`, exactly
//!   as `ast_util.ml` builds them). The `#`-suffixed names are unwritable in
//!   Sail source, so no capture is possible.
//!
//! Deliberately deferred, with the OCaml reference for each gap:
//! - The lift gate ([`lift_gate`]) is always closed: OCaml requires
//!   `not (is_empty_tannot tannot) && lexp_is_local_intro le env` (and, for
//!   the standalone case, `not (lexp_is_effectful le)`). `lexp_is_local_intro`
//!   is `Env.lookup_id == Unbound` (`rewrites.ml`), i.e. a type-environment
//!   query rather than a syntactic property — scope-local shadowing does NOT
//!   imply it (an enclosing `E_var` binds the name as `Local`, which must
//!   *not* lift). Effectfulness is `Ast_util.effectful (effect_of_annot
//!   tannot)`. [`sail_ast::Annotation`] carries only source attributes /
//!   docs, never typechecker output, so neither predicate is decidable here;
//!   lifting without the gate would shadow registers or globals and change
//!   semantics. The gate therefore preserves every assignment verbatim until
//!   the typed driver can supply the environment.
//! - `rewrite_lexp_to_rhs` on `Field` needs `Env.expand_synonyms` plus
//!   `Env.is_record` (OCaml raises "Unsupported lexp" for non-records) and
//!   returns `None`; `VectorConcat` has no OCaml arm at all (it is deemed
//!   unreachable post-typecheck) and returns `None` as well.
//! - No `effect_info` is involved on either side: unlike `early_return`,
//!   `rewrite_ast_exp_lift_assign` neither takes nor returns effect info.
//!
//! Insert-Spec (vom Main-Agent beim Merge anzuwenden, diese Datei nicht
//! aendern):
//! - Pass-Name: `"exp_lift_assign"`
//! - Position in `LEAN_REWRITES`
//!   (`crates/sail-rewrite/src/pipeline.rs`): Index 29, zwischen
//!   `"const_prop_mutrec"` (28) und `"early_return"` (30).
//! - Trait-Impl-Name: `ExpLiftAssign` (diese Datei).
//!   `passes/mod.rs`: `pub mod exp_lift_assign;` +
//!   `pub use exp_lift_assign::ExpLiftAssign;` ergaenzen.
//!   `pipeline.rs`: `"exp_lift_assign" => Some(Arc::new(ExpLiftAssign))`
//!   in `make_typed_implemented` und den Namen in
//!   `is_typed_implemented_pass` aufnehmen.

#![forbid(unsafe_code)]

use sail_ast::{Exp, ExpKind, Interner, LExp, LExpKind, Node};

use crate::pass::TypedPass;

use super::typed::{mk_exp, mk_name, visit_all};

#[derive(Clone, Copy, Debug, Default)]
pub struct ExpLiftAssign;

impl TypedPass for ExpLiftAssign {
    fn name(&self) -> &'static str {
        "exp_lift_assign"
    }

    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        visit_all(ast, &mut |exp| lift_node(exp, interner), &mut |_| {});
    }
}

/// Bottom-up callback: try the block walker on blocks and the single-assign
/// rule on assignments. Both consult [`lift_gate`] and preserve the node
/// while the gate is closed for lack of a type environment.
fn lift_node(exp: &mut Exp, interner: &mut dyn sail_ast::Interner) {
    if let ExpKind::Block(_) = &exp.kind {
        let ExpKind::Block(stmts) = std::mem::replace(&mut exp.kind, ExpKind::Undef) else {
            unreachable!("matched Block above")
        };
        exp.kind = ExpKind::Block(lift_block(stmts, interner));
    } else {
        lift_single(exp, interner);
    }
}

/// OCaml `walker`: scan a block head-first; a gated assignment becomes
/// `E_var (lhs, update(value), rest)` with the lifted tail as its body.
fn lift_block(stmts: Vec<Exp>, interner: &mut dyn sail_ast::Interner) -> Vec<Exp> {
    let pos = stmts
        .iter()
        .position(|stmt| matches!(&stmt.kind, ExpKind::Assign { lexp, .. } if lift_gate(lexp)));
    let Some(pos) = pos else { return stmts };
    let mut head = stmts;
    let mut tail = head.split_off(pos);
    let stmt = tail.remove(0);
    let Node {
        id,
        span,
        annotation,
        kind,
    } = stmt;
    let ExpKind::Assign { lexp, value } = kind else {
        unreachable!("lift position is an assignment")
    };
    match rewrite_lexp_to_rhs(&lexp, interner) {
        Some((lhs, update)) => {
            let init = update(*value);
            let body = lift_block(tail, interner);
            head.push(Exp {
                id,
                span,
                annotation,
                kind: ExpKind::Var {
                    lexp: Box::new(lhs),
                    init: Box::new(init),
                    body: Box::new(mk_exp(ExpKind::Block(body))),
                },
            });
            head
        }
        None => {
            // Record-field and concat shapes need the type environment even
            // for the split; without it the assignment stays verbatim.
            head.push(Exp {
                id,
                span,
                annotation,
                kind: ExpKind::Assign { lexp, value },
            });
            head.extend(tail);
            head
        }
    }
}

/// OCaml standalone case: a gated `E_assign (le, e)` becomes
/// `E_var (le', re' e, E_block [])`.
fn lift_single(exp: &mut Exp, interner: &mut dyn sail_ast::Interner) {
    let gated = matches!(&exp.kind, ExpKind::Assign { lexp, .. } if lift_gate(lexp));
    if !gated {
        return;
    }
    let taken = std::mem::replace(&mut exp.kind, ExpKind::Undef);
    let ExpKind::Assign { lexp, value } = taken else {
        unreachable!("gated node is an assignment")
    };
    match rewrite_lexp_to_rhs(&lexp, interner) {
        Some((lhs, update)) => {
            let init = update(*value);
            exp.kind = ExpKind::Var {
                lexp: Box::new(lhs),
                init: Box::new(init),
                body: Box::new(mk_exp(ExpKind::Block(vec![]))),
            };
        }
        None => {
            exp.kind = ExpKind::Assign { lexp, value };
        }
    }
}

/// The lift gate (`not (is_empty_tannot tannot) && lexp_is_local_intro le
/// env`, plus `not (lexp_is_effectful le)` for the standalone case).
///
/// Always closed: `lexp_is_local_intro` is `Env.lookup_id == Unbound`, an
/// environment query with no syntactic equivalent, and both the tannot and
/// the effect predicates need typechecker annotations the untyped AST does
/// not carry. Kept as an explicit function (rather than inlining `false` at
/// the call sites) so the typed driver has exactly one place to supply the
/// environment.
fn lift_gate(_lexp: &LExp) -> bool {
    false
}

/// OCaml `ast_util.ml` `lexp_to_exp`: read an lvalue as an expression.
/// Returns `None` where OCaml raises "Unsupported sub-lexp" (non-id tuple
/// elements), so callers preserve instead of guessing.
pub fn lexp_to_exp(lexp: &LExp, interner: &mut dyn Interner) -> Option<Exp> {
    match &lexp.kind {
        LExpKind::Id(id) | LExpKind::Typ { id, .. } => Some(mk_exp(ExpKind::Id(id.clone()))),
        LExpKind::Tuple(les) => {
            let mut es = Vec::with_capacity(les.len());
            for le in les {
                match &le.kind {
                    LExpKind::Id(id) | LExpKind::Typ { id, .. } => {
                        es.push(mk_exp(ExpKind::Id(id.clone())));
                    }
                    _ => return None,
                }
            }
            Some(mk_exp(ExpKind::Tuple(es)))
        }
        LExpKind::Vector { lexp: l, index } => {
            let access = mk_name(interner.intern("vector_access#"));
            Some(mk_exp(ExpKind::App {
                func: access,
                args: vec![lexp_to_exp(l, interner)?, (**index).clone()],
            }))
        }
        LExpKind::VectorRange { lexp: l, lo, hi } => {
            let subrange = mk_name(interner.intern("vector_subrange#"));
            Some(mk_exp(ExpKind::App {
                func: subrange,
                args: vec![lexp_to_exp(l, interner)?, (**lo).clone(), (**hi).clone()],
            }))
        }
        LExpKind::Field { lexp: l, field } => Some(mk_exp(ExpKind::Field {
            record: Box::new(lexp_to_exp(l, interner)?),
            field: field.clone(),
        })),
        LExpKind::VectorConcat(les) => {
            let mut parts = les.iter();
            let first = lexp_to_exp(parts.next()?, interner)?;
            let mut acc = first;
            for le in parts {
                acc = mk_exp(ExpKind::VectorAppend(
                    Box::new(acc),
                    Box::new(lexp_to_exp(le, interner)?),
                ));
            }
            Some(acc)
        }
        LExpKind::Deref(e) => {
            let deref = mk_name(interner.intern("__deref"));
            Some(mk_exp(ExpKind::App {
                func: deref,
                args: vec![(**e).clone()],
            }))
        }
    }
}

/// Whole-value update for a lowered lvalue: given the assigned value, build
/// the replacement right-hand side (OCaml's `fun exp -> ...` continuation in
/// `rewrite_lexp_to_rhs`).
pub type LexUpdate = Box<dyn FnOnce(Exp) -> Exp>;

/// OCaml `rewrite_lexp_to_rhs`: split a complex lvalue into its simple base
/// lvalue plus the whole-value update applied to the assigned value.
/// Returns `None` for shapes needing the type environment (`Field`: record
/// check; `VectorConcat`: unreachable post-typecheck in OCaml).
pub fn rewrite_lexp_to_rhs(lexp: &LExp, interner: &mut dyn Interner) -> Option<(LExp, LexUpdate)> {
    match &lexp.kind {
        LExpKind::Id(_) | LExpKind::Typ { .. } | LExpKind::Tuple(_) | LExpKind::Deref(_) => {
            Some((lexp.clone(), Box::new(|value| value)))
        }
        LExpKind::Vector { lexp: l, index } => {
            let base = lexp_to_exp(l, interner)?;
            let (lhs, rhs) = rewrite_lexp_to_rhs(l, interner)?;
            let index = (**index).clone();
            let update = mk_name(interner.intern("vector_update#"));
            Some((
                lhs,
                Box::new(move |value| {
                    rhs(mk_exp(ExpKind::App {
                        func: update,
                        args: vec![base, index, value],
                    }))
                }),
            ))
        }
        LExpKind::VectorRange { lexp: l, lo, hi } => {
            let base = lexp_to_exp(l, interner)?;
            let (lhs, rhs) = rewrite_lexp_to_rhs(l, interner)?;
            let (lo, hi) = ((**lo).clone(), (**hi).clone());
            let update = mk_name(interner.intern("vector_update_subrange#"));
            Some((
                lhs,
                Box::new(move |value| {
                    rhs(mk_exp(ExpKind::App {
                        func: update,
                        args: vec![base, lo, hi, value],
                    }))
                }),
            ))
        }
        LExpKind::Field { .. } | LExpKind::VectorConcat(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Ast, DefKind, PExpKind, PatKind, RecOpt, SimpleInterner, TAnnotOpt};

    use crate::passes::typed::{mk_lexp, mk_pat};

    fn assign(it: &mut SimpleInterner, target: &str, value: Exp) -> Exp {
        mk_exp(ExpKind::Assign {
            lexp: Box::new(mk_lexp(LExpKind::Id(mk_name(it.intern(target))))),
            value: Box::new(value),
        })
    }

    fn fun_body(body: Exp) -> Ast {
        let mut it = SimpleInterner::new();
        Ast::new(vec![sail_ast::Def::dummy(DefKind::Fundef {
            rec: RecOpt::NonRec,
            annot: TAnnotOpt::None,
            funcls: vec![sail_ast::Funcl {
                id: sail_ast::NodeId::DUMMY,
                span: sail_ast::Span::DUMMY,
                name: mk_name(it.intern("f")),
                arm: sail_ast::PExp {
                    id: sail_ast::NodeId::DUMMY,
                    span: sail_ast::Span::DUMMY,
                    kind: PExpKind::PatExp {
                        pat: mk_pat(PatKind::Wild),
                        body,
                    },
                },
            }],
        })])
    }

    #[test]
    fn preserves_assignments_without_a_type_environment() {
        let mut it = SimpleInterner::new();
        let x = mk_name(it.intern("x"));
        let idx = mk_exp(ExpKind::Id(mk_name(it.intern("i"))));
        let vector_lexp = mk_lexp(LExpKind::Vector {
            lexp: Box::new(mk_lexp(LExpKind::Id(x.clone()))),
            index: Box::new(idx),
        });
        let mut ast = fun_body(mk_exp(ExpKind::Block(vec![
            assign(&mut it, "x", mk_exp(ExpKind::Undef)),
            mk_exp(ExpKind::Assign {
                lexp: Box::new(vector_lexp),
                value: Box::new(mk_exp(ExpKind::Undef)),
            }),
        ])));
        let snapshot = ast.clone();
        ExpLiftAssign.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot, "gate closed: assignments survive verbatim");
        // Idempotent: a second run changes nothing either.
        ExpLiftAssign.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }

    #[test]
    fn gate_stays_closed_for_every_lvalue_shape() {
        let mut it = SimpleInterner::new();
        let id_lexp = mk_lexp(LExpKind::Id(mk_name(it.intern("x"))));
        assert!(!lift_gate(&id_lexp));
        assert!(!lift_gate(&mk_lexp(LExpKind::Tuple(vec![id_lexp]))));
        assert!(!lift_gate(&mk_lexp(LExpKind::Vector {
            lexp: Box::new(mk_lexp(LExpKind::Id(mk_name(it.intern("v"))))),
            index: Box::new(mk_exp(ExpKind::Undef)),
        })));
        assert!(!lift_gate(&mk_lexp(LExpKind::Field {
            lexp: Box::new(mk_lexp(LExpKind::Id(mk_name(it.intern("r"))))),
            field: mk_name(it.intern("f")),
        })));
    }

    #[test]
    fn lexp_to_exp_mirrors_the_ocaml_reading() {
        let mut it = SimpleInterner::new();
        // Id and typed-id both read as plain ids.
        let lexp = mk_lexp(LExpKind::Id(mk_name(it.intern("x"))));
        let exp = lexp_to_exp(&lexp, &mut it).expect("id reads");
        assert!(matches!(&exp.kind, ExpKind::Id(n) if it.resolve(n.symbol) == Some("x")));

        // Vector index reads as `vector_access#` of base and index.
        let lexp = mk_lexp(LExpKind::Vector {
            lexp: Box::new(mk_lexp(LExpKind::Id(mk_name(it.intern("v"))))),
            index: Box::new(mk_exp(ExpKind::Id(mk_name(it.intern("i"))))),
        });
        let exp = lexp_to_exp(&lexp, &mut it).expect("vector reads");
        let ExpKind::App { func, args } = &exp.kind else {
            panic!("expected access call, got {:?}", exp.kind)
        };
        assert_eq!(it.resolve(func.symbol), Some("vector_access#"));
        assert_eq!(args.len(), 2);

        // Range reads as `vector_subrange#` of base, lo, hi.
        let lexp = mk_lexp(LExpKind::VectorRange {
            lexp: Box::new(mk_lexp(LExpKind::Id(mk_name(it.intern("v"))))),
            lo: Box::new(mk_exp(ExpKind::Undef)),
            hi: Box::new(mk_exp(ExpKind::Undef)),
        });
        let exp = lexp_to_exp(&lexp, &mut it).expect("range reads");
        let ExpKind::App { func, args } = &exp.kind else {
            panic!("expected subrange call")
        };
        assert_eq!(it.resolve(func.symbol), Some("vector_subrange#"));
        assert_eq!(args.len(), 3);

        // Tuple with a non-id element has no reading.
        let lexp = mk_lexp(LExpKind::Tuple(vec![mk_lexp(LExpKind::Deref(Box::new(
            mk_exp(ExpKind::Undef),
        )))]));
        assert!(lexp_to_exp(&lexp, &mut it).is_none());

        // Deref reads as a `__deref` call.
        let lexp = mk_lexp(LExpKind::Deref(Box::new(mk_exp(ExpKind::Undef))));
        let exp = lexp_to_exp(&lexp, &mut it).expect("deref reads");
        let ExpKind::App { func, args } = &exp.kind else {
            panic!("expected deref call")
        };
        assert_eq!(it.resolve(func.symbol), Some("__deref"));
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn rhs_split_lowers_nested_vectors_inside_out() {
        let mut it = SimpleInterner::new();
        // `a[i][j]`: base `a`, update `vector_update#(a, i, vector_update#(a[i], j, v))`.
        let inner = mk_lexp(LExpKind::Vector {
            lexp: Box::new(mk_lexp(LExpKind::Id(mk_name(it.intern("a"))))),
            index: Box::new(mk_exp(ExpKind::Id(mk_name(it.intern("i"))))),
        });
        let outer = mk_lexp(LExpKind::Vector {
            lexp: Box::new(inner),
            index: Box::new(mk_exp(ExpKind::Id(mk_name(it.intern("j"))))),
        });
        let (lhs, update) = rewrite_lexp_to_rhs(&outer, &mut it).expect("vectors lower");
        assert!(matches!(&lhs.kind, LExpKind::Id(n) if it.resolve(n.symbol) == Some("a")));
        let updated = update(mk_exp(ExpKind::Undef));
        let ExpKind::App { func, args } = &updated.kind else {
            panic!("expected update call, got {:?}", updated.kind)
        };
        assert_eq!(it.resolve(func.symbol), Some("vector_update#"));
        assert_eq!(args.len(), 3);
        // The value position holds the inner update of the read-back vector.
        let ExpKind::App {
            func: inner_func,
            args: inner_args,
        } = &args[2].kind
        else {
            panic!("expected nested update, got {:?}", args[2].kind)
        };
        assert_eq!(it.resolve(inner_func.symbol), Some("vector_update#"));
        assert_eq!(inner_args.len(), 3);

        // Simple lvalues pass through with the identity update.
        let simple = mk_lexp(LExpKind::Id(mk_name(it.intern("x"))));
        let (lhs, update) = rewrite_lexp_to_rhs(&simple, &mut it).expect("id passes");
        assert!(matches!(&lhs.kind, LExpKind::Id(_)));
        let value = mk_exp(ExpKind::Undef);
        assert_eq!(update(value.clone()), value);

        // Record fields and concats need the type environment.
        let field = mk_lexp(LExpKind::Field {
            lexp: Box::new(mk_lexp(LExpKind::Id(mk_name(it.intern("r"))))),
            field: mk_name(it.intern("f")),
        });
        assert!(rewrite_lexp_to_rhs(&field, &mut it).is_none());
        let concat = mk_lexp(LExpKind::VectorConcat(vec![mk_lexp(LExpKind::Id(
            mk_name(it.intern("c")),
        ))]));
        assert!(rewrite_lexp_to_rhs(&concat, &mut it).is_none());
    }
}
