#![forbid(unsafe_code)]
//! Lean pattern printer: port of OCaml `doc_pat`, `doc_vector_concat`,
//! `is_bitvector_pattern` and `is_match_bv`
//! (`sail/src/sail_lean_backend/pretty_print_lean.ml` #520-589, #637-641).
//!
//! Semantik-Treue (Lane-129-Fundliste, gegen die OCaml-Zeilen geprüft):
//! - `P_as`/`P_var` drucken nur das Innen-Pattern, der Binder entfällt
//!   (#556-557). `P_typ` entfällt ebenfalls (#531); nur *innerhalb* eines
//!   Vektor-Patterns bleibt eine Breiten-Ascription (`p:1`, `p:n`,
//!   `p:Typ`, #526-530). Die reine Typ-Ascription für `let` liefert
//!   [`LeanPrinter::pat_typ_ascription`] separat (#588-589).
//! - `P_vector` aus lauter `P_lit` druckt `0b…` — aber nur außerhalb von
//!   `match_bv` (`not in_match_bv`, #543-545); Vektorelemente laufen mit
//!   `in_vector:true`.
//! - Literale in Pattern drucken mit `width:false`, tragen also nie ein
//!   `#w`-Suffix (#525, siehe
//!   [`LeanPrinter::print_lit_with_width`][crate::LeanPrinter::print_lit_with_width]).
//! - `P_id` über Enum-Mitgliedern trägt einen führenden `.` (#538);
//!   Konstruktor-Applikationen drucken als `.ctor arg, …` (#549-555,
//!   Nullär als `.A`), `None` als `none` (#548). Ids laufen durch
//!   [`fixup_match_id`][crate::fixup_match_id] (#506-510, #536, #539).
//! - `is_bitvector_pattern` folgt *strikt* nur `P_as` (#637-638): durch
//!   `P_typ`/`P_var`/`P_or` wird NICHT hindurchgeschaut.
//! - `P_not`, `P_or`, `P_vector_subrange` und `P_string_append` haben kein
//!   `doc_pat`-Gegenstück und sind harte [`Error::Unsupported`][crate::Error]
//!   (OCaml-`failwith` an #565, `P_not` an #470) — nie stilles `sorry`.
//! - `P_id` mit Bitvektor-Typ und bekannter Breite (`id:n`, #534-536)
//!   sowie `P_id` in `vector_concat` (`id:n`, #576-581) brauchen die
//!   Breite vom Aufrufer: als [`Pat::Typ`] mit [`Typ::BitVec`][crate::Typ]
//!   konstanter Breite hüllen; ein nacktes [`Pat::Id`] in
//!   [`Pat::VectorConcat`] ohne Breite ist ein harter Fehler (OCaml:
//!   `unreachable … unclear width`, #580).

use std::fmt;

use sail_ast::{Interner, Pat as AstPat, PatKind, StructName};

use crate::{
    Error, LeanPrinter, Lit, Nexp, Typ,
    conv::{lit_from_ast, typ_from_ast},
    fixup_match_id,
};

/// Lean-druckbares Wert-Pattern (druckbare Teilmenge von
/// `sail_ast::PatKind`; harte-Fehler-Formen als eigene Varianten, damit
/// der Druckort — wie OCaml — fehlschlägt statt zu raten).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pat {
    /// `_`.
    Wild,
    /// Binder / Enum-Mitglied / Bit Identifikator.
    Id(String),
    /// Literal-Pattern (druckt mit `width:false`).
    Lit(Lit),
    /// `(a, b)`.
    Tuple(Vec<Pat>),
    /// `[a, b]`.
    List(Vec<Pat>),
    /// `(h :: t)`.
    Cons(Box<Pat>, Box<Pat>),
    /// Bitvektor-Pattern fester Länge (`P_vector`).
    Vector(Vec<Pat>),
    /// Konkateniertes Bitvektor-Pattern (`P_vector_concat`, `[a, b]`).
    VectorConcat(Vec<Pat>),
    /// Konstruktor-Applikation (`.ctor arg, …`; `None` als `func:"None"`).
    App { func: String, args: Vec<Pat> },
    /// `{ f := p, … }`.
    Struct { name: String, fields: Vec<(String, Pat)> },
    /// `p as x`: druckt nur `p` (#557).
    As { pat: Box<Pat>, id: String },
    /// `p : typ`: druckt nur `p` (#531), außer Breiten-Ascription im
    /// Vektor (#526-530).
    Typ { typ: Typ, pat: Box<Pat> },
    /// Typ-Variablen-Binder: druckt nur `p` (#556).
    Var { pat: Box<Pat> },
    /// Kein `doc_pat`-Gegenstück (#470, #565): harter Fehler.
    Not(Box<Pat>),
    /// Kein `doc_pat`-Gegenstück (#565): harter Fehler.
    Or(Box<Pat>, Box<Pat>),
    /// Kein `doc_pat`-Gegenstück (#565): harter Fehler.
    VectorSubrange { id: String, lo: String, hi: String },
    /// Kein `doc_pat`-Gegenstück (#565): harter Fehler.
    StringAppend(Vec<Pat>),
}

