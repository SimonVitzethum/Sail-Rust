# Arbeitsstand und Fortsetzung — 2026-10-11

Dieses Dokument hält den bestätigten Stand, das technische Wissen und die
nächsten Arbeiten fest. Es unterscheidet den gepushten Compiler von laufender
Arbeit. Maßgeblich bleiben Quellcode, tatsächliche Prozessausgänge und Logs.

## Ziel und verbindliche Vorgaben

- Vollständige ARMv9.4-A-Sail-Spezifikation mit dem Rust-Rewrite kompilieren:
  Sail → vollständige Typprüfung → typisierte Rewrites → ausführbares Lean →
  erfolgreicher `lake build` → Prüfung ohne `sorry` oder andere Sorry-Proof-Holes.
- Gesamter ARM-Compilerlauf und Lean-Build unter systemd `MemoryMax=16G`
  und `MemorySwapMax=0`. `28G` ist die ausdrücklich erlaubte Ausweichgrenze.
- Keine Agents. OCaml-Semantik nachvollziehen und Übertragungsfehler korrigieren.
- Echtes Auslagern auf Platte, begrenzte starke Parallelisierung, deterministische
  Fehlerreihenfolge; Rekursion einschließlich Freigabe schrittweise entfernen.
- Compiler, Builds, Tests und rustfmt immer unter systemd-Speicherlimit ausführen.
- Oft committen und auf `https://github.com/SimonVitzethum/Sail-Rust` pushen;
  der Nutzer hat Pushes bereits autorisiert. Keine erneute Freigabe erforderlich.
- Die Entwicklungsprinzipien aus der Unterhaltung gelten: kleine mächtige APIs,
  zentrale Mechanismen, wenig Kopplung, Korrektheit vor Mikrooptimierungen.
- Die versehentliche Nachricht zum Windows-Dump auf sda/sdb wurde zurückgezogen;
  sie gehört nicht zu dieser Aufgabe.

**Das Gesamtziel ist nicht erreicht. Es gibt keinen erfolgreichen vollständigen
ARM-Typecheck, keine vollständige ausführbare ARM-Lean-Ausgabe und keinen grünen
ARM-`lake build`. Der Teilfortschritt darf nicht als Compilerabschluss gelten.**

## Repository und Referenzen

- Arbeitsrepository: `/home/simon/Dokumente/Sail/sail-rs`, Branch `main`.
  Das übergeordnete Verzeichnis `Sail` ist kein Git-Repository.
- Letzter gepushter Implementierungscommit vor dieser Dokumentation: `d953e91`
  (`d953e91b619d62708ca5732092f24b07af758dfd`).
  HTTPS-Fetch; SSH-Push zu demselben GitHub-Repository funktioniert.
- OCaml-Quellcode: `../sail/src/lib/`, insbesondere `initial_check.ml`,
  `type_check.ml`, `type_env.ml`, `rewrites.ml`.
- OCaml-Referenzcompiler: `/home/simon/.opam/sail/bin/sail`, Version 0.20.3,
  Git-Kennung `2ba3212`. Positive und negative Referenzfälle mit `-just_check`.
- ARM-Modell: `../sail-arm/arm-v9.4-a`; die Eingabeliste wird aus dessen Makefile
  gelesen: 22 Haupteingaben einschließlich `src/termination_measures.sail`.
- Z3: `/usr/bin/z3`, Version 4.16. Lean-Referenz: `../lean-sail`, Lean 4.29.0;
  Lake: `/home/simon/.elan/bin/lake`. Mehrere Lean-Versionen sind installiert.
  Direkte Lean-Tests brauchen ein zur Binärdatei passendes `LEAN_SYSROOT`.
- Rust/Cargo: `/usr/bin/cargo`. Bewährter PATH:
  `/usr/bin:/bin:/home/simon/.elan/bin`.

## Gepusht und bestätigt

Bis einschließlich `d953e91`:

- Disk-basiertes Frontend mit Quellreihenfolge, Includes, Makros, Fixity-Snapshots
  und begrenzten Workern, standardmäßig höchstens 16.
- Flache AST-Speicherung und iteratives Lesen, Rekonstruieren und Freigeben.
  Tiefe kanonische ASTs benötigen keine rekursive JSON-Deserialisierung.
- Der Scheduler begrenzt wartende, laufende und umgeordnete Ergebnisse zusammen.
  Fertige ASTs werden auf Platte abgelegt; die Ergebnisqueue hält IDs und Größen.
