//! Grammar AST → core AST conversion. Infix chains are folded using the
//! explicit fixity environment before conversion; operands are borrowed to
//! avoid deep-cloning expression subtrees. Known builtin type arguments use
//! their declared sorts, and numeric/bool aliases retain their result kind.
//!
//! Unsupported forms return errors. Attributes/visibility and precise node
//! locations still need core AST support. User type constructors and omitted
//! binder kinds still need whole-program kind inference; fragment conversion
//! alone is not a typechecker.

#![forbid(unsafe_code)]

use sail_ast::{
    Constraint, ConstraintKind, DecSpec, Def, DefKind, Exp, ExpKind, Extern, FExp, Fixity, Funcl,
    FunclSet, IdGen, IndexRange, IndexRangeKind, Interner as _, KOpt, Kind, LExp, LExpKind, Lit,
    LitKind, Loop, Mapcl, MapclKind, MPExpKind, Name, NExp, NExpKind, Node, Order, PExp, PExpKind, Pat,
    PatKind, QuantItem, RecOpt, ScatteredDef, SimpleInterner, Span, StructName, Subst, TAnnotOpt,
    Typ, TypArg, TypArgKind, TypKind, TypPatKind, TypSchm, TypeDefKind, TypeUnion, ValSpec,
};
use sail_grammar::ast as g;
use crate::infix::{FixityEnv, Folded, Side, fold_chain_borrowed};

/// Lower-Kontext: frische IDs + Interner, vom Aufrufer geliehen (Datei-Pfad
/// teilt sich so einen ID-Raum mit [`crate::bridge`]).
pub(crate) struct Ctx<'a> {
    pub(crate) ids: &'a mut IdGen,
    pub(crate) interner: &'a mut SimpleInterner,
    pub(crate) fixities: &'a mut FixityEnv,
    pub(crate) bound_kinds: crate::kinds::Bindings,
    pub(crate) constructors: &'a mut crate::kinds::Constructors,
}

impl Ctx<'_> {
    fn node<K>(&mut self, span: Span, kind: K) -> Node<K> {
        Node::new(self.ids.fresh(), span, kind)
    }

    /// Byte-Heuristik wie die Brücke: kein Ident-Start → Operator-Name.
    fn name(&mut self, span: Span, text: &str) -> Name {
        let is_operator = text
            .bytes()
            .next()
            .is_some_and(|b| !(b.is_ascii_alphabetic() || b == b'_'));
        Name::new(self.ids.fresh(), span, self.interner.intern(text), is_operator)
    }

    fn lit(&mut self, span: Span, l: &g::Lit) -> Lit {
        let kind = match l {
            g::Lit::True => LitKind::True,
            g::Lit::False => LitKind::False,
            g::Lit::Unit => LitKind::Unit,
            g::Lit::Num(s) => LitKind::Num(self.interner.intern(s)),
            // initial_check.to_ast_lit lowers bitzero/bitone to one-bit
            // binary literals, not integer zero/one.
            g::Lit::Zero => LitKind::Bin(self.interner.intern("0")),
            g::Lit::One => LitKind::Bin(self.interner.intern("1")),
            g::Lit::Bin(s) => LitKind::Bin(self.interner.intern(s)),
            g::Lit::Hex(s) => LitKind::Hex(self.interner.intern(s)),
            g::Lit::Str(s) => LitKind::String(self.interner.intern(s)),
            g::Lit::Real(s) => LitKind::Real(self.interner.intern(s)),
        };
        self.node(span, kind)
    }
}

// ---------------------------------------------------------------------------
// Fragment-Einstiege: Grammatik rufen, dann senken.
// ---------------------------------------------------------------------------

pub(crate) fn parse_typ_src(c: &mut Ctx, span: Span, src: &str) -> Result<Typ, String> {
    let t = sail_grammar::parse_typ(src)?;
    conv_typ(c, span, &t)
}

pub(crate) fn parse_typschm_src(
    c: &mut Ctx,
    span: Span,
    src: &str,
) -> Result<TypSchm, String> {
    let s = sail_grammar::parse_typschm(src)?;
    conv_typschm(c, span, &s)
}

pub(crate) fn parse_constraint_src(
    c: &mut Ctx,
    span: Span,
    src: &str,
) -> Result<Constraint, String> {
    // Die Grammatik hat keinen `constraint_eof`-Einstieg; Constraints sind
    // dort `Typ`-Formen (`DefAux::Constraint(Typ)`).
    let t = sail_grammar::parse_typ(src)?;
    conv_constraint(c, span, &t)
}

pub(crate) fn parse_exp_src(c: &mut Ctx, span: Span, src: &str) -> Result<Exp, String> {
    let e = sail_grammar::parse_exp(src)?;
    conv_exp(c, span, &e)
}

pub(crate) fn parse_pat_src(c: &mut Ctx, span: Span, src: &str) -> Result<Pat, String> {
    let p = sail_grammar::parse_pat(src)?;
    conv_pat(c, span, &p)
}

pub(crate) fn parse_def_src(c: &mut Ctx, span: Span, src: &str) -> Result<Def, String> {
    let d = sail_grammar::parse_def(src)?;
    let saved = c.constructors.clone();
    let kind = match conv_def(c, span, &d) {
        Ok(kind) => kind,
        Err(error) => { *c.constructors = saved; return Err(error); }
    };
    let mut def = c.node(span, kind);
    annotate_definition(&mut def, &d);
    Ok(def)
}

fn attributes(attrs: &[g::Attr]) -> Vec<sail_ast::Attribute> {
    attrs.iter().map(|attr| sail_ast::Attribute {
        name: attr.name.clone(), data: attr.data.as_ref().map(attribute_data),
    }).collect()
}

fn attribute_data(data: &g::AttrData) -> sail_ast::AttributeData {
    use sail_ast::AttributeData as A;
    match data {
        g::AttrData::Object(fields) => A::Object(fields.iter().map(|(name, data)| (name.clone(), attribute_data(data))).collect()),
        g::AttrData::Num(value) => A::Num(value.clone()),
        g::AttrData::Str(value) => A::Str(value.clone()),
        g::AttrData::Bool(value) => A::Bool(*value),
        g::AttrData::List(values) => A::List(values.iter().map(attribute_data).collect()),
    }
}

fn annotate_definition(def: &mut Def, mut source: &g::Def) {
    loop {
        match source {
            g::Def::Attr(attrs, inner) => {
                def.annotation_mut().attributes.extend(attributes(attrs));
                source = inner;
            }
            g::Def::Private(inner) => {
                def.annotation_mut().private = true;
                source = inner;
            }
            g::Def::Doc(g::DocComment::Line(doc) | g::DocComment::Block(doc), inner) => {
                def.annotation_mut().documentation = Some(doc.clone());
                source = inner;
            }
            g::Def::Aux(_) => break,
        }
    }
}

/// `let`-Def aus Mini-AST-Rohtext senken (Datei-Pfad, [`crate::real`]).
pub(crate) fn conv_let(
    c: &mut Ctx,
    span: Span,
    pat_src: &str,
    body_src: &str,
) -> Result<DefKind, String> {
    Ok(DefKind::Let {
        pat: parse_pat_src(c, span, pat_src)?,
        exp: parse_exp_src(c, span, body_src)?,
    })
}

/// `register`-Def aus Mini-AST-Rohtext senken. `type_src` hat die Form
/// `NAME : TYP` (vgl. Parser-Tests); der Namenspräfix wird abgestreift.
pub(crate) fn conv_register(
    c: &mut Ctx,
    span: Span,
    name_text: &str,
    type_src: &str,
    init_src: &Option<String>,
) -> Result<DefKind, String> {
    let typ_src = type_src.split_once(':').map(|(_, t)| t).unwrap_or(type_src);
    let typ_src = typ_src.trim();
    if typ_src.is_empty() {
        return Err("register without type".to_owned());
    }
    let typ = parse_typ_src(c, span, typ_src)?;
    let init = match init_src {
        None => None,
        Some(s) if s.trim().is_empty() => None,
        Some(s) => Some(parse_exp_src(c, span, s)?),
    };
    Ok(DefKind::Register(DecSpec {
        id: c.ids.fresh(),
        span,
        typ,
        name: c.name(span, name_text),
        init,
    }))
}

/// `constraint`-Def aus Mini-AST-Rohtext senken.
pub(crate) fn conv_constraint_def(
    c: &mut Ctx,
    span: Span,
    src: &str,
) -> Result<DefKind, String> {
    Ok(DefKind::Constraint(parse_constraint_src(c, span, src)?))
}

// ---------------------------------------------------------------------------
// Typen
// ---------------------------------------------------------------------------

fn conv_kind(k: &g::Kind) -> Result<Kind, String> {
    match k {
        g::Kind::Int => Ok(Kind::Int),
        // `Nat` existiert im Core nicht; beide sind Int-Sorte mit
        // Nichtnegativitäts-Invariante (siehe ARCHITECTURE.md).
        g::Kind::Nat => Ok(Kind::Int),
        g::Kind::Type => Ok(Kind::Type),
        g::Kind::Bool => Ok(Kind::Bool),
        g::Kind::Order => Err("kind `Order` has no sail-ast core slot".to_owned()),
    }
}

fn conv_quant_items(c: &mut Ctx, span: Span, q: &Option<g::TypQuant>, roots: &[(&g::Typ, g::Kind)]) -> Result<Vec<QuantItem>, String> {
    let Some(q) = q else { return Ok(Vec::new()); };
    let inferred = crate::kinds::infer(q, roots, &c.bound_kinds, c.fixities, c.constructors)?;
    c.bound_kinds.extend(inferred.clone());
    let mut out = Vec::new();
    for kopt in &q.kopts {
        for kid in &kopt.kids {
            let kind = &inferred[kid];
            if *kind == g::Kind::Order { continue; }
            out.push(QuantItem::Id(KOpt { kind: conv_kind(kind)?, kid: c.interner.intern(kid) }));
        }
        for kid in &kopt.kids {
            if inferred[kid] == g::Kind::Nat {
                let kid = c.interner.intern(kid);
                let variable = c.node(span, NExpKind::Var(kid));
                let zero = c.node(span, NExpKind::Constant("0".into()));
                let constraint = c.node(span, ConstraintKind::Ge(variable, zero));
                out.push(QuantItem::Constraint(constraint));
            }
        }
    }
    if let Some(t) = &q.constraint {
        out.push(QuantItem::Constraint(conv_constraint(c, span, t)?));
    }
    Ok(out)
}

fn conv_typschm(c: &mut Ctx, span: Span, s: &g::TypSchm) -> Result<TypSchm, String> {
    let saved_kinds = c.bound_kinds.clone();
    let result = (|| {
    // Legacy effect annotations are ignored by OCaml initial_check, too;
    // effects are inferred from bodies and extern purity in later stages.
    let quant = conv_quant_items(c, span, &s.quant, &[(&s.lhs, g::Kind::Type), (&s.rhs, g::Kind::Type)])?;
    // Binär abbilden (rechtsgenestete Pfeile bleiben curried `Fn`-Knoten).
    let typ = match &s.arrow {
        g::Arrow::Fn => {
            let arg = conv_typ(c, span, &s.lhs)?;
            let args = match arg.kind {
                TypKind::Tuple(args) => args,
                _ => vec![arg],
            };
            let ret = conv_typ(c, span, &s.rhs)?;
            c.node(span, TypKind::Fn {
                args,
                ret: Box::new(ret),
            })
        }
        g::Arrow::Bidir => {
            let lhs = conv_typ(c, span, &s.lhs)?;
            let rhs = conv_typ(c, span, &s.rhs)?;
            c.node(span, TypKind::Bidir {
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            })
        }
    };
    Ok(TypSchm { quant, typ })
    })();
    c.bound_kinds = saved_kinds;
    result
}