impl LeanPrinter {
    /// Pattern in Ausdruckskontext drucken (`need_parens:false`,
    /// weder Vektor-Element noch `match_bv`-Arm).
    pub fn print_pat_top<W: fmt::Write>(&self, out: &mut W, pat: &Pat) -> Result<(), Error> {
        self.print_pat_inner(out, pat, false, false, false)
    }

    /// Pattern in einem `match`/`match_bv`-Arm drucken.
    pub fn print_pat<W: fmt::Write>(
        &self,
        out: &mut W,
        pat: &Pat,
        in_match_bv: bool,
    ) -> Result<(), Error> {
        self.print_pat_inner(out, pat, false, false, in_match_bv)
    }

    /// Typ-Ascription eines `P_typ`-Patterns für `let` (#588-589):
    /// `Some(Typ)` genau dann, wenn das Pattern `P_typ` ist.
    #[must_use]
    pub fn pat_typ_ascription(pat: &Pat) -> Option<&Typ> {
        match pat {
            Pat::Typ { typ, .. } => Some(typ),
            _ => None,
        }
    }

    fn print_pat_inner<W: fmt::Write>(
        &self,
        out: &mut W,
        pat: &Pat,
        need_parens: bool,
        in_vector: bool,
        in_match_bv: bool,
    ) -> Result<(), Error> {
        match pat {
            Pat::Wild => {
                out.write_char('_')?;
                Ok(())
            }
            Pat::Lit(lit) => self.print_lit_with_width(out, lit, false),
            Pat::Typ { typ, pat } => {
                if in_vector {
                    match typ {
                        // `P_typ (bit, p)` → `p:1` (#526).
                        Typ::Bit(_) => {
                            self.print_pat_inner(out, pat, false, false, in_match_bv)?;
                            out.write_str(":1")?;
                            Ok(())
                        }
                        // `P_typ (bits(n)/bitvector(n), p)` mit Konstante
                        // → `p:n` (#527-529).
                        Typ::BitVec(n) => match n.as_ref() {
                            Nexp::Const(i) => {
                                self.print_pat_inner(out, pat, false, false, in_match_bv)?;
                                write!(out, ":{i}")?;
                                Ok(())
                            }
                            _ => {
                                self.print_pat_inner(out, pat, false, false, in_match_bv)?;
                                out.write_char(':')?;
                                self.print_typ(out, typ)?;
                                Ok(())
                            }
                        },
                        // Sonst `p:Typ` (#530).
                        _ => {
                            self.print_pat_inner(out, pat, false, false, in_match_bv)?;
                            out.write_char(':')?;
                            self.print_typ(out, typ)?;
                            Ok(())
                        }
                    }
                    // (Wie OCaml fällt `in_vector` innen auf `false`
                    // zurück; nur `P_vector`-Elemente erben `true`.)
                } else {
                    // Außerhalb von Vektoren entfällt `P_typ` (#531).
                    self.print_pat_inner(out, pat, false, false, in_match_bv)
                }
            }
            Pat::Id(name) => {
                // Enum-Mitglieder mit führendem `.` (#538).
                if self.opts.enum_members.contains(name) {
                    out.write_char('.')?;
                }
                out.write_str(&fixup_match_id(name))?;
                Ok(())
            }
            Pat::Tuple(pats) => {
                out.write_char('(')?;
                for (i, p) in pats.iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    self.print_pat_inner(out, p, false, false, in_match_bv)?;
                }
                out.write_char(')')?;
                Ok(())
            }
            Pat::List(pats) => {
                out.write_char('[')?;
                for (i, p) in pats.iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    self.print_pat_inner(out, p, false, false, in_match_bv)?;
                }
                out.write_char(']')?;
                Ok(())
            }
            Pat::Vector(pats)
                if !in_match_bv && pats.iter().all(|p| matches!(p, Pat::Lit(_))) =>
            {
                // Nur-Literale-Vektor außerhalb `match_bv` → `0b…` (#543-545;
                // `for_all` auf `[]` ist wahr, also auch `P_vector []`).
                out.write_str("0b")?;
                for p in pats {
                    self.print_pat_inner(out, p, false, true, in_match_bv)?;
                }
                Ok(())
            }
            Pat::Vector(pats) => {
                for p in pats {
                    self.print_pat_inner(out, p, false, true, in_match_bv)?;
                }
                Ok(())
            }
            Pat::VectorConcat(pats) => {
                out.write_char('[')?;
                for (i, p) in pats.iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    self.print_vector_concat_part(out, p, in_match_bv)?;
                }
                out.write_char(']')?;
                Ok(())
            }
            Pat::App { func, .. } if func == "None" => {
                // `P_app (None, _)` → `none` (#548).
                out.write_str("none")?;
                Ok(())
            }
            Pat::App { func, args } => {
                // `.ctor arg, …` (#549-555). Nulläre drucken als `.A`
                // (OCaml hängt einen umbrechbaren Space an; insignifikant).
                let mut buf = String::new();
                buf.push('.');
                buf.push_str(&fixup_match_id(func));
                if !args.is_empty() {
                    buf.push(' ');
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            buf.push_str(", ");
                        }
                        let mut arg = String::new();
                        self.print_pat_inner(&mut arg, a, true, false, in_match_bv)?;
                        buf.push_str(&arg);
                    }
                }
                if need_parens {
                    out.write_char('(')?;
                    out.write_str(&buf)?;
                    out.write_char(')')?;
                } else {
                    out.write_str(&buf)?;
                }
                Ok(())
            }
            // `P_var`/`P_as` lassen den Binder fallen (#556-557;
            // `in_vector` fällt wie dort auf `false` zurück).
            Pat::Var { pat } => self.print_pat_inner(out, pat, false, false, in_match_bv),
            Pat::As { pat, .. } => self.print_pat_inner(out, pat, false, false, in_match_bv),
            Pat::Struct { fields, .. } => {
                out.write_str("{ ")?;
                for (i, (id, p)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    // Feld-Ids ohne `fixup` (OCaml #560: plain `doc_id_ctor`).
                    self.print_id_ctor(out, id, false)?;
                    out.write_str(" := ")?;
                    self.print_pat_inner(out, p, false, false, in_match_bv)?;
                }
                out.write_str(" }")?;
                Ok(())
            }
            Pat::Cons(hd, tl) => {
                out.write_char('(')?;
                self.print_pat_inner(out, hd, false, false, in_match_bv)?;
                out.write_str(" :: ")?;
                self.print_pat_inner(out, tl, false, false, in_match_bv)?;
                out.write_char(')')?;
                Ok(())
            }
            Pat::Not(inner) => Err(Error::Unsupported {
                msg: format!("pattern P_not ({inner:?}) is not translatable"),
                span: crate::Span::DUMMY,
            }),
            Pat::Or(a, b) => Err(Error::Unsupported {
                msg: format!("pattern P_or ({a:?} | {b:?}) is not translatable"),
                span: crate::Span::DUMMY,
            }),
            Pat::VectorSubrange { id, lo, hi } => Err(Error::Unsupported {
                msg: format!("pattern P_vector_subrange ({id}[{hi}:{lo}]) is not translatable"),
                span: crate::Span::DUMMY,
            }),
            Pat::StringAppend(parts) => Err(Error::Unsupported {
                msg: format!("pattern P_string_append ({} parts) is not translatable", parts.len()),
                span: crate::Span::DUMMY,
            }),
        }
    }

    /// Ein `vector_concat`-Teil (`doc_vector_concat/doc_part`, #567-586):
    /// Literale als Roh-Bits, `P_typ` transparent, `P_vector` als
    /// Komma-Folge; ein nacktes `P_id` ohne Breite ist ein harter Fehler
    /// (`unclear width`, #580).
    fn print_vector_concat_part<W: fmt::Write>(
        &self,
        out: &mut W,
        part: &Pat,
        in_match_bv: bool,
    ) -> Result<(), Error> {
        match part {
            Pat::Lit(Lit::Bin { bits, .. }) => {
                out.write_str(bits.as_str())?;
                Ok(())
            }
            Pat::Lit(Lit::Hex { digits, .. }) => {
                for ch in digits.chars() {
                    let v = ch.to_digit(16).ok_or_else(|| Error::Unsupported {
                        msg: format!("pattern hex digit {ch:?} is not translatable"),
                        span: crate::Span::DUMMY,
                    })?;
                    write!(out, "{v:04b}")?;
                }
                Ok(())
            }
            Pat::Typ { pat, .. } => self.print_vector_concat_part(out, pat, in_match_bv),
            Pat::Vector(pats) => {
                for (i, p) in pats.iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    self.print_vector_concat_part(out, p, in_match_bv)?;
                }
                Ok(())
            }
            Pat::Id(name) => Err(Error::Unsupported {
                msg: format!(
                    "bitvector subpattern `{}` has unclear width (needs a P_typ ascription)",
                    fixup_match_id(name)
                ),
                span: crate::Span::DUMMY,
            }),
            other => Err(Error::Unsupported {
                msg: format!("unexpected pattern in match_bv vector_concat pattern ({other:?})"),
                span: crate::Span::DUMMY,
            }),
        }
    }
}