- Alle 59 Standardbibliotheksdateien sind eingebettet. ARM verwendet zusätzlich
  die unveränderte Legacy-`trace.sail` unter
  `crates/sail-prelude/compat/sail-0.17.1/`; der installierte leere Trace-Stub
  ist kein Ersatz. Herkunft und Lizenz sind dokumentiert.
- Kanonische Deklarationsprüfung mit Quellpositionssichtbarkeit, Binder-Kinds,
  Konstruktorbedingungen, Inline-Signaturen und geordneten Overloads.
- Abhängige Funktionsaufrufe mit frischen Instanzen, geliehenen Type/Int/Bool-
  Substitutionen, Z3-Beweisen und getrennten Identitäten rigider Variablen.
- Zweig- und Guard-Annahmen für abhängige Bedingungen; keine Übernahme von Fakten
  aus Geschwisterzweigen oder anderen Funktionsklauseln.
- Parametrisierte Typaliase einschließlich gemischter Type/Int/Bool-Argumente.
- Explizite `implicit`-Argumente werden wie OCaml als Integer-Singletons geprüft;
  diese Normalisierung darf nicht auf Rückgabetypen angewandt werden.
- Vektorliterale: Längenbeweise, Prüfung aller Elemente, kontextabhängig auch
  leere Vektoren, Inferenz nichtleerer Vektoren und verschachtelte Elementtypen.
- Korrigierter Parserfehler: `bitone`/`bitzero` sind ein Bit breite Binärliterale,
  keine Integerliterale; sie sind als numerische Typgrößen unzulässig.
- Mehrere 10.000-fach verschachtelte Fälle prüfen und geben auf einem
  128-KiB-Threadstack frei: Aufrufe, Substitutionen, Aliase, Vektoren,
  Let-Gültigkeitsbereiche und bedingte Fakten.

Die kanonische Prüfung liefert bisher Prüf-/Ausdruckszahlen, keine vollständige
elaborierte `TypedDef`-Pipeline. Alte Mini-AST-/Scaffold-Module sind keine
Belege dafür, dass der reale ARM-Pfad vollständig implementiert wäre.
Die CLI beendet fehlende Prüfung oder Ausgabe ausdrücklich mit Status 1.

## Laufende, noch nicht committete Implementierung

Beim Dokumentieren waren folgende Quelldateien geändert:

| Datei | Änderung |
| --- | --- |
| `crates/sail-grammar/src/sail.lalrpop` | Block-`let`/`var` bekommen den restlichen Block als Körper. |
| `crates/sail-grammar/tests/grammar_coverage.rs` | Bisherige falsche Erwartungen zu flachen Block-Deklarationen korrigiert. |
| `crates/sail-parse/tests/real_entries.rs` | Regression für den Gültigkeitsbereich von Block-Deklarationen. |
| `crates/sail-ty/src/canonical_body.rs` | Mutable Deklarationen, einfache/Tupel-Zuweisungen, Registerschreiben, foreach/while/repeat, Tupelinferenz und getrennte deklarierte/geöffnete Werttypen. |
| `crates/sail-ty/tests/canonical_program.rs` | Positive/negative Mutable-/Schleifenfälle und neuer 10.000-Level-Stresstest. |

Diese Änderungen bleiben im Worktree erhalten. Sie sind **noch kein komplett
grüner Änderungssatz**: Der neueste 128-MiB-Testlauf scheitert an OOM, siehe unten.
Kein Reset, Stash oder Verwerfen dieser Arbeit durchführen.

### Semantik, die aus OCaml übernommen wird

- OCaml `parser.mly`, Produktion `block`: Bei `{ let x = e; rest }` bzw.
  `{ var x = e; rest }` ist `rest` der Körper der Deklaration. Die Rust-Grammatik
  legte bisher Deklaration und `rest` als Geschwister mit leerem Körper ab.
  Das verlor die Sichtbarkeit der Variablen. Beide Regeln sind jetzt korrigiert.
- `type_check.ml::bind_assignment`, `bind_lexp`: Eine unannotierte Mutable-
  Deklaration behält den inferierten Typ. `var x = 0; x = 1` darf deshalb nicht
  durch automatisches Erweitern zu `int` angenommen werden.
- Für `var x = opaque_int()` muss dagegen der deklarierte Typ `int` erhalten
  bleiben. Ein für Beweise geöffnetes Singleton darf die Mutable-Deklaration
  nicht dauerhaft verengen. `InferredValue` hält dafür `declared` und `value`.
- Jeder Lesevorgang eines allgemeinen veränderlichen `int`/`bool` bekommt seine
  eigene rigide Wertidentität. Aus einer Bedingung vor einem Schreibzugriff darf
  kein Beweis über den anschließend gelesenen Wert abgeleitet werden.