fn conv_typ(c: &mut Ctx, span: Span, t: &g::Typ) -> Result<Typ, String> {
    let kind = match t {
        g::Typ::Id(s) => TypKind::Id(c.name(span, s)),
        g::Typ::Wild => TypKind::Unknown,
        g::Typ::Var(s) => TypKind::Var(c.interner.intern(s)),
        g::Typ::App(f, args) => {
            let mut cargs = Vec::with_capacity(args.len());
            let kinds = c.constructors.signature(f, args.len())?.map(|(kinds, _)| kinds);
            if kinds.as_ref().is_some_and(|kinds| kinds.len() != args.len()) {
                return Err(format!("incorrect type argument count for {f}"));
            }
            for (i, a) in args.iter().enumerate() {
                let arg = match kinds.as_ref().map(|kinds| &kinds[i]) {
                    Some(g::Kind::Order) => {
                        let valid = match a {
                            g::Typ::Dec | g::Typ::Inc => true,
                            g::Typ::Var(name) => c.bound_kinds.get(name) == Some(&g::Kind::Order),
                            _ => false,
                        };
                        if !valid { return Err(format!("invalid Order argument for {f}")); }
                        continue;
                    }
                    Some(g::Kind::Type) => { let typ = conv_typ(c, span, a)?; c.node(span, TypArgKind::Typ(typ)) }
                    Some(g::Kind::Int | g::Kind::Nat) => { let nexp = conv_nexp(c, span, a)?; c.node(span, TypArgKind::NExp(nexp)) }
                    Some(g::Kind::Bool) => { let constraint = conv_constraint(c, span, a)?; c.node(span, TypArgKind::Bool(Box::new(constraint))) }
                    None => conv_typarg(c, span, a)?,
                };
                cargs.push(arg);
            }
            TypKind::App {
                name: c.name(span, match f.as_str() { "int" => "atom", "bool" => "atom_bool", _ => f }),
                args: cargs,
            }
        }
        g::Typ::Tuple(ts) => {
            let mut cts = Vec::with_capacity(ts.len());
            for x in ts {
                cts.push(conv_typ(c, span, x)?);
            }
            TypKind::Tuple(cts)
        }
        g::Typ::Fn(a, b) => {
            let arg = conv_typ(c, span, a)?;
            let args = match arg.kind { TypKind::Tuple(args) => args, _ => vec![arg] };
            TypKind::Fn { args, ret: Box::new(conv_typ(c, span, b)?) }
        }
        g::Typ::Exist(kopts, cond, body) => {
            let quant = g::TypQuant { kopts: kopts.clone(), constraint: Some(cond.clone()) };
            let inferred = crate::kinds::infer(&quant, &[(body, g::Kind::Type)], &c.bound_kinds, c.fixities, c.constructors)?;
            let saved = c.bound_kinds.clone();
            c.bound_kinds.extend(inferred.clone());
            let result: Result<TypKind, String> = (|| {
                let mut binders = Vec::new();
                for kopt in kopts {
                    for kid in &kopt.kids {
                        if inferred[kid] == g::Kind::Order { continue; }
                        binders.push(KOpt { kind: conv_kind(&inferred[kid])?, kid: c.interner.intern(kid) });
                    }
                }
                Ok(TypKind::Exist { binders,
                    constraint: Box::new(conv_constraint(c, span, cond)?),
                    typ: Box::new(conv_typ(c, span, body)?),
                })
            })();
            c.bound_kinds = saved;
            result?
        }
        g::Typ::NSet(members) => {
            // OCaml to_ast_typ: a finite integer type is an existential
            // atom constrained to the listed members. The binder is local
            // to this generated type and cannot capture outer variables.
            let kid = c.interner.intern("'n");
            let variable = c.node(span, NExpKind::Var(kid));
            let constraint = c.node(span, ConstraintKind::Set { nexp: variable, members: members.clone() });
            let variable = c.node(span, NExpKind::Var(kid));
            let argument = c.node(span, TypArgKind::NExp(variable));
            let name = c.name(span, "atom");
            let typ = c.node(span, TypKind::App { name, args: vec![argument] });
            TypKind::Exist {
                binders: vec![KOpt { kind: Kind::Int, kid }],
                constraint: Box::new(constraint), typ: Box::new(typ),
            }
        }
        g::Typ::Lit(_)
        | g::Typ::Dec
        | g::Typ::Inc
        | g::Typ::If(..)
        | g::Typ::Infix(_) => {
            return Err("type form needs elaboration, no sail-ast core node (see ARCHITECTURE.md)".to_owned());
        }
    };
    Ok(c.node(span, kind))
}

fn conv_typarg(c: &mut Ctx, span: Span, t: &g::Typ) -> Result<TypArg, String> {
    let kind = match t {
        g::Typ::Lit(g::Lit::Num(s)) => TypArgKind::NExp(c.node(span, NExpKind::Constant(s.clone()))),
        g::Typ::Var(s) => match c.bound_kinds.get(s) {
            Some(g::Kind::Type) => TypArgKind::Typ(conv_typ(c, span, t)?),
            Some(g::Kind::Bool) => TypArgKind::Bool(Box::new(conv_constraint(c, span, t)?)),
            Some(g::Kind::Order) => return Err("Order argument needs a declared constructor signature".into()),
            // Free type variables in standalone fragments lack their enclosing
            // declaration context. Complete elaboration must resolve them.
            _ => { let sym = c.interner.intern(s); TypArgKind::NExp(c.node(span, NExpKind::Var(sym))) }
        },
        g::Typ::Lit(g::Lit::True | g::Lit::False) => TypArgKind::Bool(Box::new(conv_constraint(c, span, t)?)),
        g::Typ::Id(name) | g::Typ::App(name, _) if c.constructors.signature(name,
            match t { g::Typ::App(_, args) => args.len(), _ => 0 })?.is_some() => {
            let arity = match t { g::Typ::App(_, args) => args.len(), _ => 0 };
            let (_, result) = c.constructors.signature(name, arity)?.expect("known signature");
            match result {
                g::Kind::Type => TypArgKind::Typ(conv_typ(c, span, t)?),
                g::Kind::Int | g::Kind::Nat => TypArgKind::NExp(conv_nexp(c, span, t)?),
                g::Kind::Bool => TypArgKind::Bool(Box::new(conv_constraint(c, span, t)?)),
                g::Kind::Order => return Err("Order argument needs a declared constructor signature".into()),
            }
        }
        g::Typ::Id(_) | g::Typ::App(..) | g::Typ::Infix(_) | g::Typ::If(..) => {
            match conv_nexp(c, span, t) {
                Ok(n) => TypArgKind::NExp(n),
                // Keine NExp-Lesart (z.B. mehrgliedrige Kette): als Typ.
                Err(_) => TypArgKind::Typ(conv_typ(c, span, t)?),
            }
        }
        _ => TypArgKind::Typ(conv_typ(c, span, t)?),
    };
    Ok(c.node(span, kind))
}

fn conv_nexp(c: &mut Ctx, span: Span, t: &g::Typ) -> Result<NExp, String> {
    let kind = match t {
        g::Typ::Id(s) => NExpKind::Id(c.name(span, s)),
        g::Typ::Var(s) => NExpKind::Var(c.interner.intern(s)),
        g::Typ::Lit(g::Lit::Num(s)) => NExpKind::Constant(s.clone()),
        g::Typ::App(f, args) => {
            let mut cargs = Vec::with_capacity(args.len());
            for a in args {
                cargs.push(conv_nexp(c, span, a)?);
            }
            NExpKind::App {
                func: c.name(span, f),
                args: cargs,
            }
        }
        g::Typ::Infix(chain) => return infix_nexp(c, span, chain),
        g::Typ::If(cond, th, el) => NExpKind::If {
            cond: Box::new(conv_constraint(c, span, cond)?),
            then_branch: Box::new(conv_nexp(c, span, th)?),
            else_branch: Box::new(conv_nexp(c, span, el)?),
        },
        _ => return Err("not a numeric expression (see ARCHITECTURE.md)".to_owned()),
    };
    Ok(c.node(span, kind))
}

fn infix_nexp(c: &mut Ctx, span: Span, chain: &g::InfixChain<g::Typ>) -> Result<NExp, String> {
    let folded = fold_chain_borrowed(chain, Side::Typ, c.fixities)?;
    folded_nexp(c, span, &folded)
}

fn folded_nexp(c: &mut Ctx, span: Span, tree: &Folded<&g::Typ>) -> Result<NExp, String> {
    let kind = match tree {
        Folded::Leaf(typ) => return conv_nexp(c, span, typ),
        Folded::Prefix { op, arg } => match op.as_str() {
            "pow2" => NExpKind::Exp(Box::new(folded_nexp(c, span, arg)?)),
            "negate" => NExpKind::Neg(Box::new(folded_nexp(c, span, arg)?)),
            _ => return Err(format!("invalid numeric prefix {op}")),
        },
        Folded::Infix { op, lhs, rhs } => {
            let lhs = folded_nexp(c, span, lhs)?;
            let rhs = folded_nexp(c, span, rhs)?;
            match op.as_str() {
                "+" => NExpKind::Sum(Box::new(lhs), Box::new(rhs)),
                "-" => NExpKind::Minus(Box::new(lhs), Box::new(rhs)),
                "*" => NExpKind::Times(Box::new(lhs), Box::new(rhs)),
                _ => return Err(format!("numeric infix {op} needs a declared type constructor")),
            }
        }
    };
    Ok(c.node(span, kind))
}

fn conv_constraint(c: &mut Ctx, span: Span, t: &g::Typ) -> Result<Constraint, String> {
    let kind = match t {
        g::Typ::Id(s) if s == "true" => ConstraintKind::True,
        g::Typ::Id(s) if s == "false" => ConstraintKind::False,
        g::Typ::Id(s) => ConstraintKind::Id(c.name(span, s)),
        g::Typ::Var(s) => ConstraintKind::Var(c.interner.intern(s)),
        g::Typ::Lit(g::Lit::True) => ConstraintKind::True,
        g::Typ::Lit(g::Lit::False) => ConstraintKind::False,
        g::Typ::App(f, args) if f == "not" => {
            let [argument] = args.as_slice() else {
                return Err("not expects one Bool argument".into());
            };
            let argument = conv_constraint(c, span, argument)?;
            let argument = c.node(span, TypArgKind::Bool(Box::new(argument)));
            ConstraintKind::App { func: c.name(span, f), args: vec![argument] }
        }
        g::Typ::If(condition, then_branch, else_branch) => {
            // initial_check lowers Boolean conditionals to
            // (condition & then_branch) | (not(condition) & else_branch).
            let positive = conv_constraint(c, span, condition)?;
            let then_branch = conv_constraint(c, span, then_branch)?;
            let positive = c.node(span, ConstraintKind::And(Box::new(positive), Box::new(then_branch)));
            let condition = conv_constraint(c, span, condition)?;
            let argument = c.node(span, TypArgKind::Bool(Box::new(condition)));
            let name = c.name(span, "not");
            let negative = c.node(span, ConstraintKind::App { func: name, args: vec![argument] });
            let else_branch = conv_constraint(c, span, else_branch)?;
            let negative = c.node(span, ConstraintKind::And(Box::new(negative), Box::new(else_branch)));
            ConstraintKind::Or(Box::new(positive), Box::new(negative))
        }
        g::Typ::App(f, args) => {
            let mut cargs = Vec::with_capacity(args.len());
            for a in args {
                cargs.push(conv_typarg(c, span, a)?);
            }
            ConstraintKind::App {
                func: c.name(span, f),
                args: cargs,
            }
        }
        g::Typ::Infix(chain) => return infix_constraint(c, span, chain),
        _ => return Err("not a constraint form (see ARCHITECTURE.md)".to_owned()),
    };
    Ok(c.node(span, kind))
}

