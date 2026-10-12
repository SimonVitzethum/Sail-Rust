# Architektur und Verträge

Stand: 2026-10-11. Der Implementierungs- und Prüfstatus steht ausschließlich in
[WORK_STATE.md](WORK_STATE.md), Befehle in [TESTING.md](TESTING.md).
Die Existenz einer Crate, eines Renderers oder eines Mini-AST-Tests bedeutet
keine vollständige Integration in den realen Compilerpfad.

## Reale Pipeline

```text
Sail-Dateien und eingebettete Bibliotheken
  → Includes/Makros und Quellreihenfolge
  → Rohdefinitionen auf Platte
  → begrenzte Parser-/Konverter-Worker mit Fixity-Snapshots
  → flache kanonische Definitionen und lokale Interner auf Platte
  → geordnete Auflösung und Deklarationstabellen
  → begrenzte parallele Körperprüfung
  → noch zu vervollständigendes typisiertes Programm
  → noch zu integrierende Rewrites und ausführbare Lean-Ausgabe
  → lake build und elaborierte Prüfung auf Sorry-Proof-Holes
```

Die CLI meldet fehlende Stufen als Fehler. Sidecars mit ungeprüftem Rohtext,
leere Funktionskörper, Kommentare oder Projektgerüste dürfen den echten
Compilerlauf nicht erfolgreich abschließen.

| Bereich | Implementierungsstellen |
| --- | --- |
| CLI und disk-basierter Compilerpfad | `crates/sail-cli/src/{main,frontend,typecheck,ooc}.rs` |
| Kanonischer AST, Symbole, flache Speicherung | `crates/sail-ast/src/` |
| Generierte LALRPOP-Grammatik | `crates/sail-grammar/{build.rs,src/sail.lalrpop}` |
| RealParser, Konvertierung und Präprozessor | `crates/sail-parse/src/{real,convert,preprocess}.rs` |
| Auflösung | `crates/sail-resolve/src/` |
| Deklarationen, Aliase, Overloads, Konstruktorbeweise | `crates/sail-ty/src/program.rs` |
| Kanonische bidirektionale Körperprüfung | `crates/sail-ty/src/canonical_body.rs` |
| Gemeinsame Literale und Typvergleiche | `crates/sail-ty/src/{canonical_literal,canonical_eq}.rs` |
| Beweise und Solverprozess-Lebenszyklus | `crates/sail-solver/src/{canonical,smtlib_backend}.rs` |
| Begrenzte Parallelisierung und Speicherinfrastruktur | `crates/sail-par-pipe/`, `crates/sail-ooc/`, `crates/sail-shard/` |
| Rewrites, Effekte, Lowering, Lean-Printer und Projektlayout | `crates/sail-{rewrite,effects,lower,lean,lake}/` |
| Eingebettete Bibliotheken | `crates/sail-prelude/` |
| Diagnosen und Messinfrastruktur | `crates/sail-diag/`, `crates/sail-perf/` |

Worktree-Stand (uncommitted, Lane-13/18): typisierte Rewrite-Stufe mit 10
implementierten Pässen in `crates/sail-rewrite/src/pipeline.rs:279-311,348-394`
(`is_typed_implemented_pass`, `TypedStubPass`, `TypedPipeline`),
gemeinsame Helfer in `crates/sail-rewrite/src/passes/typed.rs`, neue Pässe
`add_unspecified_rec`, `merge_function_clauses`, `minimise_recursive_functions`,
`pat_string_append`, `remove_blocks`, `remove_duplicate_valspecs`,
`remove_superfluous_letbinds` unter `crates/sail-rewrite/src/passes/`,
Ordnungstests in `crates/sail-rewrite/tests/typed_pipeline.rs`;
`crates/sail-cli/` verbraucht `TypedPipeline` weiter nicht.

## Out-of-core und Parallelisierung

Die Auslagerungseinheit ist eine kanonische Definition mit ihrem lokalen
Interner. `DefinitionSnapshot` in `sail-ast/src/storage.rs` verwendet das interne
Format `flat-definition-v1`. Rekursive Kinder werden iterativ in flache Datensätze
überführt. Knotenmetadaten und Nutzdaten bleiben erhalten. Speicherindizes sind
unabhängig von AST-NodeIds und Symbolen.

