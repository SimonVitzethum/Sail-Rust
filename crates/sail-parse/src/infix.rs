//! Infix-Elaboration: Präzedenz-Tabelle + `InfixChain`-Faltung (Lane 109).
//!
//! Portiert die Falt-Semantik von `sail/src/lib/infix_parser.mly` (468 Zeilen)
//! samt Token-Zuordnung aus `sail/src/lib/initial_check.ml`
//! (`infix_token_supplier`, `to_infix_parser_op`).
//!
//! Die LALRPOP-Grammatik (`sail-grammar`) parst Infix-Ketten flach
//! (`InfixChain`: führende Präfixe, Kopf, `(Operator, lokale Präfixe,
//! Operand)`-Schwanz); Assoziativität/Präzedenz löst erst diese Elaboration
//! auf — wie OCaml, wo `parser.mly` flache `E_infix`-/`ATyp_infix`-Listen
//! baut und `initial_check.ml` sie durch den Infix-Parser faltet.
//!
//! ## Regeln (aus `infix_parser.mly` übernommen)
//!
//! Stufen 0 (lockerste Bindung) bis 9 (festeste); benutzerdefinierte
//! Operatoren je Stufe links (`infixl`), rechts (`infixr`) oder nicht
//! (`infix`) assoziativ. Eingebaute Operatoren schlagen die
//! Fixity-Tabelle (der OCaml-`supplier` matcht sie zuerst):
//!
//! | Seite | Stufe | Operatoren | Bindung |
//! |---|---|---|
//! | Exp | 4 | `<`, `>`, `<=`, `>=` (`E_app_infix`) | nicht |
//! | Exp | 5 | `@` (`E_vector_append`), `::` (`E_cons`) | rechts |
//! | Exp | 6 | `+`, `-` (`E_app_infix`) | links |
//! | Exp | 7 | `*` (`E_app_infix`) | links |
//! | Exp | 8 | Präfix `2^`→`pow2`, `-`→`negate` (`E_app`), Operand Stufe 9 | — |
//! | Exp | atomar | Präfix `*`→`__deref` (`E_deref`), Operand atomar | — |
//! | Typ | 4 | `<`,`<=` (lchain) / `>`,`>=` (rchain), Desugar über `&` | Kette |
//! | Typ | 5 | `in` (`ATyp_in`) | nicht |
//! | Typ | 6 | `+` (`ATyp_sum`), `-` (`ATyp_minus`) | links |
//! | Typ | 7 | `*` (`ATyp_times`) | links |
//! | Typ | 8 | Präfix `2^` (`ATyp_exp`), `-` (`ATyp_neg`), Operand Stufe 9 | — |
//!
//! Nicht mischbar auf gleicher Stufe: verschiedene Assoziativitäten
//! (z. B. eingebautes links-`+` mit benutzerdefiniertem rechts-Op auf
//! Stufe 6), nicht-assoziative Ketten (`a < b < c` im Exp ist ein Fehler),
//! lchain gegen rchain, Vergleiche gegen benutzerdefinierte Stufe-4-Ops.
//! `in` existiert nur auf der Typ-Seite, `@`/`::` nur auf der Exp-Seite,
//! `__deref` nur auf der Exp-Seite — jeweils wie im `.mly` (dort gibt es
//! schlicht keine entsprechende Regel, also ist es ein Fehler).
//!
//! ## Abbildung auf `sail-ast` (für die spätere Verdrahtung in `convert.rs`)
//!
//! * [`Folded::Infix`] mit benutzerdefiniertem/built-in `op` → `App { func:
//!   op, args: [l, r] }` (Exp) bzw. `TypKind::App` (Typ); `+`/`-`/`*` auf
//!   NExp → `Sum`/`Minus`/`Times`; `pow2`/`negate` → `Exp`/`Neg`;
//! * `Infix { op: "@", .. }` → `VectorAppend`, `op: "::"` → `Cons`;
//! * Typ-`&`-Knoten aus dem Ketten-Desugar → `ConstraintKind::And`;
//! * `Prefix { op: "pow2"|"negate", .. }` → `App { pow2|negate, [x] }`,
//!   `__deref` → `LExpKind::Deref` (nur L-Wert-Seite).

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fmt;

