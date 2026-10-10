//! Brücke von den `mini_ast`-Defs auf `sail-ast` (Lane 63).
//!
//! Stand: Lane 02 liefert nur ein Definitions-Skelett (Header + Rohtext),
//! Lane 17 (`sail-grammar`) ist Skelett ohne generierte Tabellen (siehe
//! `crates/sail-grammar/GRAMMAR.md`); die Vollgrammatik für `Exp`/`Typ`
//! fehlt also. Diese Brücke löst die Lane-61-Blockaden #4–#7
//! (`Val`/`Fundef`/`Mapdef`/`Scattered`) trotzdem minimalinvasiv auf:
//! Namen, `rec`-Flag, Fixity-, Overload-, Default- und Pragma-Metadaten
//! (inkl. der Lane-11-Shard-Marker `file_start`/`file_end`, Blockade #42)
//! landen auf echten `sail_ast`-Knoten. Ungeparste Bodies bleiben als
//! Rohtext im [`BridgeOut::unparsed`]-Sidecar für den Lane-02-Vollausbau;
//! alle übrigen Def-Arten bleiben verlustfrei als `mini_ast`-Defs in
//! [`BridgeOut::deferred`] liegen.
//!
//! Namens-/Dispatch-Infos stammen aus dem Branch-Stand-Lexer (`sail-lex`,
//! fehlertolerant): `scattered`-Unterart, `rec`-Flag und Operator-Erkennung
//! lesen Tokenarten statt String-Splits. Das Grammatik-Skelett
//! (`sail-grammar/src/ast.rs`: `Funcl` = Name + Klausel, `MapCl`-/
//! `ScatteredDef`-Varianten) ist nur als Form-Referenz gelesen — absichtlich
//! keine `sail-grammar`-Dependency: `parse_*` meldet dort Skelett-Status,
//! und Klausel-Bodies brauchen die echte Grammatik (kein Rewrite hier).
//! Platzhalter-Arms nutzen `PatKind::Wild` + `ExpKind::Undef`; es wird nie
//! Semantik erfunden (leere Klausel-Listen statt Fake-Klauseln bei `Mapdef`,
//! ein Skelett-`Funcl` pro `Fundef`-Knoten wie bisher ein Knoten pro
//! `and`-Kette, vgl. `AST_NEEDED.md`).

use sail_ast::{
    Ast, Def, DefKind, ExpKind, FileId, Fixity as AstFixity, Funcl, IdGen, Interner as _,
    Mapcl, MapclKind, Name, Node, NodeId, Order, PExp, PExpKind, PatKind, RecOpt,
    ScatteredDef, SimpleInterner, Span as AstSpan, Symbol, TAnnotOpt, TypKind, TypSchm,
    ValSpec,
};
use sail_lex::{FileId as LexFileId, TokenKind, lex};

use crate::mini_ast::{Def as MiniDef, ParsedFile, PragmaArg, SpannedDef};
use crate::parse_file;
use crate::span::{ParseError, Span as MiniSpan};

/// Rohtext einer noch nicht grammatisch geparsten Def, per `sail-ast`-`NodeId`
/// dem Brücken-Knoten zugeordnet (Lane 02 füllt die echten `Exp`/`Typ`-Knoten).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unparsed {
    pub id: NodeId,
    pub kind: &'static str,
    pub src: String,
}

/// Ergebnis der Brücke: `sail-ast` + Interner + Datei-Tabelle + Rohtext-Sidecar
/// + verlustfrei aufbewahrte `mini_ast`-Rest-Defs (spätere Lanes).
#[derive(Debug, Clone)]
pub struct BridgeOut {
    pub ast: Ast,
    pub interner: SimpleInterner,
    /// Dateipfade, `sail_ast::FileId.0` ist der Index hier (Single-File-Brücke:
    /// genau ein Eintrag; die Lane-11-Tabelle führt später mehrere).
    pub files: Vec<String>,
    pub unparsed: Vec<Unparsed>,
    pub deferred: Vec<SpannedDef>,
}

/// Eine `.sail`-Datei parsen und auf `sail-ast` brücken.
pub fn bridge_file(path: &str, src: &str) -> Result<BridgeOut, ParseError> {
    Ok(bridge_parsed(&parse_file(path, src)?))
}

/// Bereits geparste Datei auf `sail-ast` brücken (reine Konvertierung).
pub fn bridge_parsed(parsed: &ParsedFile) -> BridgeOut {
    let mut b = Bridge {
        ids: IdGen::new(),
        interner: SimpleInterner::new(),
        fid: FileId(0),
        defs: Vec::new(),
        unparsed: Vec::new(),
        deferred: Vec::new(),
    };
    for sd in &parsed.defs {
        b.def(sd);
    }
    BridgeOut {
        ast: Ast::new(b.defs),
        interner: b.interner,
        files: vec![parsed.path.clone()],
        unparsed: b.unparsed,
        deferred: b.deferred,
    }
}

