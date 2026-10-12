//! `internal_lets`: early-return-driven `E_let`/`E_var` classification.
//!
//! OCaml source: `sail/src/lib/rewrites.ml`, `rewrite_ast_internal_lets`
//! (`e_let` / `e_var` over `id_exp_alg`).
//!
//! ## Insert-Spec (Main-Agent verdrahtet; diese Datei bleibt unberuehrt)
//! - Pass-Name: `internal_lets`
//! - `LEAN_REWRITES`-Index (0-basiert): 43 (siehe `pipeline.rs`)
//! - Struct-Name: `InternalLets`
//! - Wiring: `passes/mod.rs` um `pub mod internal_lets;` und
//!   `pub use internal_lets::InternalLets;` erweitern; `impl TypedPass`
//!   delegiert an [`InternalLets::run_typed`]; `pipeline.rs`
//!   (`is_typed_implemented_pass`, `make_typed_implemented`) um
//!   `"internal_lets"` erweitern. Legacy-`Pass` (String-`Shard`):
//!   bewusst Pass-Through — das String-Modell kennt weder `let`/`var` noch
//!   `InternalPlet`; die echte Logik ist [`InternalLets::run_typed`].
//!
//! ## Echte Teilmenge (ohne Typ-/Effekt-Env, getestet)
//! - `E_let(pat, bind, body)` mit fruehem Return in `bind` wird zu
//!   `E_internal_plet(pat, bind, body)` (exakt der OCaml-Zweig
//!   `has_early_return bind`; Bottom-up-Rekursion wie `fold_exp`).
//! - `E_var(lexp, init, body)` mit fruehem Return in `init` wird zu
//!   `E_internal_plet` (Lexp-zu-Pat nur fuer `LE_id` / `LE_typ` /
//!   `LE_tuple`, sonst erhalten).
//! - `has_early_return` ist rein syntaktisch wie in `rewriter.ml`
//!   (`E_return` irgendwo oder `E_app("early_return", _)` irgendwo);
//!   nicht aufloesbare Call-Ziele zaehlen konservativ als "kein
//!   Early-Return".
//!
//! ## Dokumentierte Luecke (braucht Typ-/Effekt-Env, kein Raten)
//! - Der `effectful bind`-Disjunkt (beide Regeln) ist nicht implementiert:
//!   ohne Effektannotationen ist Unreinheit nicht beweisbar; ein `E_var`
//!   mit reiner Initialisierung wuerde OCaml zu `E_let`, bleibt hier aber
//!   bewusst `E_var` (Erhalt statt unsauberer Umwandlung).
//! - Die `E_let(Wild, E_assign(lokales lexp))`-Regel braucht
//!   `lexp_is_local` (Env), `lexp_is_effectful` und `infer_lexp` und bleibt
//!   vollstaendig erhalten.
//! - `E_var` mit Nicht-Variablen-Lexp (`LE_vector`, `LE_field`, ...)
//!   scheitert in OCaml mit `failwith`; hier bleibt der Knoten erhalten.

#![forbid(unsafe_code)]

use sail_ast::{
    Ast, Def, DefKind, Exp, ExpKind, Funcl, Interner, LExp, LExpKind, Mapcl, MapclKind, MPExp,
    MPExpKind, Node, PExp, PExpKind, Pat, PatKind,
};

/// `internal_lets` rewrite (ehrliche Teilmenge, siehe Modul-Doku).
#[derive(Clone, Copy, Debug, Default)]
pub struct InternalLets;

impl crate::pass::TypedPass for InternalLets {
    fn name(&self) -> &'static str { InternalLets::name(self) }
    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        InternalLets::run_typed(self, ast, interner)
    }
}

impl InternalLets {
    /// OCaml rewrite name; `LEAN_REWRITES[43]`.
    pub const NAME: &'static str = "internal_lets";

    /// Pass name, matching the OCaml rewrite name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    /// Run the env-free subset over the canonical AST.
    pub fn run_typed(&self, ast: &mut Ast, interner: &mut dyn Interner) {
        for def in &mut ast.defs {
            rewrite_def(def, interner);
        }
    }
}

// ---------------------------------------------------------------------------
// Syntactic early-return test (`rewriter.ml`, `has_early_return`)
// ---------------------------------------------------------------------------

