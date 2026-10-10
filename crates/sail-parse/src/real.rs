//! Echte Parser-Einstiege auf der generierten Grammatik (Lane 77).
//!
//! * [`parse_typ`], [`parse_exp`], [`parse_pat`], [`parse_def`]:
//!   Einzelfragmente → echte `sail-ast`-Knoten (Namen/IDs/Interner echt,
//!   Knoten-Spans = Fragment-Span, da [`sail_grammar::ast`] keine Locations
//!   trägt). Fehler sind [`ParseError`] mit Span.
//! * [`parse_file_real`]: ganze Datei → [`BridgeOut`], wie
//!   [`crate::bridge_file`], aber `let`/`register`/`constraint` werden
//!   zusätzlich über die neuen Einstiege echt gesenkt (statt deferred).
//!   Alles Übrige delegiert an die Brücke; was die Einstiege (noch)
//!   ablehnen, bleibt verlustfrei in `deferred` liegen — Datei-Ebene
//!   schlägt nie wegen Fragment-Lücken fehl.
//!
//! Skelett-Stand der Grammatik (keine LALRPOP-Tabellen, siehe `ADAPTER.md`):
//! Die Einstiege rufen `sail_grammar::parse_*` gegen die Schnittstelle und
//! mappen deren Fehler ehrlich auf [`ParseError`] mit Span — es werden nie
//! Fake-Knoten erfunden. Sobald die Tabellen generiert sind, liefern dieselben
//! Einstiege ohne Codeänderung echte Knoten (Konverter: [`crate::convert`]).

#![forbid(unsafe_code)]

use sail_ast::{Constraint, Def, Exp, FileId, IdGen, Node, Pat, SimpleInterner, Span, Typ, TypSchm};
use crate::bridge::{Bridge, BridgeOut};
use crate::convert::Ctx;
use crate::mini_ast::{Def as MiniDef, SpannedDef};
use crate::span::ParseError;

fn sat_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Fragment-Span: `[0, len)` in `FileId(0)` — Konvention wie
/// [`crate::parse_file`] (Dateitabelle besitzt eine spätere Lane).
fn full_span(src: &str) -> Span {
    Span::new(FileId(0), 0, sat_u32(src.len()))
}

fn wrap(span: Span, what: &str, err: String) -> ParseError {
    ParseError::new(span, format!("{what}: {err}"))
}

/// Ergebnis eines Einzelfragment-Parsers: Knoten + zugehöriger Interner
/// (Namen sind [`sail_ast::Symbol`]e und nur damit auflösbar).
#[derive(Debug, Clone)]
pub struct ParseOut<T> {
    pub value: T,
    pub interner: SimpleInterner,
}

/// Fragment-Parser mit geteiltem Interner/ID-Raum über viele Aufrufe
/// (Batch-Aufrufe ohne Neu-Internierung derselben Namen).
#[derive(Debug, Default)]
pub struct RealParser {
    ids: IdGen,
    interner: SimpleInterner,
}

impl RealParser {
    #[must_use]
    pub fn new() -> Self {
        RealParser {
            ids: IdGen::new(),
            interner: SimpleInterner::new(),
        }
    }

    #[must_use]
    pub fn interner(&self) -> &SimpleInterner {
        &self.interner
    }

    #[must_use]
    pub fn into_interner(self) -> SimpleInterner {
        self.interner
    }

    fn ctx(&mut self) -> Ctx<'_> {
        Ctx {
            ids: &mut self.ids,
            interner: &mut self.interner,
        }
    }

    /// Typ-Fragment senken (`typ_eof` der Grammatik).
    pub fn parse_typ(&mut self, src: &str, span: Span) -> Result<Typ, ParseError> {
        crate::convert::parse_typ_src(&mut self.ctx(), span, src)
            .map_err(|e| wrap(span, "parse_typ", e))
    }

    /// Typschema-Fragment senken (`typschm_eof`).
    pub fn parse_typschm(&mut self, src: &str, span: Span) -> Result<TypSchm, ParseError> {
        crate::convert::parse_typschm_src(&mut self.ctx(), span, src)
            .map_err(|e| wrap(span, "parse_typschm", e))
    }

    /// Constraint-Fragment senken (als `Typ` geparst, vgl.
    /// `DefAux::Constraint(Typ)`).
    pub fn parse_constraint(
        &mut self,
        src: &str,
        span: Span,
    ) -> Result<Constraint, ParseError> {
        crate::convert::parse_constraint_src(&mut self.ctx(), span, src)
            .map_err(|e| wrap(span, "parse_constraint", e))
    }

    /// Ausdrucks-Fragment senken (`exp_eof`).
    pub fn parse_exp(&mut self, src: &str, span: Span) -> Result<Exp, ParseError> {
        crate::convert::parse_exp_src(&mut self.ctx(), span, src)
            .map_err(|e| wrap(span, "parse_exp", e))
    }

    /// Muster-Fragment senken (`pat` + EOF).
    pub fn parse_pat(&mut self, src: &str, span: Span) -> Result<Pat, ParseError> {
        crate::convert::parse_pat_src(&mut self.ctx(), span, src)
            .map_err(|e| wrap(span, "parse_pat", e))
    }

    /// Einzeldefinition senken (`def_eof`).
    pub fn parse_def(&mut self, src: &str, span: Span) -> Result<Def, ParseError> {
        crate::convert::parse_def_src(&mut self.ctx(), span, src)
            .map_err(|e| wrap(span, "parse_def", e))
    }
}