fn infix_constraint(c: &mut Ctx, span: Span, chain: &g::InfixChain<g::Typ>) -> Result<Constraint, String> {
    let folded = fold_chain_borrowed(chain, Side::Typ, c.fixities)?;
    folded_constraint(c, span, &folded)
}

fn folded_constraint(c: &mut Ctx, span: Span, tree: &Folded<&g::Typ>) -> Result<Constraint, String> {
    let Folded::Infix { op, lhs, rhs } = tree else {
        return match tree {
            Folded::Leaf(typ) => conv_constraint(c, span, typ),
            _ => Err("numeric prefix in a boolean constraint".into()),
        };
    };
    let kind = match op.as_str() {
        "=" | "==" | "!=" => {
            let lhs = folded_nexp(c, span, lhs)?;
            let rhs = folded_nexp(c, span, rhs)?;
            let lhs = Box::new(c.node(span, TypArgKind::NExp(lhs)));
            let rhs = Box::new(c.node(span, TypArgKind::NExp(rhs)));
            if op == "!=" { ConstraintKind::NotEqual(lhs, rhs) }
            else { ConstraintKind::Equal(lhs, rhs) }
        }
        ">=" => ConstraintKind::Ge(folded_nexp(c, span, lhs)?, folded_nexp(c, span, rhs)?),
        ">" => ConstraintKind::Gt(folded_nexp(c, span, lhs)?, folded_nexp(c, span, rhs)?),
        "<=" => ConstraintKind::Le(folded_nexp(c, span, lhs)?, folded_nexp(c, span, rhs)?),
        "<" => ConstraintKind::Lt(folded_nexp(c, span, lhs)?, folded_nexp(c, span, rhs)?),
        "&" => ConstraintKind::And(Box::new(folded_constraint(c, span, lhs)?), Box::new(folded_constraint(c, span, rhs)?)),
        "|" => ConstraintKind::Or(Box::new(folded_constraint(c, span, lhs)?), Box::new(folded_constraint(c, span, rhs)?)),
        "in" => {
            let Folded::Leaf(g::Typ::NSet(members)) = rhs.as_ref() else {
                return Err("`in` needs a set literal on the right".into());
            };
            ConstraintKind::Set { nexp: folded_nexp(c, span, lhs)?, members: members.clone() }
        }
        _ => return Err(format!("constraint operator `{op}` needs a declared type constructor")),
    };
    Ok(c.node(span, kind))
}

// ---------------------------------------------------------------------------
// Muster
// ---------------------------------------------------------------------------

fn conv_pat(c: &mut Ctx, span: Span, p: &g::Pat) -> Result<Pat, String> {
    let kind = match p {
        g::Pat::Wild => PatKind::Wild,
        g::Pat::Lit(l) => PatKind::Lit(c.lit(span, l)),
        g::Pat::Id(s) => PatKind::Id(c.name(span, s)),
        g::Pat::KidVar(s) => {
            let sym = c.interner.intern(s);
            let pat = c.node(span, PatKind::Wild);
            let typ_pat = c.node(span, TypPatKind::Var(sym));
            PatKind::Var {
                pat: Box::new(pat),
                typ_pat,
            }
        }
        g::Pat::App(f, args) => {
            let mut cargs = Vec::with_capacity(args.len());
            for a in args {
                cargs.push(conv_pat(c, span, a)?);
            }
            PatKind::App {
                func: c.name(span, f),
                args: cargs,
            }
        }
        g::Pat::VectorSubrange(id, lo, hi) => PatKind::VectorSubrange {
            id: c.name(span, id),
            lo: lo.clone(),
            hi: hi.clone(),
        },
        g::Pat::Tuple(ps) => {
            let mut cps = Vec::with_capacity(ps.len());
            for x in ps {
                cps.push(conv_pat(c, span, x)?);
            }
            PatKind::Tuple(cps)
        }
        g::Pat::Vector(ps) => {
            let mut cps = Vec::with_capacity(ps.len());
            for x in ps {
                cps.push(conv_pat(c, span, x)?);
            }
            PatKind::Vector(cps)
        }
        g::Pat::List(ps) => {
            let mut cps = Vec::with_capacity(ps.len());
            for x in ps {
                cps.push(conv_pat(c, span, x)?);
            }
            PatKind::List(cps)
        }
        g::Pat::Struct(name, fields) => {
            let name = match name {
                Some(n) => StructName::Id(c.name(span, n)),
                None => StructName::Anon,
            };
            let mut cfields = Vec::new();
            let mut wildcard = false;
            for f in fields {
                match f {
                    g::FieldPat::Field(n, q) => {
                        cfields.push((c.name(span, n), conv_pat(c, span, q)?));
                    }
                    g::FieldPat::Wild => wildcard = true,
                }
            }
            PatKind::Struct {
                name,
                fields: cfields,
                wildcard,
            }
        }
        g::Pat::Typ(q, t) => PatKind::Typ {
            typ: conv_typ(c, span, t)?,
            pat: Box::new(conv_pat(c, span, q)?),
        },
        g::Pat::Infix(head, tail) => return infix_pat(c, span, head, tail),
        // Oberflächen-`as` trägt einen `Typ`; nur Namen/Variablen sind als
        // Bindung darstellbar (siehe ARCHITECTURE.md).
        g::Pat::As(q, t) => match t {
            g::Typ::Id(s) | g::Typ::Var(s) => PatKind::As {
                pat: Box::new(conv_pat(c, span, q)?),
                id: c.name(span, s),
            },
            _ => return Err("`as` with non-name has no sail-ast core node".to_owned()),
        },
        // Attribute haben im Core keinen Muster-Slot → Hülle entfällt.
        g::Pat::Attr(attrs, q) => {
            let mut pat = conv_pat(c, span, q)?;
            pat.annotation_mut().attributes.splice(0..0, attributes(attrs));
            return Ok(pat);
        },
    };
    Ok(c.node(span, kind))
}

/// Ein-Operator-Ketten (`@` → concat, `::` → rechtsgenestetes Cons,
/// `^` → append); gemischte Ketten brauchen Faltung.
fn infix_pat(
    c: &mut Ctx,
    span: Span,
    head: &g::Pat,
    tail: &[(g::PatOp, g::Pat)],
) -> Result<Pat, String> {
    let mut items = Vec::with_capacity(tail.len() + 1);
    items.push(conv_pat(c, span, head)?);
    let mut op0: Option<&g::PatOp> = None;
    for (op, q) in tail {
        if op0.replace(op).is_some_and(|first| first != op) {
            return Err("mixed pattern operators need elaboration-time folding".to_owned());
        }
        items.push(conv_pat(c, span, q)?);
    }
    let kind = match op0 {
        None => return Err("empty pattern infix chain".to_owned()),
        Some(g::PatOp::VectorConcat) => PatKind::VectorConcat(items),
        Some(g::PatOp::StringAppend) => PatKind::StringAppend(items),
        Some(g::PatOp::Cons) => {
            let mut it = items.into_iter().rev();
            let mut acc = it.next().expect("non-empty");
            for x in it {
                acc = c.node(span, PatKind::Cons(Box::new(x), Box::new(acc)));
            }
            return Ok(acc);
        }
    };
    Ok(c.node(span, kind))
}

fn conv_mpat(c: &mut Ctx, span: Span, p: &g::MPat) -> Result<Pat, String> {
    // Mapping-Muster senken auf Wert-Muster (Approximation, ARCHITECTURE.md).
    let kind = match p {
        g::MPat::Lit(l) => PatKind::Lit(c.lit(span, l)),
        g::MPat::Id(s) => PatKind::Id(c.name(span, s)),
        g::MPat::App(f, args) => {
            let mut cargs = Vec::with_capacity(args.len());
            for a in args {
                cargs.push(conv_mpat(c, span, a)?);
            }
            PatKind::App {
                func: c.name(span, f),
                args: cargs,
            }
        }
        g::MPat::VectorSubrange(id, lo, hi) => PatKind::VectorSubrange {
            id: c.name(span, id),
            lo: lo.clone(),
            hi: hi.clone(),
        },
        g::MPat::Tuple(ps) => {
            let mut cps = Vec::with_capacity(ps.len());
            for x in ps {
                cps.push(conv_mpat(c, span, x)?);
            }
            PatKind::Tuple(cps)
        }
        g::MPat::Vector(ps) => {
            let mut cps = Vec::with_capacity(ps.len());
            for x in ps {
                cps.push(conv_mpat(c, span, x)?);
            }
            PatKind::Vector(cps)
        }
        g::MPat::List(ps) => {
            let mut cps = Vec::with_capacity(ps.len());
            for x in ps {
                cps.push(conv_mpat(c, span, x)?);
            }
            PatKind::List(cps)
        }
        g::MPat::Struct(name, fields) => {
            let name = match name {
                Some(n) => StructName::Id(c.name(span, n)),
                None => StructName::Anon,
            };
            let mut cfields = Vec::with_capacity(fields.len());
            for (n, q) in fields {
                cfields.push((c.name(span, n), conv_mpat(c, span, q)?));
            }
            PatKind::Struct {
                name,
                fields: cfields,
                wildcard: false,
            }
        }
        g::MPat::Typ(q, t) => PatKind::Typ {
            typ: conv_typ(c, span, t)?,
            pat: Box::new(conv_mpat(c, span, q)?),
        },
        g::MPat::Infix(head, tail) => {
            let mut items = Vec::with_capacity(tail.len() + 1);
            items.push(conv_mpat(c, span, head)?);
            let mut op0: Option<&g::PatOp> = None;
            for (op, q) in tail {
                if op0.replace(op).is_some_and(|first| first != op) {
                    return Err("mixed mapping-pattern operators need folding".to_owned());
                }
                items.push(conv_mpat(c, span, q)?);
            }
            match op0 {
                None => return Err("empty mapping-pattern infix chain".to_owned()),
                Some(g::PatOp::VectorConcat) => PatKind::VectorConcat(items),
                Some(g::PatOp::StringAppend) => PatKind::StringAppend(items),
                Some(g::PatOp::Cons) => {
                    let mut it = items.into_iter().rev();
                    let mut acc = it.next().expect("non-empty");
                    for x in it {
                        acc = c.node(span, PatKind::Cons(Box::new(x), Box::new(acc)));
                    }
                    return Ok(acc);
                }
            }
        }
        g::MPat::As(q, s) => PatKind::As {
            pat: Box::new(conv_mpat(c, span, q)?),
            id: c.name(span, s),
        },
    };
    Ok(c.node(span, kind))
}

// ---------------------------------------------------------------------------
// Ausdrücke
// ---------------------------------------------------------------------------

/// `to_ast_assign` distinguishes a complete application from an l-value.
/// Infix parsing can wrap the application in a one-operand chain. Calls
/// inside fields or vector accesses remain invalid assignment targets.
fn assignment_call(mut exp: &g::Exp) -> Option<&g::Exp> {
    loop {
        match exp {
            g::Exp::App(..) => return Some(exp),
            g::Exp::Infix(chain) if chain.prefixes.is_empty() && chain.tail.is_empty() => exp = &chain.head,
            g::Exp::Suffix(_, suffixes) if matches!(suffixes.last(), Some(g::ExpSuffix::Call(..))) => return Some(exp),
            _ => return None,
        }
    }
}