- Mutable Deklarationen müssen neue Namen einführen; vorhandene Locals,
  Register und Enum-Mitglieder sind keine erlaubten neuen `var`-Ziele.
- Zuweisungen prüfen Schreibberechtigung getrennt vom RHS-Typ. Immutable Locals
  werden abgewiesen. Neue implizite Deklarationen durch Block-Zuweisungen sind
  noch nicht umgesetzt.
- Foreach: aufsteigend `from <= index <= to`, absteigend `to <= index <= from`;
  Index immutable, Schritt `int`, Körper `unit`. Die Grenzen sind geliehene
  Singleton-Ansichten; allgemeine Range-/Existential-Elaboration fehlt noch.
- While kann die positive Bedingung für den Körper benutzen. Repeat/until darf
  die abschließende Bedingung nicht als bereits gültige Körperannahme benutzen.

## Nachweise und aktueller fehlgeschlagener Test

Dauerhaft kopierte Logs liegen in [evidence/2026-10-11](evidence/2026-10-11/README.md).
Die bisherigen `/tmp`-Dateien sind zusätzliche Arbeitskopien, keine dauerhafte Ablage.

| Prüfung | Ergebnis und Grenze |
| --- | --- |
| Letzter vollständiger ARM-Eingang, gepushter Vektorstand | 31.267 Definitionen, 45 Dateien, 16 Worker, 1.039.913.215 AST-Bytes auf Platte; 50,687 s; cgroup-Peak 1,4 GiB, Swap 0. Status 1 bei `to_bytes_le` in `vector.sail`, noch vor Lean. |
| Vektor-Fokussuite vor Mutable-Arbeit | 322 Tests bestanden, jedes Testprogramm 128 MiB ohne Swap; Peak 87,8 MiB. Separate kanonische Suite: 35 bestanden, Peak 103 MiB. |
| Mutable-Zwischenstand vor neuem Tiefentest | 36 kanonische Tests bestanden bei 128 MiB ohne Swap; Peak 104,5 MiB. |
| OCaml-Mutable-Vergleich | Damalige 13 positive Fälle angenommen, alle 19 negativen abgewiesen; jeweils 256 MiB ohne Swap, positiver Peak 68,6 MiB. Aktuell wurden die Fixtures auf 15 positive und 21 negative Fälle erweitert; die Erweiterungen sind noch nicht erneut mit OCaml verglichen. |
| **Neuester Fokustest mit neuem Tiefentest** | **Gesamtausgang 1. Kanonisches Testprogramm bei 128 MiB durch OOM-Killer beendet.** 36 kanonische Einzeltests protokollieren Erfolg; der neue `ten_thousand_mutable_and_loop_scopes_check_and_release_iteratively` hat kein Erfolgsergebnis. Die übrigen 25 Testprogramme schließen mit insgesamt 333 bestandenen Tests ab. Build unter 4 GiB: Peak 513,4 MiB. Kein vollständiges grünes Suite-Ergebnis. |
| Älterer OOC-Teilmengentest, 16 Worker | 4.096 Definitionen, 1.052.672 Ausdrücke, 567.919.927 AST-Bytes bei 128 MiB ohne Swap; Typprüfung 1,380 s, gesamter Service 11,136 s. Erwartetes CLI-Ende wegen fehlendem Backend. Nach aktueller Mutable-Arbeit noch nicht wiederholt. |

Der OOM-Befund ist bestätigt. Seine Ursache ist **noch nicht bestimmt**. Sowohl
der neue tiefe Test als auch die zusätzliche Speicherung deklarierter/geöffneter
Typen und gleichzeitig laufende schwere Tests sind zu untersuchen. Nicht ohne
Messung behaupten, dass nur die Testparallelität verantwortlich sei. Ein
128-MiB-Testfehler beweist für sich weder Erfolg noch Scheitern des 16-GiB-ARM-Ziels.

Der letzte vollständige ARM-Lauf wurde **vor** den uncommitteten Mutable-/
Grammatikänderungen ausgeführt. Der dortige Blocker ist eine historische Messung,
keine Bestätigung des Ausführungsstands nach den neuesten Änderungen.

## Wichtige Implementierungsstellen

- `crates/sail-ast/src/storage.rs`: `DefinitionSnapshot` (`flat-definition-v1`),
  flaches Schreiben/Lesen, `into_definition`, `drop_definition`.
- `crates/sail-cli/src/frontend.rs`, `typecheck.rs`, `ooc.rs`: realer disk-basierter
  Frontend-/Prüfpfad. Rückgaben nur IDs/Größen, begrenztes Fenster.