Kinddatensätze werden vor ihren Eltern geschrieben. Der Leser prüft einen
typkorrekten azyklischen Baum und baut ihn iterativ wieder auf. Typen, Muster,
Ausdrücke, Constraints, Outcomes, interne Werte und Annotationen benutzen die
gemeinsame Traversierung. Neue Varianten brauchen explizite Traversierungsregeln.
Zum Freigeben kanonischer Bäume dient `drop_definition`; die normale rekursive
Freigabe sehr tiefer ASTs ist ungeeignet.

Worker schreiben fertige Definitionen in den disk-basierten Warm-Layer und liefern
nur Content-ID und Größe. Der geordnete Consumer lädt, verarbeitet und verwirft
eine Definition. Die Queue hält keine vollständigen fertigen AST-Körper.
Der Scheduler begrenzt wartende, laufende und auf Reihenfolge wartende Ergebnisse
gemeinsam. Vor dem nächsten Input wird ein Credit erworben; Fehler und Panics
beenden den weiteren Input. Das hält die Fehlerreihenfolge deterministisch.

Die CLI nutzt standardmäßig höchstens 16 Worker; `SAIL_RS_THREADS` setzt die
Anzahl. Der committete Speichermechanismus begrenzt ein Definitionenfenster,
noch kein striktes Bytebudget. Worktree-Stand (uncommitted, Lane-22/23):
Byte-Budget in `crates/sail-ooc/src/budget.rs:49-94`
(`parse_byte_limit`, `byte_budget_from_env`), Scheduler mit
`crates/sail-shard/src/scheduler.rs:57-59` (`new_byte_budget`) und
`:324` (`run_streaming_weighed`); Z3-Quote in
`crates/sail-solver/src/quota.rs:74-82,86,134,257,290-325`
(`BYTES_PER_SOLVER`, `QuotaConfig`, `QuotaStats`, `set_max_live`,
`QuotaPermit`, `QuotaRecommendation::recommend`), durchgesetzt über
`crates/sail-solver/src/pool.rs:104-115` (`set_max_idle`/`evict_idle`) und
`crates/sail-solver/src/cache.rs:278-289` (`set_max_bytes`/`enforce_byte_quota`),
Politik-Hilfe in `crates/sail-perf/src/quota.rs:22-39` (`QuotaAdvice::recommend`).

Spool-Priorität: `SAIL_RS_SPOOL_DIR`, sonst `$XDG_CACHE_HOME/sail-rs/spool`,
sonst `$HOME/.cache/sail-rs/spool`, ohne beide Variablen `target/sail-rs-spool`.
Jeder Lauf verwendet einen isolierten und anschließend aufgeräumten Unterordner.
Linux-RAM-Dateisysteme werden über `/proc/self/mountinfo` abgewiesen.
Ein tmpfs ersetzt bei deaktiviertem Swap keine Auslagerung auf Platte.

## Parser, AST und Interner

`sail-grammar/build.rs` erzeugt die LALRPOP-Tabellen tatsächlich. `RealParser`
führt expliziten Zustand für Fixities und Konstruktorkinds mit. Infix-Ausdrücke,
numerische Ausdrücke und Constraints werden gemeinsam gefaltet; Ausdruckskonversion
und Infix-Baumaufbau verwenden explizite Arbeitsstapel.

Die Grammatik trennt offene und geschlossene Ausdrücke für die Zuordnung von
`else`. Ausdrucks-`let`/`var` verlangen `in`; innerhalb eines Blocks werden sie
ohne `in` durch eigene Blockproduktionen erkannt. Der verbleibende Block ist ihr
Körper, kein Geschwister außerhalb des Gültigkeitsbereichs. Dieser letzte Fix ist
noch laufende Arbeit, siehe `WORK_STATE.md`.
Parenthesierte Bitfeldbereiche folgen den getrennten OCaml-Produktionen für
Bereiche und Konkatenationen; einzelne Typklammern dürfen dabei nicht doppelt
abgeleitet werden. Tabellenkonflikte werden an den Produktionen behoben.

Die separate Legacy-Dateibrücke kann nicht konvertierte Bestandteile in Sidecars
aufbewahren. Das ist kein Nachweis einer vollständigen Prüfung. Der echte
Compiler muss Lücken explizit ablehnen. Präzise Spans aller Nachfahren sowie die
vollständige Weitergabe von Annotationen bleiben zu auditieren.