struct Bridge {
    ids: IdGen,
    interner: SimpleInterner,
    fid: FileId,
    defs: Vec<Def>,
    unparsed: Vec<Unparsed>,
    deferred: Vec<SpannedDef>,
}

fn sat_u32(b: usize) -> u32 {
    u32::try_from(b).unwrap_or(u32::MAX)
}

fn cvt_span(fid: FileId, s: &MiniSpan) -> AstSpan {
    AstSpan::new(fid, sat_u32(s.start_byte), sat_u32(s.end_byte.max(s.start_byte)))
}

/// Byte-Heuristik (Fallback): kein Ident-Start → Operator-Name.
fn is_op_name(name: &str) -> bool {
    name.bytes().next().is_some_and(|b| !(b.is_ascii_alphabetic() || b == b'_'))
}

/// Tokenarten eines Header-Schnipsels aus dem Branch-Stand-Lexer.
/// `LexFileId(0)` ist Scratch: Head-Offsets sind relativ, Spans ungenutzt.
/// Payloads landen im übergebenen Interner (Lane 71: `lex` interniert);
/// die Bridge nutzt dafür ihren eigenen AST-Interner, sodass die
/// `Symbol`s direkt auflösbar bleiben.
fn lex_kinds(head: &str, interner: &mut SimpleInterner) -> Vec<TokenKind> {
    let res = lex(LexFileId(0), head, interner);
    res.code_tokens().into_iter().map(|t| t.kind.clone()).collect()
}

/// Operator-Flag aus dem Lexer: `Id(name)` vs `Op(name)`; `None` ohne Treffer.
/// Symbole werden über `interner` auf den Namen zurück abgebildet.
fn op_by_lex(head: &str, name: &str, interner: &mut SimpleInterner) -> Option<bool> {
    for k in lex_kinds(head, interner) {
        match k {
            TokenKind::Id(n) if interner.resolve(n) == Some(name) => return Some(false),
            TokenKind::Op(n) if interner.resolve(n) == Some(name) => return Some(true),
            _ => {}
        }
    }
    None
}

/// `rec`-Flag aus dem Lexer: erstes Token `rec` + Namens-Token dahinter
/// (`function rec f(...)`; ein `rec(...)`-Kopf ohne Folgenamen zählt nicht).
fn has_rec(head: &str, interner: &mut SimpleInterner) -> bool {
    let ks: Vec<TokenKind> = lex_kinds(head, interner)
        .into_iter()
        .filter(|k| !matches!(k, TokenKind::Eof | TokenKind::Error(_)))
        .collect();
    if !matches!(ks.first(), Some(TokenKind::Id(s)) if interner.resolve(*s) == Some("rec")) {
        return false;
    }
    matches!(ks.get(1), Some(TokenKind::Id(_) | TokenKind::Op(_)))
}

fn join_body(head: &str, body: &str) -> String {
    if body.trim().is_empty() {
        head.to_owned()
    } else {
        format!("{head} = {body}")
    }
}

/// Keyword-Token → kanonisches Wort; `variant` ist im Lexer kein Keyword
/// (fällt dort auf `Id`), der Aufrufer behandelt es separat.
fn kw_of(k: &TokenKind) -> Option<&'static str> {
    Some(match k {
        TokenKind::Function => "function",
        TokenKind::Mapping => "mapping",
        TokenKind::Clause => "clause",
        TokenKind::End => "end",
        TokenKind::Scattered => "scattered",
        TokenKind::Enum => "enum",
        TokenKind::Union => "union",
        _ => return None,
    })
}

/// Ident/Operator/TyVar-Wort an Position `i` (mit `operator`-Präfix-Skip).
/// Symbole werden über `interner` aufgelöst (Lane 71: keine `String`-Payloads).
fn name_word(toks: &[&TokenKind], mut i: usize, interner: &impl Interner) -> Option<String> {
    if matches!(toks.get(i), Some(TokenKind::OpKw)) {
        i += 1;
    }
    match toks.get(i) {
        Some(TokenKind::Id(s) | TokenKind::Op(s) | TokenKind::TyVar(s)) => {
            interner.resolve(*s).map(str::to_owned)
        }
        _ => None,
    }
}

impl Bridge {
    fn name(&mut self, span: AstSpan, text: &str, head: &str) -> Name {
        let is_operator =
            op_by_lex(head, text, &mut self.interner).unwrap_or_else(|| is_op_name(text));
        let sym: Symbol = self.interner.intern(text);
        Name::new(self.ids.fresh(), span, sym, is_operator)
    }

