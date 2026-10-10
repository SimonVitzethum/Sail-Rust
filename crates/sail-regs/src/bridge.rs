//! Bridge from `sail-ast` register defs to the string-based register pipeline.
//!
//! Resolves migration entries **#26** (`MinimalDef` → `DefKind::Register(DecSpec)`)
//! and **#27** (`SailTyp(String)` → `sail_ast::Typ` rendered + Lane-10 printer types)
//! without touching the Lean printer (`sail-lean` is only read, never modified).
//!
//! Design:
//! - [`render_sail_typ`] spells a `sail_ast::Typ` back to Sail source
//!   (`bits(64)`, `bool`, …) so the existing string pipeline
//!   (`lean_of_sail_typ`, `regtyp_field_stem`, `default_init_sail`) keeps working.
//! - [`lean_typ_of_ast`] maps a `sail_ast::Typ` onto the Lane-10 printer type
//!   (`sail_lean::Typ`); [`lean_typ_string_of_ast`] renders it with
//!   `LeanPrinter::print_typ`, falling back to the spelling-based
//!   `lean_of_sail_typ` for constructs outside the printer subset.
//! - [`dec_spec_to_minimal`] is the 1:1 conversion fixed in `REGS_NEEDED.md`;
//!   [`extract_registers_ast`] is the `find_registers` equivalent over
//!   `&[sail_ast::Def]`. It preserves `has_init` from `DecSpec.init.is_some()`
//!   even when the initializer is outside the [`try_render_sail_exp`] subset
//!   (then `init_sail` is `None` and `initial_values()` falls back to the type
//!   default — the gap stays visible via `has_init == true`).
//!
//! Out of scope (stays the typechecker's job, per `REGS_NEEDED.md`):
//! synonym expansion and bitfield base types; callers pass expanded spellings.

use sail_ast::{
    Constraint, ConstraintKind, DecSpec, Def, DefKind, Exp, ExpKind, Interner, Lit, NExp, NExpKind,
    Symbol, Typ, TypArg, TypArgKind, TypKind,
};
use sail_lean::{LeanPrinter, Nexp as LeanNexp, Typ as LeanTyp, TypArg as LeanTypArg};

use crate::{MinimalDef, RegisterDef, SailTyp, lean_of_sail_typ};

// ---------------------------------------------------------------------------
// Symbol helpers
// ---------------------------------------------------------------------------

fn sym_text<'a>(interner: &'a impl Interner, sym: Symbol) -> Option<&'a str> {
    interner.resolve(sym)
}

fn name_text<'a>(interner: &'a impl Interner, name: &sail_ast::Name) -> Option<&'a str> {
    interner.resolve(name.symbol)
}

// ---------------------------------------------------------------------------
// Sail-source rendering (spelling for the string pipeline)
// ---------------------------------------------------------------------------

/// Render a size expression back to Sail source.
#[must_use]
pub fn render_sail_nexp(nexp: &NExp, interner: &impl Interner) -> String {
    match &nexp.kind {
        NExpKind::Constant(s) => s.clone(),
        NExpKind::Var(sym) => sym_text(interner, *sym).unwrap_or("?").to_owned(),
        NExpKind::Id(name) => name_text(interner, name).unwrap_or("?").to_owned(),
        NExpKind::App { func, args } => {
            let f = name_text(interner, func).unwrap_or("?");
            let rendered: Vec<String> =
                args.iter().map(|a| render_sail_nexp(a, interner)).collect();
            format!("{f}({})", rendered.join(", "))
        }
        NExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => format!(
            "(if {} then {} else {})",
            render_sail_constraint(cond, interner),
            render_sail_nexp(then_branch, interner),
            render_sail_nexp(else_branch, interner)
        ),
        NExpKind::Times(a, b) => format!(
            "({} * {})",
            render_sail_nexp(a, interner),
            render_sail_nexp(b, interner)
        ),
        NExpKind::Sum(a, b) => format!(
            "({} + {})",
            render_sail_nexp(a, interner),
            render_sail_nexp(b, interner)
        ),
        NExpKind::Minus(a, b) => format!(
            "({} - {})",
            render_sail_nexp(a, interner),
            render_sail_nexp(b, interner)
        ),
        NExpKind::Exp(e) => format!("(2 ^ {})", render_sail_nexp(e, interner)),
        NExpKind::Neg(e) => format!("(-{})", render_sail_nexp(e, interner)),
    }
}

