//! `remove_e_assign`: loop-spec preamble + reference-type stripping.
//!
//! OCaml source: `sail/src/lib/rewrites.ml`, `rewrite_ast_remove_e_assign`
//! (loop-spec `gen_vs` preamble, then `remove_reference_types` composed with
//! `rewrite_var_updates`).
//!
//! ## Insert-Spec (Main-Agent verdrahtet; diese Datei bleibt unberuehrt)
//! - Pass-Name: `remove_e_assign`
//! - `LEAN_REWRITES`-Index (0-basiert): 41 (siehe `pipeline.rs`)
//! - Struct-Name: `RemoveEAssign`
//! - Wiring: `passes/mod.rs` um `pub mod remove_e_assign;` und
//!   `pub use remove_e_assign::RemoveEAssign;` erweitern; `impl TypedPass`
//!   delegiert an [`RemoveEAssign::run_typed`]; `pipeline.rs`
//!   (`is_typed_implemented_pass`, `make_typed_implemented`) um
//!   `"remove_e_assign"` erweitern. Legacy-`Pass` (String-`Shard`):
//!   bewusst Pass-Through — das String-Modell kennt weder Schleifen noch
//!   Typen; die echte Logik ist [`RemoveEAssign::run_typed`].
//!
//! ## Echte Teilmenge (ohne Typ-/Effekt-Env, getestet)
//! - Loop-Spec-Praeambel: `foreach#`-, `while#`-, `until#`-, `while#t`- und
//!   `until#t`-Valspecs werden vorangestellt (duplikatgeschuetzt,
//!   idempotent). Die polymorphen Schemata werden strukturell aufgebaut
//!   (`TypKind::Fn` plus `QuantItem::Id`-Binder `vars_in`/`vars_out : Type`,
//!   Kind-Variablen ohne Quote wie in `typquant_to_ast`); `gen_vs ~pure:true`
//!   hat im untypisierten AST keinen Traeger und entfaellt.
//! - `remove_reference_types`: `reg(t)` wird in allen Typpositionen zu `t`
//!   (exakt die OCaml-Regel, rekursiv inkl. `reg(reg(t))`).
//!
//! ## Dokumentierte Luecke (braucht Typ-Env, kein Raten)
//! `rewrite_var_updates` (Schleifen-/Assign-Threading ueber `foreach#` /
//! `while#`, mit `infer_exp`, `add_e_typ`, `find_updated_vars` und
//! Effektanalyse) ist nicht implementiert: ohne Typ- und Effektannotationen
//! liesse sich nicht entscheiden, welche Variablen wo aktualisiert werden.
//! `E_for` / `E_loop` / `E_assign` werden traversiert, aber semantisch
//! unveraendert gelassen. Wer das Threading ohne Env "implementiert",
//! aendert Register- und Effektsemantik.

#![forbid(unsafe_code)]

use sail_ast::{
    Ast, Constraint, ConstraintKind, Def, DefKind, Exp, ExpKind, Funcl, Interner, KOpt, Kind,
    LExp, LExpKind, Mapcl, MapclKind, MPExp, MPExpKind, Name, NExp, NExpKind, Node, NodeId,
    PExp, PExpKind, Pat, PatKind, QuantItem, RecOpt, ScatteredDef, Span, TAnnotOpt, Typ, TypArg,
    TypArgKind, TypKind, TypSchm, TypeDefKind, ValSpec,
};

/// `remove_e_assign` rewrite (ehrliche Teilmenge, siehe Modul-Doku).
#[derive(Clone, Copy, Debug, Default)]
pub struct RemoveEAssign;

impl crate::pass::TypedPass for RemoveEAssign {
    fn name(&self) -> &'static str { RemoveEAssign::name(self) }
    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        RemoveEAssign::run_typed(self, ast, interner)
    }
}

impl RemoveEAssign {
    /// OCaml rewrite name; `LEAN_REWRITES[41]`.
    pub const NAME: &'static str = "remove_e_assign";

