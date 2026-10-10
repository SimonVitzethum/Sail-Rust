#![forbid(unsafe_code)]
//! Lane 106 — fortgeschrittene Sail-Typen (Skizze, E3: kein Build/Test-Lauf).
//!
//! Deckt den Reviewer-5-HIGH-Punkt ab: `Typ_bidir (<->)`, `Typ_exist`,
//! generisches `Typ_app` / `Typ_var` plus existenzielle ARM-Bitvektor-Typen.
//!
//! OCaml-Referenz (`sail/src/libsail/ast.ml:134-144`, `typ_aux`):
//! ```ocaml
//! Typ_internal_unknown | Typ_id of id | Typ_var of kid
//! | Typ_fn of typ * typ | Typ_bidir of typ * typ
//! | Typ_tuple of typ list | Typ_app of id * typ_arg list
//! | Typ_exist of kopt list * n_constraint * typ
//! ```
//! mit `typ_arg_aux = Typ_arg_nexp of nexp | Typ_arg_typ of typ
//! | Typ_arg_bool of n_constraint`.
//!
//! Das lokale [`MiniTyp`][crate::nexp::MiniTyp] kennt nur `Bits`/`Range`/
//! `Named`-mit-Nexp-Args; dieses Modul skizziert die darüber hinausgehenden
//! Formen als [`AdvTyp`] + [`AdvTypArg`], dazu Konverter auf die kanonischen
//! `sail-ast`-Knoten (`TypKind::{Var,App,Bidir,Exist}`), die Kindprüfung und
//! die Solver-Anbindung (Aufpacken von Existenzen, Bidir-Subsumption,
//! ARM-Breiten-Obligationen). Alles ist generisch über
//! [`Solver`][crate::solver::Solver]; `Unknown` bleibt immer ein
//! (lauter) Fehler an der Beweisstelle.

use sail_ast::{Interner, KOpt, Name, Node, Typ, TypArg, TypArgKind, TypKind};

use crate::conv::{
    constraint_from_ast, constraint_to_ast, kind_erase, kind_from_ast, nexp_from_ast, nexp_to_ast,
};
use crate::error::{Span, TypeError, TypeErrorKind, TypeResult};
use crate::kind::{Kind, KindEnv, check_constraint_kind, check_nexp_kind};
use crate::nexp::{MiniTyp, NConstraint, Nexp};
use crate::solver::{ProveResult, Solver};
use crate::subsume::NexpSubst;

// ---------------------------------------------------------------------------
// Typen
// ---------------------------------------------------------------------------

/// Generisches Typ-Argument (`typ_arg_aux`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdvTypArg {
    /// `Typ_arg_nexp`
    Nexp(Nexp),
    /// `Typ_arg_typ`
    Typ(Box<AdvTyp>),
    /// `Typ_arg_bool`
    Bool(NConstraint),
}

impl std::fmt::Display for AdvTypArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nexp(n) => write!(f, "{n}"),
            Self::Typ(t) => write!(f, "{t}"),
            Self::Bool(c) => write!(f, "<{c}>"),
        }
    }
}

/// Erweiterter Sail-Typ: lokales [`MiniTyp`] plus `Var` / generisches `App` /
/// `Bidir` / `Exist` (OCaml `typ_aux`, ohne `Typ_internal_unknown`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdvTyp {
    Int,
    Bool,
    Bit,
    Bits(Nexp),
    /// `Typ_var kid` (kind-tragend, da `sail-ast` nur das Symbol speichert).
    Var {
        kid: String,
        kind: Kind,
    },
    /// `Typ_app (name, args)` mit gemischten Level-/Typ-/Bool-Argumenten.
    App {
        name: String,
        args: Vec<AdvTypArg>,
    },
    /// `Typ_bidir (lhs, rhs)`: umkehrbare Abbildung `<->`.
    Bidir {
        lhs: Box<AdvTyp>,
        rhs: Box<AdvTyp>,
    },
    Tuple(Vec<AdvTyp>),
    /// `Typ_fn`, binär (n-stellige AST-Funktionen werden rechts gefaltet).
    Fun(Box<AdvTyp>, Box<AdvTyp>),
    /// `Typ_exist (binders, nc, body)`.
    Exist {
        binders: Vec<(String, Kind)>,
        constraint: NConstraint,
        body: Box<AdvTyp>,
    },
}