`Symbol` ist nur innerhalb seines Interners gültig. Namen werden zum Vergleich
über den jeweiligen Interner aufgelöst; gleiche Symbolnummern aus verschiedenen
Tabellen sind keine gleichen Namen. AST, Typansichten und gespeicherte Definitionen
müssen den passenden Interner behalten. Lexer-Payloads, AST-Literale und `Kid`
verwenden Symbole; `TokenKind::Error` behält Fehlertext als `String`.
`NExpKind::Constant`, Set-Mitglieder und einige andere Nutzdaten bleiben Strings.
AST-JSON-Symbole benötigen die mitgelieferte Tabelle zur Interpretation.
`bitone` und `bitzero` werden wie OCaml zu einem Bit breiten Binärliteralen;
sie sind keine Integerliterale oder numerischen Typgrößen.

## Kanonische Typprüfung

`Program` hält immutable Signaturen und Deklarationshistorien. Jede Körperprüfung
sieht nur Deklarationen bis zur eigenen Quellposition. Spätere Val-Spezifikationen,
Overload-Erweiterungen oder globale Constraints dürfen frühere Körper nicht ändern.
Worktree-Stand (uncommitted, Lane-08/16/19): Record-/Union-Feldpayloads in
`crates/sail-ty/src/program.rs:64-80,252,554-560,1340`
(`RecordField`, `RecordDef`, `records`-Tabelle, `record()`), gelesen über
`crates/sail-ty/src/ast_env.rs:97-129`
(`record_fields_ast`, `variant_ctors_ast`, `accessor_scheme_ast`);
Vektor-Prim-Normalisierung in `crates/sail-ty/src/vector_prims.rs:47-71,123-182`
(`VectorPrim`, `classify`, `normalize`, `resolve_app`), Call-Site
`crates/sail-ty/src/canonical_body.rs:647`; Arm-Planer für fehlende
kanonische Regeln in `crates/sail-ty/src/exp_arms.rs` (Lane-19).
Konstruktorbedingungen werden nach simultaner Parameterersetzung bewiesen.
Inline-Rückgabeannotationen werden mit den Argumentmustern zu Signaturen ergänzt.
Overloads werden in OCaml-Reihenfolge auf Argumente, Ergebnis und Bedingungen
geprüft; verschachtelte Listen werden iterativ expandiert und Zyklen abgewiesen.

`Type` hält eine `Arc<Scheme>`-Wurzel, einen Pfad, eine rigide Scope-Identität
und geliehene Substitutionen. Typalias-Expansion ersetzt keine großen Bäume durch
rekursives Klonen: Type-Argumente und numerische/Boolean-Argumente werden als
Ansichten gebunden. `Substitutions` hält beide Arten gemeinsam und gibt gemischte
Ketten iterativ frei. Auch Scope-Elternketten besitzen iterative Freigabe.
Einige Kopien von Signaturen und Annotationen benutzen weiterhin AST-`Clone`;
die Rekursionsfreiheit des gesamten Compilers ist noch nicht nachgewiesen.

Frische Aufrufinstanzen trennen rigide Variablen zwischen Rekursion, Geschwistern
und Internertabellen. Allgemeine immutable Integer-/Boolean-Werte und opake
Ergebnisse werden zu eigenen Singleton-Zeugen geöffnet. Bei Mutable-Initialisierung
muss der deklarierte Typ erhalten bleiben; bei späteren Lesevorgängen darf kein
veralteter Zeuge als aktueller Wert gelten. `InferredValue` trennt in der laufenden
Implementierung deklarierte und geöffnete Ansichten.

Die Prüfung nutzt einen expliziten Taskstapel und pro Körper begrenzte
Synthese-/Aufrufcaches. AST-Pointer sind nur Identitätsschlüssel und werden darüber
nicht dereferenziert. Zweig- und Guard-Fakten sind geliehene Boolean-Argumente mit
optionaler Negation; sie gelten nur im jeweiligen Scope. Foreach-Indizes bekommen
Bereichsannahmen und sind immutable. While benutzt positive Bedingungsfakten;
Repeat/until setzt seine spätere Bedingung nicht für den Körper voraus.

