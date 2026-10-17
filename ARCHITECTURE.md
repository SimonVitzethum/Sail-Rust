# Architektur und Verträge

Stand: 2026-10-17. Messungen und offene Arbeiten stehen in [WORK_STATE.md](WORK_STATE.md),
Befehle in [TESTING.md](TESTING.md). Der vollständige ARM-/Lean-Pfad ist unvollständig.

## Reale Pipeline

`crates/sail-cli/src/main.rs:119` führt den normalen Compilerpfad aus:

```text
Sail-Dateien und Bibliotheken
  → geordnete Vorverarbeitung und parallele AST-Konvertierung
  → Deklarationsumgebung in Quellreihenfolge
  → parallele Körperprüfung
  → parallele Rewrites auf einzelnen Eingabedefinitionen
  → expliziter Fehler wegen fehlender vollständiger Lean-Ausgabe
```

Das Frontend liegt in `crates/sail-cli/src/frontend.rs:347`, die Typprüfung in
`crates/sail-cli/src/typecheck.rs:157`, die Rewrite-Anbindung in
`crates/sail-cli/src/rewrite_stage.rs:225`. Die fehlende Lean-Integration wird
in `crates/sail-cli/src/rewrite_stage.rs:102` mit Exit 1 gemeldet. Ein erfolgreiches
Frontend oder eine vollständige Teilmengenprüfung ist kein Compiler-Erfolg.

## Out-of-core und Parallelisierung

Frontend, Typprüfung und Rewrites verwenden den Scheduler aus `sail-shard`.
`crates/sail-shard/src/scheduler.rs:324` begrenzt gleichzeitig offene Jobs nach
Eingabe-Bytes und Anzahl. Credits werden erst nach der Ergebnisübernahme frei.
Ein Job oberhalb des Bytebudgets läuft allein; seine eigene Größe wird dadurch
nicht hart RAM-begrenzt. Das cgroup-Limit bleibt die äußere Speichergrenze.

`SAIL_RS_THREADS` bestimmt die Workerzahl; automatisch werden maximal 16 gewählt
(`crates/sail-cli/src/frontend.rs:347`). Das Typprüfungsfenster verwendet
`SAIL_TYPECHECK_MEMORY`, standardmäßig 32 MiB
(`crates/sail-ooc/src/budget.rs:29`). Rewrites verwenden ein unabhängiges
`SAIL_REWRITE_MEMORY`, ebenfalls standardmäßig 32 MiB
(`crates/sail-cli/src/rewrite_stage.rs:225`). Die Auswahl der Workerzahl erhöht
diese gemeinsamen Bytebudgets nicht.

Die kanonischen AST-Symbole gehören jeweils zu einem Definition-Interner.
Definitionen dürfen nicht ohne Symbol-Remapping unter einen gemeinsamen
Interner kopiert werden. `crates/sail-ast/src/interner.rs:62` serialisiert die
Symbol-Map in Textreihenfolge. Hash-Seeds nach Deserialisierung verändern damit
keine Inhalts-IDs; die bestehende JSON-Struktur bleibt erhalten.

Flache Snapshots zerlegen und rekonstruieren rekursive ASTs iterativ:
`crates/sail-ast/src/storage.rs:704` (`new`), `:729` (`write`), `:756` (`read`),
`:846` (`into_definition`) und `:875` (`drop_definition`). Direkte AST-Clones
und andere abgeleitete rekursive Operationen sind dadurch nicht automatisch
stack-sicher.

Frontend und Rewrites teilen die Speicherung über
`crates/sail-cli/src/frontend.rs:330` (`store_snapshot`) und das Laden über `:320`.
`crates/sail-ooc/src/warm.rs:183` speichert Blobs ohne sie im Live-Cache zu halten;
`:205` öffnet einen gepufferten Leser. Ein großer einzelner Snapshot und seine
serialisierten Bytes müssen während der Speicherung weiterhin in RAM passen.