/// Typ-Fragment senken (One-Shot; Span = gesamtes Fragment).
pub fn parse_typ(src: &str) -> Result<ParseOut<Typ>, ParseError> {
    let span = full_span(src);
    let mut p = RealParser::new();
    let value = p.parse_typ(src, span)?;
    Ok(ParseOut {
        value,
        interner: p.into_interner(),
    })
}

/// Ausdrucks-Fragment senken (One-Shot).
pub fn parse_exp(src: &str) -> Result<ParseOut<Exp>, ParseError> {
    let span = full_span(src);
    let mut p = RealParser::new();
    let value = p.parse_exp(src, span)?;
    Ok(ParseOut {
        value,
        interner: p.into_interner(),
    })
}

/// Muster-Fragment senken (One-Shot).
pub fn parse_pat(src: &str) -> Result<ParseOut<Pat>, ParseError> {
    let span = full_span(src);
    let mut p = RealParser::new();
    let value = p.parse_pat(src, span)?;
    Ok(ParseOut {
        value,
        interner: p.into_interner(),
    })
}

/// Einzeldefinition senken (One-Shot).
pub fn parse_def(src: &str) -> Result<ParseOut<Def>, ParseError> {
    let span = full_span(src);
    let mut p = RealParser::new();
    let value = p.parse_def(src, span)?;
    Ok(ParseOut {
        value,
        interner: p.into_interner(),
    })
}

/// Eine `.sail`-Datei parsen und echt auf `sail-ast` senken.
///
/// Ablauf: [`crate::parse_file`] spaltet streaming in [`SpannedDef`]s mit
/// Spans; `let`/`register`/`constraint` laufen über die neuen Einstiege
/// (Knoten-Span = Def-Span), alles Übrige delegiert an [`crate::bridge`].
/// Einstieg-Fehler parken die Def verlustfrei in `deferred` (Brücken-Philosophie).
pub fn parse_file_real(path: &str, src: &str) -> Result<BridgeOut, ParseError> {
    let parsed = crate::parse_file(path, src)?;
    let mut b = Bridge::new();
    for sd in &parsed.defs {
        if !lower_file_def(&mut b, sd) {
            b.def(sd);
        }
    }
    Ok(b.finish(&parsed.path))
}

/// `true`, wenn die Def vom Echt-Pfad behandelt wurde (Erfolg oder
/// ehrliches `deferred`-Parken); `false` → Brücke rufen.
fn lower_file_def(b: &mut Bridge, sd: &SpannedDef) -> bool {
    let span = sd.span;
    match &sd.def {
        MiniDef::Let { pat_src, body_src } => {
            let res = {
                let mut ctx = Ctx {
                    ids: &mut b.ids,
                    interner: &mut b.interner,
                };
                crate::convert::conv_let(&mut ctx, span, pat_src, body_src)
            };
            match res {
                Ok(kind) => {
                    let id = b.ids.fresh();
                    b.defs.push(Node::new(id, span, kind));
                }
                Err(_) => b.deferred.push(sd.clone()),
            }
            true
        }
        MiniDef::Register {
            name,
            type_src,
            init_src,
        } => {
            let res = {
                let mut ctx = Ctx {
                    ids: &mut b.ids,
                    interner: &mut b.interner,
                };
                crate::convert::conv_register(&mut ctx, span, name, type_src, init_src)
            };
            match res {
                Ok(kind) => {
                    let id = b.ids.fresh();
                    b.defs.push(Node::new(id, span, kind));
                }
                Err(_) => b.deferred.push(sd.clone()),
            }
            true
        }
        MiniDef::Constraint { src } => {
            let res = {
                let mut ctx = Ctx {
                    ids: &mut b.ids,
                    interner: &mut b.interner,
                };
                crate::convert::conv_constraint_def(&mut ctx, span, src)
            };
            match res {
                Ok(kind) => {
                    let id = b.ids.fresh();
                    b.defs.push(Node::new(id, span, kind));
                }
                Err(_) => b.deferred.push(sd.clone()),
            }
            true
        }
        _ => false,
    }
}
