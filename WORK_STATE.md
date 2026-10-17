# Arbeitsstand — 2026-10-17 (Nachtrag)

**Das vollständige Ziel ist noch nicht erreicht, aber der Typprüfungs- und
Rewrite-Teil ist für das ganze ARMv9.4-A-Modell durch.** Der Rust-Compiler
prüft jetzt alle 85.701 Definitionen (45 Dateien, 1.707.207 Ausdrücke) und
führt die 22 Rewrite-Pässe aus: Exit 1 erst an der fehlenden Lean-Stufe
(`lean-emit`), Peak **6,5 GiB** (Limit 16 GiB, Swap 0), 13 min 27 s, 16 Worker.
Nachweis: [arm-typecheck-rewrite-16g.log](evidence/2026-10-17/arm-typecheck-rewrite-16g.log).
Es gibt weiterhin **keinen** Lean-Text, keinen `lake build` und keinen
No-Sorry-Nachweis. Der Typprüfer erzeugt noch keine elaborierte Ausgabe
(Rewrites arbeiten auf den unelaborierten Eingabedefinitionen).

Neu in diesem Abschnitt (alle mit OCaml-Vergleich an kleinen Fixtures):
`range`/`nat` als Existentiale; `div`/`mod`/`2^n`-Grundaxiome im Solver;
Literalmuster mit Flow-Typing; lokale Typvariablen (`let 'm`, `'loop_i`);
Bitfeld-Expansion (Konstruktor, Getter/Updater/Setter, `r[FELD]`); Enum-
Zahlenkonvertierungen; Constraint- und Int-Synonyme; Setter-, Feld-, Deref-
und Concat-L-Werte; `if`-Join (`union_typ`); Early-Return-Fakten; aufgeschobene
Argumente; Outcome-Instantiierung (Parameter, Constraints, Pflichtfunktionen);
Scattered-Köpfe; unannotierte globale `let`s; Termination-Measures mit
Typvariablen. Bekannte Lücken: Funktionstypen bei Outcome-Instantiierung werden
nur auf Existenz geprüft (nicht auf Subtyp); Bitfelder nehmen `default Order`
für Concat-Updates an; Mapping-/Union-Klauselkörper sind in der Typprüfung noch
nicht eigenständig geprüft.

## Nächste Arbeit

1. Elaborierte Ausgabe des Typprüfers (Typen/Effekte pro Ausdruck) auf Platte.
2. Lean-Emission im normalen CLI-Pfad, dann `lake build` und No-Sorry-Prüfung.

## Aktuelle Nachweise

Alle folgenden Speicherangaben sind systemd-cgroup-Peaks einschließlich
Unterprozessen und Dateicache, nicht nur CLI-RSS. Swap war jeweils 0.

| Prüfung | Ergebnis | Peak / Umfang | Log |
| --- | --- | --- | --- |
| Workspace-Suite vor Outcome-Erweiterung | **Exit 0**, 1.795 Tests in 132 Testprogrammen | Build 460,3 MiB bei 4-GiB-Limit; jedes Testprogramm bei 128 MiB | [final-workspace-tests.log](evidence/2026-10-17/final-workspace-tests.log) |
| Out-of-core-Teilmengengate, **16 Worker** | **Gate Exit 0**; Compiler erwartungsgemäß Exit 1 wegen fehlender Lean-Ausgabe | 567.919.927 AST-Bytes; 4.096 Definitionen, 1.052.672 Ausdrücke; Typprüfung und Rewrites bei **128 MiB**; Runtime 34,523 s | [ooc-16-workers.log](evidence/2026-10-17/ooc-16-workers.log) |
| Vollständiger ARM-Eingang, aktueller Code, **16 Worker** | **Exit 1**, abhängige Aufrufbedingung im Outcome-Körper; Lean-Stufen nicht erreicht | 31.267 Definitionen, 45 Dateien, 1.018.419.399 AST-Bytes; **1,7 GiB**, Runtime 35,539 s; Limit 16 GiB | [arm-global-environment-16g.log](evidence/2026-10-17/arm-global-environment-16g.log) |
| 10.000 symbolische numerische Knoten | Exit 0; zwei numerische Regressionen bestanden | 6,5 MiB; Threadstack 128 KiB | [affected-tests.log](evidence/2026-10-17/affected-tests.log) |
| Potenz-/Zeugen-Fokussuite | Exit 0; 463 Typprüfer-/Solver-Tests | Je Testprogramm 128-MiB-Limit | [power-focused-tests.log](evidence/2026-10-17/power-focused-tests.log) |
| Verschachtelte erwartungsgeleitete Aufrufe | Exit 0; 465 Typprüfer-/Solver-Tests | Je Testprogramm 128-MiB-Limit | [unary-guidance-focused-tests.log](evidence/2026-10-17/unary-guidance-focused-tests.log) |