    /// Pass name, matching the OCaml rewrite name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    /// Run the env-free subset over the canonical AST.
    pub fn run_typed(&self, ast: &mut Ast, interner: &mut dyn Interner) {
        ensure_loop_specs(ast, interner);
        strip_reference_types(ast, interner);
    }
}

// ---------------------------------------------------------------------------
// Loop-spec preamble (`gen_vs ~pure:true [...]`, `loop_specs @ ast.defs`)
// ---------------------------------------------------------------------------

/// Argument descriptors: `Int` = `int`, `Bool` = `bool`, `In` = `vars_in`,
/// `Out` = `vars_out` (return is always `Out`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Base {
    Int,
    Bool,
    In,
    Out,
}

const LOOP_SPECS: [(&str, &[Base]); 5] = [
    (
        "foreach#",
        &[Base::Int, Base::Int, Base::Int, Base::Bool, Base::In, Base::Out],
    ),
    ("while#", &[Base::Bool, Base::In, Base::Out]),
    ("until#", &[Base::Bool, Base::In, Base::Out]),
    ("while#t", &[Base::Bool, Base::In, Base::Out, Base::Int]),
    ("until#t", &[Base::Bool, Base::In, Base::Out, Base::Int]),
];

/// Prepend the missing loop-combinator valspecs, in OCaml order.
///
/// Existing valspecs (by resolved name text) are kept; only missing ones are
/// inserted, so the pass is idempotent.
fn ensure_loop_specs(ast: &mut Ast, interner: &mut dyn Interner) {
    let mut present = std::collections::BTreeSet::new();
    for def in &ast.defs {
        if let DefKind::Val(vs) = &def.kind {
            if let Some(text) = interner.resolve(vs.name.symbol) {
                present.insert(text.to_owned());
            }
        }
    }
    let mut missing = Vec::new();
    for (name, args) in LOOP_SPECS {
        if !present.contains(name) {
            missing.push(loop_spec_def(interner, name, args));
        }
    }
    if !missing.is_empty() {
        ast.defs.splice(0..0, missing);
    }
}

fn loop_spec_def(interner: &mut dyn Interner, name: &str, args: &[Base]) -> Def {
    let vin = interner.intern("vars_in");
    let vout = interner.intern("vars_out");
    let int = Name::dummy(interner.intern("int"));
    let boolean = Name::dummy(interner.intern("bool"));
    let mut arg_typs = Vec::with_capacity(args.len());
    for base in args {
        let kind = match base {
            Base::Int => TypKind::Id(int.clone()),
            Base::Bool => TypKind::Id(boolean.clone()),
            Base::In => TypKind::Var(vin),
            Base::Out => TypKind::Var(vout),
        };
        arg_typs.push(Node::dummy(kind));
    }
    let ret = Node::dummy(TypKind::Var(vout));
    let typ = Node::dummy(TypKind::Fn {
        args: arg_typs,
        ret: Box::new(ret),
    });
    let quant = vec![
        QuantItem::Id(KOpt {
            kind: Kind::Type,
            kid: vin,
        }),
        QuantItem::Id(KOpt {
            kind: Kind::Type,
            kid: vout,
        }),
    ];
    Node::dummy(DefKind::Val(ValSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: Name::dummy(interner.intern(name)),
        typschm: TypSchm { quant, typ },
        extern_opt: None,
    }))
}

// ---------------------------------------------------------------------------
// `remove_reference_types`: `reg(t)` -> `t`
// ---------------------------------------------------------------------------

/// Strip `reg(..)` wrappers from every type annotation in `ast`.
fn strip_reference_types(ast: &mut Ast, interner: &dyn Interner) {
    for def in &mut ast.defs {
        strip_def(def, interner);
    }
}