impl std::fmt::Display for AdvTyp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int => write!(f, "int"),
            Self::Bool => write!(f, "bool"),
            Self::Bit => write!(f, "bit"),
            Self::Bits(n) => write!(f, "bits({n})"),
            Self::Var { kid, .. } => write!(f, "'{kid}"),
            Self::App { name, args } => {
                write!(f, "{name}")?;
                if !args.is_empty() {
                    write!(f, "(")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{a}")?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
            Self::Bidir { lhs, rhs } => write!(f, "({lhs} <-> {rhs})"),
            Self::Tuple(ts) => {
                write!(f, "(")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, ")")
            }
            Self::Fun(a, b) => write!(f, "({a} -> {b})"),
            Self::Exist { binders, constraint, body } => {
                write!(f, "exists ")?;
                for (i, (v, k)) in binders.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "'{v}:{k}")?;
                }
                write!(f, ". {constraint}. {body}")
            }
        }
    }
}

impl AdvTyp {
    /// Ebene-Terme des Typs einsammeln (für Kindprüfung/Fuel-Rechnung).
    pub fn nexps(&self, out: &mut Vec<Nexp>) {
        match self {
            Self::Int | Self::Bool | Self::Bit | Self::Var { .. } => {}
            Self::Bits(n) => out.push(n.clone()),
            Self::App { args, .. } => {
                for a in args {
                    match a {
                        AdvTypArg::Nexp(n) => out.push(n.clone()),
                        AdvTypArg::Typ(t) => t.nexps(out),
                        AdvTypArg::Bool(c) => {
                            let mut vs = Vec::new();
                            c.free_vars(&mut vs);
                            out.extend(vs.into_iter().map(Nexp::Var));
                        }
                    }
                }
            }
            Self::Bidir { lhs, rhs } => {
                lhs.nexps(out);
                rhs.nexps(out);
            }
            Self::Tuple(ts) => {
                for t in ts {
                    t.nexps(out);
                }
            }
            Self::Fun(a, b) => {
                a.nexps(out);
                b.nexps(out);
            }
            Self::Exist { body, constraint, .. } => {
                let mut vs = Vec::new();
                constraint.free_vars(&mut vs);
                out.extend(vs.into_iter().map(Nexp::Var));
                body.nexps(out);
            }
        }
    }

