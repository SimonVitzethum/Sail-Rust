# ADAPTER.md — Lane 77: echte Einstiege gegen die Grammatik-Schnittstelle

Stand: 2026-10-10, Branch `lane/77-parser-real`.

## 1. Warum ein Adapter?

Lane 77 braucht echte Einstiege `parse_file/typ/exp/pat/def → sail-ast`
mit Span-Fehlern „auf generierter Grammatik“. Die Grammatik (Lane 17/59)
ist **noch Skelett**:

- `crates/sail-grammar/build.rs` ist No-op (`process_root` auskommentiert).
- `crates/sail-grammar/src/lib.rs`: alle `parse_*` rufen `skeleton()`
  (lexen, dann `Err("{what}: skeleton parser (LALRPOP tables not generated …)")`).
- `GRAMMAR.md` beschreibt dagegen bereits die generierende Welt
  (`lalrpop_mod!(sail)`, `FileParser`/`TypParser`/…); `src/sail.lalrpop`
  (1495 Zeilen) und `tests/grammar_coverage.rs` (erwartet echte Parses)
  liegen vor.

Per Lane-77-Spec („falls noch Skelett: gegen Schnittstelle programmieren +
`ADAPTER.md`“) ist diese Datei der Vertrag dafür.

## 2. Was Lane 77 liefert (heute echt)

| Einstieg (`sail_parse`) | Implementierung | Heute |
|---|---|---|
| `parse_typ / parse_exp / parse_pat / parse_def` | `src/real.rs` → `sail_grammar::parse_*` → `src/convert.rs` → `sail-ast` | Schnittstelle verdrahtet; Fehler ehrlich mit Span (`FileId(0)`, `[0, len)`), nie Fake-Knoten |
| `RealParser` (Batch-Fragmente, geteilter Interner) | dto. | dto. |
| `parse_file_real(path, src) → BridgeOut` | Mini-AST-Split (Spans!) + `let`/`register`/`constraint` über die neuen Einstiege, Rest via `bridge` | Datei-Ebene läuft heute: `let`/`register`/`constraint` parken in `deferred` (Grammatik-Skelett), alles andere wie bisher |
| `convert.rs`: ~alle `sail-grammar::ast`-Varianten → `sail-ast` | total gegen die heutige `ast.rs`-Form | kompiliert gegen die Schnittstelle; aktiv ab Tabellen-Generierung |

`BridgeOut`-Form (`ast` + `interner` + `files` + `unparsed` + `deferred`)
ist unverändert, damit Lane 11/69 weiterlaufen. Datei-Ebene schlägt nie
wegen Fragment-Lücken fehl (verlustfreies Parken).

## 3. Schnittstelle, gegen die programmiert wurde

- `sail_grammar::parse_file(&str) -> Result<Vec<ast::Def>, String>`
- `sail_grammar::parse_typ / parse_typschm / parse_exp / parse_pat / parse_def`
  (gleiche Form; `parse_typschm` nur intern via `RealParser::parse_typschm` genutzt)
- `sail_grammar::ast::*` in der Form von `src/ast.rs` (526 Zeilen, Stand siehe Lane)
- `sail_grammar::lexer::tokenize` (Byte-Offsets; Fehler kommen als `String`
  aus `parse_*` und werden auf den Fragment-Span gemappt)

Annahmen über Tupel-Reihenfolgen (aus Menhir-Konvention, nicht verifizierbar
ohne Tabellen — bei der Rig-Abnahme gegen `parser.mly` prüfen):

- `Exist(kopts, cond, body)`: zweites Feld = Constraint.
- `PatExp::Exp(pat, body)` / `When(pat, guard, body)`; `PatExpBody::When(guard, body)`.
- `FieldExp::Field(designator, value)`; `MapCl::When(clause, guard)`.
- `FunDef` trägt kein `rec`-Flag → `function rec` senkt vorerst auf `NonRec`.
- `Typ::Fn` rechtsgenestet → Argumentliste wird flachgeklopft.
- `Record`-/`Enum`-WF-Flags: Grammatik trägt keine → `wf: true` angenommen.
- `Extern.pure: None` → `false`.

## 4. Konverter-Abdeckung (`convert.rs`)