- `crates/sail-ty/src/program.rs`: source-order Scheme-/Alias-/Overload-Tabellen,
  Deklarations-WF, Konstruktorbeweise und zentraler Prover.
- `crates/sail-ty/src/canonical_body.rs`: bidirektionale Prüfung mit Taskstapel;
  `Type`-Ansichten mit Pfaden, `Substitutions` mit Type/Scalar-Bindungen,
  iterative Freigabe gemischter Substitutionsketten und Scope-Elternketten.
  `proof_scope` liefert lokale Beweisannahmen; Caches gelten nur für einen
  geladenen Körper. Pointer dienen allein als AST-Identitätsschlüssel.
- `crates/sail-ty/src/canonical_literal.rs`, `canonical_eq.rs`: gemeinsame
  Literalklassifikation und iterativer Typ-/Argumentvergleich.
- `crates/sail-solver/src/canonical.rs`: iterativ validierte SMT-LIB-Erzeugung;
  gleichzeitige Substitution, getrennte Int/Bool-Sorts, Scope-Identitäten,
  negierte geliehene Fakten. Nur bewiesene UNSAT-Implikationen sind gültig.
- `crates/sail-solver/src/smtlib_backend.rs`: persistente Z3-Prozesse, Reset je
  Anfrage, exklusiver Pool, begrenzter Memo-Cache (128), Timeout inklusive
  blockierendem I/O und Kill/Wait/Join bei Abbruch. Unbekannt/Fehler gilt nicht
  als Beweis. Z3 erbt das Compiler-cgroup-Limit.

## Offene Arbeiten in sinnvoller Reihenfolge

1. Den neuen Tiefentest einzeln unter 128 MiB ausführen und Speicherverbrauch
   eingrenzen; dann denselben gesamten Fokustest erneut. Keine blinde Erhöhung
   des Limits als Lösung des Befundes. Deklarierte Typen möglichst teilen statt
   unnötig zusätzliche Ansichten/Substitutionsketten zu halten.
2. Aktuelle 15 positiven/21 negativen Mutable-Referenzfälle erneut mit OCaml
   prüfen. Rustfmt nach den letzten Änderungen mit korrektem WorkingDirectory
   ausführen. Ein früherer rustfmt-Aufruf ohne WorkingDirectory fand keine
   Dateien; späterer korrekter Aufruf formatierte die Ty-Dateien, danach wurden
   im Tiefentest noch die Literal-Symbole korrigiert.
3. Nach grüner gezielter Prüfung den Mutable-/Grammatik-Fix committen. Die
   gesamte Workspace-Suite und den 16-Worker-OOC-Test erneut ausführen.
4. ARM-Gate wiederholen. Für `to_bytes_le` fehlen insbesondere Vektor-
   Indexzuweisungen und die Typisierung der internen Index-/Subrange-Aufrufe.
   Nur die OCaml-Primitiven `vector_access#`, `vector_subrange#`,
   `vector_update#`, `vector_update_subrange#` gezielt normalisieren;
   nicht allgemein ein `#` von beliebigen Namen entfernen.
5. Vektor-LHS: mutable/registerbasierte Wurzel, Element-/Slice-Typ und echte
   Indexgrenzenbeweise. OCaml `infer_lexp` prüft `0 <= index < length` und
   ordnungsabhängige Subrange-Grenzen. Auch Indexausdrücke vollständig prüfen;
   RHS-Inferenz darf deren Fehler nicht umgehen. Default-Order berücksichtigen.
6. Fehlende kanonische Regeln vollständig ergänzen: Records/Fields, Structs,
   Match-/Konstruktor-/Listen-/Vektormuster, weitere Ausdrücke, globale Lets,
   Mappings, scattered/outcome und Terminierungsmaße. Record-/Union-Feldpayloads
   sind noch nicht durchgängig in den Programmtabellen erhalten.
7. Allgemeine Existential-/Range-/Refinement-Subsumption, ausgelassene implizite
   Argumente, arithmetische Rückwärtsinferenz und numerische/Boolean-Synonyme
   vervollständigen. SMT-Potenzen und allgemeine Constraint-/Numeric-Apps fehlen.
   Inline-Header-Vergleich mit bestehender Val-Signatur expandiert Aliase noch
   nicht vollständig. Fehler bleiben bis zur Implementierung explizit.
8. Vollständige typisierte Ausgabe und Integration der Rewrites herstellen;
   tatsächliche Lean-Körper ausgeben und den vollständigen ARM-Lake-Build prüfen.