use sail_grammar::ast as g;

/// Assoziativität eines Infix-Operators (`Fixity`-Deklaration).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assoc {
    /// `infix N op`: keine Kettung, kein Mischen auf Stufe N.
    Non,
    /// `infixl N op`: links-nestend (`(a op b) op c`).
    Left,
    /// `infixr N op`: rechts-nestend (`a op (b op c)`).
    Right,
}

impl Assoc {
    /// Abbildung von der Grammatik-Assoziativität.
    #[must_use]
    pub fn from_grammar(assoc: g::FixAssoc) -> Self {
        match assoc {
            g::FixAssoc::Infix => Assoc::Non,
            g::FixAssoc::InfixL => Assoc::Left,
            g::FixAssoc::InfixR => Assoc::Right,
        }
    }
}

/// Fixity-Eintrag: Assoziativität + Präzedenzstufe 0–9.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fixity {
    pub assoc: Assoc,
    pub level: u8,
}

/// Elaborations-Seite: Typ- vs. Exp-Regeln aus `infix_parser.mly`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Typ,
    Exp,
}

/// Fixity-Umgebung: benutzerdefinierte Operatoren aus `DefAux::Fixity`.
/// Eingebaute Operatoren stehen nicht hier (sie schlagen die Tabelle,
/// siehe [`resolve_infix`]).
#[derive(Clone, Debug, Default)]
pub struct FixityEnv {
    map: HashMap<String, Fixity>,
}

impl FixityEnv {
    /// OCaml `Initial_check.initial_ctx` builtins. `new()` remains an empty
    /// environment for callers checking explicit operator declarations.
    pub fn sail_defaults() -> Self {
        let mut env = Self::new();
        for (op, assoc, level) in [("^", Assoc::Right, 8), ("|", Assoc::Right, 2),
            ("&", Assoc::Right, 3), ("==", Assoc::Non, 4), ("!=", Assoc::Non, 4),
            ("/", Assoc::Left, 7), ("%", Assoc::Left, 7)] {
            env.insert(op, assoc, level).expect("builtin fixity");
        }
        env
    }
    #[must_use]
    pub fn new() -> Self {
        FixityEnv {
            map: HashMap::new(),
        }
    }

    /// Ein Eintrag; Stufe muss 0–9 sein (der OCaml-Lexer akzeptiert genau
    /// eine Dezimalstelle).
    pub fn insert(&mut self, op: &str, assoc: Assoc, level: u8) -> Result<(), FoldError> {
        if level > 9 {
            return Err(FoldError::new(format!(
                "fixity level {level} out of range 0..=9"
            )));
        }
        self.map.insert(
            op.to_owned(),
            Fixity { assoc, level },
        );
        Ok(())
    }

    /// Eintrag aus einer Grammatik-`Fixity`-Deklaration.
    pub fn insert_grammar(
        &mut self,
        assoc: g::FixAssoc,
        level: u8,
        op: &str,
    ) -> Result<(), FoldError> {
        self.insert(op, Assoc::from_grammar(assoc), level)
    }

    #[must_use]
    pub fn get(&self, op: &str) -> Option<Fixity> {
        self.map.get(op).copied()
    }
}

/// Umgebung aus geparsten Definitionen einsammeln (`DefAux::Fixity`,
/// durch `private`/Attr/Doku-Hüllen hindurch).
pub fn env_from_defs(defs: &[g::Def]) -> Result<FixityEnv, FoldError> {
    let mut env = FixityEnv::new();
    for d in defs {
        collect_def(d, &mut env)?;
    }
    Ok(env)
}

fn collect_def(def: &g::Def, env: &mut FixityEnv) -> Result<(), FoldError> {
    match def {
        g::Def::Aux(aux) => {
            if let g::DefAux::Fixity(assoc, level, name) = aux {
                env.insert_grammar(*assoc, *level, name)?;
            }
            Ok(())
        }
        g::Def::Private(inner) | g::Def::Attr(_, inner) | g::Def::Doc(_, inner) => {
            collect_def(inner, env)
        }
    }
}