Der Rückgabewert der committeten kanonischen Prüfung ist eine Anzahl geprüfter
Ausdrücke. Worktree-Stand (uncommitted, Lane-17): elaborierte Emission als
`crates/sail-ty/src/typed_def.rs:37-53` (`TypedDefKind`, `TypedDef` mit
Typ-/Constraint-Payload statt Zählung), verdrahtet in
`crates/sail-cli/src/typecheck.rs:3-30,229-279`; Downstream fehlt weiter,
`require_typed_program` bricht explizit mit Exit 1 ab. Vollständige typisierte Definitionen, alle Muster-/Ausdrucksregeln,
Refinement-/Existential-Elaboration und die Integration in Lean fehlen weiterhin.
Zum 128-MiB-Befund (Test-Parallelität, kein Leak) gilt der korrigierte Stand in
[WORK_STATE.md](WORK_STATE.md).

## Solver und Beweisgrenzen

Der reale kanonische Prover verwendet iterativ erzeugtes und validiertes SMT-LIB
mit Z3 als persistentem Subprozess. Numerische Strings werden validiert, Namen
kodiert, Int- und Bool-Sorts getrennt und rigide Scope-Identitäten erhalten.
Gleichzeitige Substitution vermeidet Variable-Capture. Annahmen stammen aus
sichtbaren globalen Constraints, der Funktionsquantifizierung und lokalen Fakten.
Nur UNSAT der negierten Beweisverpflichtung bestätigt eine Implikation.
SAT, Unknown, Syntaxfehler und Timeouts sind keine gültigen Beweise.

Die Solverprozesse werden exklusiv ausgeliehen und vor jeder Anfrage zurückgesetzt.
Ein begrenzter Memo-Cache hält bis zu 128 Ergebnisse. Anfragen besitzen einen
3-Sekunden-Timeout; der I/O-Worker deckt auch blockiertes Schreiben/Lesen ab.
Abbruch/Freigabe beendet Prozesse und wartet auf Prozess und Thread. Z3 erbt die
cgroup-Grenzen. Worktree-Stand (uncommitted, Lane-15): Zähler an der Quelle
(`SolverPool::smt_totals`, `MemoCache::smt_totals`) und Profiler-Senke
(`phase_with_counters`, `add_smt_to_last`) sind getestet; der Call-Site-Patch
in `sail-cli/src/typecheck.rs`/`main.rs` fehlt noch. Beleg:
`evidence/2026-10-12/lane-15-profiler.log` §1/§3.

Das optionale native `z3`-Feature und die ältere `sail-nexp`-Algebra sind eigene
Schnittstellen. Deren vereinfachende Prädikate oder Mini-AST-Tests ersetzen die
kanonischen Beweise nicht. Allgemeine Numeric-/Constraint-Apps, Potenzen und
weitere Synonym-/Refinement-Regeln sind im kanonischen Pfad noch unvollständig.

## Lean, Speichermodell und Projektlayout

Lean ist die maßgebliche Speichermodell-Semantik (`lean-sail` und ARM-Support).
Rust-Spiegel müssen durch Differentialtests mit ihr übereinstimmen. Es wird kein
paralleles unabhängiges Speichermodell eingeführt. Eigenes Rust enthält kein
Unsafe; der Workspace verbietet es mit `unsafe_code = "forbid"`.

`sail-lake` besitzt Projekt-/Registertemplates und Golden-Fixtures unter
`crates/sail-lake/golden/`. Worktree-Stand (uncommitted, Lane-14): echter
Funktionskörper-Drucker als `doc_exp`-Port in
`crates/sail-lean/src/exps.rs:152-199,505`
(`print_fun_def`, `render_fun_def`, `print_exp_top`, Hauptausdruckdrucker). Defaults im Quellcode: Lean `v4.29.0`, lean-sail `v6`;
mathlib `v4.29.0` nur bei Opt-in für reelle Zahlen. Interface V1 verwendet
`ConcurrencyInterfaceV1`/`SpecializationV1`, V2 `ArchSem`/`SpecializationArchSem`.
Das Layout enthält Toolchain, Lake-Konfiguration, Manifest, `<Camel>/Defs.lean`
und das Root-Modul. Golden-Stringvergleiche belegen Layout, keine ausgeführten
ARM-Funktionskörper. Fixtures werden nur bei bewusst geprüften Templateänderungen
angepasst, nicht zur bloßen Anpassung an einen fehlgeschlagenen Test.

Die tatsächliche Abnahme verlangt frische Compiler-Ausgabe, `lake build` und
`scripts/CheckLeanNoSorry.lean`. Dieser prüft elaborierte Typen und Werte,
auch opake Definitionen und transitiv importierte Module. Reiner Quelltext-Grep
kann durch Makros oder Taktiken erzeugte Proof-Holes nicht ausschließen.
