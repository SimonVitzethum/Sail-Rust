#![forbid(unsafe_code)]
//! Mapping / outcome / instantiation body elaboration (Lane-34).
//!
//! `check_definition` still rejects these bodies with `Unsupported`
//! (`canonical_body.rs`, `DefKind::Mapdef | Scattered | Outcome |
//! `Instantiation` arm). This module elaborates them against the
//! Lane-19 declaration tables so the Main-Agent can insert the bodies
//! without touching `program.rs`:
//!
//! - mapping clauses (`check_mapcl`, OCaml `type_check.ml` ~L4737):
//!   every `mapcl` relates a forwards pattern (checked against the
//!   mapping type's `lhs`) to a backwards pattern (checked against
//!   `rhs`); either side may carry a `when` guard (checked against
//!   `bool`). A `Mapdef` clause must be bidirectional; single-sided
//!   `Forwards` / `Backwards` fragments only occur as scattered
//!   `Mapcl` clauses.
//! - outcome bodies (`check_outcome`, OCaml `type_check.ml` ~L5379):
//!   inner `val` schemes never leak into the global table (they live
//!   in the Lane-19 `outcome_vals` table); every inner `fundef` needs
//!   a matching inner `val`; clause bodies check in the outcome-local
//!   environment (params as rigid binders + inner vals in scope).
//! - instantiation bodies: the `subst` list must cover exactly the
//!   outcome parameter telescope with kind-matching arguments
//!   (Instantiation-Pflicht, see [`missing_instantiations`]).
//!
//! This module is deliberately `Program`-free: it validates pure AST
//! shapes and returns ordered [`MapdefPlan`] / [`OutcomePlan`] /
//! [`InstantiationPlan`] values. All Lane-19 table reads stay in the
//! insertion (Main-Agent owns `canonical_body.rs`), which resolves
//! names through the def interner exactly like `Checker::name`.
//!
//! # Insert-Spec for the Main-Agent (owns `canonical_body.rs`)
//!
//! Anchor: the `check()` match arm currently reading
//! ```ignore
//! DefKind::Impl(_)
//! | DefKind::Mapdef { .. }
//! | DefKind::Scattered(_)
//! | DefKind::Outcome { .. }
//! | DefKind::Instantiation { .. }
//! | DefKind::Measure { .. }
//! | DefKind::LoopMeasures { .. }
//! | DefKind::InternalMutrec(_) => {
//!     return Err(unsupported(def.span, "mapping, scattered, outcome or measure definition"))
//! }
//! ```
//! Replace it with the following arms (Lane-19 getters live on
//! `Program` in the Lane-19 tree; `ScatteredNamespace` is
//! `program::ScatteredNamespace` there):
//!
//! INSERT Mapdef:
//! ```ignore
//! DefKind::Mapdef { name, annot, clauses } => {
//!     let name_text = checker.name(name)?.to_owned();
//!     // Lane-19 header must already be declared at this position
//!     // (`declare` runs before `check_definition` in source order).
//!     let header = program.mapping(&name_text, position).cloned().ok_or_else(||
//!         TypeError::unbound_var(name.span, name_text.clone()))?;
//!     let plan = crate::outcome_elab::plan_mapdef(annot, clauses)?;
//!     // `header.typ` is `TypKind::Bidir { lhs, rhs }` (declare guarantees
//!     // it); `header.quant` is the mapping telescope, `header.interner`
//!     // resolves its binders. Set `checker.context` to a scope carrying
//!     // that telescope (same shape as `function()` does for `val`
//!     // schemes), then per clause of `plan.clauses` (in order):
//!     //   forwards pattern <= lhs in a fresh child scope,
//!     //   forwards guard (if `MapclDir` reports one) <= bool in that scope,
//!     //   backwards pattern <= rhs in a second fresh child scope,
//!     //   backwards guard (if any) <= bool in that scope.
//!     // Pattern work reuses `Checker::pattern` (Lane-09 bindings);
//!     // guard work reuses the `run` task queue with `builtin("bool")`.
//! }
//! ```
//!
//! INSERT scattered Mapcl (`DefKind::Scattered(ScatteredDef::Mapcl { name, clause })`;
//! other scattered variants keep their current behaviour):
//! ```ignore
//! // Require the open block at this position, mirroring declare's
//! // clause gate:
//! let open = program.scattered(ScatteredNamespace::Mapping, name_text, position)
//!     .is_some_and(|decl| !decl.closed);
//! if !open { return Err(/* QuantError: clause has no open scattered block */); }
//! let plan = crate::outcome_elab::plan_scattered_mapcl(clause)?;
//! // Check the present side(s) exactly like a Mapdef clause side, with
//! // lhs/rhs taken from the scattered opening annotation when present
//! // (`decl.ret` + `decl.quant`), else from the closed mapping header
//! // when the name is also a declared mapping.
//! ```
//!
//! INSERT Outcome:
//! ```ignore
//! DefKind::Outcome { name, typschm: _, quant: params, defs } => {
//!     let name_text = checker.name(name)?.to_owned();
//!     let header = program.outcome(&name_text, position).cloned().ok_or_else(||
//!         TypeError::unbound_var(name.span, name_text.clone()))?;
//!     let plan = crate::outcome_elab::plan_outcome(defs, &checker.it)?;
//!     // Well-formedness the insertion still owes (Lane-19 `declare`
//!     // already ran it, re-run for the body view): header params +
//!     // typschm via `program.check_scheme`-equivalents; each inner val
//!     // scheme via `program.check_scheme(&val.typschm, &checker.it, position)`.
//!     // Then seed a root scope with the outcome-local environment:
//!     // for every `OutcomeLocalVal { name, .. }` in `plan.vals`, resolve
//!     // `program.outcome_val(&name_text, &val.name, position)` (must be
//!     // `Some`; else the declare/check views diverged => internal error)
//!     // and bind it as an immutable local under the outcome-param scope
//!     // (`header.params` as rigid binders, same as function quant scope).
//!     // Finally check every inner `DefKind::Fundef` in `defs` with the
//!     // existing `Checker::function` per funcl under that seeded scope
//!     // (inline `annot` on an inner fundef runs through
//!     // `check_function_header`-equivalent logic against the
//!     // outcome-local scheme, flagged by `OutcomeLocalFun::has_annot`).
//! }
//! ```
//!
//! INSERT Instantiation:
//! ```ignore
//! DefKind::Instantiation { name, substs } => {
//!     let name_text = checker.name(name)?.to_owned();
//!     let header = program.outcome(&name_text, position).cloned().ok_or_else(||
//!         TypeError::unbound_var(name.span, name_text.clone()))?;
//!     let plan = crate::outcome_elab::plan_instantiation(substs, &checker.it)?;
//!     crate::outcome_elab::check_instantiation_params(
//!         &plan, &header.params, &header.interner, &checker.it, def.span)?;
//!     // `declare` already recorded `(name_text, def.span)` in the
//!     // Lane-19 `instantiations` table; no expression work remains.
//! }
//! ```
//!
//! Instantiation-Pflicht (driver-level, after the last definition at
//! position `end`): `missing_instantiations(program.outcome_names(end),
//! |name| program.is_instantiated(name, end))` lists outcomes no
//! `Instantiation` ever covered; the driver reports each with its
//! outcome header span.