/// Gefalteter Infix-Baum: Blätter sind die (bereits konvertierten)
/// Operanden, innere Knoten Operator-Anwendungen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Folded<T> {
    Leaf(T),
    Prefix {
        op: String,
        arg: Box<Folded<T>>,
    },
    Infix {
        op: String,
        lhs: Box<Folded<T>>,
        rhs: Box<Folded<T>>,
    },
}

/// Elaborations-Fehler (trägt nur eine Meldung, wie `convert.rs`-`Err`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldError {
    message: String,
}

impl FoldError {
    #[must_use]
    pub fn new(message: String) -> Self {
        FoldError { message }
    }

    #[must_use]
    pub fn message(self) -> String {
        self.message
    }
}

impl fmt::Display for FoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FoldError {}

impl From<FoldError> for String {
    fn from(err: FoldError) -> String {
        err.message
    }
}

// ---------------------------------------------------------------------------
// Operator-Auflösung (Präzedenz-Tabelle)
// ---------------------------------------------------------------------------

/// Assoziativitäts-Klasse in der Faltung. `ChainL`/`ChainR` sind die
/// exklusiven Typ-Vergleichsketten (`lchain`/`rchain`); sie mischen weder
/// untereinander noch mit benutzerdefinierten Stufe-4-Ops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Left,
    Right,
    Non,
    ChainL,
    ChainR,
}

struct OpInfo {
    level: u8,
    class: Class,
}

fn class_of_assoc(assoc: Assoc) -> Class {
    match assoc {
        Assoc::Left => Class::Left,
        Assoc::Right => Class::Right,
        Assoc::Non => Class::Non,
    }
}

/// Infix-Operator auflösen: Eingebaute zuerst (wie der OCaml-`supplier`),
/// dann die Fixity-Tabelle, sonst „undeclared fixity“.
fn resolve_infix(side: Side, env: &FixityEnv, op: &str) -> Result<OpInfo, FoldError> {
    // Eingebaute beider Seiten.
    match op {
        "+" | "-" => return Ok(OpInfo { level: 6, class: Class::Left }),
        "*" => return Ok(OpInfo { level: 7, class: Class::Left }),
        "<" | "<=" => {
            return Ok(OpInfo {
                level: 4,
                class: match side {
                    Side::Typ => Class::ChainL,
                    Side::Exp => Class::Non,
                },
            });
        }
        ">" | ">=" => {
            return Ok(OpInfo {
                level: 4,
                class: match side {
                    Side::Typ => Class::ChainR,
                    Side::Exp => Class::Non,
                },
            });
        }
        _ => {}
    }
    // Seitenspezifische Eingebaute.
    match side {
        Side::Typ => {
            if op == "in" {
                return Ok(OpInfo { level: 5, class: Class::Non });
            }
            if op == "@" || op == "::" {
                return Err(FoldError::new(format!(
                    "operator `{op}` is not a type operator (infix_parser.mly typ rules)"
                )));
            }
        }
        Side::Exp => {
            if op == "@" || op == "::" {
                return Ok(OpInfo { level: 5, class: Class::Right });
            }
            if op == "in" {
                return Err(FoldError::new(
                    "operator `in` is not an expression operator (ATyp_in is type-level; infix_parser.mly typ5)"
                        .to_owned(),
                ));
            }
        }
    }
    // Benutzerdefiniert (`to_infix_parser_op`: Stufe 0–9 je Deklaration).
    match env.get(op) {
        Some(fix) => Ok(OpInfo { level: fix.level, class: class_of_assoc(fix.assoc) }),
        None => Err(FoldError::new(format!("Undeclared fixity for operator {op}"))),
    }
}