fn strip_typ(typ: &mut Typ, interner: &dyn Interner) {
    match &mut typ.kind {
        TypKind::Unknown | TypKind::Id(_) | TypKind::Var(_) => {}
        TypKind::Fn { args, ret } => {
            for arg in args.iter_mut() {
                strip_typ(arg, interner);
            }
            strip_typ(ret, interner);
        }
        TypKind::Bidir { lhs, rhs } => {
            strip_typ(lhs, interner);
            strip_typ(rhs, interner);
        }
        TypKind::Tuple(items) => {
            for item in items.iter_mut() {
                strip_typ(item, interner);
            }
        }
        TypKind::App { args, .. } => {
            for arg in args.iter_mut() {
                strip_typarg(arg, interner);
            }
        }
        TypKind::Exist { typ, .. } => strip_typ(typ, interner),
    }
    // Unwrap `reg(t)` after the children are stripped, so `reg(reg(t))`
    // collapses in one walk.
    if let TypKind::App { name, args } = &mut typ.kind {
        if args.len() == 1 && interner.resolve(name.symbol) == Some("reg") {
            let arg = args.pop().expect("checked len 1");
            if let TypArgKind::Typ(inner) = arg.kind {
                typ.kind = inner.kind;
            } else {
                // Malformed `reg` application (non-type argument): restore.
                let TypKind::App { args, .. } = &mut typ.kind else {
                    unreachable!("still App after pop");
                };
                args.push(arg);
            }
        }
    }
}

fn strip_typarg(arg: &mut TypArg, interner: &dyn Interner) {
    match &mut arg.kind {
        TypArgKind::NExp(nexp) => strip_nexp(nexp, interner),
        TypArgKind::Typ(typ) => strip_typ(typ, interner),
        TypArgKind::Bool(constraint) => strip_constraint(constraint, interner),
    }
}

fn strip_nexp(nexp: &mut NExp, interner: &dyn Interner) {
    match &mut nexp.kind {
        NExpKind::Id(_) | NExpKind::Var(_) | NExpKind::Constant(_) => {}
        NExpKind::App { args, .. } => {
            for arg in args.iter_mut() {
                strip_nexp(arg, interner);
            }
        }
        NExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            strip_constraint(cond, interner);
            strip_nexp(then_branch, interner);
            strip_nexp(else_branch, interner);
        }
        NExpKind::Times(a, b) | NExpKind::Sum(a, b) | NExpKind::Minus(a, b) => {
            strip_nexp(a, interner);
            strip_nexp(b, interner);
        }
        NExpKind::Exp(a) | NExpKind::Neg(a) => strip_nexp(a, interner),
    }
}

fn strip_constraint(constraint: &mut Constraint, interner: &dyn Interner) {
    match &mut constraint.kind {
        ConstraintKind::Equal(a, b) | ConstraintKind::NotEqual(a, b) => {
            strip_typarg(a, interner);
            strip_typarg(b, interner);
        }
        ConstraintKind::Ge(a, b)
        | ConstraintKind::Gt(a, b)
        | ConstraintKind::Le(a, b)
        | ConstraintKind::Lt(a, b) => {
            strip_nexp(a, interner);
            strip_nexp(b, interner);
        }
        ConstraintKind::Set { nexp, .. } => strip_nexp(nexp, interner),
        ConstraintKind::And(a, b) | ConstraintKind::Or(a, b) => {
            strip_constraint(a, interner);
            strip_constraint(b, interner);
        }
        ConstraintKind::Not(a) => strip_constraint(a, interner),
        ConstraintKind::App { args, .. } => {
            for arg in args.iter_mut() {
                strip_typarg(arg, interner);
            }
        }
        ConstraintKind::Id(_)
        | ConstraintKind::Var(_)
        | ConstraintKind::True
        | ConstraintKind::False => {}
    }
}

fn strip_quant_item(item: &mut QuantItem, interner: &dyn Interner) {
    if let QuantItem::Constraint(constraint) = item {
        strip_constraint(constraint, interner);
    }
}

fn strip_typschm(schm: &mut TypSchm, interner: &dyn Interner) {
    for item in schm.quant.iter_mut() {
        strip_quant_item(item, interner);
    }
    strip_typ(&mut schm.typ, interner);
}