/// Oberflächen-Ausdruck als L-Wert lesen (Teilmenge mit Core-Slots).
fn exp_to_lexp(c: &mut Ctx, span: Span, e: &g::Exp) -> Result<LExp, String> {
    let kind = match e {
        g::Exp::Id(s) => LExpKind::Id(c.name(span, s)),
        g::Exp::Suffix(head, sufs) => {
            let mut acc = exp_to_lexp(c, span, head)?;
            for suffix in sufs {
                let kind = match suffix {
                    g::ExpSuffix::Field(field) => LExpKind::Field {
                        lexp: Box::new(acc), field: c.name(span, field),
                    },
                    g::ExpSuffix::Index(index) => LExpKind::Vector {
                        lexp: Box::new(acc), index: Box::new(conv_exp(c, span, index)?),
                    },
                    g::ExpSuffix::Subrange(lo, hi) => LExpKind::VectorRange {
                        lexp: Box::new(acc), lo: Box::new(conv_exp(c, span, lo)?),
                        hi: Box::new(conv_exp(c, span, hi)?),
                    },
                    g::ExpSuffix::TypAnnot(typ) => {
                        let LExpKind::Id(id) = acc.kind else {
                            return Err("annotated lexp needs a plain identifier".into());
                        };
                        LExpKind::Typ { typ: conv_typ(c, span, typ)?, id }
                    }
                    g::ExpSuffix::Call(..) | g::ExpSuffix::Slice(..) =>
                        return Err("function calls cannot be assignment targets".into()),
                };
                acc = c.node(span, kind);
            }
            return Ok(acc);
        }
        g::Exp::Tuple(es) => {
            let mut ces = Vec::with_capacity(es.len());
            for x in es {
                let target = exp_to_lexp(c, span, x)?;
                if matches!(target.kind, LExpKind::Deref(_)) {
                    return Err("dereferences cannot be set in a tuple assignment".into());
                }
                ces.push(target);
            }
            LExpKind::Tuple(ces)
        }
        g::Exp::Infix(chain) => {
            let tree = fold_chain_borrowed(chain, Side::Exp, c.fixities)?;
            return folded_lexp(c, span, &tree);
        }
        _ => return Err("expression is not a valid l-value".to_owned()),
    };
    Ok(c.node(span, kind))
}

// L-values use the same precedence and prefix folding as expressions.
fn folded_lexp(c: &mut Ctx, span: Span, tree: &Folded<&g::Exp>) -> Result<LExp, String> {
    let kind = match tree {
        Folded::Leaf(exp) => return exp_to_lexp(c, span, exp),
        Folded::Prefix { op, arg } if op == "__deref" =>
            LExpKind::Deref(Box::new(folded_exp(c, span, arg)?)),
        Folded::Infix { op, lhs, rhs } if op == "@" => {
            let mut parts = Vec::new();
            for part in [folded_lexp(c, span, lhs)?, folded_lexp(c, span, rhs)?] {
                match part.kind {
                    LExpKind::VectorConcat(items) => parts.extend(items),
                    _ => parts.push(part),
                }
            }
            LExpKind::VectorConcat(parts)
        }
        _ => return Err("expression is not a valid l-value".into()),
    };
    Ok(c.node(span, kind))
}

fn conv_fexp_field(c: &mut Ctx, span: Span, f: &g::FieldExp, ready: &mut ExpResults) -> Result<FExp, String> {
    match f {
        // Annahme: erstes Element ist der Feld-Designator (Id), zweites
        // der Wert (siehe ARCHITECTURE.md).
        g::FieldExp::Field(designator, value) => {
            let g::Exp::Id(n) = &**designator else {
                return Err("computed struct field has no sail-ast core node".to_owned());
            };
            Ok(FExp {
                id: c.ids.fresh(),
                span,
                field: c.name(span, n),
                value: take_exp(ready, value)?,
            })
        }
        // Punning `f` = `{ f = f }` (Desugar, dokumentiert).
        g::FieldExp::Punning(n) => {
            let id = c.ids.fresh();
            let field = c.name(span, n);
            let id_name = c.name(span, n);
            let value = c.node(span, ExpKind::Id(id_name));
            Ok(FExp {
                id,
                span,
                field,
                value,
            })
        }
    }
}

fn conv_patexp(c: &mut Ctx, span: Span, mut p: &g::PatExp, mut ready: Option<&mut ExpResults>) -> Result<PExp, String> {
    while let g::PatExp::Attr(_, inner) = p { p = inner; }
    let kind = match p {
        // Annahme: Tupel-Reihenfolge ist (Muster, Rumpf) bzw.
        // (Muster, Guard, Rumpf) (siehe ARCHITECTURE.md).
        g::PatExp::Exp(q, body) => PExpKind::PatExp {
            pat: conv_pat(c, span, q)?,
            body: match &mut ready { Some(ready) => take_exp(ready, body)?, None => conv_exp(c, span, body)? },
        },
        g::PatExp::When(q, guard, body) => PExpKind::PatWhen {
            pat: conv_pat(c, span, q)?,
            guard: match &mut ready { Some(ready) => take_exp(ready, guard)?, None => conv_exp(c, span, guard)? },
            body: match &mut ready { Some(ready) => take_exp(ready, body)?, None => conv_exp(c, span, body)? },
        },
        g::PatExp::Attr(..) => unreachable!(),
    };
    Ok(PExp {
        id: c.ids.fresh(),
        span,
        kind,
    })
}

fn folded_exp(c: &mut Ctx, span: Span, tree: &Folded<&g::Exp>) -> Result<Exp, String> {
    folded_exp_ready(c, span, tree, None)
}

fn folded_exp_ready(c: &mut Ctx, span: Span, tree: &Folded<&g::Exp>, mut ready: Option<&mut ExpResults>) -> Result<Exp, String> {
    let mut pending = vec![(tree, false)];
    let mut values = Vec::new();
    while let Some((tree, finish)) = pending.pop() {
        if !finish {
            match tree {
                Folded::Leaf(exp) => values.push(match &mut ready {
                    Some(ready) => take_exp(ready, exp)?, None => conv_exp(c, span, exp)?,
                }),
                Folded::Prefix { arg, .. } => {
                    pending.push((tree, true)); pending.push((arg, false));
                }
                Folded::Infix { lhs, rhs, .. } => {
                    pending.push((tree, true)); pending.push((rhs, false)); pending.push((lhs, false));
                }
            }
        } else {
            let kind = match tree {
                Folded::Prefix { op, .. } => ExpKind::App {
                    func: c.name(span, op), args: vec![values.pop().expect("prefix operand")],
                },
                Folded::Infix { op, .. } => {
                    let rhs = values.pop().expect("right operand");
                    let lhs = values.pop().expect("left operand");
                    match op.as_str() {
                        "@" => ExpKind::VectorAppend(Box::new(lhs), Box::new(rhs)),
                        "::" => ExpKind::Cons(Box::new(lhs), Box::new(rhs)),
                        _ => ExpKind::App { func: c.name(span, op), args: vec![lhs, rhs] },
                    }
                }
                Folded::Leaf(_) => unreachable!(),
            };
            values.push(c.node(span, kind));
        }
    }
    Ok(values.pop().expect("nonempty infix expression"))
}

fn drop_folded<T>(tree: Folded<T>) {
    let mut pending = vec![tree];
    while let Some(tree) = pending.pop() {
        match tree {
            Folded::Leaf(_) => {}
            Folded::Prefix { arg, .. } => pending.push(*arg),
            Folded::Infix { lhs, rhs, .. } => { pending.push(*lhs); pending.push(*rhs); }
        }
    }
}

// Node addresses are local lookup keys only, never persisted or iterated for
// output order. Each expression child is moved exactly once into its parent.
type ExpResults = std::collections::HashMap<*const g::Exp, Exp>;

fn take_exp(ready: &mut ExpResults, exp: &g::Exp) -> Result<Exp, String> {
    ready.remove(&(exp as *const g::Exp))
        .ok_or_else(|| "internal error: missing postorder expression child".into())
}

/// True, wenn `e` ein `()`-Literal ist (leerer Körper einer Block-Deklaration
/// in alter Geschwister-Ablage bzw. explizites `in ()`).
fn is_unit_body(e: &Exp) -> bool {
    matches!(&e.kind, ExpKind::Lit(l) if matches!(l.kind, LitKind::Unit))
}

/// Bare `let`/`var` mit leerem Körper: alte Grammatikform legt Deklaration
/// und Rest-Block als Geschwister ab (`[Let(p,e,()), rest...]`). Die
/// Lane-03-Form liefert bereits `Let/Var(p, e, Block(rest))` — dann ist der
/// Körper kein `Unit` und es wird nichts gefaltet.
fn is_bare_decl(e: &Exp) -> bool {
    match &e.kind {
        ExpKind::Let { body, .. } | ExpKind::Var { body, .. } => is_unit_body(body),
        _ => false,
    }
}

/// Faltet alte Geschwister-Form `[Let(p,e,()), rest...]` zu
/// `[Let(p,e, Block(rest'))]` (OCaml `parser.mly`, Produktion `block`:
/// `rest` ist der Körper der Deklaration, kein Geschwister). Neue Form mit
/// `Block`-Körper bleibt unverändert. IDs/Spans/Annotationen der Deklaration
/// bleiben erhalten; der Rest-Block wird als neuer `Block`-Knoten mit dem
/// Def-/Fragment-Span verpackt. Iterativ von hinten, sodass Ketten
/// `{ let a; let b; x }` zu `Let(a, Block([Let(b, Block([x]))]))` werden.
/// Hinweis: Die Oberfläche unterscheidet `{ let x = e; rest }` nicht von
/// `{ let x = e in (); rest }` (beide `Let((), ...)`); Faltung nimmt die
/// Deklaration an (üblicher Fall, vgl. OCaml-`block`).
fn fold_block_decls(c: &mut Ctx, span: Span, items: Vec<Exp>) -> Vec<Exp> {
    use std::collections::VecDeque;
    if items.is_empty() {
        return items;
    }
    let mut rest: VecDeque<Exp> = VecDeque::with_capacity(items.len());
    for mut head in items.into_iter().rev() {
        if !rest.is_empty() && is_bare_decl(&head) {
            let tail: Vec<Exp> = std::mem::take(&mut rest).into_iter().collect();
            let body = c.node(span, ExpKind::Block(tail));
            match &mut head.kind {
                ExpKind::Let { body: b, .. } | ExpKind::Var { body: b, .. } => **b = body,
                _ => unreachable!("is_bare_decl geprüft"),
            }
            rest.push_front(head);
        } else {
            rest.push_front(head);
        }
    }
    rest.into_iter().collect()
}