Die Rewrite-Stufe speichert alle erzeugten Definitionen eines Jobs in
Passreihenfolge und behält Inhalts-IDs statt ganzer ASTs. Ergebnisgruppen werden
nach Quellposition sortiert. Sie verarbeitet weiterhin einzelne Eingabedefinitionen;
programmübergreifende Pässe sind dadurch noch nicht vollständig integriert.
Ein registrierter Typed-Pass ist kein Beleg für vollständige Semantik: etwa
`crates/sail-rewrite/src/passes/undefined.rs:27` bewahrt untypisierte Knoten
absichtlich, bis vollständige Typinformationen verfügbar sind.

## Typen, Gültigkeitsbereiche und Beweise

`crates/sail-ty/src/program.rs:408` baut Deklarationstabellen in Quellreihenfolge
auf. Lookups berücksichtigen die Position eines Körpers, beispielsweise
`Program::value` in `:1453`. Spätere Deklarationen dürfen frühere Körper nicht
rückwirkend typisieren. Record-/Union- und Outcome-Deklarationen haben eigene
Tabellen; Outcome-lokale Val-Spezifikationen sind über `:1729` zugänglich und
werden nicht als globale Werte eingetragen.

Annotierte globale `let`-Muster werden über dieselbe Werthistorie sichtbar,
erst ab der folgenden Quellposition. Doppelte globale Bindungsnamen werden
abgewiesen. Ohne Annotation wird derzeit noch keine globale Typsignatur aus
dem Initialisierer übernommen. Die Body-Prüfung bleibt auch für annotierte
Initialisierer verpflichtend (`crates/sail-ty/src/program.rs:690`).

Outcome-Implementierungen verwenden die Outcome-Signatur mit starren äußeren
Parametern. Lokale Vals gelangen in Quellreihenfolge in eine unveränderliche
Scope-Kette; Implementierungsnamen benötigen keine globale Val-Signatur.
Header-Quantoren gelten in Implementierungen, nicht in lokalen Val-Spezifikationen
(`crates/sail-ty/src/canonical_body.rs:505`). Outcome-Instantiierungen fehlen noch.

Der Körperprüfer in `crates/sail-ty/src/canonical_body.rs` verwendet Typansichten
mit Quelltabellen, Substitutionen und getrennten rigiden Bindungsidentitäten.
`infer_app_actuals` in `:907` nutzt bekannte Geschwistertypen für ausgelassene
implizite Argumente. Eine eindeutige Resultatsignatur kann auch die Prüfung noch
unaufgelöster innerer Argumente führen. Diese Argumente werden geprüft, bevor
ihre Typen weiterverwendet werden. Direkte Verschachtelung ist begrenzt; tiefe
Aufrufketten verwenden einen iterativen Synthesepfad. Nicht auflösbare Fälle
bleiben explizite Fehler.

Die Synthese verarbeitet `if`- und Blockresultate auf einem Arbeitsstapel mit
einer Scope-Sicht pro Frame. Bedingungen liefern getrennte Zweigannahmen.
Unterschiedliche boolesche Singletons werden zu `bool`, unterschiedliche
Integer-Singletons gegebenenfalls zu `int` verbreitert; allgemeine präzise
numerische Vereinigungen fehlen noch. Deklarierte Existentialtypen bleiben
getrennt von den geöffneten Wertzeugen. Die spätere Körperprüfung kontrolliert
Bedingungen, beide Zweige und Block-Präfixe vollständig
(`crates/sail-ty/src/canonical_body.rs:3685`).

Erwartete Existentialtypen sind Beweisverpflichtungen.
`check_existential` in `:2370` bindet Zeugen aus dem vorhandenen Werttyp und
übernimmt das Zielprädikat in die zu beweisenden Constraints. Nur bereits
vorhandene Zeugen liefern Marker-Annahmen; ein gewöhnlicher erwarteter
Call-Rückgabetyp darf keine Annahme über sein eigenes Ergebnis erzeugen.
Äußere Substitutionen bleiben bei der Instantiierung erhalten, und lokale
Existentialbinder verdecken gleichnamige äußere Substitutionen.