fn strip_tannot(annot: &mut TAnnotOpt, interner: &dyn Interner) {
    if let TAnnotOpt::Some { quant, typ } = annot {
        for item in quant.iter_mut() {
            strip_quant_item(item, interner);
        }
        strip_typ(typ, interner);
    }
}

fn strip_pat(pat: &mut Pat, interner: &dyn Interner) {
    match &mut pat.kind {
        PatKind::Lit(_) | PatKind::Wild | PatKind::Id(_) | PatKind::VectorSubrange { .. } => {}
        PatKind::Or(a, b) | PatKind::Cons(a, b) => {
            strip_pat(a, interner);
            strip_pat(b, interner);
        }
        PatKind::Not(p) => strip_pat(p, interner),
        PatKind::As { pat: p, .. } => strip_pat(p, interner),
        PatKind::Typ { typ, pat: p } => {
            strip_typ(typ, interner);
            strip_pat(p, interner);
        }
        PatKind::Var { pat: p, .. } => strip_pat(p, interner),
        PatKind::App { args, .. } => {
            for arg in args.iter_mut() {
                strip_pat(arg, interner);
            }
        }
        PatKind::Vector(pats)
        | PatKind::VectorConcat(pats)
        | PatKind::Tuple(pats)
        | PatKind::List(pats)
        | PatKind::StringAppend(pats) => {
            for p in pats.iter_mut() {
                strip_pat(p, interner);
            }
        }
        PatKind::Struct { fields, .. } => {
            for (_, p) in fields.iter_mut() {
                strip_pat(p, interner);
            }
        }
    }
}

fn strip_lexp(lexp: &mut LExp, interner: &dyn Interner) {
    match &mut lexp.kind {
        LExpKind::Id(_) => {}
        LExpKind::Deref(exp) => strip_exp(exp, interner),
        LExpKind::Typ { typ, .. } => strip_typ(typ, interner),
        LExpKind::Tuple(parts) | LExpKind::VectorConcat(parts) => {
            for part in parts.iter_mut() {
                strip_lexp(part, interner);
            }
        }
        LExpKind::Vector { lexp, index } => {
            strip_lexp(lexp, interner);
            strip_exp(index, interner);
        }
        LExpKind::VectorRange { lexp, lo, hi } => {
            strip_lexp(lexp, interner);
            strip_exp(lo, interner);
            strip_exp(hi, interner);
        }
        LExpKind::Field { lexp, .. } => strip_lexp(lexp, interner),
    }
}

fn strip_pexp(pexp: &mut PExp, interner: &dyn Interner) {
    match &mut pexp.kind {
        PExpKind::PatExp { pat, body } => {
            strip_pat(pat, interner);
            strip_exp(body, interner);
        }
        PExpKind::PatWhen { pat, guard, body } => {
            strip_pat(pat, interner);
            strip_exp(guard, interner);
            strip_exp(body, interner);
        }
    }
}

fn strip_mpexp(mpexp: &mut MPExp, interner: &dyn Interner) {
    match &mut mpexp.kind {
        MPExpKind::Pat(pat) => strip_pat(pat, interner),
        MPExpKind::When { pat, guard } => {
            strip_pat(pat, interner);
            strip_exp(guard, interner);
        }
    }
}

fn strip_mapcl(mapcl: &mut Mapcl, interner: &dyn Interner) {
    match &mut mapcl.kind {
        MapclKind::Bidir {
            forwards,
            backwards,
        } => {
            strip_mpexp(forwards, interner);
            strip_mpexp(backwards, interner);
        }
        MapclKind::Forwards(pexp) | MapclKind::Backwards(pexp) => {
            strip_pexp(pexp, interner);
        }
    }
}

fn strip_funcl(funcl: &mut Funcl, interner: &dyn Interner) {
    strip_pexp(&mut funcl.arm, interner);
}