Treue Abbildung: `Id/Var/App/Tuple/Fn/Bidir/Exist`, alle Literale
(`Zero`/`One` → `"0"`/`"1"`), `TypSchm` (Pfeil → `Fn`/`Bidir`-Knoten),
`Pat` (14 Varianten), `MPat` (→ Wert-Muster), fast alle `Exp` (33 minus
unten), `FunDef/Funcl/MapDef/MapCl/TypeDef/ValSpec/RegDef/DefaultDef/
ScatteredDef/Subst/Pragma/Outcome/MutRec/Measure/LoopMeasures`,
`NExp`-/`Constraint`-Teilmengen (`+ - *`, Vergleiche, `& |`, `in {…}`),
`LExp`-Teilmenge (Id, Feld, Typ-Annotation, Tupel, `@`).

Bewusste `Err`-Fälle (Fehler mit Span statt erfundener Semantik):

- Infix-Ketten allgemein (Faltung ist Elaboration; `GRAMMAR.md`: out of scope).
- `Typ::{Lit (außer Num/Zero/One in NExp-Position), Dec, Inc, NSet, If}`.
- `Exp::{VectorUpdate}` + Suffixe außer Feld/Typ-Annotation (kein Core-Knoten).
- `Pat::As` mit Nicht-Namen; gemischte Pattern-Operatoren; Doppel-Guards.
- `TypeUnion::AnonRecord`; Enum-Typparameter/Enum-Werte; `Kind::Order`;
  nicht-`Order`-Defaults; Outcome-Rümpfe; Effekte in Typschemata.
- `Pat/Exp/Mapcl/Def`-Attribute, Doku, `private`-Hüllen (Core-Slots fehlen;
  Brücken-Präzedenz).
- `Bidir`-Mapcl-Rümpfe → `Undef` (Muster-als-Ausdruck braucht Elaboration;
  Präzedenz: Brücken-`dummy_arm`).
- `KOpt` ohne Kind → `Type`; `Kind::Nat` → `Int`; `MPat` → `Pat`;
  `Config(s)` → `Config([s])`; `Fixity`-Level `u8` → String;
  `Punning(f)` → `{ f = f }`; `foreach` ohne Schritt/Ordnung → `1`/`Inc`.

Alle Knoten erhalten den Fragment-Span (Grammatik-AST ohne Locations);
Namen via Byte-Heuristik (`is_operator`) wie die Brücke.

## 5. Deferred-Gründe in `parse_file_real`

- `Type / Measure / Outcome / Instantiation / Other`: Mini-AST wirft
  Schlüssel-Header-Infos weg (Type-Keyword unter `type/struct/enum/…`,
  Header bei nichtleerem Body), Rekonstruktion wäre Ratespiel → `deferred`.
- `Let / Register / Constraint`: heute `deferred` (Skelett), nach
  Tabellen-Generierung echt (keine Codeänderung nötig).

## 6. Flip-Checkliste (Tabellen landen)

1. `sail-grammar/build.rs`: `lalrpop::process_root()` aktivieren.
2. `sail-grammar/src/lib.rs`: `lalrpop_mod!(sail)` + echte `parse_*`
   (Form in `GRAMMAR.md` § Wiring).
3. Rig: `cargo test -p sail-grammar` grün (D1/D2-Entscheid per `GRAMMAR.md`).
4. Rig: `cargo test -p sail-parse` grün — `tests/real_entries.rs` enthält
   `#[ignore]`-Erfolgstests, die dann zu aktivieren sind
   (`real_typ_bool_is_id`, `real_exp_app_and_let`, `real_pat_some_is_app`,
   `real_def_val_has_typschm`, `file_real_lowers_let_register_constraint`).
5. Tupel-Annahmen aus §3 gegen `parser.mly` verifizieren.
6. Danach kann diese Datei auf einen Verweis auf `GRAMMAR.md` schrumpfen.

## 7. Nicht-Ziele (Follow-up-Lanes)

Elaboration-Faltung (Infix/Mixed-Ops), echte Knoten-Spans (Grammatik ohne
Locations — braucht Lexer-Offset-Threading durch LALRPOP), `Type`-Vollsenkung
(Header-Treue im Mini-AST), `VectorUpdate`/Vektorzugriff-Core-Knoten
(`sail-ast`-Erweiterung), `function rec`-Flag im Grammatik-AST.