9. Restliche Rekursion auditieren: kanonisches AST-`Clone` bei Signaturen und
   Annotationen, Typ-/Muster-Grammatikkonvertierung und weitere Compilerpfade.
10. Speichersteuerung weiter verbessern: aktuell Definitionenfenster statt
    striktem Bytebudget; eine große Definition muss in RAM passen. Residente
    Signaturtabellen wachsen mit dem Programm. Keine adaptive Speicherquote für
    parallele Z3-Prozesse. Z3-intensive 16-Worker-Prüfung bei 128 MiB ist nicht
    nachgewiesen. Canonical-Prover-Zähler sind noch nicht an den CLI-Profiler
    angeschlossen; gedruckte SMT-Nullen bedeuten nicht null Beweisanfragen.

## Bewährte Befehle

Vom Repository `sail-rs` aus; keine Compiler-/Testausführung ohne systemd-Limit:

```bash
# Build 4 GiB, jedes Testprogramm 128 MiB, jeweils kein Swap:
scripts/test-memory-limited.sh -p sail-ty -p sail-parse -p sail-grammar -p sail-cli -p sail-solver

# Kanonische Tests:
scripts/test-memory-limited.sh -p sail-ty --test canonical_program

# Ganze Workspace-Suite einschließlich Doctests:
systemd-run --user --wait --pipe --collect \
  -p MemoryMax=4G -p MemorySwapMax=0 \
  -p WorkingDirectory=/home/simon/Dokumente/Sail/sail-rs \
  /usr/bin/cargo test --workspace --profile ci -j 2

# Out-of-core-Teilmengennachweis, kein vollständiger ARM-/Lean-Build:
SAIL_RS_THREADS=16 scripts/typecheck-ooc-gate.sh

# Tatsächliche ARM-Akzeptanz:
scripts/arm-lean-gate.sh
# Erlaubte Ausweichgrenze:
SAIL_ARM_MEMORY=28G scripts/arm-lean-gate.sh

# Formatierung; WorkingDirectory ist auch für relative Pfade notwendig:
systemd-run --user --wait --pipe --collect \
  -p MemoryMax=256M -p MemorySwapMax=0 \
  -p WorkingDirectory=/home/simon/Dokumente/Sail/sail-rs \
  /usr/bin/rustfmt --edition 2021 --config skip_children=true \
  crates/sail-ty/src/canonical_body.rs crates/sail-ty/tests/canonical_program.rs
```

`scripts/test-memory-limited.sh` läuft mit vier Testthreads pro Binärdatei.
Sein Build nutzt JSON-Ausgabe, weshalb Rust-Compilerdiagnosen im allgemeinen Log
verkürzt erscheinen können. Bei Buildfehlern einen gezielten Cargo-Aufruf unter
systemd mit normaler Textausgabe verwenden. Bestehende Prozesse/Handles prüfen,
nicht allein wegen Beobachtungs-Timeout neu starten.

## Lean-Akzeptanz und Speichergrenzen

`scripts/arm-lean-gate.sh` erstellt einen frischen Ausgabeordner, liest die
Modell-Eingaben aus dem Makefile, prüft echte Compiler-Ausgabe und einen Lakefile,
führt `lake build` aus und untersucht anschließend die elaborierten Konstanten
mit `scripts/CheckLeanNoSorry.lean`. Dieser prüft Typen und Werte einschließlich
opaker Definitionen und transitiver Imports mit `hasSorry`.
`scripts/test-lean-no-sorry.sh` besitzt positive/negative Gate-Regressionen.
Ein lediglich erfolgreicher Lean-Build mit akzeptiertem `sorry` reicht nicht.

Spool-Daten gehören auf echte Platte: `SAIL_RS_SPOOL_DIR` kann das Verzeichnis
festlegen; Linux-RAM-Dateisysteme werden abgewiesen. `/tmp` ist hier tmpfs und
für Produktions-OOC ungeeignet, aber für Testlogs verwendbar. Standardspool ist
`$XDG_CACHE_HOME/sail-rs/spool` bzw. `$HOME/.cache/sail-rs/spool` mit isoliertem,
automatisch aufgeräumtem Unterordner. Ausgabedaten und Arbeitsfixtures liegen
unter `target/`; Logs und Befunde dieses Standes zusätzlich unter `docs/evidence/`.

Weitere Details: [ARM_BUILD_PROGRESS.md](ARM_BUILD_PROGRESS.md),
[canonical-typechecking.md](canonical-typechecking.md),
[flat-ast-storage.md](flat-ast-storage.md).
