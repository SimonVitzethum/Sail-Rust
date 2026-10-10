# CORPUS.md — E3c/E4-Akzeptanzdeltas im Sail-Korpus (Lane 66)

Status: **analytisch (E3)** — rein lesend gezählt, kein Parser-Bau, keine Läufe.
Bindend entscheidet `tests/corpus.rs` auf dem Test-Rig.

## Scope und Methode

- **sail/lib (59 `.sail`-Dateien)**: vollständig manuell verifiziert.
  Obermenge von `crates/sail-prelude/sail-lib` (26 Top-Level-Dateien,
  inhaltsgleich — per `diff` Dateiliste + Stichprobe geprüft); das ist der
  Scope von `tests/corpus.rs`.
- **sail/test (1488 `.sail`-Dateien)** als Test-Modelle: heuristisch
  (Schlüsselwort-Bilanz + Risiko-Muster + Stichproben). Die
  Positions-Klassifikation jedes Treffers braucht den Parser-Lauf auf dem Rig.
- Werkzeug: lexikalisches Scannen (Kommentar-/String-Stripping:
  `//`, `/* */`, `"..."`), `then`/`else`-Bilanz plus gezielte Muster
  (`then if`, `do if`, `return/throw if`, `(let`, `do let`, `=> let`, `\{\}`).
  `$else`/`$then`-Pragmas sind ausgenommen; Mapping-/Case-Guard-`if`
  (ohne `then`) verfälscht die Bilanz nicht.

## Befund sail/lib (59 Dateien, verifiziert)

| # | Abweichung (E3c/E4-Delta) | Anzahl | Fundstellen |
| --- | --- | --- | --- |
| A | nacktes `if..then` (ohne `else`) als Then-/Else-Branch | **0** | — |
| B | `else`-`if`-Kette ohne Abschluss-`else` | **0** | — |
| C | bare `let`/`var` (ohne `in`) außerhalb von Blöcken | **0** | — |
| D | `{}` (leerer Block) | **0** | — |
| — | nackte `if`s gesamt (alle Statement-Position, E3c-konform) | **2** | `concurrency_interface/read_write_v2.sail:241` (`if constraint('CHERI) then {`), `:254` (`if request.size > 0 then {`) |

Kennzahlen: `then`=101, `else`=99 (Differenz = die 2 obigen);
`{}`=0; `then if`/`do if`/`return if`/`throw if`=0;
`=> let` ohne `in`=0; `do let`=0; Call-Arg-`let` ohne `in`=0.
Alle bare `let`/`var` ohne `in` sind `BlockItem`s in `{...}`-Funktionsrümpfen
oder Top-Level-`DefAux::Let` (z. B. `smt.sail:78`, `float/common.sail:22-36`);
alle `let..in`-Ketten enden in geschlossenen Ausdrücken.

## Befund sail/test (1488 Dateien, heuristisch + Stichproben)

- `then`=851, `else`=755 → Diff **96** (Obergrenze nackter `if`s).
  **59 Dateien** mit `then`-Überschuss. Top:
  `lean/early_return.sail` (18/5), `lean/SailTinyArmV2.sail` (34/26),
  `smt/real.unsat.sail` (4/0), `exec/loop_exception.sail` (4/1),
  `exec/vector_example.sail` (3/0).
- Stichproben (`early_return`, `loop_termination`, `loop_exception`,
  `try_return`, `vector_example`, `real_prop`, `all_even_vector_length`):
  alle geprüften nackten `if`s stehen in **Statement-Position**
  (`if c then { ... };`, `if i > 5 then return false;`) — E3c-konform als
  `BlockItem`-`Exp`; Zweige sind geschlossen (`return …`, Blöcke).
- `else if`: 351 Treffer; alle geprüften Ketten haben ein Abschluss-`else`;
  kein Gegenbeispiel gefunden.
- bare `let`/`var`: 1890 heuristische Zeilen (inkl. False Positives:
  mehrzeilige `… in`, Def-Level, BlockItems); alle Risiko-Muster negativ
  (einziges Call-Arg-`let` mit `in`: `typecheck/pass/and_let_bool.sail:6`).
- `{}`: 34 rohe Treffer, alle verifiziert **kein leerer Block**:
  31× `$project#`-Direktiven (project_parser, out of scope),
  1× Attribut-Objekt `$[empty_object {}]` (grammatisch `AttrData`-Objekt),
  1× leeres Bitfield `typecheck/pass/bitfield_empty.sail:7`
  (grammatisch `RDefBody?`), 1× String-Stripping-Artefakt.

## Empfehlung je Abweichung

- **A/B (sail/lib: 0)**: kein Handlungsbedarf. Die E3c-Regel
  (Then-/Else-Zweige + Tails = `ClosedExp`) lehnt im Bibliothekskorpus
  nichts ab — der `tests/corpus.rs`-Lauf sollte diesbezüglich grün sein.
- **Nackte Statement-`if`s (sail/lib: 2, sail/test: ~96)**: kein
  Handlungsbedarf. Offene `if`s als `BlockItem` sind grammatisch vorgesehen
  (`BlockItem ::= Exp`, `Exp` kennt `if..then` ohne `else`); kein Fix,
  kein Rewrite der Modelle.
- **C (0)**: kein Handlungsbedarf. E4 (`in`-Pflicht außerhalb von Blöcken,
  strikter `Block`) deckt alle Fundstellen ab (BlockItems, Def-Level,
  `let..in`-Ketten, `=> let..in`-Arme).
- **D (0 echte)**: kein Handlungsbedarf. `$project`-`{}` und leere
  Attribut-Objekte/Bitfields sind keine `Block`s; strikter `Block`
  (E4) bleibt.
- **Test-Modelle allgemein**: Falls der Rig-`corpus.rs`-Lauf (derzeit nur
  `sail-lib`) je auf `sail/test` erweitert wird, sind E3c-Ablehnungen dort
  **erwartete** Deltas (Test-Modelle enthalten u. a. Fail-Fixtures), keine
  Grammatik-Bugs. Erst bei Rejections in `sail-lib` die Grammatik anfassen.
- **D1/D2-Begriffsklärung**: D1/D2 im Sinne von `GRAMMAR.md`
  (`Typ`-Index-Range vs. Tupel, Paren vs. Tupel) sind grammatikinterne,
  bewusst unveränderte Regeln und lexikalisch ohne Parser nicht trennbar
  zählbar — hier nicht quantifiziert; der erste Rig-Build entscheidet.
  Die vier oben gezählten Kategorien sind die E3c/E4-Akzeptanzdeltas, die
  `tests/corpus.rs` überwacht.