/// Enumerate only expression children consumed by finish_exp. Assignment
/// targets are handled as l-values, except whole setter applications.
fn expression_children<'a>(exp: &'a g::Exp, children: &mut Vec<&'a g::Exp>) {
    use g::Exp as E;
    match exp {
        E::Id(_) | E::Ref(_) | E::Lit(_) | E::Undef | E::Config(_)
        | E::Sizeof(_) | E::Constraint(_) => {}
        E::Exit(x) | E::Return(x) | E::Throw(x) | E::InternalReturn(x)
        | E::InternalAssume(_, x) | E::Attr(_, x) => children.push(x),
        E::Assert(x, y) | E::Let(_, x, y) | E::InternalPlet(_, x, y)
        | E::Var(_, x, y) => { children.push(x); children.push(y); }
        E::Assign(x, y) => {
            if let Some(call) = assignment_call(x) { children.push(call); }
            children.push(y);
        }
        E::If(x, y, z) => { children.push(x); children.push(y); children.push(z); }
        E::App(_, items) | E::Block(items) | E::Vector(items) | E::List(items)
        | E::Tuple(items) => children.extend(items),
        E::Infix(chain) => {
            children.push(&chain.head);
            children.extend(chain.tail.iter().map(|(_, _, rhs)| rhs));
        }
        E::Suffix(head, suffixes) => {
            children.push(head);
            for suffix in suffixes {
                match suffix {
                    g::ExpSuffix::Field(_) | g::ExpSuffix::TypAnnot(_) => {}
                    g::ExpSuffix::Index(x) => children.push(x),
                    g::ExpSuffix::Subrange(x, y) | g::ExpSuffix::Slice(x, y) => {
                        children.push(x); children.push(y);
                    }
                    g::ExpSuffix::Call(_, args) => children.extend(args),
                }
            }
        }
        E::Match(x, arms) | E::Try(x, arms) => {
            children.push(x);
            for arm in arms {
                let mut arm = arm;
                while let g::PatExp::Attr(_, inner) = arm { arm = inner; }
                match arm {
                    g::PatExp::Exp(_, body) => children.push(body),
                    g::PatExp::When(_, guard, body) => { children.push(guard); children.push(body); }
                    g::PatExp::Attr(..) => unreachable!(),
                }
            }
        }
        E::For(head, body) => {
            if let Some(step) = &head.step { children.push(step); }
            children.push(&head.from); children.push(&head.to); children.push(body);
        }
        E::Loop(_, measure, condition, body) => {
            if let g::Measure::Some(x) = measure { children.push(x); }
            children.push(condition); children.push(body);
        }
        E::Struct(_, fields) | E::StructUpdate(_, fields) => {
            if let E::StructUpdate(x, _) = exp { children.push(x); }
            for field in fields {
                if let g::FieldExp::Field(_, value) = field { children.push(value); }
            }
        }
        E::VectorUpdate(head, updates) => {
            children.push(head);
            for update in updates {
                match update {
                    g::VectorUpdate::Single(index, value) => { children.push(index); children.push(value); }
                    g::VectorUpdate::Range(lo, hi, value) => { children.push(lo); children.push(hi); children.push(value); }
                }
            }
        }
    }
}

fn conv_exp(c: &mut Ctx, span: Span, root: &g::Exp) -> Result<Exp, String> {
    let mut pending = vec![(root, false)];
    let mut ready = ExpResults::new();
    let mut children = Vec::new();
    while let Some((exp, finish)) = pending.pop() {
        if finish {
            let converted = finish_exp(c, span, exp, &mut ready)?;
            ready.insert(exp as *const g::Exp, converted);
        } else {
            pending.push((exp, true));
            expression_children(exp, &mut children);
            pending.extend(children.drain(..).rev().map(|child| (child, false)));
        }
    }
    let result = take_exp(&mut ready, root)?;
    debug_assert!(ready.is_empty(), "postorder conversion left unused children");
    Ok(result)
}

fn finish_exp(c: &mut Ctx, span: Span, e: &g::Exp, ready: &mut ExpResults) -> Result<Exp, String> {
    let kind = match e {
        g::Exp::Id(s) => ExpKind::Id(c.name(span, s)),
        g::Exp::Ref(s) => ExpKind::Ref(c.name(span, s)),
        g::Exp::Lit(l) => ExpKind::Lit(c.lit(span, l)),
        g::Exp::Undef => ExpKind::Undef,
        g::Exp::Config(k) => ExpKind::Config(vec![k.clone()]),
        g::Exp::Exit(x) => ExpKind::Exit(Box::new(take_exp(ready, x)?)),
        g::Exp::Sizeof(t) => ExpKind::Sizeof(conv_nexp(c, span, t)?),
        g::Exp::Constraint(t) => ExpKind::Constraint(conv_constraint(c, span, t)?),
        g::Exp::Assert(x, y) => ExpKind::Assert {
            cond: Box::new(take_exp(ready, x)?),
            msg: Box::new(take_exp(ready, y)?),
        },
        g::Exp::App(f, args) => {
            let mut cargs = Vec::with_capacity(args.len());
            for a in args {
                cargs.push(take_exp(ready, a)?);
            }
            ExpKind::App {
                func: c.name(span, f),
                args: cargs,
            }
        }
        g::Exp::Suffix(head, sufs) => {
            let mut acc = take_exp(ready, head)?;
            for suf in sufs {
                let next = match suf {
                    g::ExpSuffix::Field(f) => ExpKind::Field {
                        record: Box::new(acc),
                        field: c.name(span, f),
                    },
                    g::ExpSuffix::TypAnnot(t) => ExpKind::Typ {
                        typ: conv_typ(c, span, t)?,
                        exp: Box::new(acc),
                    },
                    g::ExpSuffix::Index(index) => ExpKind::App {
                        func: c.name(span, "vector_access#"),
                        args: vec![acc, take_exp(ready, index)?],
                    },
                    g::ExpSuffix::Subrange(lo, hi) | g::ExpSuffix::Slice(lo, hi) => ExpKind::App {
                        func: c.name(span, if matches!(suf, g::ExpSuffix::Slice(..)) { "slice" } else { "vector_subrange#" }),
                        args: vec![acc, take_exp(ready, lo)?, take_exp(ready, hi)?],
                    },
                    g::ExpSuffix::Call(name, args) => {
                        let mut actual = Vec::with_capacity(args.len() + 1);
                        actual.push(acc);
                        for arg in args { actual.push(take_exp(ready, arg)?); }
                        ExpKind::App { func: c.name(span, &format!("_mod_{name}")), args: actual }
                    }
                };
                acc = c.node(span, next);
            }
            return Ok(acc);
        }
        g::Exp::Assign(x, y) => {
            if let Some(call) = assignment_call(x) {
                let application = take_exp(ready, call)?;
                let ExpKind::App { func, mut args } = application.kind else {
                    return Err("setter target must be a function application".into());
                };
                // OCaml treats a sole unit as no arguments and flattens a
                // sole tuple. All other argument lists retain their shape.
                if args.len() == 1 {
                    let only = args.pop().expect("one argument");
                    match only.kind {
                        ExpKind::Lit(Node { kind: LitKind::Unit, .. }) => {}
                        ExpKind::Tuple(items) => args = items,
                        _ => args.push(only),
                    }
                }
                args.push(take_exp(ready, y)?);
                let mut setter = c.node(span, ExpKind::App { func, args });
                setter.annotation_mut().attributes.push(sail_ast::Attribute {
                    name: "setter".into(), data: None,
                });
                return Ok(setter);
            }
            ExpKind::Assign {
                lexp: Box::new(exp_to_lexp(c, span, x)?),
                value: Box::new(take_exp(ready, y)?),
            }
        },
        g::Exp::Let(q, x, y) => ExpKind::Let {
            pat: conv_pat(c, span, q)?,
            bind: Box::new(take_exp(ready, x)?),
            body: Box::new(take_exp(ready, y)?),
        },
        g::Exp::Var(x, y, z) => ExpKind::Var {
            lexp: Box::new(exp_to_lexp(c, span, x)?),
            init: Box::new(take_exp(ready, y)?),
            body: Box::new(take_exp(ready, z)?),
        },
        g::Exp::Block(es) => {
            let mut ces = Vec::with_capacity(es.len());
            for x in es {
                ces.push(take_exp(ready, x)?);
            }
            ExpKind::Block(fold_block_decls(c, span, ces))
        }
        g::Exp::Return(x) => ExpKind::Return(Box::new(take_exp(ready, x)?)),
        g::Exp::Throw(x) => ExpKind::Throw(Box::new(take_exp(ready, x)?)),
        g::Exp::If(x, y, z) => ExpKind::If {
            cond: Box::new(take_exp(ready, x)?),
            then_branch: Box::new(take_exp(ready, y)?),
            else_branch: Box::new(take_exp(ready, z)?),
        },
        g::Exp::Match(x, arms) => {
            let mut carms = Vec::with_capacity(arms.len());
            for a in arms {
                carms.push(conv_patexp(c, span, a, Some(ready))?);
            }
            ExpKind::Match {
                scrutinee: Box::new(take_exp(ready, x)?),
                arms: carms,
            }
        }
        g::Exp::Try(x, arms) => {
            let mut carms = Vec::with_capacity(arms.len());
            for a in arms {
                carms.push(conv_patexp(c, span, a, Some(ready))?);
            }
            ExpKind::Try {
                exp: Box::new(take_exp(ready, x)?),
                arms: carms,
            }
        }
        g::Exp::For(head, body) => {
            let step = match &head.step {
                // Ohne Schritt: `1` (Sail-Default, dokumentiert).
                None => {
                    let one = c.interner.intern("1");
                    let lit = c.node(span, LitKind::Num(one));
                    c.node(span, ExpKind::Lit(lit))
                }
                Some(s) => take_exp(ready, s)?,
            };
            let order = match &head.order {
                // The surface `to`/`downto` determines the implicit order.
                None => match head.ord { g::LoopOrd::To => Order::Inc, g::LoopOrd::Downto => Order::Dec },
                Some(g::Typ::Inc) => Order::Inc,
                Some(g::Typ::Dec) => Order::Dec,
                Some(_) => return Err("foreach order needs `inc`/`dec`".to_owned()),
            };
            ExpKind::For {
                var: c.name(span, &head.var),
                from: Box::new(take_exp(ready, &head.from)?),
                to: Box::new(take_exp(ready, &head.to)?),
                step: Box::new(step),
                order,
                body: Box::new(take_exp(ready, body)?),
            }
        }
        g::Exp::Loop(lk, m, cond, body) => ExpKind::Loop {
            loop_kind: match lk {
                g::LoopKind::While => Loop::While,
                g::LoopKind::Until => Loop::Until,
            },
            measure: match m {
                g::Measure::None => None,
                g::Measure::Some(x) => Some(Box::new(take_exp(ready, x)?)),
            },
            cond: Box::new(take_exp(ready, cond)?),
            body: Box::new(take_exp(ready, body)?),
        },
        g::Exp::Infix(chain) => {
            let folded = fold_chain_borrowed(chain, Side::Exp, c.fixities)?;
            let result = folded_exp_ready(c, span, &folded, Some(ready));
            drop_folded(folded);
            return result;
        }
        g::Exp::Struct(name, fields) => {
            let name = match name {
                Some(n) => StructName::Id(c.name(span, n)),
                None => StructName::Anon,
            };
            let mut cfields = Vec::with_capacity(fields.len());
            for f in fields {
                cfields.push(conv_fexp_field(c, span, f, ready)?);
            }
            ExpKind::Struct { name, fields: cfields }
        }
        g::Exp::StructUpdate(x, fields) => {
            let mut cfields = Vec::with_capacity(fields.len());
            for f in fields {
                cfields.push(conv_fexp_field(c, span, f, ready)?);
            }
            ExpKind::StructUpdate {
                record: Box::new(take_exp(ready, x)?),
                fields: cfields,
            }
        }
        g::Exp::Vector(es) => {
            let mut ces = Vec::with_capacity(es.len());
            for x in es {
                ces.push(take_exp(ready, x)?);
            }
            ExpKind::Vector(ces)
        }
        g::Exp::VectorUpdate(head, updates) => {
            let mut acc = take_exp(ready, head)?;
            for update in updates {
                let (name, args) = match update {
                    g::VectorUpdate::Single(index, value) => ("vector_update#",
                        vec![acc, take_exp(ready, index)?, take_exp(ready, value)?]),
                    g::VectorUpdate::Range(lo, hi, value) => ("vector_update_subrange#",
                        vec![acc, take_exp(ready, lo)?, take_exp(ready, hi)?, take_exp(ready, value)?]),
                };
                let func = c.name(span, name);
                acc = c.node(span, ExpKind::App { func, args });
            }
            return Ok(acc);
        }
        g::Exp::List(es) => {
            let mut ces = Vec::with_capacity(es.len());
            for x in es {
                ces.push(take_exp(ready, x)?);
            }
            ExpKind::List(ces)
        }
        g::Exp::Tuple(es) => {
            let mut ces = Vec::with_capacity(es.len());
            for x in es {
                ces.push(take_exp(ready, x)?);
            }
            ExpKind::Tuple(ces)
        }
        g::Exp::InternalPlet(q, x, y) => ExpKind::InternalPlet {
            pat: conv_pat(c, span, q)?,
            bind: Box::new(take_exp(ready, x)?),
            body: Box::new(take_exp(ready, y)?),
        },
        g::Exp::InternalReturn(x) => ExpKind::InternalReturn(Box::new(take_exp(ready, x)?)),
        g::Exp::InternalAssume(t, x) => ExpKind::InternalAssume {
            constraint: conv_constraint(c, span, t)?,
            exp: Box::new(take_exp(ready, x)?),
        },
        // Retain source attributes on the converted child.
        g::Exp::Attr(attrs, x) => {
            let mut exp = take_exp(ready, x)?;
            exp.annotation_mut().attributes.splice(0..0, attributes(attrs));
            return Ok(exp);
        },
    };
    Ok(c.node(span, kind))
}