/// Präfix-Operator auflösen → eigene Stufe (Operand: eine Stufe fester).
/// Die Grammatik normalisiert bereits (`PrefixOp`: `2^`→`pow2`,
/// `-`→`negate`, `*`→`__deref`).
fn resolve_prefix(side: Side, op: &str) -> Result<u8, FoldError> {
    match op {
        "pow2" | "negate" => Ok(8),
        "__deref" => match side {
            Side::Exp => Ok(9),
            Side::Typ => Err(FoldError::new(
                "prefix `*` (__deref) is not a type operator (infix_parser.mly typ8 has no Star arm)"
                    .to_owned(),
            )),
        },
        _ => Err(FoldError::new(format!("Unknown prefix operator {op}"))),
    }
}

// ---------------------------------------------------------------------------
// Faltung (Precedence-Climbing über der flachen Kette)
// ---------------------------------------------------------------------------

/// Flache Kette als Token-Folge: Präfixe gehören jeweils zum folgenden
/// Operanden (so baut sie auch `parser.mly`).
#[derive(Clone, Debug)]
enum Tok<T> {
    Pre(String),
    Op(String),
    Atom(T),
}

struct Climber<'a, T> {
    toks: &'a [Tok<T>],
    pos: usize,
    side: Side,
    env: &'a FixityEnv,
}