Die 128-MiB-Teilmengenprüfung belegt Speicherbegrenzung mit mehreren Workern;
sie ist kein erfolgreicher Sail-/Lean-Compilerlauf. Auch der ARM-Peak von
1,7 GiB ist nur für den bis zum Typfehler ausgeführten Pfad nachgewiesen.
Das 16-GiB-Ziel der vollständigen Kompilierung bleibt offen.

## Aktueller ARM-Blocker

Die behobenen Fehler in `mono_rewrites.sail` werden passiert. Der jüngste Lauf
endet in `<sail-lib>/concurrency_interface/read_write_v1.sail`, Bereich
`6288–7970`, im ersten `outcome sail_mem_read`:

```text
constraint not proved: dependent call quantifier constraint
```

`Program` besitzt bereits Outcome-Header und lokale Val-Spezifikationen.
Die reale Körperprüfung in `crates/sail-ty/src/canonical_body.rs` prüft jetzt
Outcome-Implementierungen gegen die Outcome-Signatur mit starren Parametern und
lokalen Val-Spezifikationen in Quellreihenfolge. Bedingte Initialisierer und annotierte globale Bindungen werden bereits
verarbeitet. Der neue Zeugen-Fix für Mengenbedingungen und Alias-Symboltabellen
besteht den reduzierten OCaml-Vergleich; die erneute ARM-Prüfung steht noch aus.
Allgemeine unannotierte globale Bindungen und Outcome-Instantiierungen fehlen.

## Geprüfte Änderungen dieser Session

- Symboltabellen werden deterministisch serialisiert. Ein- und Vier-Worker-
  Rewrites liefern gleiche Inhalts-IDs nach erneutem Laden.
- Numerische Walker und Normalisierung bauen iterativ auf. Kindresultate werden
  konsumiert, statt vollständige Präfixbäume im Zwischencache zu kopieren.
  Die anfangs fehlgeschlagene Faltung `normalize_nexp(2^3) = 8` ist repariert.
- Unknown-Verdikte werden nicht mehr als dauerhaftes Solver-Ergebnis gespeichert.
- Rewrites verwenden denselben Bytebudget-Scheduler wie die Typprüfung. Alle
  erzeugten Definitionen werden auf Platte gespeichert; vorher wurde nur die
  letzte Ausgabe behalten und der Ergebnisvektor hielt vollständige ASTs.
- Ausgelassene implizite Argumente können Typinformationen aus Geschwistern,
  verschachtelten Overloads und eindeutigen äußeren Resultatsignaturen erhalten.
  Geführte innere Argumente werden weiterhin vollständig geprüft.
- Existentiale Zielprädikate müssen bewiesen werden. Zwei ursprüngliche
  Negativtests zeigten zuvor fälschlich akzeptierte Wertebereiche. Äußere
  Substitutionen bleiben erhalten, gleichnamige lokale Zeugen bleiben getrennt,
  und gewünschte Rückgabetypen werden nicht als eigene Annahmen benutzt.
- Potenz-Zeugen behalten Kongruenz durch eine konservative SMT-Funktion ohne
  hinzugefügte arithmetische Axiome. Transitive vorhandene Zeugenbedingungen
  werden entlang ihrer Substitutionen eingesammelt. Ohne passende Annahmen
  werden beliebige Potenzwerte und kleinere Schranken nicht bestätigt.
- Outcome-Implementierungen verwenden die Outcome-Signatur statt einer globalen
  Signatur ihres Implementierungsnamens. Lokale Vals bleiben auf den Block
  begrenzt. Header-Quantoren sind in lokalen Val-Spezifikationen unsichtbar,
  wie durch OCaml bestätigt.
- Annotierte globale Initialisierer werden gegen ihre Annotation geprüft.
  `return` außerhalb einer Funktion wird abgewiesen.
- Annotierte globale Bindungen werden in derselben Werthistorie erst ab der
  folgenden Quellposition sichtbar. Selbst- und Vorwärtsreferenzen bleiben
  ungültig, doppelte globale Namen werden wie in OCaml abgewiesen.
- `if`- und Blockresultate lassen sich iterativ inferieren; deklarierte
  Existentialtypen und geöffnete Wertzeugen bleiben getrennt. Der Tiefentest
  prüft 10.000 Bedingungen mit Blockhüllen auf einem 128-KiB-Threadstack.
  Allgemeine numerische Typvereinigungen fehlen noch.
