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
//! Lane 110 hebt zusätzlich die Bridge-`deferred`-Reste (`Type`/`Let`/
//! `Constraint`/`Register`/`Measure`/`Outcome`/`Instantiation`) als
//! Skelett-Knoten auf `sail_ast::DefKind`: echte Namen, unbekannte Rümpfe
//! als `Unknown`/`Wild`/`Undef`-Platzhalter (vgl. `Val`-Skelett, leere
//! `Mapdef`-Klauseln statt Fake-Klauseln). In [`BridgeOut::deferred`]
//! liegt nur noch wirklich Unbekanntes (`Other`, z.B. `mutual`).
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
    Ast, ConstraintKind, DecSpec, Def, DefKind, Exp, ExpKind, FileId, Fixity as AstFixity,
    Funcl, IdGen, Interner, Mapcl, MapclKind, Name, Node, NodeId, Order, PExp, PExpKind,
    Pat, PatKind, RecOpt, ScatteredDef, SimpleInterner, Span as AstSpan, Subst, Symbol,
    TAnnotOpt, Typ, TypArgKind, TypKind, TypSchm, TypeDefKind, ValSpec,
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
/// + nur noch wirklich unbekannte `mini_ast`-Rest-Defs (`Other`, spätere Lanes).
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
    let mut b = Bridge::new();
    for sd in &parsed.defs {
        b.def(sd);
    }
    b.finish(&parsed.path)
}

pub(crate) struct Bridge {
    pub(crate) ids: IdGen,
    pub(crate) interner: SimpleInterner,
    pub(crate) fixities: crate::infix::FixityEnv,
    pub(crate) fid: FileId,
    pub(crate) defs: Vec<Def>,
    pub(crate) unparsed: Vec<Unparsed>,
    pub(crate) deferred: Vec<SpannedDef>,
}

impl Bridge {
    pub(crate) fn new() -> Self {
        Bridge {
            ids: IdGen::new(),
            interner: SimpleInterner::new(),
            fixities: crate::infix::FixityEnv::sail_defaults(),
            fid: FileId(0),
            defs: Vec::new(),
            unparsed: Vec::new(),
            deferred: Vec::new(),
        }
    }

    pub(crate) fn finish(self, path: &str) -> BridgeOut {
        BridgeOut {
            ast: Ast::new(self.defs),
            interner: self.interner,
            files: vec![path.to_owned()],
            unparsed: self.unparsed,
            deferred: self.deferred,
        }
    }
}

fn cvt_span(fid: FileId, s: &MiniSpan) -> AstSpan {
    AstSpan::new(fid, s.start, s.end.max(s.start))
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

/// Erstes Ident-ähnliches Wort (vgl. Parser-`first_ident`):
/// `Reduce__1(op, ...)` → `Reduce__1`. `None` ohne Treffer.
fn first_word(text: &str) -> Option<&str> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let mut j = i + 1;
            while j < b.len()
                && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'\'')
            {
                j += 1;
            }
            return Some(&text[i..j]);
        }
        i += 1;
    }
    None
}

/// Erster Top-Level-`=` außerhalb von Klammern/Strings/Kommentaren
/// (`==`/`=>`/`<=`/`>=`/`!=` zählen nicht). `None` ohne Treffer
/// (Loop-Measures-Form). Schnittstellen sind ASCII (`=`), daher sind die
/// Schnitt-Offsets immer UTF-8-Grenzen.
fn split_top_eq(src: &str) -> Option<(&str, &str)> {
    let b = src.as_bytes();
    let mut depth: u32 = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() {
                    match b[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let mut nested: u32 = 1;
                i += 2;
                while i < b.len() && nested > 0 {
                    if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        nested += 1;
                        i += 2;
                    } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                        nested -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            b'(' | b'[' | b'{' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b'=' if depth == 0 => {
                let prev = if i > 0 { b[i - 1] } else { b' ' };
                let next = b.get(i + 1).copied().unwrap_or(b' ');
                if matches!(prev, b'<' | b'>' | b'=' | b'!')
                    || matches!(next, b'=' | b'>')
                {
                    i += 1;
                } else {
                    return Some((src[..i].trim_end(), src[i + 1..].trim_start()));
                }
            }
            _ => i += 1,
        }
    }
    None
}