#[allow(clippy::too_many_lines)]
fn strip_exp(exp: &mut Exp, interner: &dyn Interner) {
    match &mut exp.kind {
        ExpKind::Id(_)
        | ExpKind::Lit(_)
        | ExpKind::Ref(_)
        | ExpKind::Config(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_) => {}
        ExpKind::Typ { typ, exp: inner } => {
            strip_typ(typ, interner);
            strip_exp(inner, interner);
        }
        ExpKind::App { args, .. } => {
            for arg in args.iter_mut() {
                strip_exp(arg, interner);
            }
        }
        ExpKind::Block(exprs)
        | ExpKind::Tuple(exprs)
        | ExpKind::Vector(exprs)
        | ExpKind::List(exprs) => {
            for e in exprs.iter_mut() {
                strip_exp(e, interner);
            }
        }
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            strip_exp(cond, interner);
            strip_exp(then_branch, interner);
            strip_exp(else_branch, interner);
        }
        ExpKind::Loop {
            measure, cond, body, ..
        } => {
            if let Some(measure) = measure {
                strip_exp(measure, interner);
            }
            strip_exp(cond, interner);
            strip_exp(body, interner);
        }
        ExpKind::For {
            from, to, step, body, ..
        } => {
            strip_exp(from, interner);
            strip_exp(to, interner);
            strip_exp(step, interner);
            strip_exp(body, interner);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            strip_exp(a, interner);
            strip_exp(b, interner);
        }
        ExpKind::Struct { fields, .. } => {
            for field in fields.iter_mut() {
                strip_exp(&mut field.value, interner);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            strip_exp(record, interner);
            for field in fields.iter_mut() {
                strip_exp(&mut field.value, interner);
            }
        }
        ExpKind::Field { record, .. } => strip_exp(record, interner),
        ExpKind::Match { scrutinee, arms } => {
            strip_exp(scrutinee, interner);
            for arm in arms.iter_mut() {
                strip_pexp(arm, interner);
            }
        }
        ExpKind::Let { pat, bind, body } => {
            strip_pat(pat, interner);
            strip_exp(bind, interner);
            strip_exp(body, interner);
        }
        ExpKind::Assign { lexp, value } => {
            strip_lexp(lexp, interner);
            strip_exp(value, interner);
        }
        ExpKind::Sizeof(nexp) => strip_nexp(nexp, interner),
        ExpKind::Return(e)
        | ExpKind::Exit(e)
        | ExpKind::Throw(e)
        | ExpKind::InternalReturn(e) => strip_exp(e, interner),
        ExpKind::Try { exp: e, arms } => {
            strip_exp(e, interner);
            for arm in arms.iter_mut() {
                strip_pexp(arm, interner);
            }
        }
        ExpKind::Assert { cond, msg } => {
            strip_exp(cond, interner);
            strip_exp(msg, interner);
        }
        ExpKind::Var { lexp, init, body } => {
            strip_lexp(lexp, interner);
            strip_exp(init, interner);
            strip_exp(body, interner);
        }
        ExpKind::InternalPlet { pat, bind, body } => {
            strip_pat(pat, interner);
            strip_exp(bind, interner);
            strip_exp(body, interner);
        }
        ExpKind::InternalAssume { constraint, exp: e } => {
            strip_constraint(constraint, interner);
            strip_exp(e, interner);
        }
        ExpKind::Constraint(constraint) => strip_constraint(constraint, interner),
    }
}