/// Render a numeric constraint back to Sail source.
#[must_use]
pub fn render_sail_constraint(constraint: &Constraint, interner: &impl Interner) -> String {
    match &constraint.kind {
        ConstraintKind::True => "true".to_owned(),
        ConstraintKind::False => "false".to_owned(),
        ConstraintKind::Equal(a, b) => format!(
            "({} == {})",
            render_sail_typarg(a, interner),
            render_sail_typarg(b, interner)
        ),
        ConstraintKind::NotEqual(a, b) => format!(
            "({} != {})",
            render_sail_typarg(a, interner),
            render_sail_typarg(b, interner)
        ),
        ConstraintKind::Ge(a, b) => format!(
            "({} >= {})",
            render_sail_nexp(a, interner),
            render_sail_nexp(b, interner)
        ),
        ConstraintKind::Gt(a, b) => format!(
            "({} > {})",
            render_sail_nexp(a, interner),
            render_sail_nexp(b, interner)
        ),
        ConstraintKind::Le(a, b) => format!(
            "({} <= {})",
            render_sail_nexp(a, interner),
            render_sail_nexp(b, interner)
        ),
        ConstraintKind::Lt(a, b) => format!(
            "({} < {})",
            render_sail_nexp(a, interner),
            render_sail_nexp(b, interner)
        ),
        ConstraintKind::Set { nexp, members } => format!(
            "({} in {{{}}})",
            render_sail_nexp(nexp, interner),
            members.join(", ")
        ),
        ConstraintKind::And(a, b) => format!(
            "({} & {})",
            render_sail_constraint(a, interner),
            render_sail_constraint(b, interner)
        ),
        ConstraintKind::Or(a, b) => format!(
            "({} | {})",
            render_sail_constraint(a, interner),
            render_sail_constraint(b, interner)
        ),
        ConstraintKind::Not(x) => format!("(!{})", render_sail_constraint(x, interner)),
        ConstraintKind::App { func, args } => {
            let f = name_text(interner, func).unwrap_or("?");
            let rendered: Vec<String> =
                args.iter().map(|a| render_sail_typarg(a, interner)).collect();
            format!("{f}({})", rendered.join(", "))
        }
        ConstraintKind::Id(name) => name_text(interner, name).unwrap_or("?").to_owned(),
        ConstraintKind::Var(sym) => sym_text(interner, *sym).unwrap_or("?").to_owned(),
    }
}

/// Render a type argument back to Sail source.
#[must_use]
pub fn render_sail_typarg(arg: &TypArg, interner: &impl Interner) -> String {
    match &arg.kind {
        TypArgKind::NExp(n) => render_sail_nexp(n, interner),
        TypArgKind::Typ(t) => render_sail_typ(t, interner),
        TypArgKind::Bool(c) => render_sail_constraint(c, interner),
    }
}

/// Render a `sail_ast::Typ` back to its Sail source spelling
/// (`bits(64)`, `bool`, …).
///
/// Quantifiers of `TypKind::Exist` are erased to the inner spelling:
/// registers are monomorphic, and the old string pipeline never saw
/// binders either (callers pass the already-expanded spelling).
#[must_use]
pub fn render_sail_typ(typ: &Typ, interner: &impl Interner) -> String {
    match &typ.kind {
        TypKind::Unknown => "unknown".to_owned(),
        TypKind::Id(name) => name_text(interner, name).unwrap_or("unknown").to_owned(),
        TypKind::Var(sym) => sym_text(interner, *sym).unwrap_or("?").to_owned(),
        TypKind::Fn { args, ret } => {
            let rendered: Vec<String> =
                args.iter().map(|a| render_sail_typ(a, interner)).collect();
            format!("({} -> {})", rendered.join(", "), render_sail_typ(ret, interner))
        }
        TypKind::Bidir { lhs, rhs } => format!(
            "({} <-> {})",
            render_sail_typ(lhs, interner),
            render_sail_typ(rhs, interner)
        ),
        TypKind::Tuple(tys) => {
            let rendered: Vec<String> =
                tys.iter().map(|t| render_sail_typ(t, interner)).collect();
            format!("({})", rendered.join(", "))
        }
        TypKind::App { name, args } => {
            let f = name_text(interner, name).unwrap_or("unknown");
            if args.is_empty() {
                return f.to_owned();
            }
            let rendered: Vec<String> =
                args.iter().map(|a| render_sail_typarg(a, interner)).collect();
            format!("{f}({})", rendered.join(", "))
        }
        TypKind::Exist { typ, .. } => render_sail_typ(typ, interner),
    }
}