use crate::error::{Span, TypeError, TypeErrorKind, TypeResult};
use sail_ast::*;

/// One side of a mapping clause (`mpexp_aux`): a pattern with an
/// optional `when` guard. The guard is the only expression a mapping
/// clause owns; the insertion checks it against `bool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapclDir {
    /// `mapcl` bidirectional clause: forwards pattern <= mapping `lhs`,
    /// backwards pattern <= mapping `rhs` (OCaml `check_mapcl` ~L4737).
    Bidir {
        forwards_guard: bool,
        backwards_guard: bool,
    },
    /// Scattered `Forwards` fragment: single pattern <= `lhs`.
    Forwards { guard: bool },
    /// Scattered `Backwards` fragment: single pattern <= `rhs`.
    Backwards { guard: bool },
}

/// One elaborated mapping clause in source order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapclPlan {
    pub span: Span,
    pub dir: MapclDir,
}

impl MapclPlan {
    /// Number of guard expressions this clause owns (0-2).
    #[must_use]
    pub fn guard_count(self) -> usize {
        match self.dir {
            MapclDir::Bidir {
                forwards_guard,
                backwards_guard,
            } => forwards_guard as usize + backwards_guard as usize,
            MapclDir::Forwards { guard } | MapclDir::Backwards { guard } => guard as usize,
        }
    }
}