/// Striktes Gegenstück zu OCaml `is_bitvector_pattern` (#637-638):
/// `P_vector`/`P_vector_concat`, durch genau `P_as` hindurch. Durch
/// `P_typ`/`P_var`/`P_or` wird — anders als in `sail-patterns` — NICHT
/// hindurchgeschaut (Fundliste: `P_as`-unwrap).
#[must_use]
pub fn is_bitvector_pattern(pat: &Pat) -> bool {
    match pat {
        Pat::Vector(_) | Pat::VectorConcat(_) => true,
        Pat::As { pat, .. } => is_bitvector_pattern(pat),
        _ => false,
    }
}

/// Gegenstück zu OCaml `is_match_bv` (#640-641): ein Arm (hier direkt
/// als Pattern) erzwingt `match_bv`-Emission, sobald ein Arm ein
/// [`is_bitvector_pattern`] ist.
#[must_use]
pub fn is_match_bv(arms: &[Pat]) -> bool {
    arms.iter().any(is_bitvector_pattern)
}

/// Lower a canonical value pattern onto the printer subset
/// ([`Pat`]). Returns `None` when the pattern needs type information or
/// name resolution the caller cannot provide:
/// - unresolvable identifiers / literals (see
///   [`lit_from_ast`][crate::conv::lit_from_ast]; real literals have no
///   printer counterpart),
/// - `P_typ` whose type does not lower ([`typ_from_ast`][crate::conv::typ_from_ast]).
///
/// `P_not`/`P_or`/`P_vector_subrange`/`P_string_append` lower to their
/// hard-error [`Pat`] variants on purpose, so printing still fails with
/// [`Error::Unsupported`] at the exact OCaml `failwith` site instead of
/// succeeding silently.
#[must_use]
pub fn pat_from_ast(p: &AstPat, it: &impl Interner) -> Option<Pat> {
    fn lower_all<I: Interner>(ps: &[AstPat], it: &I) -> Option<Vec<Pat>> {
        ps.iter().map(|q| pat_from_ast(q, it)).collect()
    }
    match &p.kind {
        PatKind::Wild => Some(Pat::Wild),
        PatKind::Lit(l) => lit_from_ast(l, it).map(Pat::Lit),
        PatKind::Id(name) => it.resolve(name.symbol).map(|s| Pat::Id(s.to_owned())),
        PatKind::Tuple(ps) => lower_all(ps, it).map(Pat::Tuple),
        PatKind::List(ps) => lower_all(ps, it).map(Pat::List),
        PatKind::Cons(hd, tl) => Some(Pat::Cons(
            Box::new(pat_from_ast(hd, it)?),
            Box::new(pat_from_ast(tl, it)?),
        )),
        PatKind::Vector(ps) => lower_all(ps, it).map(Pat::Vector),
        PatKind::VectorConcat(ps) => lower_all(ps, it).map(Pat::VectorConcat),
        PatKind::App { func, args } => Some(Pat::App {
            func: it.resolve(func.symbol)?.to_owned(),
            args: lower_all(args, it)?,
        }),
        PatKind::Struct { name, fields, .. } => {
            let struct_name = match name {
                StructName::Id(n) => it.resolve(n.symbol)?.to_owned(),
                StructName::Anon => String::new(),
            };
            let mut out = Vec::with_capacity(fields.len());
            for (id, fp) in fields {
                out.push((it.resolve(id.symbol)?.to_owned(), pat_from_ast(fp, it)?));
            }
            Some(Pat::Struct { name: struct_name, fields: out })
        }
        PatKind::As { pat, id } => Some(Pat::As {
            pat: Box::new(pat_from_ast(pat, it)?),
            id: it.resolve(id.symbol)?.to_owned(),
        }),
        PatKind::Typ { typ, pat } => Some(Pat::Typ {
            typ: typ_from_ast(typ, it)?,
            pat: Box::new(pat_from_ast(pat, it)?),
        }),
        // `P_var` prints only the inner pattern (#556).
        PatKind::Var { pat, .. } => Some(Pat::Var { pat: Box::new(pat_from_ast(pat, it)?) }),
        PatKind::Not(inner) => Some(Pat::Not(Box::new(pat_from_ast(inner, it)?))),
        PatKind::Or(a, b) => Some(Pat::Or(
            Box::new(pat_from_ast(a, it)?),
            Box::new(pat_from_ast(b, it)?),
        )),
        PatKind::VectorSubrange { id, lo, hi } => Some(Pat::VectorSubrange {
            id: it.resolve(id.symbol)?.to_owned(),
            lo: lo.clone(),
            hi: hi.clone(),
        }),
        PatKind::StringAppend(ps) => lower_all(ps, it).map(Pat::StringAppend),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FunSig, Nexp, Typ};

    fn printer() -> LeanPrinter {
        LeanPrinter::with_default()
    }

    fn render(pat: &Pat, in_match_bv: bool) -> Result<String, Error> {
        let p = printer();
        let mut buf = String::new();
        p.print_pat(&mut buf, pat, in_match_bv)?;
        Ok(buf)
    }

    #[test]
    fn wild_and_lit_drop_width_in_patterns() {
        assert_eq!(render(&Pat::Wild, false).unwrap(), "_");
        // `width:false` (#525): kein `#w`-Suffix im Pattern …
        let lit = Pat::Lit(Lit::Hex { digits: "ff".into(), width: Some(8) });
        assert_eq!(render(&lit, false).unwrap(), "0xFF");
        assert_eq!(render(&lit, true).unwrap(), "0xFF");
        // … aber negative Zahlen bleiben geklammert (#226, #402).
        let num = Pat::Lit(Lit::Num(-7));
        assert_eq!(render(&num, false).unwrap(), "(-7)");
    }

    #[test]
    fn p_typ_drops_outside_vectors() {
        // `P_typ` entfällt außerhalb von Vektoren (#531).
        let pat = Pat::Typ {
            typ: Typ::Int,
            pat: Box::new(Pat::Id("x".into())),
        };
        assert_eq!(render(&pat, false).unwrap(), "x");
        // Ascription separat (#588-589).
        assert_eq!(LeanPrinter::pat_typ_ascription(&pat), Some(&Typ::Int));
        assert_eq!(LeanPrinter::pat_typ_ascription(&Pat::Wild), None);
    }

    #[test]
    fn p_typ_width_ascriptions_in_vectors() {
        // `p:1` für `bit` (#526).
        let bit = Pat::Typ {
            typ: Typ::Bit("b".into()),
            pat: Box::new(Pat::Id("x".into())),
        };
        let mut buf = String::new();
        printer().print_pat_inner(&mut buf, &bit, false, true, true).unwrap();
        assert_eq!(buf, "x:1");
        // `p:n` für `bits(n)` mit Konstante (#527-529).
        let bits = Pat::Typ {
            typ: Typ::BitVec(Box::new(Nexp::Const(3))),
            pat: Box::new(Pat::Id("y".into())),
        };
        let mut buf = String::new();
        printer().print_pat_inner(&mut buf, &bits, false, true, true).unwrap();
        assert_eq!(buf, "y:3");
        // Sonst `p:Typ` (#530).
        let other = Pat::Typ {
            typ: Typ::Bool,
            pat: Box::new(Pat::Id("z".into())),
        };
        let mut buf = String::new();
        printer().print_pat_inner(&mut buf, &other, false, true, true).unwrap();
        assert_eq!(buf, "z:Bool");
    }

    #[test]
    fn id_uses_fixup_and_enum_dot() {
        // `fixup_match_id` (#506-510, #539).
        assert_eq!(render(&Pat::Id("Some".into()), false).unwrap(), "some");
        assert_eq!(render(&Pat::Id("early_return".into()), false).unwrap(), "throw");
        // Enum-Mitglieder mit `.` (#538).
        let mut opts = crate::Options::default();
        opts.enum_members.insert("A".to_string());
        let p = LeanPrinter::new(opts);
        let mut buf = String::new();
        p.print_pat(&mut buf, &Pat::Id("A".into()), false).unwrap();
        assert_eq!(buf, ".A");
    }

    #[test]
    fn vector_lits_only_outside_match_bv() {
        let bits = || {
            Pat::Vector(vec![
                Pat::Lit(Lit::Bin { bits: "1".into(), width: None }),
                Pat::Lit(Lit::Bin { bits: "0".into(), width: None }),
                Pat::Lit(Lit::Bin { bits: "1".into(), width: None }),
            ])
        };
        // `0b…` nur außerhalb `match_bv` (#543-545).
        assert_eq!(render(&bits(), false).unwrap(), "0b101");
        assert_eq!(render(&bits(), true).unwrap(), "101");
        // Gemischte Vektoren konkatenieren ohne `0b`.
        let mixed = Pat::Vector(vec![Pat::Wild, Pat::Id("x".into())]);
        assert_eq!(render(&mixed, false).unwrap(), "_x");
    }

    #[test]
    fn app_ctor_dot_prefix_and_none() {
        assert_eq!(render(&Pat::App { func: "None".into(), args: vec![] }, false).unwrap(), "none");
        // `.ctor arg, …` (#549-555).
        let app = Pat::App {
            func: "Some".into(),
            args: vec![Pat::Id("x".into())],
        };
        assert_eq!(render(&app, false).unwrap(), ".some x");
        // Nullär als `.A`.
        let nullary = Pat::App { func: "A".into(), args: vec![] };
        assert_eq!(render(&nullary, false).unwrap(), ".A");
        // Verschachtelt mit Klammern (`need_parens`, #554).
        let nested = Pat::App {
            func: "Pair".into(),
            args: vec![
                Pat::App { func: "Some".into(), args: vec![Pat::Wild] },
                Pat::Wild,
            ],
        };
        assert_eq!(render(&nested, false).unwrap(), ".Pair (.some _), _");
    }

    #[test]
    fn as_and_var_drop_binder() {
        // `P_as`/`P_var` drucken nur innen (#556-557).
        let as_pat = Pat::As { pat: Box::new(Pat::Wild), id: "v".into() };
        assert_eq!(render(&as_pat, false).unwrap(), "_");
        let var_pat = Pat::Var { pat: Box::new(Pat::Id("x".into())) };
        assert_eq!(render(&var_pat, false).unwrap(), "x");
    }

    #[test]
    fn struct_and_cons_shapes() {
        let st = Pat::Struct {
            name: "S".into(),
            fields: vec![
                ("f".into(), Pat::Wild),
                ("g".into(), Pat::Id("x".into())),
            ],
        };
        assert_eq!(render(&st, false).unwrap(), "{ f := _, g := x }");
        let cons = Pat::Cons(Box::new(Pat::Wild), Box::new(Pat::Wild));
        assert_eq!(render(&cons, false).unwrap(), "(_ :: _)");
        let tup = Pat::Tuple(vec![Pat::Wild, Pat::Wild]);
        assert_eq!(render(&tup, false).unwrap(), "(_, _)");
        let list = Pat::List(vec![Pat::Wild]);
        assert_eq!(render(&list, false).unwrap(), "[_]");
    }

    #[test]
    fn vector_concat_parts_and_width_errors() {
        // Hex expandiert zu Roh-Bits, `P_typ` ist transparent (#570-583).
        let concat = Pat::VectorConcat(vec![
            Pat::Lit(Lit::Hex { digits: "a".into(), width: None }),
            Pat::Typ {
                typ: Typ::BitVec(Box::new(Nexp::Const(2))),
                pat: Box::new(Pat::Lit(Lit::Bin { bits: "10".into(), width: None })),
            },
        ]);
        assert_eq!(render(&concat, true).unwrap(), "[1010, 10]");
        // Nacktes `P_id` ohne Breite: harter Fehler (#580).
        let bad = Pat::VectorConcat(vec![Pat::Id("x".into())]);
        assert!(matches!(render(&bad, true), Err(Error::Unsupported { .. })));
    }

    #[test]
    fn not_or_subrange_stringappend_are_hard_errors() {
        for pat in [
            Pat::Not(Box::new(Pat::Wild)),
            Pat::Or(Box::new(Pat::Wild), Box::new(Pat::Wild)),
            Pat::VectorSubrange { id: "x".into(), lo: "0".into(), hi: "3".into() },
            Pat::StringAppend(vec![Pat::Wild]),
        ] {
            let err = render(&pat, false).unwrap_err();
            assert!(matches!(err, Error::Unsupported { .. }), "{pat:?}");
            assert!(!err.to_string().contains("sorry"));
        }
    }

    #[test]
    fn bitvector_detection_is_strict_as_only() {
        // Direkt + durch `P_as` (#637-638).
        assert!(is_bitvector_pattern(&Pat::Vector(vec![])));
        assert!(is_bitvector_pattern(&Pat::VectorConcat(vec![])));
        assert!(is_bitvector_pattern(&Pat::As {
            pat: Box::new(Pat::Vector(vec![])),
            id: "v".into(),
        }));
        // Strikt: NICHT durch `P_typ`/`P_var`/`P_or` (#638 wörtlich).
        assert!(!is_bitvector_pattern(&Pat::Typ {
            typ: Typ::Bool,
            pat: Box::new(Pat::Vector(vec![])),
        }));
        assert!(!is_bitvector_pattern(&Pat::Var {
            pat: Box::new(Pat::Vector(vec![])),
        }));
        assert!(!is_match_bv(&[Pat::Wild, Pat::Id("x".into())]));
        assert!(is_match_bv(&[Pat::Wild, Pat::Vector(vec![])]));
    }

    #[test]
    fn ceiling_sig_still_renders() {
        // Rauchtest: neue Options-Felder ändern keine bestehenden Pfade.
        let sig = FunSig {
            name: "f".into(),
            args: vec![("x".into(), Typ::Nat)],
            ret: Typ::Nat,
            is_monadic: false,
            early_return: false,
            has_loop: false,
            span: crate::Span::new(crate::FileId(0), 1, 1),
        };
        assert_eq!(
            printer().render_fun_stub(&sig, Some("x")).unwrap(),
            "def f (x : Nat) : Nat := x\n"
        );
    }
}