/// Whether `exp` contains `E_return` or an `early_return` call anywhere.
fn has_early_return(exp: &Exp, interner: &dyn Interner) -> bool {
    match &exp.kind {
        ExpKind::Return(_) => true,
        ExpKind::App { func, args } => {
            interner.resolve(func.symbol) == Some("early_return")
                || args.iter().any(|arg| has_early_return(arg, interner))
        }
        ExpKind::Id(_)
        | ExpKind::Lit(_)
        | ExpKind::Ref(_)
        | ExpKind::Config(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Constraint(_) => false,
        ExpKind::Typ { exp: inner, .. } => has_early_return(inner, interner),
        ExpKind::Tuple(exprs)
        | ExpKind::Block(exprs)
        | ExpKind::Vector(exprs)
        | ExpKind::List(exprs) => exprs.iter().any(|e| has_early_return(e, interner)),
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            has_early_return(cond, interner)
                || has_early_return(then_branch, interner)
                || has_early_return(else_branch, interner)
        }
        ExpKind::Loop {
            measure, cond, body, ..
        } => {
            measure
                .as_ref()
                .is_some_and(|m| has_early_return(m, interner))
                || has_early_return(cond, interner)
                || has_early_return(body, interner)
        }
        ExpKind::For {
            from, to, step, body, ..
        } => {
            has_early_return(from, interner)
                || has_early_return(to, interner)
                || has_early_return(step, interner)
                || has_early_return(body, interner)
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            has_early_return(a, interner) || has_early_return(b, interner)
        }
        ExpKind::Struct { fields, .. } => fields
            .iter()
            .any(|f| has_early_return(&f.value, interner)),
        ExpKind::StructUpdate { record, fields } => {
            has_early_return(record, interner)
                || fields.iter().any(|f| has_early_return(&f.value, interner))
        }
        ExpKind::Field { record, .. } => has_early_return(record, interner),
        ExpKind::Match { scrutinee, arms } => {
            has_early_return(scrutinee, interner)
                || arms.iter().any(|a| pexp_has_early_return(a, interner))
        }
        ExpKind::Let { bind, body, .. } => {
            has_early_return(bind, interner) || has_early_return(body, interner)
        }
        ExpKind::Assign { lexp, value } => {
            lexp_has_early_return(lexp, interner) || has_early_return(value, interner)
        }
        ExpKind::Exit(e)
        | ExpKind::Throw(e)
        | ExpKind::InternalReturn(e) => has_early_return(e, interner),
        ExpKind::Try { exp: e, arms } => {
            has_early_return(e, interner)
                || arms.iter().any(|a| pexp_has_early_return(a, interner))
        }
        ExpKind::Assert { cond, msg } => {
            has_early_return(cond, interner) || has_early_return(msg, interner)
        }
        ExpKind::Var { lexp, init, body } => {
            lexp_has_early_return(lexp, interner)
                || has_early_return(init, interner)
                || has_early_return(body, interner)
        }
        ExpKind::InternalPlet { bind, body, .. } => {
            has_early_return(bind, interner) || has_early_return(body, interner)
        }
        ExpKind::InternalAssume { exp: e, .. } => has_early_return(e, interner),
    }
}

fn lexp_has_early_return(lexp: &LExp, interner: &dyn Interner) -> bool {
    match &lexp.kind {
        LExpKind::Id(_) | LExpKind::Typ { .. } => false,
        LExpKind::Deref(exp) => has_early_return(exp, interner),
        LExpKind::Tuple(parts) | LExpKind::VectorConcat(parts) => parts
            .iter()
            .any(|p| lexp_has_early_return(p, interner)),
        LExpKind::Vector { lexp, index } => {
            lexp_has_early_return(lexp, interner) || has_early_return(index, interner)
        }
        LExpKind::VectorRange { lexp, lo, hi } => {
            lexp_has_early_return(lexp, interner)
                || has_early_return(lo, interner)
                || has_early_return(hi, interner)
        }
        LExpKind::Field { lexp, .. } => lexp_has_early_return(lexp, interner),
    }
}

fn pexp_has_early_return(pexp: &PExp, interner: &dyn Interner) -> bool {
    match &pexp.kind {
        PExpKind::PatExp { body, .. } => has_early_return(body, interner),
        PExpKind::PatWhen { guard, body, .. } => {
            has_early_return(guard, interner) || has_early_return(body, interner)
        }
    }
}

// ---------------------------------------------------------------------------
// `pat_of_local_lexp` (nur lokale Formen; sonst `None` = erhalten)
// ---------------------------------------------------------------------------