/// Elaborated `Mapdef` body: one [`MapclPlan`] per clause, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapdefPlan {
    pub clauses: Vec<MapclPlan>,
}

impl MapdefPlan {
    /// Total guard expressions across all clauses.
    #[must_use]
    pub fn guard_count(&self) -> usize {
        self.clauses.iter().copied().map(MapclPlan::guard_count).sum()
    }
}

fn quant_error(span: Span, message: impl Into<String>) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::QuantError(message.into()),
    }
}

fn unsupported(span: Span, message: impl Into<String>) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::Unsupported(message.into()),
    }
}

fn mpexp_guard(mpexp: &MPExp) -> bool {
    matches!(mpexp.kind, MPExpKind::When { .. })
}

fn plan_mpexp_dir(kind: &MapclKind, span: Span, single_sided_ok: bool) -> TypeResult<MapclDir> {
    match kind {
        MapclKind::Bidir {
            forwards,
            backwards,
        } => Ok(MapclDir::Bidir {
            forwards_guard: mpexp_guard(forwards),
            backwards_guard: mpexp_guard(backwards),
        }),
        MapclKind::Forwards(pexp) => {
            if !single_sided_ok {
                return Err(quant_error(
                    span,
                    "single-sided forwards clause in a mapping definition; use a scattered mapcl",
                ));
            }
            Ok(MapclDir::Forwards {
                guard: matches!(pexp.kind, PExpKind::PatWhen { .. }),
            })
        }
        MapclKind::Backwards(pexp) => {
            if !single_sided_ok {
                return Err(quant_error(
                    span,
                    "single-sided backwards clause in a mapping definition; use a scattered mapcl",
                ));
            }
            Ok(MapclDir::Backwards {
                guard: matches!(pexp.kind, PExpKind::PatWhen { .. }),
            })
        }
    }
}

/// Elaborate a `Mapdef` body against its header shape.
///
/// Rejects a missing annotation with `Unsupported` (signature
/// synthesis from clauses needs pattern elaboration, mirroring
/// `declare`) and a non-`Bidir` annotation or any single-sided
/// clause with `QuantError`. Every returned clause is bidirectional.
pub fn plan_mapdef(annot: &TAnnotOpt, clauses: &[Mapcl]) -> TypeResult<MapdefPlan> {
    let TAnnotOpt::Some { typ, .. } = annot else {
        return Err(unsupported(
            Span::DUMMY,
            "mapping signature synthesis from clauses requires pattern elaboration",
        ));
    };
    if !matches!(typ.kind, TypKind::Bidir { .. }) {
        return Err(quant_error(
            typ.span,
            "mapping annotation must have a bi-directional mapping type",
        ));
    }
    let mut plans = Vec::with_capacity(clauses.len());
    for clause in clauses {
        plans.push(MapclPlan {
            span: clause.span,
            dir: plan_mpexp_dir(&clause.kind, clause.span, false)?,
        });
    }
    Ok(MapdefPlan { clauses: plans })
}

/// Elaborate one scattered `Mapcl` clause. Unlike [`plan_mapdef`],
/// single-sided `Forwards` / `Backwards` fragments are legal here;
/// the open-block gate stays in the insertion (Lane-19
/// `Program::scattered` view).
pub fn plan_scattered_mapcl(clause: &Mapcl) -> TypeResult<MapclPlan> {
    Ok(MapclPlan {
        span: clause.span,
        dir: plan_mpexp_dir(&clause.kind, clause.span, true)?,
    })
}

/// One inner `val` of an outcome body (OCaml outcome-local
/// environment entry, Lane-19 `outcome_vals` table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutcomeLocalVal {
    pub name: String,
    pub span: Span,
}

/// One inner function of an outcome body. `has_annot` flags an
/// inline annotation the insertion must reconcile with the
/// outcome-local scheme; `clause_count` preserves source order
/// width for the per-funcl check loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutcomeLocalFun {
    pub name: String,
    pub span: Span,
    pub has_annot: bool,
    pub clause_count: usize,
}