/// Render a literal back to Sail source. Payloads resolve through
/// `interner` (Lane 71: interned [`sail_ast::LitKind`]); an unresolvable
/// symbol renders as `?` like the other renderers above.
#[must_use]
pub fn render_sail_lit(lit: &Lit, interner: &impl Interner) -> String {
    let text = |sym: &Symbol| interner.resolve(*sym).unwrap_or("?");
    match &lit.kind {
        sail_ast::LitKind::Unit => "()".to_owned(),
        sail_ast::LitKind::True => "true".to_owned(),
        sail_ast::LitKind::False => "false".to_owned(),
        sail_ast::LitKind::Num(s) => text(s).to_owned(),
        sail_ast::LitKind::Hex(d) => format!("0x{}", text(d)),
        sail_ast::LitKind::Bin(b) => format!("0b{}", text(b)),
        sail_ast::LitKind::String(s) => format!("{:?}", text(s)),
        sail_ast::LitKind::Real(s) => text(s).to_owned(),
    }
}

/// Try to render an initializer expression back to Sail source.
///
/// Covers the `reset.sail` subset (literals and plain variable references);
/// anything else yields `None` so callers can keep `has_init == true` while
/// leaving `init_sail` empty instead of inventing a spelling.
#[must_use]
pub fn try_render_sail_exp(exp: &Exp, interner: &impl Interner) -> Option<String> {
    match &exp.kind {
        ExpKind::Lit(lit) => Some(render_sail_lit(lit, interner)),
        ExpKind::Id(name) => name_text(interner, name).map(str::to_owned),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Lane-10 printer-type mapping (sail_ast -> sail_lean, read-only)
// ---------------------------------------------------------------------------

/// Map a `sail_ast` size expression onto the Lane-10 printer `Nexp`.
///
/// `Constant` parses decimal text to `Const`; `Var`/`Id` become `Var` by
/// symbol text; `Sum`/`Minus`/`Times`/`Neg` map directly; `Exp` is `Pow2`
/// (matching migration note #29). `App`/`If` have no printer counterpart
/// and yield `None`.
#[must_use]
pub fn lean_nexp_of_ast(nexp: &NExp, interner: &impl Interner) -> Option<LeanNexp> {
    match &nexp.kind {
        NExpKind::Constant(s) => s.parse::<i128>().ok().map(LeanNexp::Const),
        NExpKind::Var(sym) => sym_text(interner, *sym).map(|t| LeanNexp::Var(t.to_owned())),
        NExpKind::Id(name) => name_text(interner, name).map(|t| LeanNexp::Var(t.to_owned())),
        NExpKind::Sum(a, b) => Some(LeanNexp::Sum(
            Box::new(lean_nexp_of_ast(a, interner)?),
            Box::new(lean_nexp_of_ast(b, interner)?),
        )),
        NExpKind::Minus(a, b) => Some(LeanNexp::Minus(
            Box::new(lean_nexp_of_ast(a, interner)?),
            Box::new(lean_nexp_of_ast(b, interner)?),
        )),
        NExpKind::Times(a, b) => Some(LeanNexp::Times(
            Box::new(lean_nexp_of_ast(a, interner)?),
            Box::new(lean_nexp_of_ast(b, interner)?),
        )),
        NExpKind::Neg(e) => Some(LeanNexp::Neg(Box::new(lean_nexp_of_ast(e, interner)?))),
        NExpKind::Exp(e) => Some(LeanNexp::Pow2(Box::new(lean_nexp_of_ast(e, interner)?))),
        NExpKind::App { .. } | NExpKind::If { .. } => None,
    }
}

fn lean_typarg_of_ast(arg: &TypArg, interner: &impl Interner) -> Option<LeanTypArg> {
    match &arg.kind {
        TypArgKind::NExp(n) => lean_nexp_of_ast(n, interner).map(LeanTypArg::Nexp),
        TypArgKind::Typ(t) => lean_typ_of_ast(t, interner).map(LeanTypArg::Typ),
        // The printer subset has no boolean type argument (migration note #31).
        TypArgKind::Bool(_) => None,
    }
}

/// Map a `sail_ast::Typ` onto the Lane-10 printer `Typ`.
///
/// Returns `None` for constructs outside the printer subset (`Fn`, `Bidir`,
/// `Exist`, `Unknown`, `register(t)`, boolean type arguments, …); callers
/// fall back to the spelling-based [`crate::lean_of_sail_typ`].
/// The printer is only read through this mapping — never modified.
#[must_use]
pub fn lean_typ_of_ast(typ: &Typ, interner: &impl Interner) -> Option<LeanTyp> {
    match &typ.kind {
        TypKind::Id(name) => {
            let text = name_text(interner, name)?;
            match text {
                "bool" => Some(LeanTyp::Bool),
                "int" => Some(LeanTyp::Int),
                "nat" => Some(LeanTyp::Nat),
                "string" => Some(LeanTyp::String),
                "unit" => Some(LeanTyp::Unit),
                "bit" => Some(LeanTyp::Bit(text.to_owned())),
                _ => Some(LeanTyp::Named {
                    name: text.to_owned(),
                    args: Vec::new(),
                    span: sail_lean::Span::default(),
                }),
            }
        }
        TypKind::Var(sym) => sym_text(interner, *sym).map(|t| LeanTyp::Var(t.to_owned())),
        TypKind::Tuple(tys) => {
            let mut out = Vec::with_capacity(tys.len());
            for t in tys {
                out.push(lean_typ_of_ast(t, interner)?);
            }
            Some(LeanTyp::Tuple(out))
        }
        TypKind::App { name, args } => {
            let func = name_text(interner, name)?;
            match func {
                "bits" | "bitvector" => {
                    let [arg] = args.as_slice() else {
                        return None;
                    };
                    match &arg.kind {
                        TypArgKind::NExp(n) => Some(LeanTyp::BitVec(Box::new(lean_nexp_of_ast(
                            n, interner,
                        )?))),
                        _ => None,
                    }
                }
                "option" => {
                    let [arg] = args.as_slice() else {
                        return None;
                    };
                    match &arg.kind {
                        TypArgKind::Typ(t) => {
                            Some(LeanTyp::Option(Box::new(lean_typ_of_ast(t, interner)?)))
                        }
                        _ => None,
                    }
                }
                "list" => {
                    let [arg] = args.as_slice() else {
                        return None;
                    };
                    match &arg.kind {
                        TypArgKind::Typ(t) => {
                            Some(LeanTyp::List(Box::new(lean_typ_of_ast(t, interner)?)))
                        }
                        _ => None,
                    }
                }
                "result" => {
                    let [e, a] = args.as_slice() else {
                        return None;
                    };
                    match (&e.kind, &a.kind) {
                        (TypArgKind::Typ(e), TypArgKind::Typ(a)) => Some(LeanTyp::Result(
                            Box::new(lean_typ_of_ast(e, interner)?),
                            Box::new(lean_typ_of_ast(a, interner)?),
                        )),
                        _ => None,
                    }
                }
                "vector" => {
                    // Sail spells `vector(n, t)`; the printer takes `(Vector elem n)`.
                    // Accept both argument orders, keyed by argument kind.
                    let [first, second] = args.as_slice() else {
                        return None;
                    };
                    let (n, t) = match (&first.kind, &second.kind) {
                        (TypArgKind::NExp(n), TypArgKind::Typ(t))
                        | (TypArgKind::Typ(t), TypArgKind::NExp(n)) => (n, t),
                        _ => return None,
                    };
                    Some(LeanTyp::Vector(
                        Box::new(lean_typ_of_ast(t, interner)?),
                        Box::new(lean_nexp_of_ast(n, interner)?),
                    ))
                }
                // `register(t)` prints via `RegisterInfo::register_ref_ty`
                // (`(RegisterRef T)`); the printer `Typ` has no such case.
                "register" => None,
                _ => {
                    let mut mapped = Vec::with_capacity(args.len());
                    for a in args {
                        mapped.push(lean_typarg_of_ast(a, interner)?);
                    }
                    Some(LeanTyp::Named {
                        name: func.to_owned(),
                        args: mapped,
                        span: sail_lean::Span::default(),
                    })
                }
            }
        }
        TypKind::Fn { .. }
        | TypKind::Bidir { .. }
        | TypKind::Exist { .. }
        | TypKind::Unknown => None,
    }
}

/// Render the Lean type spelling for a `sail_ast::Typ`.
///
/// Prefers the typed bridge (`lean_typ_of_ast` + `LeanPrinter::print_typ`,
/// printer used read-only); falls back to the spelling-based
/// [`crate::lean_of_sail_typ`] outside the printer subset.
#[must_use]
pub fn lean_typ_string_of_ast(typ: &Typ, interner: &impl Interner) -> String {
    if let Some(lean_ty) = lean_typ_of_ast(typ, interner) {
        let printer = LeanPrinter::with_default();
        let mut buf = String::new();
        if printer.print_typ(&mut buf, &lean_ty).is_ok() {
            return buf;
        }
    }
    lean_of_sail_typ(&render_sail_typ(typ, interner))
}

// ---------------------------------------------------------------------------
// Register extraction over sail-ast defs (#26 + #27)
// ---------------------------------------------------------------------------

/// Convert one `DecSpec` to the 1:1 `MinimalDef` input (migration entry #26).
///
/// Returns `None` when the register name symbol cannot be resolved; the type
/// spelling comes from [`render_sail_typ`], the initializer from
/// [`try_render_sail_exp`] (`None` = absent or outside the subset).
#[must_use]
pub fn dec_spec_to_minimal(dec: &DecSpec, interner: &impl Interner) -> Option<MinimalDef> {
    let name = name_text(interner, &dec.name)?.to_owned();
    Some(MinimalDef::Register {
        name,
        typ: SailTyp::new(render_sail_typ(&dec.typ, interner)),
        init: dec.init.as_ref().and_then(|e| try_render_sail_exp(e, interner)),
    })
}

/// Extract registers from `sail_ast` defs (`find_registers` equivalent).
///
/// Non-register defs are skipped and definition order is preserved, exactly
/// like [`crate::extract_registers`]. Registers whose name symbol cannot be
/// resolved are skipped. The Lean spelling prefers the typed bridge
/// ([`lean_typ_string_of_ast`]); `has_init` always mirrors
/// `DecSpec.init.is_some()`, even when the initializer is outside the
/// [`try_render_sail_exp`] subset.
#[must_use]
pub fn extract_registers_ast(defs: &[Def], interner: &impl Interner) -> Vec<RegisterDef> {
    defs.iter()
        .filter_map(|d| match &d.kind {
            DefKind::Register(dec) => {
                let name = name_text(interner, &dec.name)?.to_owned();
                let sail_typ = render_sail_typ(&dec.typ, interner);
                let lean_typ = lean_typ_string_of_ast(&dec.typ, interner);
                Some(RegisterDef {
                    name,
                    sail_typ,
                    lean_typ,
                    has_init: dec.init.is_some(),
                    init_sail: dec
                        .init
                        .as_ref()
                        .and_then(|e| try_render_sail_exp(e, interner)),
                })
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{
        DecSpec, LitKind, Name, Node, NodeId, SimpleInterner, Span, Symbol, TypKind,
    };
    use sail_ast::Interner as _;

    struct Ctx {
        interner: SimpleInterner,
    }

    impl Ctx {
        fn new() -> Self {
            Ctx {
                interner: SimpleInterner::new(),
            }
        }

        fn name(&mut self, text: &str) -> Name {
            let symbol = self.interner.intern(text);
            Name::new(NodeId::DUMMY, Span::dummy(), symbol, false)
        }

        fn typ_id(&mut self, text: &str) -> Typ {
            let name = self.name(text);
            Node::dummy(TypKind::Id(name))
        }

        fn nexp_const(text: &str) -> NExp {
            Node::dummy(NExpKind::Constant(text.to_owned()))
        }

        fn typ_app(&mut self, func: &str, args: Vec<TypArg>) -> Typ {
            let name = self.name(func);
            Node::dummy(TypKind::App { name, args })
        }

        fn typarg_nexp(nexp: NExp) -> TypArg {
            Node::dummy(TypArgKind::NExp(nexp))
        }

        fn typarg_typ(typ: Typ) -> TypArg {
            Node::dummy(TypArgKind::Typ(typ))
        }

        fn bits(&mut self, width: &str) -> Typ {
            self.typ_app("bits", vec![Self::typarg_nexp(Self::nexp_const(width))])
        }

        fn dec(&mut self, name: &str, typ: Typ, init: Option<Exp>) -> DecSpec {
            DecSpec {
                id: NodeId::DUMMY,
                span: Span::dummy(),
                typ,
                name: self.name(name),
                init,
            }
        }

        fn lit_exp(&mut self, text: &str) -> Exp {
            let kind = LitKind::Bin(self.interner.intern(text));
            Node::dummy(ExpKind::Lit(Node::dummy(kind)))
        }
    }

    #[test]
    fn sail_spelling_covers_register_types() {
        let mut ctx = Ctx::new();
        let cases = [
            ("bool", "bool"),
            ("int", "int"),
            ("nat", "nat"),
            ("string", "string"),
            ("unit", "unit"),
            ("bit", "bit"),
        ];
        for (sail, want) in cases {
            let ty = ctx.typ_id(sail);
            assert_eq!(render_sail_typ(&ty, &ctx.interner), want);
        }
        let bits64 = ctx.bits("64");
        assert_eq!(render_sail_typ(&bits64, &ctx.interner), "bits(64)");
        let bv = ctx.typ_app("bitvector", vec![Ctx::typarg_nexp(Ctx::nexp_const("32"))]);
        assert_eq!(render_sail_typ(&bv, &ctx.interner), "bitvector(32)");
    }

    #[test]
    fn typed_bridge_renders_via_printer_types() {
        let mut ctx = Ctx::new();
        // bits(64) -> sail_lean::Typ::BitVec -> "(BitVec 64)".
        let bits64 = ctx.bits("64");
        assert_eq!(
            lean_typ_of_ast(&bits64, &ctx.interner),
            Some(LeanTyp::BitVec(Box::new(LeanNexp::Const(64))))
        );
        assert_eq!(lean_typ_string_of_ast(&bits64, &ctx.interner), "(BitVec 64)");
        // bit -> Bit -> "(BitVec 1)" (printer ignores the payload).
        let bit = ctx.typ_id("bit");
        assert_eq!(lean_typ_string_of_ast(&bit, &ctx.interner), "(BitVec 1)");
        assert_eq!(lean_typ_string_of_ast(&ctx.typ_id("bool"), &ctx.interner), "Bool");
        assert_eq!(lean_typ_string_of_ast(&ctx.typ_id("int"), &ctx.interner), "Int");
        assert_eq!(lean_typ_string_of_ast(&ctx.typ_id("nat"), &ctx.interner), "Nat");
        // option/list/vector go through the printer, not the spelling fallback.
        let opt = ctx.typ_app("option", vec![Ctx::typarg_typ(ctx.typ_id("bool"))]);
        assert_eq!(lean_typ_string_of_ast(&opt, &ctx.interner), "(Option Bool)");
        let list = ctx.typ_app("list", vec![Ctx::typarg_typ(ctx.typ_id("int"))]);
        assert_eq!(lean_typ_string_of_ast(&list, &ctx.interner), "(List Int)");
        let vec = ctx.typ_app(
            "vector",
            vec![
                Ctx::typarg_nexp(Ctx::nexp_const("8")),
                Ctx::typarg_typ(ctx.typ_id("bool")),
            ],
        );
        assert_eq!(lean_typ_string_of_ast(&vec, &ctx.interner), "(Vector Bool 8)");
    }

    #[test]
    fn untranslatable_typ_falls_back_to_spelling() {
        let mut ctx = Ctx::new();
        // Fn types are outside the printer subset: spelling fallback.
        let ret = ctx.typ_id("bool");
        let arg = ctx.typ_id("int");
        let fun: Typ = Node::dummy(TypKind::Fn {
            args: vec![arg],
            ret: Box::new(ret),
        });
        assert_eq!(lean_typ_of_ast(&fun, &ctx.interner), None);
        assert_eq!(
            lean_typ_string_of_ast(&fun, &ctx.interner),
            lean_of_sail_typ(&render_sail_typ(&fun, &ctx.interner))
        );
        // register(t) has no printer-`Typ` case: fallback, callers use
        // `RegisterInfo::register_ref_ty` for the `(RegisterRef T)` spelling.
        let reg = ctx.typ_app("register", vec![Ctx::typarg_typ(ctx.typ_id("bool"))]);
        assert_eq!(lean_typ_of_ast(&reg, &ctx.interner), None);
        // Non-decimal-widest nexp constants are outside `Nexp::Const(i128)`.
        let huge = "999999999999999999999999999999999999999999999999999999999999";
        let big = ctx.typ_app("bits", vec![Ctx::typarg_nexp(Ctx::nexp_const(huge))]);
        assert_eq!(lean_nexp_of_ast(&Ctx::nexp_const(huge), &ctx.interner), None);
        assert_eq!(
            lean_typ_string_of_ast(&big, &ctx.interner),
            lean_of_sail_typ(&render_sail_typ(&big, &ctx.interner))
        );
    }

    #[test]
    fn ast_extraction_matches_string_pipeline() {
        let mut ctx = Ctx::new();
        let init: Exp = ctx.lit_exp("00000000000000000000000000000000");
        let defs: Vec<Def> = vec![
            Node::dummy(DefKind::Register(ctx.dec("PSTATE", ctx.bits("32"), Some(init)))),
            Node::dummy(DefKind::Register(ctx.dec("SP", ctx.bits("64"), None))),
            // Non-register defs are skipped, order of registers preserved.
            Node::dummy(DefKind::Pragma {
                key: "x".to_owned(),
                payload: "y".to_owned(),
            }),
        ];
        let regs = extract_registers_ast(&defs, &ctx.interner);
        assert_eq!(
            regs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["PSTATE", "SP"]
        );
        assert_eq!(regs[0].sail_typ, "bits(32)");
        assert_eq!(regs[0].lean_typ, "(BitVec 32)");
        assert!(regs[0].has_init);
        assert_eq!(
            regs[0].init_sail.as_deref(),
            Some("0b00000000000000000000000000000000")
        );
        assert_eq!(regs[1].lean_typ, "(BitVec 64)");
        assert!(!regs[1].has_init);
        assert_eq!(regs[1].init_sail, None);
        // Same result as the string pipeline fed with equivalent spellings.
        let from_strings = crate::extract_registers(&crate::parse_register_lines(
            "register PSTATE : bits(32) = 0b00000000000000000000000000000000\nregister SP : bits(64)\n",
        ));
        assert_eq!(regs[0].name, from_strings[0].name);
        assert_eq!(regs[0].lean_typ, from_strings[0].lean_typ);
        assert_eq!(regs[1].lean_typ, from_strings[1].lean_typ);
    }

    #[test]
    fn unrenderable_init_keeps_has_init() {
        let mut ctx = Ctx::new();
        // An application initializer is outside the `try_render_sail_exp` subset:
        // `has_init` stays true, `init_sail` stays empty (no invented spelling).
        let callee = ctx.name("f");
        let app: Exp = Node::dummy(ExpKind::App {
            func: callee,
            args: Vec::new(),
        });
        let defs: Vec<Def> =
            vec![Node::dummy(DefKind::Register(ctx.dec("R", ctx.bits("8"), Some(app))))];
        let regs = extract_registers_ast(&defs, &ctx.interner);
        assert_eq!(regs.len(), 1);
        assert!(regs[0].has_init);
        assert_eq!(regs[0].init_sail, None);
        // `dec_spec_to_minimal` (#26) mirrors the same register.
        let callee2 = ctx.name("g");
        let app2: Exp = Node::dummy(ExpKind::App {
            func: callee2,
            args: Vec::new(),
        });
        let dec = ctx.dec("R", ctx.bits("8"), Some(app2));
        match dec_spec_to_minimal(&dec, &ctx.interner).expect("resolvable name") {
            MinimalDef::Register { name, typ, init } => {
                assert_eq!(name, "R");
                assert_eq!(typ.as_str(), "bits(8)");
                assert_eq!(init, None);
            }
            MinimalDef::Other => panic!("expected Register"),
        }
    }

    #[test]
    fn unresolvable_names_are_skipped() {
        let ctx = Ctx::new();
        let ghost = sail_ast::Name::new(NodeId::DUMMY, Span::dummy(), Symbol::DUMMY, false);
        let typ: Typ = Node::dummy(TypKind::Id(ghost.clone()));
        let dec = DecSpec {
            id: NodeId::DUMMY,
            span: Span::dummy(),
            typ,
            name: ghost,
            init: None,
        };
        assert_eq!(dec_spec_to_minimal(&dec, &ctx.interner), None);
        let defs: Vec<Def> = vec![Node::dummy(DefKind::Register(dec))];
        assert!(extract_registers_ast(&defs, &ctx.interner).is_empty());
    }
}