`marker_assumptions` in `:2619` sammelt vorhandene Zeugenbedingungen auch entlang
ihrer Substitutionssichten. Die ursprünglichen Bindungsidentitäten bleiben dabei
erhalten. Endliche Mengenbedingungen werden dabei ebenfalls übernommen.
Alias-Sichten besitzen andere Symboltabellen; Marker werden nur beim
ursprünglichen Besitzer emittiert, der über die Substitutionen erreichbar
bleibt. Die Regressionen in `crates/sail-ty/tests/existential_subsumption.rs`
prüfen gültige Zeugen, falsche Wertebereiche, Abschwächung, verbotene Verstärkung,
Shadowing und über mehrere Aufrufe weitergegebene Schranken.

Numerische Normalisierung in `crates/sail-ty/src/nexp.rs:1445` und `:1632`
verwendet Arbeitsstapel. Neu aufgebaute Kinder werden konsumiert, statt alle
Präfixbäume im Zwischencache zu kopieren. Der Test
`crates/sail-ty/tests/numeric_normalization.rs` prüft 10.000 symbolische Knoten
auf einem 128-KiB-Stack und gibt die Testbäume iterativ frei. Das ersetzt keinen
Audit aller rekursiven Clone-/Drop-Pfade des Compilers.

## Solver und Beweisgrenzen

`crates/sail-solver/src/canonical.rs:49` erzeugt validiertes SMT-LIB für
Implikationen, `:70` für Typargument-Gleichheiten. Die Skripte enthalten Annahmen
und die negierte Verpflichtung. Numerische Literale werden validiert, Namen
kodiert und Int-/Bool-Sorts getrennt. Nur UNSAT bestätigt den Beweis.

Potenzen werden konservativ durch die Integerfunktion `sail_pow2` abstrahiert
(`crates/sail-solver/src/canonical.rs:117`). Jede mathematische Potenzfunktion
ist eine zulässige Interpretation dieser Funktion. Deshalb bleibt UNSAT ein
gültiger Beweis, obwohl keine arithmetischen Axiome ergänzt werden. Kongruenz
und vorhandene Annahmen wie `p = 2^l` können benutzt werden. Beliebige
Potenzwerte oder Positivität werden ohne Belege nicht bestätigt. Allgemeine
Potenzarithmetik und weitere numerische Anwendungen bleiben unvollständig.
`crates/sail-solver/tests/power_congruence.rs` prüft das mit dem echten Solver.

`crates/sail-solver/src/smtlib_backend.rs:114` hält einen Z3-Prozess pro Solver.
Anfragen setzen dessen Zustand zurück; Zeitüberschreitungen, Syntax- und
Prozessfehler beweisen keine Verpflichtung. Z3 erbt die Compiler-cgroup-Grenze.
Unknown-Ergebnisse werden im kanonischen Programmpfad und in den Memo-/Pool-
Wrappern nicht gespeichert; spätere Anfragen dürfen erneut versuchen zu lösen.
OCaml Sail wird ausschließlich für Vergleichstests verwendet.

## Lean, Speichermodell und Projektlayout

`crates/sail-ty/src/typed_def.rs:53` trägt derzeit einen deklarierten Scheme-
Snapshot und Beobachtungszahlen. Eine vollständig elaborierte, serialisierte
Typ-/Effektannotation jedes Ausdrucks ist damit noch nicht hergestellt.
Die Rewrite-/Lean-Stufen benötigen weiterhin diese Integration und korrekte
programmübergreifende Rewrites.

Der Körperdrucker ist in `crates/sail-lean/src/exps.rs:184` und `:199` vorhanden;
Scheme-/Definition-Emission in `crates/sail-lean/src/emit.rs:147` und `:339`.
Diese Bausteine sind noch kein vollständiger produktiver ARM-Ausgabepfad.
`sail-lake` besitzt Projekt-/Registertemplates und Golden-Fixtures unter
`crates/sail-lake/golden/`. Lean-Sail und die ARM-Supportdateien bleiben die
Referenz für Speichermodell und Nebenläufigkeit.

`scripts/arm-lean-gate.sh` verlangt Compiler-Ausgabe, ein Lake-Projekt,
`lake build` und die Prüfung elaborierter Konstanten.
`scripts/CheckLeanNoSorry.lean:6` untersucht Typen und Werte einschließlich
opaker Definitionen und transitiver Imports auf `hasSorry`. Der aktuelle
ARM-Typprüfungsfehler verhindert noch das Erreichen dieser Lean-Stufen.