// ---------------------------------------------------------------------------
// Klauseln, Definitionen
// ---------------------------------------------------------------------------

fn conv_tannot(
    c: &mut Ctx,
    span: Span,
    t: &Option<(Option<g::TypQuant>, g::Typ)>,
    patterns: &[&g::Pat],
) -> Result<TAnnotOpt, String> {
    let Some((quant, typ)) = t else {
        return Ok(TAnnotOpt::None);
    };
    let mut roots=vec![(typ,g::Kind::Type)];
    let mut work=patterns.to_vec();
    while let Some(pat)=work.pop() {
        match pat {
            g::Pat::Typ(p,t)=>{roots.push((t,g::Kind::Type));work.push(p);}
            g::Pat::Attr(_,p)|g::Pat::As(p,_)=>work.push(p),
            g::Pat::App(_,ps)|g::Pat::Tuple(ps)|g::Pat::List(ps)|g::Pat::Vector(ps)=>work.extend(ps),
            g::Pat::Struct(_,fields)=>work.extend(fields.iter().filter_map(|f|if let g::FieldPat::Field(_,p)=f {Some(p)} else {None})),
            g::Pat::Infix(head,tail)=>{work.push(head);work.extend(tail.iter().map(|(_,p)|p));}
            g::Pat::Wild|g::Pat::Lit(_)|g::Pat::Id(_)|g::Pat::KidVar(_)|g::Pat::VectorSubrange(..)=>{},
        }
    }
    Ok(TAnnotOpt::Some {
        quant: conv_quant_items(c, span, quant, &roots)?,
        typ: conv_typ(c, span, typ)?,
    })
}

fn conv_funcl(c: &mut Ctx, span: Span, f: &g::Funcl) -> Result<Funcl, String> {
    // Klausel-Annotationen (`private`/Attr/Doku) haben im Core keinen
    // `Funcl`-Slot → entfallen (siehe ARCHITECTURE.md).
    let (pat, body_guard) = match &f.pat {
        g::GuardedPat::Plain(p) => (conv_pat(c, span, p)?, None),
        g::GuardedPat::Guarded(p, g) => (conv_pat(c, span, p)?, Some(conv_exp(c, span, g)?)),
    };
    let arm_kind = match &f.body {
        // Annahme: `When` trägt (Guard, Rumpf) (siehe ARCHITECTURE.md).
        g::PatExpBody::Exp(e) => match body_guard {
            None => PExpKind::PatExp {
                pat,
                body: conv_exp(c, span, e)?,
            },
            Some(guard) => PExpKind::PatWhen {
                pat,
                guard,
                body: conv_exp(c, span, e)?,
            },
        },
        g::PatExpBody::When(g, b) => {
            if body_guard.is_some() {
                return Err("clause has two guards".to_owned());
            }
            PExpKind::PatWhen {
                pat,
                guard: conv_exp(c, span, g)?,
                body: conv_exp(c, span, b)?,
            }
        }
    };
    Ok(Funcl {
        id: c.ids.fresh(),
        span,
        name: c.name(span, &f.name),
        arm: PExp {
            id: c.ids.fresh(),
            span,
            kind: arm_kind,
        },
    })
}

/// Mapping-Muster + optionaler Guard zu Muster + optionalem Guard senken.
fn conv_mpexp_parts(
    c: &mut Ctx,
    span: Span,
    m: &g::MPExp,
) -> Result<(Pat, Option<Exp>), String> {
    Ok((
        conv_mpat(c, span, &m.pat)?,
        m.guard
            .as_ref()
            .map(|g| conv_exp(c, span, g))
            .transpose()?,
    ))
}

fn conv_mapcl(c: &mut Ctx, span: Span, m: &g::MapCl) -> Result<Mapcl, String> {
    let kind = match m {
        g::MapCl::Bidir(fwd, bwd) => {
            let mut side = |surface: &g::MPExp| {
                let (pat, guard) = conv_mpexp_parts(c, span, surface)?;
                let kind = match guard {
                    None => MPExpKind::Pat(pat),
                    Some(guard) => MPExpKind::When { pat, guard },
                };
                Ok::<_, String>(c.node(span, kind))
            };
            MapclKind::Bidir { forwards: side(fwd)?, backwards: side(bwd)? }
        }
        g::MapCl::Forwards(p) => MapclKind::Forwards(conv_patexp(c, span, p, None)?),
        g::MapCl::ForwardsDeprecated(mp, body) => {
            let (pat, guard) = conv_mpexp_parts(c, span, mp)?;
            let body = conv_exp(c, span, body)?;
            MapclKind::Forwards(PExp {
                id: c.ids.fresh(),
                span,
                kind: match guard {
                    None => PExpKind::PatExp { pat, body },
                    Some(guard) => PExpKind::PatWhen { pat, guard, body },
                },
            })
        }
        g::MapCl::Backwards(p) => MapclKind::Backwards(conv_patexp(c, span, p, None)?),
        g::MapCl::When(inner, guard) => {
            let guard = conv_exp(c, span, guard)?;
            let inner = conv_mapcl(c, span, inner)?;
            let mut attach = |arm: PExp| -> Result<PExp, String> {
                match arm.kind {
                    PExpKind::PatExp { pat, body } => Ok(PExp {
                        id: c.ids.fresh(),
                        span,
                        kind: PExpKind::PatWhen {
                            pat,
                            guard: guard.clone(),
                            body,
                        },
                    }),
                    PExpKind::PatWhen { .. } => Err("mapping clause has two guards".to_owned()),
                }
            };
            match inner.kind {
                MapclKind::Forwards(a) => MapclKind::Forwards(attach(a)?),
                MapclKind::Backwards(a) => MapclKind::Backwards(attach(a)?),
                MapclKind::Bidir { forwards, backwards } => {
                    let mut attach = |side: sail_ast::MPExp| {
                        let MPExpKind::Pat(pat) = side.kind else {
                            return Err("mapping clause has both an 'if' and a 'when' guard".to_owned());
                        };
                        Ok(c.node(span, MPExpKind::When { pat, guard: guard.clone() }))
                    };
                    MapclKind::Bidir { forwards: attach(forwards)?, backwards: attach(backwards)? }
                }
            }
        }
        g::MapCl::Attr(_, inner) => return conv_mapcl(c, span, inner),
        g::MapCl::Doc(_, inner) => return conv_mapcl(c, span, inner),
    };
    Ok(Mapcl {
        id: c.ids.fresh(),
        span,
        kind,
    })
}

fn conv_fundef_parts(
    c: &mut Ctx,
    span: Span,
    f: &g::FunDef,
) -> Result<(RecOpt, TAnnotOpt, Vec<Funcl>), String> {
    // `function rec` ist im Grammatik-AST nicht repräsentiert (kein Flag);
    // nur ein explizites Maß wird erkannt (siehe ARCHITECTURE.md).
    let rec = match &f.rec_measure {
        None => RecOpt::NonRec,
        Some((p, e)) => RecOpt::Measure {
            pat: conv_pat(c, span, p)?,
            exp: conv_exp(c, span, e)?,
        },
    };
    let patterns:Vec<_>=std::iter::once(&f.head).chain(&f.tail).map(|clause|match &clause.pat {g::GuardedPat::Plain(p)|g::GuardedPat::Guarded(p,_)=>p}).collect();
    let annot = conv_tannot(c, span, &f.head.typ,&patterns)?;
    let mut funcls = Vec::with_capacity(f.tail.len() + 1);
    funcls.push(conv_funcl(c, span, &f.head)?);
    for t in &f.tail {
        funcls.push(conv_funcl(c, span, t)?);
    }
    Ok((rec, annot, funcls))
}

fn conv_mapdef(c: &mut Ctx, span: Span, m: &g::MapDef) -> Result<(Name, TAnnotOpt, Vec<Mapcl>), String> {
    let name = c.name(span, &m.name);
    let annot = match &m.typschm {
        None => TAnnotOpt::None,
        Some(s) => {
            let schm = conv_typschm(c, span, s)?;
            TAnnotOpt::Some {
                quant: schm.quant,
                typ: schm.typ,
            }
        }
    };
    let mut clauses = Vec::with_capacity(m.clauses.len());
    for cl in &m.clauses {
        clauses.push(conv_mapcl(c, span, cl)?);
    }
    Ok((name, annot, clauses))
}

fn conv_struct_field_one(
    c: &mut Ctx,
    span: Span,
    f: &g::StructField,
) -> Result<(Name, Typ), String> {
    match f {
        g::StructField::Item(n, t) => Ok((c.name(span, n), conv_typ(c, span, t)?)),
        g::StructField::Doc(_, inner) => conv_struct_field_one(c, span, inner),
        g::StructField::Attr(_, inner) => conv_struct_field_one(c, span, inner),
    }
}

fn conv_struct_fields(
    c: &mut Ctx,
    span: Span,
    fs: &[g::StructField],
) -> Result<Vec<(Name, Typ)>, String> {
    let mut out = Vec::with_capacity(fs.len());
    for f in fs {
        out.push(conv_struct_field_one(c, span, f)?);
    }
    Ok(out)
}

fn conv_enum_member(c: &mut Ctx, span: Span, m: &g::EnumMember) -> Result<Name, String> {
    match m {
        g::EnumMember::Item(n) => Ok(c.name(span, n)),
        g::EnumMember::Doc(_, inner) => conv_enum_member(c, span, inner),
        g::EnumMember::Attr(_, inner) => conv_enum_member(c, span, inner),
    }
}

fn conv_type_union(c: &mut Ctx, span: Span, u: &g::TypeUnion) -> Result<TypeUnion, String> {
    match u {
        g::TypeUnion::Id(ctor, t) => Ok(TypeUnion {
            id: c.ids.fresh(),
            span,
            typ: conv_typ(c, span, t)?,
            ctor: c.name(span, ctor),
        }),
        g::TypeUnion::Private(inner) => conv_type_union(c, span, inner),
        g::TypeUnion::Attr(_, inner) => conv_type_union(c, span, inner),
        g::TypeUnion::Doc(_, inner) => conv_type_union(c, span, inner),
        g::TypeUnion::AnonRecord(..) => {
            Err("anonymous-record union ctors have no sail-ast core node".to_owned())
        }
    }
}