/// Elaborated outcome body (OCaml `check_outcome` ~L5379): inner
/// value headers plus the functions checked against them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutcomePlan {
    pub vals: Vec<OutcomeLocalVal>,
    pub funs: Vec<OutcomeLocalFun>,
    /// Inner `Constraint` definitions (checked by the body pass like
    /// top-level constraints).
    pub constraints: usize,
}

fn resolve(it: &impl Interner, symbol: Symbol, span: Span) -> TypeResult<String> {
    it.resolve(symbol)
        .map(str::to_owned)
        .ok_or_else(|| TypeError::unbound_var(span, format!("symbol {symbol}")))
}

/// Elaborate an outcome body: collect inner `val` headers and the
/// `fundef`s checked against them.
///
/// - inner `Constraint` defs are counted for the body pass;
/// - every inner fundef name must have a matching inner `val`
///   (`UnboundVar` otherwise);
/// - duplicate inner `val` names are `QuantError`;
/// - any other inner definition (nested type/mapping/outcome/...) is
///   `Unsupported`: outcome bodies only elaborate values here.
pub fn plan_outcome(defs: &[Def], it: &impl Interner) -> TypeResult<OutcomePlan> {
    let mut vals: Vec<OutcomeLocalVal> = Vec::new();
    let mut funs: Vec<OutcomeLocalFun> = Vec::new();
    let mut constraints = 0usize;
    for def in defs {
        match &def.kind {
            DefKind::Val(val) => {
                let name = resolve(it, val.name.symbol, val.name.span)?;
                if vals.iter().any(|v| v.name == name) {
                    return Err(quant_error(
                        val.name.span,
                        format!("duplicate local val '{name}' in outcome"),
                    ));
                }
                vals.push(OutcomeLocalVal {
                    name,
                    span: val.span,
                });
            }
            DefKind::Fundef { annot, funcls, .. } => {
                let Some(first) = funcls.first() else {
                    return Err(quant_error(def.span, "function has no clauses"));
                };
                let name = resolve(it, first.name.symbol, first.span)?;
                for clause in funcls.iter().skip(1) {
                    if resolve(it, clause.name.symbol, clause.span)? != name {
                        return Err(quant_error(
                            clause.span,
                            "all clauses in a function definition must have the same name",
                        ));
                    }
                }
                if !vals.iter().any(|v| v.name == name) {
                    return Err(TypeError::unbound_var(
                        first.span,
                        format!("function '{name}' in outcome has no local val specification"),
                    ));
                }
                funs.push(OutcomeLocalFun {
                    name,
                    span: def.span,
                    has_annot: matches!(annot, TAnnotOpt::Some { .. }),
                    clause_count: funcls.len(),
                });
            }
            DefKind::Constraint(_) => constraints += 1,
            _ => {
                return Err(unsupported(
                    def.span,
                    "outcome body definition is not a val, function clause or constraint",
                ))
            }
        }
    }
    Ok(OutcomePlan {
        vals,
        funs,
        constraints,
    })
}

/// Argument kind of one `Typ` substitution in an instantiation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubstArgKind {
    Typ,
    NExp,
    Bool,
}

/// Elaborated instantiation body: type substitutions plus identifier
/// renames, in source order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstantiationPlan {
    /// `(parameter name, argument kind, span)`.
    pub types: Vec<(String, SubstArgKind, Span)>,
    /// `(from, to, span)`.
    pub renames: Vec<(String, String, Span)>,
}