impl<T: Clone> Climber<'_, T> {
    /// Operand: höchstens ein Präfix pro Stufe (OCaml lehnt z. B. `- -x`
    /// ab: `Minus exp9`, und `exp9` kennt kein Präfix).
    fn unary(&mut self, min: u8) -> Result<Folded<T>, FoldError> {
        let tok: Option<Tok<T>> = self.toks.get(self.pos).cloned();
        match tok {
            Some(Tok::Pre(p)) => {
                let level = resolve_prefix(self.side, &p)?;
                if level < min {
                    return Err(FoldError::new(format!(
                        "prefix operator `{p}` binds looser than its operand context (level {level} < {min}); add parentheses"
                    )));
                }
                self.pos += 1;
                let arg = self.climb(level + 1)?;
                Ok(Folded::Prefix { op: p, arg: Box::new(arg) })
            }
            Some(Tok::Atom(a)) => {
                self.pos += 1;
                Ok(Folded::Leaf(a))
            }
            Some(Tok::Op(o)) => Err(FoldError::new(format!(
                "unexpected infix operator `{o}` where an operand was expected"
            ))),
            None => Err(FoldError::new(
                "unexpected end of infix chain where an operand was expected".to_owned(),
            )),
        }
    }

    /// Kette ab Stufe `min` falten. Ops gleicher Stufe mischen nur bei
    /// gleicher Klasse (links mit links etc.); `Non` kettet nie; `ChainL`/
    /// `ChainR` verbrauchen ihren Lauf geschlossen mit `&`-Desugar.
    fn climb(&mut self, min: u8) -> Result<Folded<T>, FoldError> {
        let mut lhs = self.unary(min)?;
        // (Stufe, Klasse) des zuletzt in *diesem* Rahmen gefalteten Ops.
        let mut last: Option<(u8, Class)> = None;
        loop {
            let next: Option<String> = match self.toks.get(self.pos) {
                Some(Tok::Op(o)) => Some(o.clone()),
                _ => None,
            };
            let Some(op) = next else { break };
            let info = resolve_infix(self.side, self.env, &op)?;
            if info.level < min {
                break;
            }
            if let Some((prev_level, prev_class)) = last {
                if info.level == prev_level
                    && (info.class != prev_class || info.class == Class::Non)
                {
                    return Err(FoldError::new(format!(
                        "operators at level {} with different associativity need parentheses (infix_parser.mly arms do not mix)",
                        info.level
                    )));
                }
            }
            match info.class {
                Class::ChainL | Class::ChainR => {
                    lhs = self.fold_chain_run(op, info.level, info.class, lhs)?;
                    last = Some((info.level, info.class));
                }
                Class::Left => {
                    self.pos += 1;
                    let rhs = self.climb(info.level + 1)?;
                    lhs = Folded::Infix { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
                    last = Some((info.level, Class::Left));
                }
                Class::Right => {
                    self.pos += 1;
                    let rhs = self.climb(info.level)?;
                    lhs = Folded::Infix { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
                    last = Some((info.level, Class::Right));
                }
                Class::Non => {
                    self.pos += 1;
                    let rhs = self.climb(info.level + 1)?;
                    lhs = Folded::Infix { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
                    last = Some((info.level, Class::Non));
                }
            }
        }
        Ok(lhs)
    }

    /// Vergleichs-Lauf (`lchain`/`rchain`) geschlossen falten: mittlerer
    /// Operand wird dupliziert, Paare rechts-nestend mit `&` verknüpft
    /// (`desugar_lchain`/`desugar_rchain`).
    fn fold_chain_run(
        &mut self,
        first_op: String,
        level: u8,
        family: Class,
        lhs: Folded<T>,
    ) -> Result<Folded<T>, FoldError> {
        debug_assert!(level == 4);
        debug_assert!(family == Class::ChainL || family == Class::ChainR);
        self.pos += 1;
        // Ketten-Operanden sind Stufe-5-Formen (`.mly`: `typ5 LtEq ...`).
        let rhs = self.climb(5)?;
        let mut prev = rhs.clone();
        let mut pairs = vec![Folded::Infix {
            op: first_op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }];
        loop {
            let next: Option<String> = match self.toks.get(self.pos) {
                Some(Tok::Op(o)) => Some(o.clone()),
                _ => None,
            };
            let Some(op) = next else { break };
            let info = resolve_infix(self.side, self.env, &op)?;
            if info.level != level || info.class != family {
                break;
            }
            self.pos += 1;
            let rhs = self.climb(5)?;
            let pair = Folded::Infix {
                op,
                lhs: Box::new(prev),
                rhs: Box::new(rhs.clone()),
            };
            prev = rhs;
            pairs.push(pair);
        }
        let mut acc = pairs.pop().expect("first comparison");
        while let Some(pair) = pairs.pop() {
            acc = Folded::Infix {
                op: "&".to_owned(),
                lhs: Box::new(pair),
                rhs: Box::new(acc),
            };
        }
        Ok(acc)
    }
}

/// Fold references without deep-cloning surface operands. Long ARM functions
/// can contain large subtrees inside a single infix operand.
pub fn fold_chain_borrowed<'a, T>(chain: &'a g::InfixChain<T>, side: Side, env: &FixityEnv)
    -> Result<Folded<&'a T>, FoldError>
{
    let borrowed = g::InfixChain {
        prefixes: chain.prefixes.clone(), head: &chain.head,
        tail: chain.tail.iter().map(|(op, prefixes, rhs)| (op.clone(), prefixes.clone(), rhs)).collect(),
    };
    fold_chain(&borrowed, side, env)
}
/// `InfixChain` unter der Fixity-Umgebung falten.
///
/// * Unbekannte Operatoren → `Undeclared fixity`-Fehler;
/// * falsche Kettung/Mischung → Fehler (statt erfundener Semantik);
/// * Typ-Vergleichsketten duplizieren den mittleren Operanden (`T: Clone`),
///   wie `desugar_lchain`/`desugar_rchain` denselben Knoten wiederverwenden.
pub fn fold_chain<T: Clone>(
    chain: &g::InfixChain<T>,
    side: Side,
    env: &FixityEnv,
) -> Result<Folded<T>, FoldError> {
    let mut toks: Vec<Tok<T>> = Vec::with_capacity(chain.tail.len() * 2 + 2);
    for p in &chain.prefixes {
        toks.push(Tok::Pre(p.clone()));
    }
    toks.push(Tok::Atom(chain.head.clone()));
    for (op, local, arg) in &chain.tail {
        toks.push(Tok::Op(op.clone()));
        for p in local {
            toks.push(Tok::Pre(p.clone()));
        }
        toks.push(Tok::Atom(arg.clone()));
    }
    let mut climber = Climber { toks: &toks, pos: 0, side, env };
    let tree = climber.climb(0)?;
    if climber.pos != toks.len() {
        return Err(FoldError::new(
            "trailing tokens after infix chain".to_owned(),
        ));
    }
    Ok(tree)
}