fn conv_index_range(c: &mut Ctx, span: Span, r: &g::IndexRange) -> Result<IndexRange, String> {
    let kind = match r {
        g::IndexRange::Single(t) => IndexRangeKind::Single(conv_nexp(c, span, t)?),
        g::IndexRange::Range(a, b) => IndexRangeKind::Range {
            lo: conv_nexp(c, span, a)?,
            hi: conv_nexp(c, span, b)?,
        },
        g::IndexRange::Concat(a, b) => IndexRangeKind::Concat(
            Box::new(conv_index_range(c, span, a)?),
            Box::new(conv_index_range(c, span, b)?),
        ),
    };
    Ok(c.node(span, kind))
}

fn conv_reg_field(
    c: &mut Ctx,
    span: Span,
    f: &g::RegField,
) -> Result<(Name, IndexRange), String> {
    match f {
        g::RegField::Item(n, r) => Ok((c.name(span, n), conv_index_range(c, span, r)?)),
        g::RegField::Doc(_, inner) => conv_reg_field(c, span, inner),
        g::RegField::Attr(_, inner) => conv_reg_field(c, span, inner),
    }
}

fn field_type_roots<'a>(fields: &'a [g::StructField], roots: &mut Vec<(&'a g::Typ, g::Kind)>) {
    for field in fields {
        match field {
            g::StructField::Item(_, typ) => roots.push((typ, g::Kind::Type)),
            g::StructField::Attr(_, inner) | g::StructField::Doc(_, inner) => field_type_roots(std::slice::from_ref(inner), roots),
        }
    }
}
fn union_type_roots<'a>(union: &'a g::TypeUnion, roots: &mut Vec<(&'a g::Typ, g::Kind)>) {
    match union {
        g::TypeUnion::Id(_, typ) => roots.push((typ, g::Kind::Type)),
        g::TypeUnion::AnonRecord(_, fields) => field_type_roots(fields, roots),
        g::TypeUnion::Attr(_, inner) | g::TypeUnion::Doc(_, inner) | g::TypeUnion::Private(inner) => union_type_roots(inner, roots),
    }
}

fn conv_typedef(c: &mut Ctx, span: Span, t: &g::TypeDef) -> Result<TypeDefKind, String> {
    let converted: Result<TypeDefKind, String> = match t {
        g::TypeDef::Abbrev(name, quant, kind, body) => {
            let name = c.name(span, name);
            let result_kind = match kind { Some(kind) => kind.clone(), None => crate::kinds::result_kind(quant, body, &c.bound_kinds, c.fixities, c.constructors)? };
            let quant = conv_quant_items(c, span, &Some(quant.clone()), &[(body, result_kind.clone())])?;
            let arg = match &result_kind {
                g::Kind::Type => TypArgKind::Typ(conv_typ(c, span, body)?),
                g::Kind::Int | g::Kind::Nat => TypArgKind::NExp(conv_nexp(c, span, body)?),
                g::Kind::Bool => TypArgKind::Bool(Box::new(conv_constraint(c, span, body)?)),
                g::Kind::Order => return Err("order abbreviation has no core AST representation".into()),
            };
            let arg = c.node(span, arg);
            Ok(TypeDefKind::Abbrev { name, quant, arg })
        }
        g::TypeDef::Abstract(name, kind, _aliases) => Ok(TypeDefKind::Abstract {
            name: c.name(span, name),
            kind: conv_kind(kind)?,
        }),
        g::TypeDef::Record(name, quant, fields) => {
            let mut roots = Vec::new();
            field_type_roots(fields, &mut roots);
            let lowered_quant = conv_quant_items(c, span, &Some(quant.clone()), &roots)?;
            c.constructors.declare(name, quant, &c.bound_kinds, g::Kind::Type);
            let quant = lowered_quant;
            Ok(TypeDefKind::Record { name: c.name(span, name), quant,
                fields: conv_struct_fields(c, span, fields)?, wf: true })
        }
        g::TypeDef::Enum(name, params, members) => {
            if !params.is_empty() {
                return Err("enum type parameters have no sail-ast core slot".to_owned());
            }
            let mut cnames = Vec::with_capacity(members.len());
            for (m, value) in members {
                if value.is_some() {
                    return Err("enum values have no sail-ast core slot".to_owned());
                }
                cnames.push(conv_enum_member(c, span, m)?);
            }
            Ok(TypeDefKind::Enum {
                name: c.name(span, name),
                members: cnames,
                wf: true,
            })
        }
        g::TypeDef::Variant(name, quant, ctors, wf) => {
            let mut roots = Vec::new();
            for ctor in ctors { union_type_roots(ctor, &mut roots); }
            let lowered_quant = conv_quant_items(c, span, &Some(quant.clone()), &roots)?;
            c.constructors.declare(name, quant, &c.bound_kinds, g::Kind::Type);
            let quant = lowered_quant;
            let mut cctors = Vec::with_capacity(ctors.len());
            for u in ctors {
                cctors.push(conv_type_union(c, span, u)?);
            }
            Ok(TypeDefKind::Variant {
                name: c.name(span, name),
                quant,
                ctors: cctors,
                wf: *wf,
            })
        }
        g::TypeDef::Bitfield(name, typ, fields) => {
            let mut cfields = Vec::with_capacity(fields.len());
            for f in fields {
                cfields.push(conv_reg_field(c, span, f)?);
            }
            Ok(TypeDefKind::Bitfield {
                name: c.name(span, name),
                typ: conv_typ(c, span, typ)?,
                fields: cfields,
            })
        }
    };
    let converted = converted?;
    let empty = g::TypQuant { kopts: vec![], constraint: None };
    let (name, quant, result) = match t {
        g::TypeDef::Abbrev(name, quant, _, _) => {
            let TypeDefKind::Abbrev { arg, .. } = &converted else { unreachable!() };
            let result = match &arg.kind {
                TypArgKind::Typ(_) => g::Kind::Type,
                TypArgKind::NExp(_) => g::Kind::Int,
                TypArgKind::Bool(_) => g::Kind::Bool,
            };
            (name, quant, result)
        }
        g::TypeDef::Record(name, quant, _) | g::TypeDef::Variant(name, quant, _, _) => (name, quant, g::Kind::Type),
        g::TypeDef::Abstract(name, kind, _) => (name, &empty, kind.clone()),
        g::TypeDef::Enum(name, _, _) | g::TypeDef::Bitfield(name, _, _) => (name, &empty, g::Kind::Type),
    };
    c.constructors.declare(name, quant, &c.bound_kinds, result);
    Ok(converted)
}

fn conv_valspec(c: &mut Ctx, span: Span, v: &g::ValSpec) -> Result<ValSpec, String> {
    Ok(ValSpec {
        id: c.ids.fresh(),
        span,
        name: c.name(span, &v.name),
        typschm: conv_typschm(c, span, &v.typschm)?,
        extern_opt: v
            .ext
            .as_ref()
            .map(|e| Extern {
                // `pure` defaultet auf false (Oberflächen-Default).
                pure: e.pure.unwrap_or(false),
                bindings: e.bindings.clone(),
            }),
    })
}

fn conv_regdef(c: &mut Ctx, span: Span, r: &g::RegDef) -> Result<DecSpec, String> {
    Ok(DecSpec {
        id: c.ids.fresh(),
        span,
        typ: conv_typ(c, span, &r.typ)?,
        name: c.name(span, &r.name),
        init: r
            .init
            .as_ref()
            .map(|e| conv_exp(c, span, e))
            .transpose()?,
    })
}

fn conv_defaultdef(d: &g::DefaultDef) -> Result<Order, String> {
    if d.kind != g::Kind::Order {
        return Err("only `default Order` has a sail-ast core node".to_owned());
    }
    match &d.order {
        g::Typ::Inc => Ok(Order::Inc),
        g::Typ::Dec => Ok(Order::Dec),
        _ => Err("default order needs `inc`/`dec`".to_owned()),
    }
}

fn conv_scattered(c: &mut Ctx, span: Span, s: &g::ScatteredDef) -> Result<ScatteredDef, String> {
    match s {
        g::ScatteredDef::Enum(n) => Ok(ScatteredDef::Enum(c.name(span, n))),
        g::ScatteredDef::Variant(n, q) => Ok(ScatteredDef::Variant {
            name: c.name(span, n),
            quant: conv_quant_items(c, span, &Some(q.clone()), &[])?,
        }),
        g::ScatteredDef::Function(n) => Ok(ScatteredDef::Function {
            name: c.name(span, n),
            annot: TAnnotOpt::None,
        }),
        // Mitgeführte Mapping-Annotation entfällt (Core-Slot fehlt).
        g::ScatteredDef::Mapping(n, _) => Ok(ScatteredDef::Mapping {
            name: c.name(span, n),
            annot: TAnnotOpt::None,
        }),
        g::ScatteredDef::EnumClause(e, m) => Ok(ScatteredDef::Enumcl {
            enum_name: c.name(span, e),
            member: c.name(span, m),
        }),
        g::ScatteredDef::FunClause(f) => Ok(ScatteredDef::Funcl(conv_funcl(c, span, f)?)),
        g::ScatteredDef::UnionClause(n, u) => Ok(ScatteredDef::Unioncl {
            name: c.name(span, n),
            union: conv_type_union(c, span, u)?,
        }),
        g::ScatteredDef::MapClause(n, m) => Ok(ScatteredDef::Mapcl {
            name: c.name(span, n),
            clause: conv_mapcl(c, span, m)?,
        }),
        g::ScatteredDef::End(n) => Ok(ScatteredDef::End(c.name(span, n))),
    }
}

fn conv_subst(c: &mut Ctx, span: Span, s: &g::Subst) -> Result<Subst, String> {
    match s {
        g::Subst::Typ(kid, t) => {
            let kid = c.interner.intern(kid);
            let typ = conv_typ(c, span, t)?;
            let arg = c.node(span, TypArgKind::Typ(typ));
            Ok(Subst::Typ { kid, arg })
        }
        g::Subst::Id(from, to) => Ok(Subst::Id {
            from: c.name(span, from),
            to: c.name(span, to),
        }),
    }
}

fn attr_data_string(d: &g::AttrData) -> String {
    match d {
        g::AttrData::Num(s) => s.clone(),
        g::AttrData::Str(s) => format!("{s:?}"),
        g::AttrData::Bool(b) => b.to_string(),
        g::AttrData::List(v) => {
            let inner: Vec<String> = v.iter().map(attr_data_string).collect();
            format!("[{}]", inner.join(", "))
        }
        g::AttrData::Object(kvs) => {
            let inner: Vec<String> = kvs
                .iter()
                .map(|(k, v)| format!("{k} = {}", attr_data_string(v)))
                .collect();
            format!("{{ {} }}", inner.join(", "))
        }
    }
}

fn conv_pragma(p: &g::Pragma) -> (String, String) {
    match p {
        g::Pragma::Line(k, v) => (k.clone(), v.clone()),
        // Strukturierte Nutzdaten werden kompakt gerendert (ARCHITECTURE.md).
        g::Pragma::Structured(k, kvs) => {
            let inner: Vec<String> = kvs
                .iter()
                .map(|(k, v)| format!("{k} = {}", attr_data_string(v)))
                .collect();
            (k.clone(), inner.join(", "))
        }
    }
}