/// Split an instantiation substitution list into type arguments and
/// renames, resolving every name through the instantiation def
/// interner. Duplicate type substitutions for one parameter are
/// `QuantError`.
pub fn plan_instantiation(substs: &[Subst], it: &impl Interner) -> TypeResult<InstantiationPlan> {
    let mut types = Vec::with_capacity(substs.len());
    let mut renames = Vec::new();
    for subst in substs {
        match subst {
            Subst::Typ { kid, arg } => {
                let name = resolve(it, *kid, arg.span)?;
                if types.iter().any(|(bound, _, _)| *bound == name) {
                    return Err(quant_error(
                        arg.span,
                        format!("duplicate substitution for '{name}' in instantiation"),
                    ));
                }
                let kind = match &arg.kind {
                    TypArgKind::Typ(_) => SubstArgKind::Typ,
                    TypArgKind::NExp(_) => SubstArgKind::NExp,
                    TypArgKind::Bool(_) => SubstArgKind::Bool,
                };
                types.push((name, kind, arg.span));
            }
            Subst::Id { from, to } => {
                renames.push((
                    resolve(it, from.symbol, from.span)?,
                    resolve(it, to.symbol, to.span)?,
                    from.span,
                ));
            }
        }
    }
    Ok(InstantiationPlan { types, renames })
}

/// Check an [`InstantiationPlan`] against the outcome parameter
/// telescope (`OutcomeDef.params` with `OutcomeDef.interner`):
/// every `K_type` / `K_int` / `K_bool` binder needs exactly one
/// kind-matching substitution; missing binders are `QuantError`,
/// kind mismatches are `QuantError`, substitutions for unknown
/// binders are `UnboundVar`. `QuantItem::Constraint` entries need no
/// substitution. `span` attributes whole-plan errors (missing
/// coverage) to the instantiation definition.
pub fn check_instantiation_params(
    plan: &InstantiationPlan,
    params: &[QuantItem],
    outcome_it: &impl Interner,
    inst_it: &impl Interner,
    span: Span,
) -> TypeResult<()> {
    // Binders by resolved name; duplicate telescope binders are a
    // header error the declare pass already rejected, so the first
    // entry wins here.
    let mut binders: Vec<(String, Kind)> = Vec::new();
    for param in params {
        let QuantItem::Id(binder) = param else {
            continue;
        };
        let name = outcome_it
            .resolve(binder.kid)
            .ok_or_else(|| TypeError::unbound_var(span, format!("symbol {}", binder.kid)))?;
        if !binders.iter().any(|(bound, _)| bound == name) {
            binders.push((name.to_owned(), binder.kind));
        }
    }
    for (name, kind, arg_span) in &plan.types {
        let Some((_, expected)) = binders.iter().find(|(bound, _)| bound == name) else {
            return Err(TypeError::unbound_var(
                *arg_span,
                format!("instantiation substitutes unknown outcome parameter '{name}'"),
            ));
        };
        // `K_nat` binders never appear in outcome telescopes
        // (`sail_ast::Kind` has no `Nat`), so exact kind equality is
        // the right gate; `inst_it` only serves future diagnostics.
        let _ = inst_it;
        let matches = matches!(
            (expected, kind),
            (Kind::Type, SubstArgKind::Typ)
                | (Kind::Int, SubstArgKind::NExp)
                | (Kind::Bool, SubstArgKind::Bool)
        );
        if !matches {
            return Err(quant_error(
                *arg_span,
                format!("instantiation substitutes '{name}' with a mismatched kind"),
            ));
        }
    }
    for (name, _) in &binders {
        if !plan.types.iter().any(|(bound, _, _)| bound == name) {
            return Err(quant_error(
                span,
                format!("instantiation is missing a substitution for '{name}'"),
            ));
        }
    }
    Ok(())
}