- Endliche Mengenbedingungen vorhandener Existentialzeugen werden als
  SMT-Annahmen erhalten. Alias-Sichten emittieren diese Bedingungen nicht unter
  einer fremden Symboltabelle; die ursprünglichen Besitzer bleiben über ihre
  Substitutionen erreichbar. `{32, 64}` erlaubt die Breitenwahl im kleinen
  Speicher-Outcome, `{16, 64}` wird korrekt abgewiesen.

Die semantischen Änderungen haben positive und negative OCaml-Vergleiche.
Fixtures und Logs liegen unter `evidence/2026-10-17/`: Präfixe
`overload-ocaml`, `existential-ocaml`, `power-witness-ocaml`,
`unary-guidance-ocaml`, `global-bindings-ocaml` und `outcome-bodies`, `global-environment` und `outcome-record` (OCaml-Logs).
Ein untersuchter verschachtelter Existentialtyp war bereits in OCaml ungültig;
Rust weist ihn ebenfalls in der Deklarationsprüfung ab. Er wurde nicht als
zusätzliche gültige Sprachform freigeschaltet.

## Speicher und temporäre Dateien

- Schwere Läufe werden über `/tmp/opencode/sail-gate/gated-run.sh` serialisiert.
  Start nur bei `MemAvailable >= angefordertes Limit + 2048 MiB`, sonst Exit 99.
  Das beim Start fehlende Skript wurde wiederhergestellt.
- Cargo bleibt bei höchstens zwei Jobs und 4 GiB. ARM bleibt bei 16 GiB;
  die 28-GiB-Ausweichgrenze wurde nicht verwendet.
- `SAIL_RS_THREADS` steuert Frontend, Typprüfung und Rewrites; automatisch
  maximal 16. `SAIL_TYPECHECK_MEMORY` und `SAIL_REWRITE_MEMORY` begrenzen deren
  gemeinsame Eingabefenster, jeweils standardmäßig 32 MiB.
- Die Test-/ARM-/OOC-Gates reichen `TMPDIR=target/tmp` als absoluten Pfad an
  ihre systemd-Dienste weiter. ARM-/OOC-Spool liegt standardmäßig unter
  `target/spool`; Snapshot-Code lehnt RAM-Dateisysteme als Spool ab.
- Eigene Analyse-Sicherungen liegen unter `target/session-backups`. Unter
  `/tmp` benötigt das lokale RAM-Gate ungefähr 4 KiB plus eine leere Lockdatei.
  Der letzte vollständige ARM-Lauf hat seine Spool-Daten wieder freigegeben.

## Erforderliche nächste Arbeiten

1. Outcome-Instantiierung mit substitutions- und quellpositionskorrekten
   Signaturen integrieren. Allgemeine unannotierte globale Bindungen müssen
   ihre inferierten Typen in die gefrorene Umgebung übernehmen.
2. Fehlende Synthese-/Subsumptions-, Mapping-/Muster- und Constraint-Fälle
   weiter mit minimalen positiven und negativen OCaml-Fixtures schließen.
   Potenz-Kongruenz ist keine vollständige Potenzarithmetik.
3. Vollständige elaborierte Ausdrucks-/Effektinformationen erzeugen und
   auf Platte halten. `TypedDef` trägt bislang vor allem Scheme-Snapshots.
4. Programmübergreifende Rewrites mit korrekter Symbolauflösung integrieren;
   einzelne Definitionen sind dafür kein vollständiger Ersatz.
5. Tatsächliche Lean-Körper, Typdefinitionen und Instantiierungen in den normalen
   CLI-Pfad integrieren; danach vollständiger ARM-Lake-Build und elaborierte
   No-Sorry-Prüfung unter 16 GiB. Fehlende Stufen bleiben bis dahin Exit 1.
6. Restliche rekursive AST-/Mini-Typ-Clone-/Drop-Pfade und Speicherbedarf großer
   Einzeldefinitionen prüfen. Bytebudgets begrenzen Eingabe-Bytes, nicht jeden
   Zwischenzustand. CLI-SMT-Profilzähler sind noch nicht vollständig verdrahtet;
   gedruckte Nullen belegen keine fehlenden Solver-Aufrufe.

Aktuelle Codeverträge stehen in [ARCHITECTURE.md](ARCHITECTURE.md), alle Befehle
in [TESTING.md](TESTING.md). Thematische Compiler-Commits dieser Session reichen
von `38fc9f1` bis `dc7ad87`; abschließende Dokumentation und Nachweise werden
separat committed und gepusht. Kein Reset, Stash oder Verwerfen fremder Arbeit.