fn strip_def(def: &mut Def, interner: &dyn Interner) {
    match &mut def.kind {
        DefKind::Type(typedef) => match &mut typedef.kind {
            TypeDefKind::Abbrev { quant, arg, .. } => {
                for item in quant.iter_mut() {
                    strip_quant_item(item, interner);
                }
                strip_typarg(arg, interner);
            }
            TypeDefKind::Record { quant, fields, .. } => {
                for item in quant.iter_mut() {
                    strip_quant_item(item, interner);
                }
                for (_, typ) in fields.iter_mut() {
                    strip_typ(typ, interner);
                }
            }
            TypeDefKind::Variant { quant, ctors, .. } => {
                for item in quant.iter_mut() {
                    strip_quant_item(item, interner);
                }
                for ctor in ctors.iter_mut() {
                    strip_typ(&mut ctor.typ, interner);
                }
            }
            TypeDefKind::Enum { .. } | TypeDefKind::Abstract { .. } => {}
            TypeDefKind::Bitfield { typ, fields, .. } => {
                strip_typ(typ, interner);
                for (_, range) in fields.iter_mut() {
                    strip_index_range(range, interner);
                }
            }
        },
        DefKind::Constraint(constraint) => strip_constraint(constraint, interner),
        DefKind::Fundef { rec, annot, funcls } => {
            strip_tannot(annot, interner);
            if let RecOpt::Measure { pat, exp } = rec {
                strip_pat(pat, interner);
                strip_exp(exp, interner);
            }
            for funcl in funcls.iter_mut() {
                strip_funcl(funcl, interner);
            }
        }
        DefKind::Mapdef { annot, clauses, .. } => {
            strip_tannot(annot, interner);
            for clause in clauses.iter_mut() {
                strip_mapcl(clause, interner);
            }
        }
        DefKind::Impl(funcl) => strip_funcl(funcl, interner),
        DefKind::Let { pat, exp } => {
            strip_pat(pat, interner);
            strip_exp(exp, interner);
        }
        DefKind::Val(vs) => strip_typschm(&mut vs.typschm, interner),
        DefKind::Outcome {
            typschm, quant, defs, ..
        } => {
            strip_typschm(typschm, interner);
            for item in quant.iter_mut() {
                strip_quant_item(item, interner);
            }
            for def in defs.iter_mut() {
                strip_def(def, interner);
            }
        }
        DefKind::Instantiation { substs, .. } => {
            for subst in substs.iter_mut() {
                if let sail_ast::Subst::Typ { arg, .. } = subst {
                    strip_typarg(arg, interner);
                }
            }
        }
        DefKind::Fixity { .. } | DefKind::Overload { .. } | DefKind::Default(_) => {}
        DefKind::Scattered(scattered) => match scattered {
            ScatteredDef::Function { annot, .. } | ScatteredDef::Mapping { annot, .. } => {
                strip_tannot(annot, interner);
            }
            ScatteredDef::Funcl(funcl) => strip_funcl(funcl, interner),
            ScatteredDef::Variant { quant, .. } => {
                for item in quant.iter_mut() {
                    strip_quant_item(item, interner);
                }
            }
            ScatteredDef::Unioncl { union, .. } => strip_typ(&mut union.typ, interner),
            ScatteredDef::Mapcl { clause, .. } => strip_mapcl(clause, interner),
            ScatteredDef::Enum(_)
            | ScatteredDef::Enumcl { .. }
            | ScatteredDef::End(_) => {}
        },
        DefKind::Measure { pat, exp, .. } => {
            strip_pat(pat, interner);
            strip_exp(exp, interner);
        }
        DefKind::LoopMeasures { measures, .. } => {
            for (_, exp) in measures.iter_mut() {
                strip_exp(exp, interner);
            }
        }
        DefKind::Register(spec) => {
            strip_typ(&mut spec.typ, interner);
            if let Some(init) = spec.init.as_mut() {
                strip_exp(init, interner);
            }
        }
        DefKind::InternalMutrec(sets) => {
            for set in sets.iter_mut() {
                for funcl in set.funcls.iter_mut() {
                    strip_funcl(funcl, interner);
                }
            }
        }
        DefKind::Pragma { .. } => {}
    }
}

fn strip_index_range(range: &mut sail_ast::IndexRange, interner: &dyn Interner) {
    match &mut range.kind {
        sail_ast::IndexRangeKind::Single(nexp) => strip_nexp(nexp, interner),
        sail_ast::IndexRangeKind::Range { lo, hi } => {
            strip_nexp(lo, interner);
            strip_nexp(hi, interner);
        }
        sail_ast::IndexRangeKind::Concat(a, b) => {
            strip_index_range(a, interner);
            strip_index_range(b, interner);
        }
    }
}