/// Instantiation-Pflicht: list outcome names with no covering
/// `Instantiation` at the queried position. The insertion feeds
/// `program.outcome_names(position)` plus a closure over
/// `program.is_instantiated(name, position)`; the driver reports
/// each returned name with its outcome header span.
#[must_use]
pub fn missing_instantiations(
    outcome_names: Vec<String>,
    is_instantiated: impl Fn(&str) -> bool,
) -> Vec<String> {
    outcome_names
        .into_iter()
        .filter(|name| !is_instantiated(name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interner() -> SimpleInterner {
        SimpleInterner::new()
    }

    fn name(it: &mut SimpleInterner, text: &str) -> Name {
        Name::dummy(it.intern(text))
    }

    fn int_typ(it: &mut SimpleInterner) -> Typ {
        Node::dummy(TypKind::Id(name(it, "int")))
    }

    fn bidir_annot(it: &mut SimpleInterner) -> TAnnotOpt {
        let lhs = int_typ(it);
        let rhs = int_typ(it);
        TAnnotOpt::Some {
            quant: Vec::new(),
            typ: Node::dummy(TypKind::Bidir {
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            }),
        }
    }

    fn wild_pat() -> Pat {
        Node::dummy(PatKind::Wild)
    }

    fn mpexp(pat: Pat, guard: Option<Exp>) -> MPExp {
        match guard {
            None => Node::dummy(MPExpKind::Pat(pat)),
            Some(guard) => Node::dummy(MPExpKind::When { pat, guard }),
        }
    }

    fn undef_exp() -> Exp {
        Node::dummy(ExpKind::Undef)
    }

    fn mapcl(kind: MapclKind) -> Mapcl {
        Mapcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind,
        }
    }

    fn bidir_clause(forwards_guard: bool, backwards_guard: bool) -> Mapcl {
        let fwd = mpexp(
            wild_pat(),
            forwards_guard.then(undef_exp),
        );
        let bwd = mpexp(
            wild_pat(),
            backwards_guard.then(undef_exp),
        );
        mapcl(MapclKind::Bidir {
            forwards: fwd,
            backwards: bwd,
        })
    }

    fn val_def(it: &mut SimpleInterner, fname: &str) -> Def {
        let arg = int_typ(it);
        let ret = int_typ(it);
        Node::dummy(DefKind::Val(ValSpec {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: name(it, fname),
            typschm: TypSchm {
                quant: Vec::new(),
                typ: Node::dummy(TypKind::Fn {
                    args: vec![arg],
                    ret: Box::new(ret),
                }),
            },
            extern_opt: None,
        }))
    }

    fn fun_def(it: &mut SimpleInterner, fname: &str, with_annot: bool) -> Def {
        let arm = PExp {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind: PExpKind::PatExp {
                pat: wild_pat(),
                body: undef_exp(),
            },
        };
        Node::dummy(DefKind::Fundef {
            rec: RecOpt::NonRec,
            annot: if with_annot {
                TAnnotOpt::Some {
                    quant: Vec::new(),
                    typ: int_typ(it),
                }
            } else {
                TAnnotOpt::None
            },
            funcls: vec![Funcl {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                name: name(it, fname),
                arm,
            }],
        })
    }

    #[test]
    fn mapdef_bidir_clauses_record_guard_sides() {
        let mut it = interner();
        let annot = bidir_annot(&mut it);
        let clauses = vec![bidir_clause(false, false), bidir_clause(true, true)];
        let plan = plan_mapdef(&annot, &clauses).unwrap();
        assert_eq!(plan.clauses.len(), 2);
        assert_eq!(
            plan.clauses[0].dir,
            MapclDir::Bidir {
                forwards_guard: false,
                backwards_guard: false,
            }
        );
        assert_eq!(
            plan.clauses[1].dir,
            MapclDir::Bidir {
                forwards_guard: true,
                backwards_guard: true,
            }
        );
        assert_eq!(plan.guard_count(), 2);
    }

    #[test]
    fn mapdef_rejects_missing_annot_as_unsupported() {
        let clauses = vec![bidir_clause(false, false)];
        let err = plan_mapdef(&TAnnotOpt::None, &clauses).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::Unsupported(_)));
    }

    #[test]
    fn mapdef_rejects_non_bidir_annot() {
        let mut it = interner();
        let typ = int_typ(&mut it);
        let annot = TAnnotOpt::Some {
            quant: Vec::new(),
            typ,
        };
        let err = plan_mapdef(&annot, &[]).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::QuantError(_)));
    }

    #[test]
    fn mapdef_rejects_single_sided_clause_but_scattered_accepts_it() {
        let mut it = interner();
        let annot = bidir_annot(&mut it);
        let arm = PExp {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind: PExpKind::PatExp {
                pat: wild_pat(),
                body: undef_exp(),
            },
        };
        let clause = mapcl(MapclKind::Forwards(arm));
        let err = plan_mapdef(&annot, std::slice::from_ref(&clause)).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::QuantError(_)));
        let single = plan_scattered_mapcl(&clause).unwrap();
        assert_eq!(single.dir, MapclDir::Forwards { guard: false });
        assert_eq!(single.guard_count(), 0);
    }

    #[test]
    fn scattered_backwards_with_guard_records_guard() {
        let arm = PExp {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind: PExpKind::PatWhen {
                pat: wild_pat(),
                guard: undef_exp(),
                body: undef_exp(),
            },
        };
        let clause = mapcl(MapclKind::Backwards(arm));
        let single = plan_scattered_mapcl(&clause).unwrap();
        assert_eq!(single.dir, MapclDir::Backwards { guard: true });
        assert_eq!(single.guard_count(), 1);
    }

    #[test]
    fn outcome_collects_vals_and_matching_fundefs() {
        let mut it = interner();
        let defs = vec![val_def(&mut it, "helper"), fun_def(&mut it, "helper", false)];
        let plan = plan_outcome(&defs, &it).unwrap();
        assert_eq!(plan.vals.len(), 1);
        assert_eq!(plan.vals[0].name, "helper");
        assert_eq!(plan.funs.len(), 1);
        assert!(!plan.funs[0].has_annot);
        assert_eq!(plan.funs[0].clause_count, 1);
        assert_eq!(plan.constraints, 0);
    }

    #[test]
    fn outcome_flags_inline_annot_for_header_reconciliation() {
        let mut it = interner();
        let defs = vec![val_def(&mut it, "f"), fun_def(&mut it, "f", true)];
        let plan = plan_outcome(&defs, &it).unwrap();
        assert!(plan.funs[0].has_annot);
    }

    #[test]
    fn outcome_rejects_fundef_without_local_val() {
        let mut it = interner();
        let defs = vec![fun_def(&mut it, "ghost", false)];
        let err = plan_outcome(&defs, &it).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::UnboundVar(_)));
    }

    #[test]
    fn outcome_rejects_duplicate_local_val() {
        let mut it = interner();
        let defs = vec![val_def(&mut it, "h"), val_def(&mut it, "h")];
        let err = plan_outcome(&defs, &it).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::QuantError(_)));
    }

    #[test]
    fn outcome_rejects_nested_non_value_def() {
        let mut it = interner();
        let nested = Node::dummy(DefKind::Mapdef {
            name: name(&mut it, "m"),
            annot: TAnnotOpt::None,
            clauses: Vec::new(),
        });
        let err = plan_outcome(&[nested], &it).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::Unsupported(_)));
    }

    #[test]
    fn outcome_counts_inner_constraints() {
        let mut it = interner();
        let constraint = Node::dummy(DefKind::Constraint(Node::dummy(ConstraintKind::True)));
        let defs = vec![val_def(&mut it, "h"), constraint];
        let plan = plan_outcome(&defs, &it).unwrap();
        assert_eq!(plan.constraints, 1);
    }

    fn outcome_telescope(it: &mut SimpleInterner) -> (Vec<QuantItem>, SimpleInterner) {
        let t = it.intern("T");
        let n = it.intern("n");
        let b = it.intern("b");
        (
            vec![
                QuantItem::Id(KOpt {
                    kind: Kind::Type,
                    kid: t,
                }),
                QuantItem::Id(KOpt {
                    kind: Kind::Int,
                    kid: n,
                }),
                QuantItem::Id(KOpt {
                    kind: Kind::Bool,
                    kid: b,
                }),
            ],
            SimpleInterner::new(),
        )
    }

    fn typ_subst(it: &mut SimpleInterner, kid: &str, arg: TypArgKind) -> Subst {
        Subst::Typ {
            kid: it.intern(kid),
            arg: Node::dummy(arg),
        }
    }

    #[test]
    fn instantiation_covers_telescope_with_matching_kinds() {
        let mut outcome_it = interner();
        let (params, _) = outcome_telescope(&mut outcome_it);
        let mut inst_it = interner();
        let helper = name(&mut inst_it, "int");
        let substs = vec![
            typ_subst(
                &mut inst_it,
                "T",
                TypArgKind::Typ(Node::dummy(TypKind::Id(helper))),
            ),
            typ_subst(
                &mut inst_it,
                "n",
                TypArgKind::NExp(Node::dummy(NExpKind::Constant("8".to_owned()))),
            ),
            typ_subst(
                &mut inst_it,
                "b",
                TypArgKind::Bool(Box::new(Node::dummy(ConstraintKind::True))),
            ),
        ];
        let plan = plan_instantiation(&substs, &inst_it).unwrap();
        assert_eq!(plan.types.len(), 3);
        assert!(plan.renames.is_empty());
        check_instantiation_params(&plan, &params, &outcome_it, &inst_it, Span::DUMMY).unwrap();
    }

    #[test]
    fn instantiation_rejects_missing_substitution() {
        let mut outcome_it = interner();
        let (params, _) = outcome_telescope(&mut outcome_it);
        let mut inst_it = interner();
        let helper = name(&mut inst_it, "int");
        let substs = vec![typ_subst(
            &mut inst_it,
            "T",
            TypArgKind::Typ(Node::dummy(TypKind::Id(helper))),
        )];
        let plan = plan_instantiation(&substs, &inst_it).unwrap();
        let err =
            check_instantiation_params(&plan, &params, &outcome_it, &inst_it, Span::DUMMY)
                .unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::QuantError(_)));
    }

    #[test]
    fn instantiation_rejects_kind_mismatch() {
        let mut outcome_it = interner();
        let (params, _) = outcome_telescope(&mut outcome_it);
        let mut inst_it = interner();
        let helper = name(&mut inst_it, "int");
        let substs = vec![
            typ_subst(
                &mut inst_it,
                "T",
                TypArgKind::NExp(Node::dummy(NExpKind::Constant("8".to_owned()))),
            ),
            typ_subst(
                &mut inst_it,
                "n",
                TypArgKind::NExp(Node::dummy(NExpKind::Constant("8".to_owned()))),
            ),
            typ_subst(
                &mut inst_it,
                "b",
                TypArgKind::Bool(Box::new(Node::dummy(ConstraintKind::True))),
            ),
        ];
        let plan = plan_instantiation(&substs, &inst_it).unwrap();
        let err =
            check_instantiation_params(&plan, &params, &outcome_it, &inst_it, Span::DUMMY)
                .unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::QuantError(_)));
        let _ = helper;
    }

    #[test]
    fn instantiation_rejects_unknown_parameter() {
        let mut outcome_it = interner();
        let (params, _) = outcome_telescope(&mut outcome_it);
        let mut inst_it = interner();
        let substs = vec![typ_subst(
            &mut inst_it,
            "ghost",
            TypArgKind::NExp(Node::dummy(NExpKind::Constant("1".to_owned()))),
        )];
        let plan = plan_instantiation(&substs, &inst_it).unwrap();
        let err =
            check_instantiation_params(&plan, &params, &outcome_it, &inst_it, Span::DUMMY)
                .unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::UnboundVar(_)));
    }

    #[test]
    fn instantiation_rejects_duplicate_substitution() {
        let mut inst_it = interner();
        let substs = vec![
            typ_subst(
                &mut inst_it,
                "n",
                TypArgKind::NExp(Node::dummy(NExpKind::Constant("1".to_owned()))),
            ),
            typ_subst(
                &mut inst_it,
                "n",
                TypArgKind::NExp(Node::dummy(NExpKind::Constant("2".to_owned()))),
            ),
        ];
        let err = plan_instantiation(&substs, &inst_it).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::QuantError(_)));
    }

    #[test]
    fn instantiation_keeps_id_renames() {
        let mut inst_it = interner();
        let from = name(&mut inst_it, "a");
        let to = name(&mut inst_it, "b");
        let substs = vec![Subst::Id { from, to }];
        let plan = plan_instantiation(&substs, &inst_it).unwrap();
        assert!(plan.types.is_empty());
        assert_eq!(plan.renames.len(), 1);
        assert_eq!(plan.renames[0].0, "a");
        assert_eq!(plan.renames[0].1, "b");
    }

    #[test]
    fn missing_instantiations_reports_only_gaps() {
        let missing = missing_instantiations(
            vec!["A".to_owned(), "B".to_owned(), "C".to_owned()],
            |name| name == "B",
        );
        assert_eq!(missing, vec!["A".to_owned(), "C".to_owned()]);
        let none_missing =
            missing_instantiations(vec!["A".to_owned()], |_| true);
        assert!(none_missing.is_empty());
    }
}