/// `with`-Substitutionen einer `instantiation`-Rumpfzeile (`NAME with
/// a = b, c = d`): nur reine `Id = Id`-Paare werden übernommen
/// (`TyVar`-Seiten = Typsubstis bleiben Lane 02); fehlende `with`-Liste,
/// Fehlertoken oder Form-Abweichung → leere Liste (ehrliches Skelett).
/// Ein trailing Komma (`with a = b,`) akzeptiert die Grammatik ebenfalls.
fn instantiation_substs(
    src: &str,
    span: AstSpan,
    ids: &mut IdGen,
    interner: &mut SimpleInterner,
) -> Vec<Subst> {
    let kinds = lex_kinds(src, interner);
    let toks: Vec<&TokenKind> = kinds
        .iter()
        .filter(|k| !matches!(k, TokenKind::Eof | TokenKind::Error(_)))
        .collect();
    let Some(with) = toks.iter().position(|k| matches!(k, TokenKind::With)) else {
        return Vec::new();
    };
    let rest = &toks[with + 1..];
    if rest.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let from = match rest.get(i) {
            Some(TokenKind::Id(s)) => *s,
            _ => return Vec::new(),
        };
        if !matches!(rest.get(i + 1), Some(TokenKind::Eq)) {
            return Vec::new();
        }
        let to = match rest.get(i + 2) {
            Some(TokenKind::Id(s)) => *s,
            _ => return Vec::new(),
        };
        out.push(Subst::Id {
            from: Name::new(ids.fresh(), span, from, false),
            to: Name::new(ids.fresh(), span, to, false),
        });
        i += 3;
        match rest.get(i) {
            None => break,
            Some(TokenKind::Comma) => {
                i += 1;
                if i >= rest.len() {
                    break;
                }
            }
            _ => return Vec::new(),
        }
    }
    out
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
    pub(crate) fn name(&mut self, span: AstSpan, text: &str, head: &str) -> Name {
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

    pub(crate) fn push_unparsed(&mut self, kind: &'static str, src: String) -> NodeId {
        let id = self.ids.fresh();
        self.unparsed.push(Unparsed { id, kind, src });
        id
    }

    /// Unbekannter Typ-Rumpf: `Unknown`-Platzhalter wie im `Val`-Skelett
    /// (füllt der Typechecker / Lane 02).
    pub(crate) fn unknown_typ(&mut self, span: AstSpan) -> Typ {
        Node::new(self.ids.fresh(), span, TypKind::Unknown)
    }

    /// Unbekannter Ausdrucks-Rumpf (vgl. Brücken-`dummy_arm`).
    pub(crate) fn undef_exp(&mut self, span: AstSpan) -> Exp {
        Node::new(self.ids.fresh(), span, ExpKind::Undef)
    }

    /// Unbekanntes Muster (vgl. Brücken-`dummy_arm`).
    pub(crate) fn wild_pat(&mut self, span: AstSpan) -> Pat {
        Node::new(self.ids.fresh(), span, PatKind::Wild)
    }

    /// `type`/`struct`/`enum`/`union`/`bitfield`/`newtype`-Rumpf (Lane 110):
    /// Schlüsselwort → `TypeDefKind`, Name echt, Inhalte leer/`Unknown`
    /// (Mitglieder/Felder/Ctors brauchen die Vollgrammatik, Lane 02).
    /// Rohtext ins Sidecar; unbekanntes Schlüsselwort → `deferred`.
    pub(crate) fn typedef_skeleton(
        &mut self,
        span: AstSpan,
        name_text: &str,
        keyword: &str,
        head_src: &str,
        body_src: &str,
        sd: &SpannedDef,
    ) {
        let name = self.name(span, name_text, name_text);
        let kind = match keyword {
            "type" | "newtype" => {
                let typ = self.unknown_typ(span);
                let arg = Node::new(self.ids.fresh(), span, TypArgKind::Typ(typ));
                TypeDefKind::Abbrev {
                    name,
                    quant: Vec::new(),
                    arg,
                }
            }
            "struct" => TypeDefKind::Record {
                name,
                quant: Vec::new(),
                fields: Vec::new(),
                wf: true,
            },
            "enum" => TypeDefKind::Enum {
                name,
                members: Vec::new(),
                wf: true,
            },
            "union" => TypeDefKind::Variant {
                name,
                quant: Vec::new(),
                ctors: Vec::new(),
                wf: true,
            },
            "bitfield" => TypeDefKind::Bitfield {
                name,
                typ: self.unknown_typ(span),
                fields: Vec::new(),
            },
            _ => {
                self.deferred.push(sd.clone());
                return;
            }
        };
        let src = if body_src == head_src {
            head_src.to_owned()
        } else {
            join_body(head_src, body_src)
        };
        let id = self.push_unparsed("type", src);
        let typedef = Node::new(id, span, kind);
        self.defs.push(Node::new(id, span, DefKind::Type(typedef)));
    }

    /// `let`-Rumpf (Lane 110): Muster/Rumpf brauchen die Vollgrammatik →
    /// `Wild`/`Undef`, Rohtext ins Sidecar.
    pub(crate) fn let_skeleton(&mut self, span: AstSpan, pat_src: &str, body_src: &str) {
        let pat = self.wild_pat(span);
        let exp = self.undef_exp(span);
        let id = self.push_unparsed("let", join_body(pat_src, body_src));
        self.defs.push(Node::new(id, span, DefKind::Let { pat, exp }));
    }

    /// `constraint`-Rumpf (Lane 110): Platzhalter `True` (schwächste
    /// Annahme — Lane 02 ersetzt ihn durch den echten Constraint, Rohtext
    /// liegt im Sidecar).
    pub(crate) fn constraint_skeleton(&mut self, span: AstSpan, src: &str) {
        let constraint = Node::new(self.ids.fresh(), span, ConstraintKind::True);
        let id = self.push_unparsed("constraint", src.to_owned());
        self.defs
            .push(Node::new(id, span, DefKind::Constraint(constraint)));
    }

    /// `register`-Rumpf (Lane 110): Name echt, Typ `Unknown`, Init-Präsenz
    /// als `Some(Undef)`/`None` erhalten (Wert füllt Lane 02).
    pub(crate) fn register_skeleton(
        &mut self,
        span: AstSpan,
        name_text: &str,
        type_src: &str,
        init_src: &Option<String>,
    ) {
        let typ = self.unknown_typ(span);
        let init = init_src.as_ref().map(|_| self.undef_exp(span));
        let src = match init_src {
            Some(init) => format!("{type_src} = {init}"),
            None => type_src.to_owned(),
        };
        let id = self.push_unparsed("register", src);
        let name = self.name(span, name_text, name_text);
        let spec = DecSpec {
            id,
            span,
            typ,
            name,
            init,
        };
        self.defs.push(Node::new(id, span, DefKind::Register(spec)));
    }

    /// `termination_measure`-Rumpf (Lane 110): mit Top-Level-`=` →
    /// `Measure` (Name echt, Muster/Rumpf `Wild`/`Undef`), sonst →
    /// `LoopMeasures` mit leerer Liste (Schleifenmaße brauchen die
    /// Vollgrammatik). Ohne Namen → `deferred`.
    pub(crate) fn measure_skeleton(&mut self, span: AstSpan, src: &str, sd: &SpannedDef) {
        let (head, is_fun) = match split_top_eq(src) {
            Some((head, _)) => (head, true),
            None => (src, false),
        };
        let Some(name_text) = first_word(head) else {
            self.deferred.push(sd.clone());
            return;
        };
        let name = self.name(span, name_text, head);
        let id = self.push_unparsed("termination_measure", src.to_owned());
        if is_fun {
            let pat = self.wild_pat(span);
            let exp = self.undef_exp(span);
            self.defs.push(Node::new(
                id,
                span,
                DefKind::Measure { name, pat, exp },
            ));
        } else {
            self.defs.push(Node::new(
                id,
                span,
                DefKind::LoopMeasures {
                    name,
                    measures: Vec::new(),
                },
            ));
        }
    }

    /// `outcome`-Rumpf (Lane 110): Name echt, Typschema/Quantor Skelett
    /// (`Unknown`, leer — `with`-Parameter und Rümpfe brauchen die
    /// Vollgrammatik). Ohne Namen → `deferred`.
    pub(crate) fn outcome_skeleton(&mut self, span: AstSpan, src: &str, sd: &SpannedDef) {
        let Some(name_text) = first_word(src) else {
            self.deferred.push(sd.clone());
            return;
        };
        let name = self.name(span, name_text, src);
        let id = self.push_unparsed("outcome", src.to_owned());
        let typ = self.unknown_typ(span);
        self.defs.push(Node::new(
            id,
            span,
            DefKind::Outcome {
                name,
                typschm: TypSchm {
                    quant: Vec::new(),
                    typ,
                },
                quant: Vec::new(),
            },
        ));
    }

    /// `instantiation`-Rumpf (Lane 110): Name echt; `with`-Liste mit reinen
    /// `Id = Id`-Paaren → `Subst::Id` (direkt abgelesen, keine Erfindung);
    /// Typsubstis/Unförmiges → leere Liste (Volltext im Sidecar).
    pub(crate) fn instantiation_skeleton(&mut self, span: AstSpan, src: &str, sd: &SpannedDef) {
        let Some(name_text) = first_word(src) else {
            self.deferred.push(sd.clone());
            return;
        };
        let name = self.name(span, name_text, src);
        let substs = instantiation_substs(src, span, &mut self.ids, &mut self.interner);
        let id = self.push_unparsed("instantiation", src.to_owned());
        self.defs.push(Node::new(
            id,
            span,
            DefKind::Instantiation { name, substs },
        ));
    }

    pub(crate) fn def(&mut self, sd: &SpannedDef) {
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
            MiniDef::Type { name, keyword, head_src, body_src } => {
                self.typedef_skeleton(span, name, keyword, head_src, body_src, sd);
            }
            MiniDef::Let { pat_src, body_src } => {
                self.let_skeleton(span, pat_src, body_src);
            }
            MiniDef::Constraint { src } => {
                self.constraint_skeleton(span, src);
            }
            MiniDef::Register { name, type_src, init_src } => {
                self.register_skeleton(span, name, type_src, init_src);
            }
            MiniDef::Measure { src } => {
                self.measure_skeleton(span, src, sd);
            }
            MiniDef::Outcome { src } => {
                self.outcome_skeleton(span, src, sd);
            }
            MiniDef::Instantiation { src } => {
                self.instantiation_skeleton(span, src, sd);
            }
            MiniDef::Other { .. } => self.deferred.push(sd.clone()),
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

#[cfg(test)]
mod symbol_threading_tests {
    //! Lane 121: Brücken-Skelette nach Borrow-Hoist (E0499) — Verhalten
    //! identisch (E3: Code ohne Run).

    use super::*;
    use sail_ast::Interner as _;

    fn span() -> AstSpan {
        AstSpan::new(FileId(0), 0, 1)
    }

    #[test]
    fn measure_fun_skeleton_keeps_name_and_placeholders() {
        let mut b = Bridge::new();
        let sd = crate::mini_ast::SpannedDef::new(
            crate::mini_ast::Def::Measure {
                src: "f(x) = x".to_owned(),
            },
            crate::span::Span::new(crate::span::FileId(0), 0, 8),
        );
        b.measure_skeleton(span(), "f(x) = x", &sd);
        let def = b.defs.last().expect("Measure fehlt");
        match &def.kind {
            DefKind::Measure { name, pat, exp } => {
                assert_eq!(b.interner.resolve(name.symbol), Some("f"));
                assert!(matches!(pat.kind, PatKind::Wild));
                assert!(matches!(exp.kind, ExpKind::Undef));
            }
            other => panic!("erwarte Measure, got {other:?}"),
        }
        assert!(b.deferred.is_empty());
    }

    #[test]
    fn outcome_skeleton_keeps_name_and_unknown_typ() {
        let mut b = Bridge::new();
        let sd = crate::mini_ast::SpannedDef::new(
            crate::mini_ast::Def::Outcome {
                src: "o : int".to_owned(),
            },
            crate::span::Span::new(crate::span::FileId(0), 0, 7),
        );
        b.outcome_skeleton(span(), "o : int", &sd);
        let def = b.defs.last().expect("Outcome fehlt");
        match &def.kind {
            DefKind::Outcome { name, typschm, .. } => {
                assert_eq!(b.interner.resolve(name.symbol), Some("o"));
                assert!(matches!(typschm.typ.kind, TypKind::Unknown));
            }
            other => panic!("erwarte Outcome, got {other:?}"),
        }
        assert!(b.deferred.is_empty());
    }
}