    fn dummy_arm(&mut self, span: AstSpan) -> PExp {
        let pat = Node::new(self.ids.fresh(), span, PatKind::Wild);
        let body = Node::new(self.ids.fresh(), span, ExpKind::Undef);
        PExp {
            id: self.ids.fresh(),
            span,
            kind: PExpKind::PatExp { pat, body },
        }
    }

    fn skeleton_funcl(&mut self, span: AstSpan, fname: &str, head: &str) -> Funcl {
        let name = self.name(span, fname, head);
        let arm = self.dummy_arm(span);
        Funcl {
            id: self.ids.fresh(),
            span,
            name,
            arm,
        }
    }

    fn push_unparsed(&mut self, kind: &'static str, src: String) -> NodeId {
        let id = self.ids.fresh();
        self.unparsed.push(Unparsed { id, kind, src });
        id
    }

    fn def(&mut self, sd: &SpannedDef) {
        let span = cvt_span(self.fid, &sd.span);
        match &sd.def {
            MiniDef::Val { name, sig_src } => {
                let nm = self.name(span, name, sig_src);
                let typ = Node::new(self.ids.fresh(), span, TypKind::Unknown);
                let id = self.push_unparsed("val", sig_src.clone());
                let spec = ValSpec {
                    id,
                    span,
                    name: nm,
                    typschm: TypSchm {
                        quant: Vec::new(),
                        typ,
                    },
                    extern_opt: None,
                };
                self.defs.push(Node::new(id, span, DefKind::Val(spec)));
            }
            MiniDef::Fundef {
                name,
                head_src,
                body_src,
            } => {
                let funcl = self.skeleton_funcl(span, name, head_src);
                let rec = if has_rec(head_src, &mut self.interner) {
                    RecOpt::Rec
                } else {
                    RecOpt::NonRec
                };
                let id = self.push_unparsed("function", join_body(head_src, body_src));
                self.defs.push(Node::new(
                    id,
                    span,
                    DefKind::Fundef {
                        rec,
                        annot: TAnnotOpt::None,
                        funcls: vec![funcl],
                    },
                ));
            }
            MiniDef::Mapdef {
                name,
                head_src,
                body_src,
            } => {
                let nm = self.name(span, name, head_src);
                let id = self.push_unparsed("mapping", join_body(head_src, body_src));
                self.defs.push(Node::new(
                    id,
                    span,
                    DefKind::Mapdef {
                        name: nm,
                        annot: TAnnotOpt::None,
                        clauses: Vec::new(),
                    },
                ));
            }
            MiniDef::Impl {
                name,
                head_src,
                body_src,
            } => {
                let funcl = self.skeleton_funcl(span, name, head_src);
                let id = self.push_unparsed("impl", join_body(head_src, body_src));
                self.defs.push(Node::new(id, span, DefKind::Impl(funcl)));
            }
            MiniDef::Scattered { head_src, body_src } => {
                if let Some(def) = self.scattered(span, head_src, body_src) {
                    self.defs.push(def);
                } else {
                    self.deferred.push(sd.clone());
                }
            }
            MiniDef::Fixity { prec, level, name } => {
                let fixity = match prec.as_str() {
                    "infix" => AstFixity::Infix,
                    "infixl" => AstFixity::InfixL,
                    "infixr" => AstFixity::InfixR,
                    _ => {
                        self.deferred.push(sd.clone());
                        return;
                    }
                };
                let idn = self.name(span, name, name);
                let id = self.ids.fresh();
                self.defs.push(Node::new(
                    id,
                    span,
                    DefKind::Fixity {
                        fixity,
                        level: level.clone(),
                        id: idn,
                    },
                ));
            }
            MiniDef::Overload { name, ids } => {
                let nm = self.name(span, name, name);
                let mut overloads = Vec::with_capacity(ids.len());
                for o in ids {
                    overloads.push(self.name(span, o, o));
                }
                let id = self.ids.fresh();
                self.defs.push(Node::new(
                    id,
                    span,
                    DefKind::Overload { name: nm, overloads },
                ));
            }
            MiniDef::Default { kind, value } => {
                let order = match (kind.as_str(), value.as_str()) {
                    ("Order", "inc") => Order::Inc,
                    ("Order", "dec") => Order::Dec,
                    _ => {
                        self.deferred.push(sd.clone());
                        return;
                    }
                };
                let id = self.ids.fresh();
                self.defs.push(Node::new(id, span, DefKind::Default(order)));
            }
            MiniDef::Pragma { name, arg } => {
                let payload = match arg {
                    PragmaArg::Line(s) | PragmaArg::Structured(s) => s.clone(),
                };
                let id = self.ids.fresh();
                self.defs.push(Node::new(
                    id,
                    span,
                    DefKind::Pragma {
                        key: name.clone(),
                        payload,
                    },
                ));
            }
            MiniDef::Type { .. }
            | MiniDef::Let { .. }
            | MiniDef::Constraint { .. }
            | MiniDef::Register { .. }
            | MiniDef::Measure { .. }
            | MiniDef::Outcome { .. }
            | MiniDef::Instantiation { .. }
            | MiniDef::Other { .. } => self.deferred.push(sd.clone()),
        }
    }