/// Convert a variable-like lvalue to its binding pattern.
///
/// `None` for effectful/complex lvalues (`LE_deref`, `LE_vector`, ...):
/// OCaml scheitert dort mit `failwith`, wir erhalten den Knoten.
fn lexp_to_pat(lexp: &LExp) -> Option<Pat> {
    match &lexp.kind {
        LExpKind::Id(id) => Some(Node::dummy(PatKind::Id(id.clone()))),
        LExpKind::Typ { typ, id } => Some(Node::dummy(PatKind::Typ {
            typ: typ.clone(),
            pat: Box::new(Node::dummy(PatKind::Id(id.clone()))),
        })),
        LExpKind::Tuple(parts) => {
            let mut pats = Vec::with_capacity(parts.len());
            for part in parts {
                pats.push(lexp_to_pat(part)?);
            }
            Some(Node::dummy(PatKind::Tuple(pats)))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Bottom-up rewrite (`fold_exp`-Ordnung: Kinder zuerst)
// ---------------------------------------------------------------------------

/// Rewrite one expression node after its children.
fn rewrite_exp(exp: &mut Exp, interner: &dyn Interner) {
    rewrite_children(exp, interner);
    // `E_let(pat, bind, body)` mit Early-Return in `bind` -> `E_internal_plet`.
    let let_to_plet = matches!(&exp.kind, ExpKind::Let { bind, .. } if has_early_return(bind, interner));
    if let_to_plet {
        let ExpKind::Let { pat, bind, body } =
            std::mem::replace(&mut exp.kind, ExpKind::Undef)
        else {
            unreachable!("checked Let above");
        };
        exp.kind = ExpKind::InternalPlet { pat, bind, body };
        return;
    }
    // `E_var(lexp, init, body)` mit Early-Return in `init` -> `E_internal_plet`.
    let var_to_plet = matches!(&exp.kind, ExpKind::Var { init, .. } if has_early_return(init, interner));
    if var_to_plet {
        let ExpKind::Var { lexp, init, body } =
            std::mem::replace(&mut exp.kind, ExpKind::Undef)
        else {
            unreachable!("checked Var above");
        };
        if let Some(pat) = lexp_to_pat(&lexp) {
            exp.kind = ExpKind::InternalPlet {
                pat,
                bind: init,
                body,
            };
        } else {
            exp.kind = ExpKind::Var { lexp, init, body };
        }
    }
}

fn rewrite_pexp(pexp: &mut PExp, interner: &dyn Interner) {
    match &mut pexp.kind {
        PExpKind::PatExp { body, .. } => rewrite_exp(body, interner),
        PExpKind::PatWhen { guard, body, .. } => {
            rewrite_exp(guard, interner);
            rewrite_exp(body, interner);
        }
    }
}

fn rewrite_mpexp(mpexp: &mut MPExp, interner: &dyn Interner) {
    if let MPExpKind::When { guard, .. } = &mut mpexp.kind {
        rewrite_exp(guard, interner);
    }
}

fn rewrite_mapcl(mapcl: &mut Mapcl, interner: &dyn Interner) {
    match &mut mapcl.kind {
        MapclKind::Bidir {
            forwards,
            backwards,
        } => {
            rewrite_mpexp(forwards, interner);
            rewrite_mpexp(backwards, interner);
        }
        MapclKind::Forwards(pexp) | MapclKind::Backwards(pexp) => {
            rewrite_pexp(pexp, interner);
        }
    }
}

fn rewrite_funcl(funcl: &mut Funcl, interner: &dyn Interner) {
    rewrite_pexp(&mut funcl.arm, interner);
}

#[allow(clippy::too_many_lines)]
fn rewrite_children(exp: &mut Exp, interner: &dyn Interner) {
    match &mut exp.kind {
        ExpKind::Id(_)
        | ExpKind::Lit(_)
        | ExpKind::Ref(_)
        | ExpKind::Config(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Constraint(_) => {}
        ExpKind::Typ { exp: inner, .. } => rewrite_exp(inner, interner),
        ExpKind::App { args, .. } => {
            for arg in args.iter_mut() {
                rewrite_exp(arg, interner);
            }
        }
        ExpKind::Block(exprs)
        | ExpKind::Tuple(exprs)
        | ExpKind::Vector(exprs)
        | ExpKind::List(exprs) => {
            for e in exprs.iter_mut() {
                rewrite_exp(e, interner);
            }
        }
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            rewrite_exp(cond, interner);
            rewrite_exp(then_branch, interner);
            rewrite_exp(else_branch, interner);
        }
        ExpKind::Loop {
            measure, cond, body, ..
        } => {
            if let Some(measure) = measure {
                rewrite_exp(measure, interner);
            }
            rewrite_exp(cond, interner);
            rewrite_exp(body, interner);
        }
        ExpKind::For {
            from, to, step, body, ..
        } => {
            rewrite_exp(from, interner);
            rewrite_exp(to, interner);
            rewrite_exp(step, interner);
            rewrite_exp(body, interner);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            rewrite_exp(a, interner);
            rewrite_exp(b, interner);
        }
        ExpKind::Struct { fields, .. } => {
            for field in fields.iter_mut() {
                rewrite_exp(&mut field.value, interner);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            rewrite_exp(record, interner);
            for field in fields.iter_mut() {
                rewrite_exp(&mut field.value, interner);
            }
        }
        ExpKind::Field { record, .. } => rewrite_exp(record, interner),
        ExpKind::Match { scrutinee, arms } => {
            rewrite_exp(scrutinee, interner);
            for arm in arms.iter_mut() {
                rewrite_pexp(arm, interner);
            }
        }
        ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
            rewrite_exp(bind, interner);
            rewrite_exp(body, interner);
        }
        ExpKind::Assign { lexp, value } => {
            rewrite_lexp_children(lexp, interner);
            rewrite_exp(value, interner);
        }
        ExpKind::Return(e)
        | ExpKind::Exit(e)
        | ExpKind::Throw(e)
        | ExpKind::InternalReturn(e) => rewrite_exp(e, interner),
        ExpKind::Try { exp: e, arms } => {
            rewrite_exp(e, interner);
            for arm in arms.iter_mut() {
                rewrite_pexp(arm, interner);
            }
        }
        ExpKind::Assert { cond, msg } => {
            rewrite_exp(cond, interner);
            rewrite_exp(msg, interner);
        }
        ExpKind::Var { lexp, init, body } => {
            rewrite_lexp_children(lexp, interner);
            rewrite_exp(init, interner);
            rewrite_exp(body, interner);
        }
        ExpKind::InternalAssume { exp: e, .. } => rewrite_exp(e, interner),
    }
}

fn rewrite_lexp_children(lexp: &mut LExp, interner: &dyn Interner) {
    match &mut lexp.kind {
        LExpKind::Id(_) | LExpKind::Typ { .. } => {}
        LExpKind::Deref(exp) => rewrite_exp(exp, interner),
        LExpKind::Tuple(parts) | LExpKind::VectorConcat(parts) => {
            for part in parts.iter_mut() {
                rewrite_lexp_children(part, interner);
            }
        }
        LExpKind::Vector { lexp, index } => {
            rewrite_lexp_children(lexp, interner);
            rewrite_exp(index, interner);
        }
        LExpKind::VectorRange { lexp, lo, hi } => {
            rewrite_lexp_children(lexp, interner);
            rewrite_exp(lo, interner);
            rewrite_exp(hi, interner);
        }
        LExpKind::Field { lexp, .. } => rewrite_lexp_children(lexp, interner),
    }
}

fn rewrite_def(def: &mut Def, interner: &dyn Interner) {
    match &mut def.kind {
        DefKind::Fundef { funcls, .. } => {
            for funcl in funcls.iter_mut() {
                rewrite_funcl(funcl, interner);
            }
        }
        DefKind::Mapdef { clauses, .. } => {
            for clause in clauses.iter_mut() {
                rewrite_mapcl(clause, interner);
            }
        }
        DefKind::Impl(funcl) => rewrite_funcl(funcl, interner),
        DefKind::Let { exp, .. } => rewrite_exp(exp, interner),
        DefKind::Outcome { defs, .. } => {
            for def in defs.iter_mut() {
                rewrite_def(def, interner);
            }
        }
        DefKind::Measure { exp, .. } => rewrite_exp(exp, interner),
        DefKind::LoopMeasures { measures, .. } => {
            for (_, exp) in measures.iter_mut() {
                rewrite_exp(exp, interner);
            }
        }
        DefKind::Register(spec) => {
            if let Some(init) = spec.init.as_mut() {
                rewrite_exp(init, interner);
            }
        }
        DefKind::InternalMutrec(sets) => {
            for set in sets.iter_mut() {
                for funcl in set.funcls.iter_mut() {
                    rewrite_funcl(funcl, interner);
                }
            }
        }
        DefKind::Scattered(scattered) => {
            use sail_ast::ScatteredDef;
            match scattered {
                ScatteredDef::Funcl(funcl) => rewrite_funcl(funcl, interner),
                ScatteredDef::Mapcl { clause, .. } => rewrite_mapcl(clause, interner),
                _ => {}
            }
        }
        DefKind::Type(_)
        | DefKind::Constraint(_)
        | DefKind::Val(_)
        | DefKind::Instantiation { .. }
        | DefKind::Fixity { .. }
        | DefKind::Overload { .. }
        | DefKind::Default(_)
        | DefKind::Pragma { .. } => {}
    }
}