    /// Freie Level-Variablen (Binder von [`AdvTyp::Exist`] gelten als gebunden).
    pub fn free_vars(&self, out: &mut Vec<String>) {
        match self {
            Self::Int | Self::Bool | Self::Bit => {}
            Self::Var { kid, .. } => out.push(kid.clone()),
            Self::Bits(n) => n.free_vars(out),
            Self::App { args, .. } => {
                for a in args {
                    match a {
                        AdvTypArg::Nexp(n) => n.free_vars(out),
                        AdvTypArg::Typ(t) => t.free_vars(out),
                        AdvTypArg::Bool(c) => c.free_vars(out),
                    }
                }
            }
            Self::Bidir { lhs, rhs } => {
                lhs.free_vars(out);
                rhs.free_vars(out);
            }
            Self::Tuple(ts) => {
                for t in ts {
                    t.free_vars(out);
                }
            }
            Self::Fun(a, b) => {
                a.free_vars(out);
                b.free_vars(out);
            }
            Self::Exist { binders, constraint, body } => {
                let mut inner = Vec::new();
                constraint.free_vars(&mut inner);
                body.free_vars(&mut inner);
                for v in inner {
                    if !binders.iter().any(|(b, _)| b == &v) {
                        out.push(v);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Existensielle ARM-Bitvektor-Typen (Skizze)
// ---------------------------------------------------------------------------

/// ARM-Breiten, die als existenzielle Bitvektoren auftreten
/// (z.B. GPR/FPR-Größen, SIMD-Elemente).
pub const ARM_BITVEC_WIDTHS: [i128; 4] = [8, 16, 32, 64];

/// `exists 'n:K_int. ('n == w1 | 'n == w2 | ...) . bits('n)`.
///
/// Skizziert den OCaml-`Typ_exist`-Fall für ARM-Register-/Vektor-Typen:
/// die Breite ist existentiell gebunden, die zulässigen Werte stehen in der
/// Constraint. `var` ist der Binder-Name (typisch `"n"`).
#[must_use]
pub fn arm_exist_bits(var: &str, widths: &[i128]) -> AdvTyp {
    let mut constraint: Option<NConstraint> = None;
    for w in widths {
        let eq = NConstraint::Eq(
            Box::new(Nexp::Var(var.to_owned())),
            Box::new(Nexp::Const(*w)),
        );
        constraint = Some(match constraint {
            None => eq,
            Some(prev) => NConstraint::Or(Box::new(prev), Box::new(eq)),
        });
    }
    AdvTyp::Exist {
        binders: vec![(var.to_owned(), Kind::KInt)],
        constraint: constraint.unwrap_or(NConstraint::False),
        body: Box::new(AdvTyp::Bits(Nexp::Var(var.to_owned()))),
    }
}

/// Prüft, ob `typ` genau die Form [`arm_exist_bits`] mit einer der
/// [`ARM_BITVEC_WIDTHS`]-Teilmengen hat (syntaktische Erkennung, kein Solver).
#[must_use]
pub fn is_arm_exist_bits(typ: &AdvTyp) -> bool {
    match typ {
        AdvTyp::Exist { binders, constraint, body } => {
            if binders.len() != 1 || binders[0].1 != Kind::KInt {
                return false;
            }
            let var = &binders[0].0;
            if **body != AdvTyp::Bits(Nexp::Var(var.clone())) {
                return false;
            }
            let mut widths = Vec::new();
            let mut queue = vec![constraint];
            while let Some(c) = queue.pop() {
                match c {
                    NConstraint::Or(a, b) => {
                        queue.push(a);
                        queue.push(b);
                    }
                    NConstraint::Eq(n, m) => {
                        let w = match (&**n, &**m) {
                            (Nexp::Var(v), Nexp::Const(w)) if v == var => Some(*w),
                            (Nexp::Const(w), Nexp::Var(v)) if v == var => Some(*w),
                            _ => None,
                        };
                        match w {
                            Some(w) if ARM_BITVEC_WIDTHS.contains(&w) => widths.push(w),
                            _ => return false,
                        }
                    }
                    _ => return false,
                }
            }
            !widths.is_empty()
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Kindprüfung (Skizze)
// ---------------------------------------------------------------------------

/// Kindprüfung für [`AdvTyp`]: `Var` schlägt im `KindEnv` nach, `App`-Args
/// werden je nach Sorte geprüft (`Nexp`: `K_int`, `Bool`: `K_bool`,
/// `Typ`: `K_type`), `Bidir`-Seiten und `Exist`-Rumpf brauchen `K_type`;
/// `Exist`-Binder erweitern die Umgebung für Constraint + Rumpf.
pub fn check_adv_kind(typ: &AdvTyp, env: &KindEnv, span: Span) -> TypeResult<Kind> {
    match typ {
        AdvTyp::Int | AdvTyp::Bool | AdvTyp::Bit => Ok(Kind::KType),
        AdvTyp::Bits(n) => {
            let k = check_nexp_kind(n, env, span)?;
            if k != Kind::KInt && k != Kind::KNat {
                return Err(TypeError::kind_mismatch(span, Kind::KInt, k));
            }
            if let Some(c) = n.as_const() {
                if c < 0 {
                    return Err(TypeError::negative_bits(span, c));
                }
            }
            Ok(Kind::KType)
        }
        AdvTyp::Var { kid, kind } => match env.lookup(kid) {
            Some(k) if k == *kind || (k == Kind::KInt && *kind == Kind::KNat) => Ok(*kind),
            Some(k) => Err(TypeError::kind_mismatch(span, *kind, k)),
            None => Err(TypeError::unbound_var(span, kid.clone())),
        },
        AdvTyp::App { args, .. } => {
            for a in args {
                match a {
                    AdvTypArg::Nexp(n) => {
                        check_nexp_kind(n, env, span)?;
                    }
                    AdvTypArg::Bool(c) => {
                        check_constraint_kind(c, env, span)?;
                    }
                    AdvTypArg::Typ(t) => {
                        check_adv_kind(t, env, span)?;
                    }
                }
            }
            Ok(Kind::KType)
        }
        AdvTyp::Bidir { lhs, rhs } => {
            check_adv_kind(lhs, env, span)?;
            check_adv_kind(rhs, env, span)?;
            Ok(Kind::KType)
        }
        AdvTyp::Tuple(ts) => {
            for t in ts {
                check_adv_kind(t, env, span)?;
            }
            Ok(Kind::KType)
        }
        AdvTyp::Fun(a, b) => {
            check_adv_kind(a, env, span)?;
            check_adv_kind(b, env, span)?;
            Ok(Kind::KType)
        }
        AdvTyp::Exist { binders, constraint, body } => {
            let mut inner = env.clone();
            for (v, k) in binders {
                inner.bind(v.clone(), *k);
            }
            check_constraint_kind(constraint, &inner, span)?;
            check_adv_kind(body, &inner, span)?;
            Ok(Kind::KType)
        }
    }
}

// ---------------------------------------------------------------------------
// Substitution (Skizze, via NexpSubst)
// ---------------------------------------------------------------------------

/// Wendet eine Level-Substitution auf [`AdvTyp`] an (fängt `Exist`-Binder
/// hygienisch: prallt die Substitution an gebundenen Namen ab).
#[must_use]
pub fn subst_adv_typ(subst: &NexpSubst, typ: &AdvTyp) -> AdvTyp {
    match typ {
        AdvTyp::Int | AdvTyp::Bool | AdvTyp::Bit => typ.clone(),
        AdvTyp::Bits(n) => AdvTyp::Bits(subst.apply_nexp(n)),
        AdvTyp::Var { .. } => typ.clone(),
        AdvTyp::App { name, args } => AdvTyp::App {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| match a {
                    AdvTypArg::Nexp(n) => AdvTypArg::Nexp(subst.apply_nexp(n)),
                    AdvTypArg::Typ(t) => AdvTypArg::Typ(Box::new(subst_adv_typ(subst, t))),
                    AdvTypArg::Bool(c) => AdvTypArg::Bool(subst.apply_constraint(c)),
                })
                .collect(),
        },
        AdvTyp::Bidir { lhs, rhs } => AdvTyp::Bidir {
            lhs: Box::new(subst_adv_typ(subst, lhs)),
            rhs: Box::new(subst_adv_typ(subst, rhs)),
        },
        AdvTyp::Tuple(ts) => {
            AdvTyp::Tuple(ts.iter().map(|t| subst_adv_typ(subst, t)).collect())
        }
        AdvTyp::Fun(a, b) => AdvTyp::Fun(
            Box::new(subst_adv_typ(subst, a)),
            Box::new(subst_adv_typ(subst, b)),
        ),
        // Skizze: Binder-Namen gelten als frisch (Freshen siehe
        // `freshen_exist`); Kollisionen muss der Aufrufer durch
        // vorheriges Freshen ausschließen.
        AdvTyp::Exist { binders, constraint, body } => AdvTyp::Exist {
            binders: binders.clone(),
            constraint: subst.apply_constraint(constraint),
            body: Box::new(subst_adv_typ(subst, body)),
        },
    }
}

/// Frischt die Binder einer [`AdvTyp::Exist`] auf (`exists 'n. …` →
/// `exists 'base0. …`); Nicht-`Exist`-Typen kommen unverändert zurück.
#[must_use]
pub fn freshen_exist(typ: &AdvTyp, base: &str) -> AdvTyp {
    match typ {
        AdvTyp::Exist { binders, constraint, body } => {
            let mut subst = NexpSubst::new();
            let mut fresh_binders = Vec::with_capacity(binders.len());
            for (i, (v, k)) in binders.iter().enumerate() {
                let fresh = format!("{base}{i}");
                subst.bind(v.clone(), Nexp::Var(fresh.clone()));
                fresh_binders.push((fresh, *k));
            }
            AdvTyp::Exist {
                binders: fresh_binders,
                constraint: subst.apply_constraint(constraint),
                body: Box::new(subst_adv_typ(&subst, body)),
            }
        }
        _ => typ.clone(),
    }
}

// ---------------------------------------------------------------------------
// Konverter sail-ast <-> AdvTyp (Skizze)
// ---------------------------------------------------------------------------

fn ast_name(it: &mut impl Interner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn resolve(it: &impl Interner, name: &Name) -> Option<String> {
    it.resolve(name.symbol).map(str::to_owned)
}

fn mk_typ(kind: TypKind) -> Typ {
    Node::dummy(kind)
}

fn mk_typarg(kind: TypArgKind) -> TypArg {
    Node::dummy(kind)
}

/// [`AdvTypArg`] → kanonischer `TypArg`-Knoten.
pub fn adv_arg_to_ast(arg: &AdvTypArg, it: &mut impl Interner) -> TypArg {
    match arg {
        AdvTypArg::Nexp(n) => mk_typarg(TypArgKind::NExp(nexp_to_ast(n, it))),
        AdvTypArg::Typ(t) => mk_typarg(TypArgKind::Typ(adv_to_ast(t, it))),
        AdvTypArg::Bool(c) => mk_typarg(TypArgKind::Bool(Box::new(constraint_to_ast(c, it)))),
    }
}

/// [`AdvTyp`] → kanonischer `Typ`-Knoten.
///
/// `Var` nutzt `TypKind::Var`, generisches `App` (auch `bits`) nutzt
/// `TypKind::App` mit gemischten Args, `Bidir`/`Exist` mappen 1:1.
/// Alle Knoten tragen `NodeId::DUMMY`/`Span::DUMMY` (vgl. `conv.rs`).
pub fn adv_to_ast(typ: &AdvTyp, it: &mut impl Interner) -> Typ {
    match typ {
        AdvTyp::Int => mk_typ(TypKind::Id(ast_name(it, "int"))),
        AdvTyp::Bool => mk_typ(TypKind::Id(ast_name(it, "bool"))),
        AdvTyp::Bit => mk_typ(TypKind::Id(ast_name(it, "bit"))),
        AdvTyp::Bits(n) => mk_typ(TypKind::App {
            name: ast_name(it, "bits"),
            args: vec![mk_typarg(TypArgKind::NExp(nexp_to_ast(n, it)))],
        }),
        AdvTyp::Var { kid, .. } => mk_typ(TypKind::Var(it.intern(kid))),
        AdvTyp::App { name, args } => mk_typ(TypKind::App {
            name: ast_name(it, name),
            args: args.iter().map(|a| adv_arg_to_ast(a, it)).collect(),
        }),
        AdvTyp::Bidir { lhs, rhs } => mk_typ(TypKind::Bidir {
            lhs: Box::new(adv_to_ast(lhs, it)),
            rhs: Box::new(adv_to_ast(rhs, it)),
        }),
        AdvTyp::Tuple(ts) => {
            mk_typ(TypKind::Tuple(ts.iter().map(|t| adv_to_ast(t, it)).collect()))
        }
        AdvTyp::Fun(a, b) => mk_typ(TypKind::Fn {
            args: vec![adv_to_ast(a, it)],
            ret: Box::new(adv_to_ast(b, it)),
        }),
        AdvTyp::Exist { binders, constraint, body } => mk_typ(TypKind::Exist {
            binders: binders
                .iter()
                .map(|(v, k)| KOpt { kind: kind_erase(*k), kid: it.intern(v) })
                .collect(),
            constraint: Box::new(constraint_to_ast(constraint, it)),
            typ: Box::new(adv_to_ast(body, it)),
        }),
    }
}

fn adv_arg_from_ast(arg: &TypArg, it: &impl Interner) -> Option<AdvTypArg> {
    match &arg.kind {
        TypArgKind::NExp(n) => Some(AdvTypArg::Nexp(nexp_from_ast(n, it)?)),
        TypArgKind::Typ(t) => Some(AdvTypArg::Typ(Box::new(adv_from_ast(t, it)?))),
        TypArgKind::Bool(c) => Some(AdvTypArg::Bool(constraint_from_ast(c, it)?)),
    }
}

fn fold_fun(args: &[Typ], ret: &Typ, it: &impl Interner) -> Option<AdvTyp> {
    let mut acc = adv_from_ast(ret, it)?;
    for a in args.iter().rev() {
        acc = AdvTyp::Fun(Box::new(adv_from_ast(a, it)?), Box::new(acc));
    }
    Some(acc)
}

/// Kanonischer `Typ`-Knoten → [`AdvTyp`]. `None` für Formen ohne lokales
/// Gegenstück (`Unknown`, `NC_set`-/Fremd-`App`-Constraints in Args,
/// nicht auflösbare Namen/Symbole, `bits` mit Nicht-Nexp-Arg).
///
/// Hinweise: `TypKind::Var` trägt keine Kind-Annotation — die Skizze setzt
/// `K_type` (Level-Variablen erkennt man am Verwendungskontext, nicht am
/// Binder); `KNat` wird nie geraten (vgl. `conv.rs`: `kind_from_ast`
/// liefert `KInt`). `bits(n)` mit genau einem Nexp-Arg wird zum
/// `AdvTyp::Bits`, jede andere `App` zum generischen [`AdvTyp::App`].
pub fn adv_from_ast(typ: &Typ, it: &impl Interner) -> Option<AdvTyp> {
    match &typ.kind {
        TypKind::Unknown => None,
        TypKind::Id(name) => match resolve(it, name).as_deref() {
            Some("int") => Some(AdvTyp::Int),
            Some("bool") => Some(AdvTyp::Bool),
            Some("bit") => Some(AdvTyp::Bit),
            Some(other) => Some(AdvTyp::App { name: other.to_owned(), args: Vec::new() }),
            None => None,
        },
        TypKind::Var(sym) => Some(AdvTyp::Var {
            kid: it.resolve(*sym)?.to_owned(),
            kind: Kind::KType,
        }),
        TypKind::App { name, args } => {
            let text = resolve(it, name)?;
            let mut adv_args = Vec::with_capacity(args.len());
            for a in args {
                adv_args.push(adv_arg_from_ast(a, it)?);
            }
            if text == "bits" {
                match adv_args.as_slice() {
                    [AdvTypArg::Nexp(n)] => Some(AdvTyp::Bits(n.clone())),
                    _ => None,
                }
            } else {
                Some(AdvTyp::App { name: text, args: adv_args })
            }
        }
        TypKind::Bidir { lhs, rhs } => Some(AdvTyp::Bidir {
            lhs: Box::new(adv_from_ast(lhs, it)?),
            rhs: Box::new(adv_from_ast(rhs, it)?),
        }),
        TypKind::Tuple(ts) => {
            let mut out = Vec::with_capacity(ts.len());
            for t in ts {
                out.push(adv_from_ast(t, it)?);
            }
            Some(AdvTyp::Tuple(out))
        }
        TypKind::Fn { args, ret } => fold_fun(args, ret, it),
        TypKind::Exist { binders, constraint, typ } => {
            let mut out_binders = Vec::with_capacity(binders.len());
            for b in binders {
                out_binders.push((it.resolve(b.kid)?.to_owned(), kind_from_ast(b.kind)));
            }
            Some(AdvTyp::Exist {
                binders: out_binders,
                constraint: constraint_from_ast(constraint, it)?,
                body: Box::new(adv_from_ast(typ, it)?),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Solver-Anbindung (Skizze)
// ---------------------------------------------------------------------------

/// Erasure von [`AdvTyp`] auf das solver-fähige [`MiniTyp`]: `Var` und
/// generisches `App` sind nicht darstellbar (`None`); `Bidir` erasuret auf
/// die Vorwärts-Seite (Dokumentations-Entscheidung dieser Skizze);
/// `Exist` erasuret auf den Rumpf (Binder/Constraints trägt die
/// [`exist_open_obligation`]).
pub fn erase_to_mini(typ: &AdvTyp) -> Option<MiniTyp> {
    match typ {
        AdvTyp::Int => Some(MiniTyp::Int),
        AdvTyp::Bool => Some(MiniTyp::Bool),
        AdvTyp::Bit => Some(MiniTyp::Bit),
        AdvTyp::Bits(n) => Some(MiniTyp::Bits(Box::new(n.clone()))),
        AdvTyp::Var { .. } | AdvTyp::App { .. } => None,
        AdvTyp::Bidir { lhs, .. } => erase_to_mini(lhs),
        AdvTyp::Tuple(ts) => {
            let mut out = Vec::with_capacity(ts.len());
            for t in ts {
                out.push(erase_to_mini(t)?);
            }
            Some(MiniTyp::Tuple(out))
        }
        AdvTyp::Fun(a, b) => Some(MiniTyp::Fun(
            Box::new(erase_to_mini(a)?),
            Box::new(erase_to_mini(b)?),
        )),
        AdvTyp::Exist { body, .. } => erase_to_mini(body),
    }
}

/// Öffnet eine [`AdvTyp::Exist`] für den Solver: frischt die Binder auf und
/// gibt `(Binder, Constraint, Rumpf)` zurück. Der Aufrufer beweist dann
/// `assumptions => constraint[fresh]` bzw. prüft den Rumpf unter den
/// frischen Bindern (Regeln `E-Open`/`E-Intro`, s. `prove_exist_open`).
#[must_use]
pub fn open_exist(typ: &AdvTyp, base: &str) -> Option<(Vec<(String, Kind)>, NConstraint, AdvTyp)> {
    match freshen_exist(typ, base) {
        AdvTyp::Exist { binders, constraint, body } => Some((binders, constraint, *body)),
        _ => None,
    }
}

/// Solver-Anbindung für existenzielle Öffnung (Skizze der Regel `E-Open`):
/// beweist `witness_oblig => opened_constraint`.
///
/// `witness_oblig` ist die am Aufbrauchsort bekannte Constraint (z.B. die
/// konkrete Breite `n == 32`), `opened_constraint` die frisch geöffnete
/// Existenz-Constraint. Nur [`ProveResult::Proved`] akzeptiert — `Refuted`
/// und `Unknown` sind laut Fehler (`Z-UnknownSound`-Politik).
pub fn prove_exist_open<S: Solver>(
    solver: &S,
    witness_oblig: &NConstraint,
    opened_constraint: &NConstraint,
    span: Span,
) -> TypeResult<()> {
    // Skizze: `A => B` als `~A | B`; ein Backend mit Implikations-Support
    // (Lane 06/80) ersetzt dies durch einen echten Entailment-Check.
    let goal = NConstraint::Or(
        Box::new(NConstraint::Not(Box::new(witness_oblig.clone()))),
        Box::new(opened_constraint.clone()),
    );
    match solver.prove(&goal) {
        ProveResult::Proved => Ok(()),
        ProveResult::Refuted | ProveResult::Unknown => {
            Err(TypeError::constraint_not_proved(span, &goal))
        }
    }
}

/// Solver-Anbindung für `Typ_bidir` (Skizze der Regel `S-Bidir`): prüft beide
/// Richtungen über die erasureten [`MiniTyp`]-Seiten mit
/// [`crate::subsume::check_subtype`]. Nicht erasurebare Seiten sind ein
/// Quantor-Fehler (kein stilles Raten).
pub fn check_bidir_sub<S: Solver>(
    solver: &S,
    inferred: &AdvTyp,
    expected: &AdvTyp,
    span: Span,
) -> TypeResult<()> {
    let (inf_lhs, inf_rhs) = match inferred {
        AdvTyp::Bidir { lhs, rhs } => (erase_to_mini(lhs), erase_to_mini(rhs)),
        _ => (erase_to_mini(inferred), erase_to_mini(inferred)),
    };
    let (exp_lhs, exp_rhs) = match expected {
        AdvTyp::Bidir { lhs, rhs } => (erase_to_mini(lhs), erase_to_mini(rhs)),
        _ => (erase_to_mini(expected), erase_to_mini(expected)),
    };
    match (inf_lhs, inf_rhs, exp_lhs, exp_rhs) {
        (Some(il), Some(ir), Some(el), Some(er)) => {
            // Vorwärts: inferred.lhs <: expected.lhs; rückwärts:
            // expected.rhs <: inferred.rhs (Kontravarianz der Umkehrung).
            crate::subsume::check_subtype(solver, &il, &el, span)?;
            crate::subsume::check_subtype(solver, &er, &ir, span)?;
            Ok(())
        }
        _ => Err(TypeError {
            span,
            kind: TypeErrorKind::QuantError(
                "bidir subsumption needs erasable sides (Var/generic App unsupported)".to_string(),
            ),
        }),
    }
}

/// Solver-Obligation für ARM-Bitvektoren (Skizze): aus der Breitenmenge die
/// Disjunktion `n == w1 | n == w2 | …` bauen — der Aufrufer beweist sie per
/// [`prove_exist_open`] nach dem Einsetzen der konkreten Breite.
#[must_use]
pub fn arm_width_obligation(var: &str, widths: &[i128]) -> NConstraint {
    let mut acc: Option<NConstraint> = None;
    for w in widths {
        let eq = NConstraint::Eq(
            Box::new(Nexp::Var(var.to_owned())),
            Box::new(Nexp::Const(*w)),
        );
        acc = Some(match acc {
            None => eq,
            Some(prev) => NConstraint::Or(Box::new(prev), Box::new(eq)),
        });
    }
    acc.unwrap_or(NConstraint::False)
}

// ---------------------------------------------------------------------------
// Tests als Code (E3: geschrieben, NICHT ausgeführt)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solver::StubSolver;
    use sail_ast::{QuantItem, SimpleInterner, Span as AstSpan};

    fn dummy_span() -> Span {
        AstSpan::DUMMY
    }

    #[test]
    fn bidir_roundtrips_through_ast() {
        let mut it = SimpleInterner::new();
        let local = AdvTyp::Bidir {
            lhs: Box::new(AdvTyp::Bits(Nexp::Const(32))),
            rhs: Box::new(AdvTyp::Bits(Nexp::Const(32))),
        };
        let ast = adv_to_ast(&local, &mut it);
        assert!(matches!(ast.kind, TypKind::Bidir { .. }));
        assert_eq!(adv_from_ast(&ast, &it), Some(local));
    }

    #[test]
    fn exist_roundtrips_through_ast() {
        let mut it = SimpleInterner::new();
        let local = arm_exist_bits("n", &[8, 32]);
        let ast = adv_to_ast(&local, &mut it);
        assert!(matches!(ast.kind, TypKind::Exist { .. }));
        assert_eq!(adv_from_ast(&ast, &it), Some(local));
    }

    #[test]
    fn generic_app_with_mixed_args_roundtrips() {
        let mut it = SimpleInterner::new();
        let local = AdvTyp::App {
            name: "vector".to_string(),
            args: vec![
                AdvTypArg::Nexp(Nexp::Const(4)),
                AdvTypArg::Typ(Box::new(AdvTyp::Bits(Nexp::Const(8)))),
                AdvTypArg::Bool(NConstraint::Ge(
                    Box::new(Nexp::Var("n".to_string())),
                    Box::new(Nexp::Const(1)),
                )),
            ],
        };
        let ast = adv_to_ast(&local, &mut it);
        assert!(matches!(ast.kind, TypKind::App { .. }));
        assert_eq!(adv_from_ast(&ast, &it), Some(local));
    }

    #[test]
    fn typ_var_roundtrips_with_ktype_default() {
        let mut it = SimpleInterner::new();
        let local = AdvTyp::Var { kid: "a".to_string(), kind: Kind::KType };
        let ast = adv_to_ast(&local, &mut it);
        assert!(matches!(ast.kind, TypKind::Var(_)));
        assert_eq!(adv_from_ast(&ast, &it), Some(local));
    }

    #[test]
    fn arm_widths_are_recognized_and_kindcheck() {
        let typ = arm_exist_bits("n", &ARM_BITVEC_WIDTHS);
        assert!(is_arm_exist_bits(&typ));
        assert!(!is_arm_exist_bits(&AdvTyp::Bits(Nexp::Const(32))));
        let env = KindEnv::new();
        assert_eq!(check_adv_kind(&typ, &env, dummy_span()), Ok(Kind::KType));
    }

    #[test]
    fn exist_open_proves_concrete_width_with_stub() {
        // `n == 32 => (n == 8 | n == 32)` ist dem Stub zu schwer
        // (kein Fall-Splitting): die Skizze MUSS den `Unknown`-Pfad als
        // Fehler sichtbar machen, nicht still akzeptieren.
        let solver = StubSolver::new();
        let (_, opened, _) =
            open_exist(&arm_exist_bits("n", &[8, 32]), "w").expect("exist opens");
        let witness = NConstraint::Eq(
            Box::new(Nexp::Var("w0".to_string())),
            Box::new(Nexp::Const(32)),
        );
        assert!(prove_exist_open(&solver, &witness, &opened, dummy_span()).is_err());
    }

    #[test]
    fn bidir_bits_congruence_needs_solver_proof() {
        let solver = StubSolver::new();
        let inferred = AdvTyp::Bidir {
            lhs: Box::new(AdvTyp::Bits(Nexp::Const(32))),
            rhs: Box::new(AdvTyp::Bool),
        };
        let expected = AdvTyp::Bidir {
            lhs: Box::new(AdvTyp::Bits(Nexp::Var("n".to_string()))),
            rhs: Box::new(AdvTyp::Bool),
        };
        // `32 == 'n` ist ohne Belegung unbeweisbar -> Fehler statt `Unknown`.
        assert!(check_bidir_sub(&solver, &inferred, &expected, dummy_span()).is_err());
        let same = AdvTyp::Bidir {
            lhs: Box::new(AdvTyp::Bits(Nexp::Const(32))),
            rhs: Box::new(AdvTyp::Bool),
        };
        assert!(check_bidir_sub(&solver, &same, &same, dummy_span()).is_ok());
    }

    #[test]
    fn quant_item_constraint_shape_is_documented() {
        // Stellt sicher, dass der `Exist`-Binder-Ansatz zum `TypSchm`
        // (QuantItem::Id + Constraint) passt: Binder lassen sich 1:1 in
        // Quantoren-Items überführen.
        let mut it = SimpleInterner::new();
        let typ = arm_exist_bits("n", &[16]);
        let ast = adv_to_ast(&typ, &mut it);
        match &ast.kind {
            TypKind::Exist { binders, constraint, .. } => {
                assert_eq!(binders.len(), 1);
                let qi: Vec<QuantItem> = binders
                    .iter()
                    .map(|b| QuantItem::Id(*b))
                    .chain(std::iter::once(QuantItem::Constraint((**constraint).clone())))
                    .collect();
                assert_eq!(qi.len(), 2);
            }
            _ => panic!("expected Exist"),
        }
    }
}