    /// `scattered`-Kopf → `ScatteredDef`. Bare `clause ...`-Köpfe (aus dem
    /// `function`/`mapping`-Zweig, ohne Eltern-Keyword) werden per `<->` im
    /// Body getrennt: `Bidir`-Token → `Mapcl`, sonst `Funcl` (Lane 11 gruppiert
    /// später ohnehin nach Namen; Lane 02 ersetzt die Heuristik).
    /// `scattered union` fällt auf `Variant` (Ctor-Köpfe folgen als `Unioncl`,
    /// Header-Form identisch). Ohne Namen → `None` (Aufrufer stellt zurück).
    fn scattered(&mut self, span: AstSpan, head: &str, body: &str) -> Option<Def> {
        let kinds = lex_kinds(head, &mut self.interner);
        let toks: Vec<&TokenKind> = kinds
            .iter()
            .filter(|k| {
                !matches!(
                    k,
                    TokenKind::Eof
                        | TokenKind::Error(_)
                        | TokenKind::DocBlock(_)
                        | TokenKind::DocLine(_)
                )
            })
            .collect();
        if toks.is_empty() {
            return None;
        }
        // `end NAME`-Köpfe aus dem `end`-Zweig des Skelett-Parsers.
        if kw_of(toks[0]) == Some("end") {
            let nm = name_word(&toks, 1, &self.interner)?;
            let name = self.name(span, &nm, head);
            let id = self.push_unparsed("scattered", head.to_owned());
            return Some(Node::new(id, span, DefKind::Scattered(ScatteredDef::End(name))));
        }
        let mut i = 0;
        if kw_of(toks[0]) == Some("scattered") {
            i = 1;
        }
        let sub = toks.get(i).and_then(|k| kw_of(k));
        // Bare `clause ...` (Eltern-Keyword im Skelett verloren): Heuristik.
        if toks.get(i).and_then(|k| kw_of(k)) == Some("clause") {
            let nm = name_word(&toks, i + 1, &self.interner)?;
            let is_map = lex_kinds(body, &mut self.interner)
                .iter()
                .any(|k| matches!(k, TokenKind::Bidir));
            let id = self.push_unparsed("scattered", join_body(head, body));
            let kind = if is_map {
                let clause = Mapcl {
                    id: self.ids.fresh(),
                    span,
                    kind: MapclKind::Forwards(self.dummy_arm(span)),
                };
                ScatteredDef::Mapcl {
                    name: self.name(span, &nm, head),
                    clause,
                }
            } else {
                ScatteredDef::Funcl(self.skeleton_funcl(span, &nm, head))
            };
            return Some(Node::new(id, span, DefKind::Scattered(kind)));
        }
        match sub {
            Some("function") => {
                let nm = name_word(&toks, i + 1, &self.interner)?;
                let name = self.name(span, &nm, head);
                let id = self.push_unparsed("scattered", join_body(head, body));
                Some(Node::new(
                    id,
                    span,
                    DefKind::Scattered(ScatteredDef::Function {
                        name,
                        annot: TAnnotOpt::None,
                    }),
                ))
            }
            Some("mapping") => {
                let nm = name_word(&toks, i + 1, &self.interner)?;
                let name = self.name(span, &nm, head);
                let id = self.push_unparsed("scattered", join_body(head, body));
                Some(Node::new(
                    id,
                    span,
                    DefKind::Scattered(ScatteredDef::Mapping {
                        name,
                        annot: TAnnotOpt::None,
                    }),
                ))
            }
            Some("enum") => {
                let nm = name_word(&toks, i + 1, &self.interner)?;
                let name = self.name(span, &nm, head);
                let id = self.push_unparsed("scattered", join_body(head, body));
                Some(Node::new(id, span, DefKind::Scattered(ScatteredDef::Enum(name))))
            }
            // `union` + `variant` (letzteres lexiert als `Id`): Header-Form
            // identisch, Ctor-Klauseln folgen separat.
            _ => {
                let is_union = sub == Some("union")
                    || matches!(toks.get(i), Some(TokenKind::Id(s))
                        if self.interner.resolve(*s) == Some("variant"));
                if !is_union {
                    return None;
                }
                let nm = name_word(&toks, i + 1, &self.interner)?;
                let name = self.name(span, &nm, head);
                let id = self.push_unparsed("scattered", join_body(head, body));
                Some(Node::new(
                    id,
                    span,
                    DefKind::Scattered(ScatteredDef::Variant {
                        name,
                        quant: Vec::new(),
                    }),
                ))
            }
        }
    }
}