fn conv_defaux(c: &mut Ctx, span: Span, d: &g::DefAux) -> Result<DefKind, String> {
    match d {
        g::DefAux::Fun(f) => {
            let (rec, annot, funcls) = conv_fundef_parts(c, span, f)?;
            Ok(DefKind::Fundef {
                rec,
                annot,
                funcls,
            })
        }
        g::DefAux::Map(m) => {
            let (name, annot, clauses) = conv_mapdef(c, span, m)?;
            Ok(DefKind::Mapdef {
                name,
                annot,
                clauses,
            })
        }
        g::DefAux::Impl(f) => Ok(DefKind::Impl(conv_funcl(c, span, f)?)),
        g::DefAux::Fixity(assoc, level, name) => Ok(DefKind::Fixity {
            fixity: match assoc {
                g::FixAssoc::Infix => Fixity::Infix,
                g::FixAssoc::InfixL => Fixity::InfixL,
                g::FixAssoc::InfixR => Fixity::InfixR,
            },
            level: level.to_string(),
            id: c.name(span, name),
        }),
        g::DefAux::Val(v) => Ok(DefKind::Val(conv_valspec(c, span, v)?)),
        g::DefAux::Outcome(spec, defs) => {
            // Outcome-local declarations and implementations remain nested;
            // local fixities must not escape into subsequent global code.
            let quant = conv_quant_items(c, span, &spec.param, &[(&spec.typschm.lhs, g::Kind::Type), (&spec.typschm.rhs, g::Kind::Type)])?;
            let typschm = conv_typschm(c, span, &spec.typschm)?;
            let saved_fixities = c.fixities.clone();
            let saved_constructors = c.constructors.clone();
            let nested = (|| {
                defs.iter().map(|def| {
                    let kind = conv_def(c, span, def)?;
                    let mut converted = c.node(span, kind);
                    annotate_definition(&mut converted, def);
                    Ok(converted)
                }).collect::<Result<Vec<_>, String>>()
            })();
            *c.fixities = saved_fixities;
            *c.constructors = saved_constructors;
            Ok(DefKind::Outcome {
                defs: nested?,
                name: c.name(span, &spec.name),
                typschm,
                quant,
            })
        }
        g::DefAux::Instantiation(name, substs) => {
            let mut csubsts = Vec::with_capacity(substs.len());
            for s in substs {
                csubsts.push(conv_subst(c, span, s)?);
            }
            Ok(DefKind::Instantiation {
                name: c.name(span, name),
                substs: csubsts,
            })
        }
        g::DefAux::Type(t) => {
            let typedef = conv_typedef(c, span, t)?;
            Ok(DefKind::Type(c.node(span, typedef)))
        }
        g::DefAux::Let(p, e) => Ok(DefKind::Let {
            pat: conv_pat(c, span, p)?,
            exp: conv_exp(c, span, e)?,
        }),
        g::DefAux::Register(r) => Ok(DefKind::Register(conv_regdef(c, span, r)?)),
        g::DefAux::Overload(n, ids) => {
            let mut overloads = Vec::with_capacity(ids.len());
            for o in ids {
                overloads.push(c.name(span, o));
            }
            Ok(DefKind::Overload {
                name: c.name(span, n),
                overloads,
            })
        }
        g::DefAux::Default(d) => Ok(DefKind::Default(conv_defaultdef(d)?)),
        g::DefAux::Constraint(t) => Ok(DefKind::Constraint(conv_constraint(c, span, t)?)),
        g::DefAux::MutRec(fs) => {
            let mut sets = Vec::with_capacity(fs.len());
            for f in fs {
                let (rec, annot, funcls) = conv_fundef_parts(c, span, f)?;
                sets.push(FunclSet {
                    rec,
                    annot,
                    funcls,
                });
            }
            Ok(DefKind::InternalMutrec(sets))
        }
        g::DefAux::Pragma(p) => {
            let (key, payload) = conv_pragma(p);
            Ok(DefKind::Pragma { key, payload })
        }
        g::DefAux::Measure(n, p, e) => Ok(DefKind::Measure {
            name: c.name(span, n),
            pat: conv_pat(c, span, p)?,
            exp: conv_exp(c, span, e)?,
        }),
        g::DefAux::LoopMeasures(n, ms) => {
            let mut cms = Vec::with_capacity(ms.len());
            for (k, e) in ms {
                let k = match k {
                    g::LoopKind::While => Loop::While,
                    g::LoopKind::Until => Loop::Until,
                };
                cms.push((k, conv_exp(c, span, e)?));
            }
            Ok(DefKind::LoopMeasures {
                name: c.name(span, n),
                measures: cms,
            })
        }
        g::DefAux::Scattered(s) => Ok(DefKind::Scattered(conv_scattered(c, span, s)?)),
    }
}

fn conv_def(c: &mut Ctx, span: Span, d: &g::Def) -> Result<DefKind, String> {
    let saved_kinds = c.bound_kinds.clone();
    let result = (|| {
    match d {
        g::Def::Aux(a) => {
            if let g::DefAux::Fixity(assoc, level, name) = a {
                c.fixities.insert_grammar(*assoc, *level, name)?;
            }
            conv_defaux(c, span, a)
        }
        // Hüllen entfallen (Core-Slots fehlen; siehe ARCHITECTURE.md).
        g::Def::Private(inner) => conv_def(c, span, inner),
        g::Def::Attr(_, inner) => conv_def(c, span, inner),
        g::Def::Doc(_, inner) => conv_def(c, span, inner),
    }
    })();
    c.bound_kinds = saved_kinds;
    result
}

#[cfg(test)]
mod symbol_threading_tests {
    //! Lane 121: Literale tragen `Symbol`-Payloads (Lane 71), über den
    //! geliehenen Interner an der Konversionsgrenze interniert; Borrow-
    //! Hoists ändern kein Verhalten (E3: Code ohne Run).

    use super::*;
    use sail_ast::Interner as _;

    fn ctx<'a>(ids: &'a mut IdGen, interner: &'a mut SimpleInterner, fixities: &'a mut FixityEnv, constructors: &'a mut crate::kinds::Constructors) -> Ctx<'a> {
        Ctx { ids, interner, fixities, bound_kinds: Default::default(), constructors }
    }

    #[test]
    fn lit_payloads_are_interned_symbols() {
        let mut ids = IdGen::new();
        let mut fixities = FixityEnv::sail_defaults();
        let mut interner = SimpleInterner::new();
        let span = Span::DUMMY;
        for (lit, expect) in [
            (g::Lit::Num("42".to_owned()), "42"),
            (g::Lit::Zero, "0"),
            (g::Lit::One, "1"),
            (g::Lit::Bin("101".to_owned()), "101"),
            (g::Lit::Hex("FF".to_owned()), "FF"),
            (g::Lit::Str("hi".to_owned()), "hi"),
            (g::Lit::Real("1.5".to_owned()), "1.5"),
        ] {
            let node = {
                let mut constructors = crate::kinds::Constructors::default();
            let mut c = ctx(&mut ids, &mut interner, &mut fixities, &mut constructors);
                c.lit(span, &lit)
            };
            if matches!(lit,g::Lit::Zero|g::Lit::One) {assert!(matches!(node.kind,LitKind::Bin(_)));}
            let sym = match node.kind {
                LitKind::Num(s) | LitKind::Bin(s) | LitKind::Hex(s) | LitKind::String(s) | LitKind::Real(s) => s,
                other => panic!("erwarte Symbol-Payload, got {other:?}"),
            };
            assert_eq!(interner.resolve(sym), Some(expect));
        }
    }

    #[test]
    fn lit_interning_deduplicates() {
        let mut ids = IdGen::new();
        let mut fixities = FixityEnv::sail_defaults();
        let mut interner = SimpleInterner::new();
        let span = Span::DUMMY;
        let lit = g::Lit::Num("7".to_owned());
        let a = {
            let mut constructors = crate::kinds::Constructors::default();
            let mut c = ctx(&mut ids, &mut interner, &mut fixities, &mut constructors);
            c.lit(span, &lit)
        };
        let b = {
            let mut constructors = crate::kinds::Constructors::default();
            let mut c = ctx(&mut ids, &mut interner, &mut fixities, &mut constructors);
            c.lit(span, &lit)
        };
        let sym = |n: &Lit| match n.kind {
            LitKind::Num(s) => s,
            _ => panic!("erwarte Num"),
        };
        assert_eq!(sym(&a), sym(&b));
        assert_eq!(interner.resolve(sym(&a)), Some("7"));
    }

    #[test]
    fn typschm_fn_and_bidir_lower() {
        // Übte den E0499-Hoist in `conv_typschm` (Fn + Bidir).
        let mut ids = IdGen::new();
        let mut fixities = FixityEnv::sail_defaults();
        let mut interner = SimpleInterner::new();
        let span = Span::DUMMY;
        for arrow in [g::Arrow::Fn, g::Arrow::Bidir] {
            let is_fn = arrow == g::Arrow::Fn;
            let schm = g::TypSchm {
                quant: None,
                lhs: g::Typ::Id("int".to_owned()),
                arrow,
                rhs: g::Typ::Id("bool".to_owned()),
                effect: None,
            };
            let out = {
                let mut constructors = crate::kinds::Constructors::default();
            let mut c = ctx(&mut ids, &mut interner, &mut fixities, &mut constructors);
                conv_typschm(&mut c, span, &schm).expect("senkbar")
            };
            assert!(out.quant.is_empty());
            match &out.typ.kind {
                TypKind::Fn { args, ret } if is_fn => {
                    assert_eq!(args.len(), 1);
                    assert!(matches!(ret.kind, TypKind::Id(_)));
                }
                TypKind::Bidir { lhs, rhs } if !is_fn => {
                    assert!(matches!(lhs.kind, TypKind::Id(_)));
                    assert!(matches!(rhs.kind, TypKind::Id(_)));
                }
                other => panic!("unerwartete Typschm-Form: {other:?}"),
            }
        }
        let sym = interner.intern("int");
        assert_eq!(interner.resolve(sym), Some("int"));
    }

    #[test]
    fn subst_typ_threads_interner() {
        // Übte den E0499-Hoist in `conv_subst`.
        let mut ids = IdGen::new();
        let mut fixities = FixityEnv::sail_defaults();
        let mut interner = SimpleInterner::new();
        let span = Span::DUMMY;
        let s = g::Subst::Typ("n".to_owned(), g::Typ::Id("int".to_owned()));
        let out = {
            let mut constructors = crate::kinds::Constructors::default();
            let mut c = ctx(&mut ids, &mut interner, &mut fixities, &mut constructors);
            conv_subst(&mut c, span, &s).expect("senkbar")
        };
        match out {
            Subst::Typ { kid, arg } => {
                assert_eq!(interner.resolve(kid), Some("n"));
                assert!(matches!(arg.kind, TypArgKind::Typ(_)));
            }
            other => panic!("erwarte Typ-Subst, got {other:?}"),
        }
    }

    #[test]
    fn typedef_abbrev_threads_interner() {
        // Übte den E0499-Hoist in `conv_typedef` (Abbrev-Arg).
        let mut ids = IdGen::new();
        let mut fixities = FixityEnv::sail_defaults();
        let mut interner = SimpleInterner::new();
        let span = Span::DUMMY;
        let t = g::TypeDef::Abbrev(
            "t".to_owned(),
            g::TypQuant {
                kopts: Vec::new(),
                constraint: None,
            },
            None,
            g::Typ::Id("int".to_owned()),
        );
        let out = {
            let mut constructors = crate::kinds::Constructors::default();
            let mut c = ctx(&mut ids, &mut interner, &mut fixities, &mut constructors);
            conv_typedef(&mut c, span, &t).expect("senkbar")
        };
        match out {
            TypeDefKind::Abbrev { name, arg, .. } => {
                assert_eq!(interner.resolve(name.symbol), Some("t"));
                assert!(matches!(arg.kind, TypArgKind::Typ(_)));
            }
            other => panic!("erwarte Abbrev, got {other:?}"),
        }
    }
}
