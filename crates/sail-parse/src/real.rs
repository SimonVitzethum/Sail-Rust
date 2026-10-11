//! Grammar-backed fragment conversion into `sail-ast` with explicit errors.
//!
//! `RealParser` retains interner, IDs and fixity declarations across fragment
//! calls. Each parser starts with OCaml's default fixities. Fragment spans are
//! currently attached to all converted descendants; the grammar does not yet
//! carry individual node locations.
//!
//! `parse_file_real` is a collecting, tolerant compatibility API. It first
//! converts every exact definition through the grammar, including function
//! bodies. Unsupported constructs fall back to the legacy bridge and its
//! unparsed sidecar. Compilers must reject those gaps before code generation;
//! they must not treat a successful tolerant parse as complete elaboration.

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
#[derive(Debug)]
pub struct RealParser {
    ids: IdGen,
    interner: SimpleInterner,
    fixities: crate::infix::FixityEnv,
}

impl Default for RealParser {
    fn default() -> Self { Self::new() }
}

impl RealParser {
    #[must_use]
    pub fn new() -> Self {
        RealParser {
            ids: IdGen::new(),
            interner: SimpleInterner::new(),
            fixities: crate::infix::FixityEnv::sail_defaults(),
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
            fixities: &mut self.fixities,
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
/// Spans; `let`/`register`/`constraint`/`type`/`measure`/`outcome`/
/// `instantiation` laufen über die neuen Einstiege (Knoten-Span = Def-Span),
/// alles Übrige delegiert an [`crate::bridge`]. Einstieg-Fehler fallen auf
/// das Brücken-Skelett zurück (Brücken-Philosophie: echte Namen, Rohtext im
/// `unparsed`-Sidecar für Lane 02).
pub fn parse_file_real(path: &str, src: &str) -> Result<BridgeOut, ParseError> {
    let mut b = Bridge::new();
    for sd in crate::iter_file(path, src) {
        let sd = sd?;
        // Feed the exact source through the grammar for every definition,
        // including functions and scattered clauses. Reconstructed headers
        // can lose visibility, recursion, annotations and operator names.
        if !sd.def.is_shard_marker() {
            let text = &src[sd.span.start as usize..sd.span.end as usize];
            let result = crate::convert::parse_def_src(&mut Ctx {
                ids: &mut b.ids, interner: &mut b.interner, fixities: &mut b.fixities,
            }, sd.span, text);
            if let Ok(def) = result {
                b.defs.push(def);
                continue;
            }
        }
        if !lower_file_def(&mut b, &sd) {
            b.def(&sd);
        }
    }
    Ok(b.finish(path))
}

/// `true`, wenn die Def vom Echt-Pfad behandelt wurde (Erfolg oder
/// ehrliches Brücken-Skelett); `false` → Brücke rufen.
fn lower_file_def(b: &mut Bridge, sd: &SpannedDef) -> bool {
    let span = sd.span;
    match &sd.def {
        MiniDef::Let { pat_src, body_src } => {
            let res = {
                let mut ctx = Ctx {
                    ids: &mut b.ids,
                    interner: &mut b.interner,
                    fixities: &mut b.fixities,
                };
                crate::convert::conv_let(&mut ctx, span, pat_src, body_src)
            };
            match res {
                Ok(kind) => {
                    let id = b.ids.fresh();
                    b.defs.push(Node::new(id, span, kind));
                }
                Err(_) => b.let_skeleton(span, pat_src, body_src),
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
                    fixities: &mut b.fixities,
                };
                crate::convert::conv_register(&mut ctx, span, name, type_src, init_src)
            };
            match res {
                Ok(kind) => {
                    let id = b.ids.fresh();
                    b.defs.push(Node::new(id, span, kind));
                }
                Err(_) => b.register_skeleton(span, name, type_src, init_src),
            }
            true
        }
        MiniDef::Constraint { src } => {
            let res = {
                let mut ctx = Ctx {
                    ids: &mut b.ids,
                    interner: &mut b.interner,
                    fixities: &mut b.fixities,
                };
                crate::convert::conv_constraint_def(&mut ctx, span, src)
            };
            match res {
                Ok(kind) => {
                    let id = b.ids.fresh();
                    b.defs.push(Node::new(id, span, kind));
                }
                Err(_) => b.constraint_skeleton(span, src),
            }
            true
        }
        MiniDef::Type { name, keyword, head_src, body_src } => {
            let full = if body_src == head_src {
                format!("{keyword} {head_src}")
            } else {
                format!("{keyword} {head_src} = {body_src}")
            };
            lower_def_text(b, span, &full, |b| {
                b.typedef_skeleton(span, name, keyword, head_src, body_src, sd);
            });
            true
        }
        MiniDef::Measure { src } => {
            let full = format!("termination_measure {src}");
            lower_def_text(b, span, &full, |b| {
                b.measure_skeleton(span, src, sd);
            });
            true
        }
        MiniDef::Outcome { src } => {
            let full = format!("outcome {src}");
            lower_def_text(b, span, &full, |b| {
                b.outcome_skeleton(span, src, sd);
            });
            true
        }
        MiniDef::Instantiation { src } => {
            let full = format!("instantiation {src}");
            lower_def_text(b, span, &full, |b| {
                b.instantiation_skeleton(span, src, sd);
            });
            true
        }
        _ => false,
    }
}

/// Volltext über `parse_def_src` senken (Grammatik-Tabellen, sobald
/// generiert); bei Fehler das Brücken-Skelett bauen — nie `deferred`.
/// Der rekonstruierte Volltext braucht das Schlüsselwort (Lane 110:
/// `MiniDef::Type` trägt es, sonst wäre Rekonstruktion Ratespiel).
fn lower_def_text(
    b: &mut Bridge,
    span: Span,
    full: &str,
    fallback: impl FnOnce(&mut Bridge),
) {
    let res = {
        let mut ctx = Ctx {
            ids: &mut b.ids,
            interner: &mut b.interner,
                    fixities: &mut b.fixities,
        };
        crate::convert::parse_def_src(&mut ctx, span, full)
    };
    match res {
        Ok(def) => b.defs.push(def),
        Err(_) => fallback(b),
    }
}
