# Arbeitsstand und Fortsetzung — 2026-10-13 (Main-Agent, B2-Slice solventiert)

## B2 (`res[i]`, `to_bytes_le`/`from_bytes_le`): Index- und Slice-Bounds bewiesen

- `vector_target_type`-Hook prüft Vektor-LHS Ende-zu-Ende: Wurzel + Index-
  Typ + Bounds-Beweis zuerst + substituierter Element-Typ. Chaingemessen:
  Solo-Tiefentest 93,6M; Hook-Beweise laufen mit rigiden Variablen (GoalMap).
- Neu: `Union`-Interner-Sicht (mehrere Tabellen, frische Namen lokal),
  `GoalMap`-Bindings (fresh→real mit je eigenem Kontext),
  Elem-Substitution via `.resolved()` (z. B. `'a := bits(8)`).
- Neu: `vector_len_nexp` + `ground_scalar`/`ground_nexp`/`find_binding`
  (`canonical_body.rs`): Längen-Argument folgt der Substitutions-Kette
  rekursiv (`sail_zeros(8*n)` → `8*'n` statt rohem `'n`); Konstanten werden
  gegroundet, rigide Reste bleiben für Scope-/Funktionssicht-Linking.
  Bindungsregel strikt same-table (Fremdtabellen kollidieren per Nummer
  UND Text — Funktion-`'n` vs. Instanz-`'n` aliasierten sonst zu `8*8`);
  Zyklen/Fuel-gesichert, Lücken ehrlich `Unsupported`.
- Neu: numerische Index-Bounds (`+ - *`, unäres `-`) über Union-Sicht
  (`vector_lexp_bridge.rs`); `default Order`-Deklaration (`program.rs`,
  `frontend.rs`: `DefKind::Default` wird deklariert und positionsgeprüft).
- Suite grün (sail-ty + sail-solver via Gate, RC 0, 26 ok).
- `vector.sail` komplett grün (Fixture + ARM-Pfad bis `option.sail`).
- Erledigt (B3): Union-Konstruktor-Pattern (`PatKind::App`) in `pattern()`:
  Scheme aus `values`, via `instantiate` gegen Skrutinee-Typ binden
  (quantifizierte Variablen + Constraint-Beweise), Payload-Patterns an
  substituierte Formal-Typen. Unbekannte Namen ehrlich `UnboundVar`,
  Nicht-Funktion/Arität harte Fehler. `option.sail is_none`
  (`Some(_)`/`None()`) und Tiny-`describe` (`Circle(n)`/`Square(_)`)
  checken; OCaml-Referenz stimmt zu (`sail -just_check` RC 0).
  Rot-Tabelle `lane70_tiny`: nur noch `get_x` (E_field).
- Erledigt (`@`/Append): `ExpKind::VectorAppend`-Arm in `run()`: beide
  Seiten inferieren zum selben Flavor, Ergebnis summiert gegroundete
  Längen (`bits(n)++bits(m)=bits(n+m)`); Ergebnis-Wurzel mit
  Seiten-Sicht-Links (same-table-präzise); Flavor-Mismatch hart,
  ohne `append`-Überladung ehrlich `UnboundVar` (wie OCaml).
  `reverse_endianness`-Muster als Fixture + OCaml-paritätisch verifiziert;
  4 Regressionstests (`vector_append.rs`) grün.
- Erledigt (Append-Synthese): `append_result`-Helper aus `run()`-Arm
  extrahiert, in `synthesize()` als `Synthesis::VectorAppend`
  verdrahtet (anmerkungsfreie `let`-Bindungen wie in
  `reverse_endianness` checken).
- Infra: `/tmp` von 6,4G auf 356M aufgeräumt (alte Lane-Verzeichnisse;
  tmpfs belegte direkt RAM); avail zurück bei ~17GiB.
  `sail_smt_cache` (memo-Ablage) steht in `.gitignore`.
- Erledigt (`E_sizeof`/`E_field`): `'n` synthetisiert `atom('n)` mit
  Funktions-Scope-Linking (`__size`-Deklarations-Gate wie OCaml);
  Vergleichs-Guards steuern Constraints über deklarierte Operand-Sichten
  bei (`m < 'n` ⇒ `'m < 'n`, `extzv`-Muster beidseitig grün);
  `record.field` via Accessor-Teleskop (`field_access.rs`, Tiny-`get_x`
  grün, Rot-Tabelle leer).
- Erledigt (Kontrollausdrücke): `throw` gegen `exception`-Union,
  `exit` gegen `unit`, `constraint(..)` als Faktquelle, `assert` mit
  Continuation-Fakten (Block-threading, `impdefs`-Muster beidseitig
  grün); `select_call` probiert alle Overload-Kandidaten;
  `P_vector`/`P_vector_concat` mit Konstantbreiten (Decode-Form
  `[..] @ 0x0 @ _`, 4 Tests grün).

# Arbeitsstand und Fortsetzung — 2026-10-12 abends (Main-Agent, committet)

> Alle Lane-Arbeiten 01–70 sind im Haupt-Worktree zusammengeführt, verifiziert
> und in thematischen Commits auf `main` festgeschrieben (Basis `a972e3a` →
> 10 Commits, ~20k Rust-Zeilen netto). Das Kapitel „2026-10-13 (Lane-39)“
> unten bleibt als Herleitungs-Historie erhalten; seine „uncommittet“-Aussagen
> sind durch die Commits überholt.

## Commit-Reihe (alle verifiziert grün vor dem Schreiben)

| Commit | Inhalt |
| --- | --- |
| grammar-Block-let/var | `sail.lalrpop`, Parse-Faltung, Grammatik-/Parse-Tests |
| ty-Tabellen | Records/Unions/Mappings/Scattered/Outcomes mit Quellpositions-Sichtbarkeit |
| ty-Inferenz | Synonyme, Implicit-Equations, Rückwärtsinferenz, Stub-Schichtung |
| ty-kanonisch | `canonical_body`-Arme (Struct/Match/Try/Scattered/Vektor/Measure/Mapping/Outcome), geteilte Arc-Typen, iterative Drops |
| ty-Tests | Mutable-/Vektor-/Tiny-/Rekursions-Abdeckung, Rot-Tabellen |
| rewrite | 34 echte Pässe in Lean-Reihenfolge (typed + string), Pin-Tests |
| solver/ooc | SMT-Zähler, Solver-Quota, Byte-Budget-Scheduler, iterative Drops |
| cli/lean/lake | TypedDef-Emission, Rewrite-/Lean-Stufen, Drucker, Lake-Emission, ehrliche Exit-1-Pfade |
| scripts/evidence | Runner-Knobs (Threads 2/Retry), Gate-Logs, OOM-/Paritäts-/Harness-Nachweise |

## Gate-Stand (Prozessausgang + Peak, systemd, Swap 0)

| Prüfung | Ergebnis |
| --- | --- |
| `test-memory-limited.sh` 7 Pakete | RC 0 (50 Binaries ok; Threads-Default 2) |
| OOC-Gate 16 Worker | bestanden (568 MB AST unter 128M) |
| NoSorry-Gate | verifiziert (positiv/negativ) |
| ARM-Gate | OFFEN — Kopfblocker B1 (Outcome-Scoping, Lane-40 gemergt), dann B2/B3 |

## Offen (Code)

1. Arm-Calls: StructUpdate-Port drin; Measure/Mapping/Outcome-Calls, Field/Ref/Config/Exit/Throw/Assert/VectorAppend-Arme (Module alle vorhanden).
2. ARM-Gate nach B1 neu treiben; danach `lake build` + voller NoSorry-Nachweis.
3. Effekt-/Guard-/Prover-Pässe (Spec aus Lane-62 liegt vor).
4. Mapping-App-Gate-Patch (`evidence/2026-10-13/lane-53/`) nach Direction-Infra.

# Arbeitsstand und Fortsetzung — 2026-10-13 (Lane-39)

> Lane-39 (Doku-Konsolidierung, nur lesend geprüft, keine Codeänderung):
> Dieser Stand ersetzt die OOM- und Offen-Aussagen von 2026-10-11 unten.
> „Gemergt je Lane“ = eigener Branch-Commit und/oder Inhalt im uncommitteten
> Sammel-Worktree mit Dateibeleg — **nicht** „in `main` committet“ (`main`
> steht weiter auf `a972e3a`). Alle Erfolgsaussagen nennen Prozessausgang
> und Peak; Herleitung in
> [lane-39/NOTIZ.md](evidence/2026-10-13/lane-39/NOTIZ.md).

## Lane-Stand 01–24 (mit Dateibeleg)

| Lane | Inhalt | Beleg | Verifikation |
| --- | --- | --- | --- |
| 01 OOM-Repro | Mess-Skript + Solo-Nachweis Tiefentest | `scripts/lane-01-oom-repro.sh`, `evidence/2026-10-12/lane-01-oom-single.log` | Build rc=0/199,7M; solo t1 rc=0/120,4M, t4 rc=0/120,7M |
| 02 Speicherfix | `canonical_body.rs`-Diff im Sammel-Worktree | `crates/sail-ty/src/canonical_body.rs` (uncommitted) | Solo-Nachweis via Lane-01-Log (s. o.) |
| 03 Grammatik/Convert | Block-let/var-Körper-Faltung | `crates/sail-grammar/src/sail.lalrpop`, `crates/sail-parse/src/convert.rs` (uncommitted) | Kein eigener Lane-Log |
| 04 Nexp-Typen | Lokale Minimaltypen (Alt-System) | `crates/sail-ty/src/nexp.rs:1-7` | Committed, kein neuer Nachweis |
| 05 OCaml-Mutable | Nur gemessen (15+/21−) | `evidence/2026-10-12/lane-05-ocaml-mutable.md`, `lane-05-logs/`, Positiv-/Negativ-Logs | 16× rc=0 / 21× rc=1, 0 gekreuzt, Peaks ~12M |
| 06 Solver/Z3 | Solver-Trait/Pool/Cache (Alt-System) | `crates/sail-ty/src/solver.rs`, `crates/sail-solver/` | Committed, kein neuer Nachweis |
| 07 Vektor-LHS | `infer_lexp` LE_vector/LE_vector_range | Branch-Commit `34f79f1`; `crates/sail-ty/src/lexp_bounds.rs`, `crates/sail-ty/tests/lexp_bounds.rs` (ungetrackt) | Branch unmerged |
| 08 Record-/Union-Tabellen | Feld-Payloads in Programm-Tabellen | Branch-Commit `9a3b6a0`; gestagt: `crates/sail-ty/src/program.rs:64-80,252,554-560,1340`, `crates/sail-ty/src/ast_env.rs:97-129`, 2 Testdateien | Branch unmerged |
| 09 Muster | `pat.rs`/Literal-/Eq-Diffs im Sammel-Worktree | `crates/sail-ty/src/pat.rs`, `canonical_literal.rs`, `canonical_eq.rs` (uncommitted) | Aktiv, kein eigener Lane-Log |
| 10 Synonym/Atom | Regel-Layer im Sammel-Worktree | `crates/sail-ty/src/rules.rs` (uncommitted) | Kein eigener Nachweis |
| 11 Inline-Header | `typ_from_pat` + Alias-Expansion | `crates/sail-ty/src/conv.rs` (uncommitted) | Kein eigener Nachweis |
| 12 TypedDef-Fehlerpfad | Expliziter Missing-Pfad statt Stillstand | `crates/sail-cli/src/typecheck.rs:55-90` (`TypedProgramStatus::Missing`, `require_typed_program` → Exit 1) | Fehlerpfad, kein Erfolgs-Log nötig |
| 13 Rewrite-Gerüst | 10 typisierte Pässe + Stubs | `crates/sail-rewrite/src/pipeline.rs:279-311,348`, `passes/typed.rs`, 6 neue Pass-Dateien, `tests/typed_pipeline.rs` (uncommitted) | Kein eigener Lane-Log |
| 14 Lean-Exps | `doc_exp`-Port Funktionsdrucker | Branch-Commit `9961e01`; `crates/sail-lean/src/exps.rs:152-199,505` (ungetrackt) | Branch unmerged |
| 15 Profiler/Quota | Zähler-Senke + Quota-Primitive + OOC-Gate | `evidence/2026-10-12/lane-15-gate-raw.log`, `lane-15-profiler.log` (lane-berichtet) | Unit 140 Tests rc=0 (Peak 989,8M/136,6M); OOC-16 bestanden, Peak 128M |
| 16 Vektor-Prims | 4-Hash-Prim-Normalisierung, verdrahtet | `crates/sail-ty/src/vector_prims.rs:47-71,123-182`, Call-Site `canonical_body.rs:647` (uncommitted) | Kein eigener Lane-Log |
| 17 TypedDef-Emission | Emission statt Zählungen; CLI bricht weiter ab | `crates/sail-ty/src/typed_def.rs:37-53`, `crates/sail-cli/src/typecheck.rs:3-30,229-279`, `main.rs:127` (uncommitted) | Kein Erfolgs-Log (Abbruch beabsichtigt) |
| 18 Rewrite-Anbindung | TypedPipeline ohne CLI-Verbrauch | `crates/sail-rewrite/src/pipeline.rs:348-394`, `tests/typed_pipeline.rs`; kein Treffer in `crates/sail-cli/` (uncommitted) | Kein eigener Lane-Log |
| 19 Exp-Regeln | Arm-Planer für fehlende kanonische Regeln | `crates/sail-ty/src/exp_arms.rs`, `lib.rs:28` (uncommitted) | Kein eigener Lane-Log |
| 20 Nexp-Reste | Range-/arithm. Inferenz-Bausteine | Branch-Commit `755b5e3` (`nexp.rs`, `solver.rs`, `subsume.rs`, Tests; uncommitted gespiegelt) | Branch unmerged |
| 21 Rekursions-Audit | Report + Guards + Tests | `evidence/2026-10-12/lane-21-recursion.md`, `crates/sail-ty/src/recursion.rs`, `tests/lane21_recursion.rs` (ungetrackt) | 10k-Sonden: nativ SIGABRT, Guards weisen zurück |
| 22 Bytebudget | Budget statt Definitionsfenster | `crates/sail-ooc/src/budget.rs:49-94`, `crates/sail-shard/src/scheduler.rs:57-59,324` (uncommitted) | Kein eigener Lane-Log |
| 23 Z3-Quote | Permit-Vergabe + Empfehlung | `crates/sail-solver/src/quota.rs:74-82,86,134,257,290-325`, `pool.rs:104-115`, `cache.rs:278-289` (uncommitted) | Kein eigener Lane-Log |
| 24 Flakiness-Knobs | Threads/Retry-Knobs + Vollsuite/OOC-Evidence | Branch-Commits `2955a2f`, `f5ce597`; `scripts/test-memory-limited.sh`, `evidence/2026-10-12/lane-24/` (REPORT + 11 Logs) | Vollsuite seq. rc=0 (104 Suiten, 1179 Tests); OOC-16 Gate rc=0 |

Lanes 25/26/28–30/33–35/37/38 sind aktive leere Worktrees (Basis `a972e3a`,
kein Commit); `lane/41–44` ist ein separater Multicore-/Boot-Track.
Nichts davon ist ARM-Pfad-Nachweis.

## OOM-Befund (korrigiert 2026-10-13)

Der Befund von 2026-10-11 („Ursache noch nicht bestimmt“) ist überholt:
**Test-Parallelität, kein Leak.** Solo wie 4-thread derselbe Tiefentest grün
(Lane-01: 120,4M vs. 120,7M); 35-Test-Binärdatei ~1,9× (53,5M → ~103M).
Nur die schwere 39-Test-Kombination (inkl. uncommittetem Mutable-Tiefentest)
kippt an der 128M-Decke je nach Scheduling (~126,6M ok vs. OOM-Kill);
solo ~93,6M und sequenziell ~95,5M grün. Default bleibt 4, stabil ist
`SAIL_TEST_THREADS=1`. Belege: [NOTIZ.md](evidence/2026-10-13/lane-39/NOTIZ.md) §2.

## Gate-Tabelle (Stand 2026-10-13)

| Gate | Urteil | Beleg |
| --- | --- | --- |
| Fokussuite 128M sequenziell | Bestanden (rc=0) | `evidence/2026-10-12/lane-24/vollsuite-sequenziell.log`: 104 Suiten ok, 1179 Tests, 0 failed |
| OOC-Teilmenge 16 Worker | Bestanden (Gate rc=0, Compiler-Exit 1 erwartet) | `evidence/2026-10-12/lane-24/ooc-16workers.log`: 567919927 AST-Bytes > 128M, 0,56 s, Peak 128M |
| OCaml-Mutable-Parität 15+/21− | Bestanden | `evidence/2026-10-12/lane-05-ocaml-mutable*.log`: 16× rc=0, 21× rc=1 |
| ARM-Gate Ende-zu-Ende | Offen (Kette O-1→O-4 aus Audit) | `main.rs:127` bricht vor Rewrites/Lean ab; letzter ARM-Lauf historisch bei `to_bytes_le` |
| NoSorry-Gate | Offen, blockiert (nicht defekt) | Skripte vorhanden, ohne Compiler-Output nicht ausführbar |

---

# Arbeitsstand und Fortsetzung — 2026-10-11

Diese Datei ersetzt die alten Fortschritts-, Lane- und Übergabedokumente.
Sie hält den bestätigten Stand, das technische Wissen und die
nächsten Arbeiten fest. Sie unterscheidet den gepushten Compiler von laufender
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

Die Zuordnung der dauerhaft gesicherten Logs steht in
[TESTING.md](TESTING.md#gesicherte-prüfnachweise). Die Dateien liegen in [evidence/2026-10-11](evidence/2026-10-11).
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

## Dokumentationsbereinigung

Die 181 bisherigen Markdown-Dateien wurden auf fünf aktuelle Dateien im
Projektroot konsolidiert: `README.md`, `WORK_STATE.md`, `ARCHITECTURE.md`,
`TESTING.md` und `THIRD_PARTY.md`. Alte Lane-Pläne und widersprüchliche
Fortschrittsberichte sind über die Git-Historie erreichbar. Dokumentverweise
in Quellkommentaren, Diagnosen, Korpusmetadaten und Skripten wurden aktualisiert.
Die öffentlichen Legacy-Feldnamen der Metadaten bleiben kompatibel.

Die gezielte Diagnose-/Prelude-Suite besteht mit 31 Tests; Build unter 4 GiB
(Peak 882,8 MiB), jedes Testprogramm unter 128 MiB ohne Swap. Die Layout-/Link-
Prüfung besteht bei 128 MiB (Peak 10,3 MiB); die acht ursprünglichen Logs,
Originalfixtures und vorhandenen Compileränderungen bleiben erhalten. Das
behebt oder widerlegt den oben dokumentierten Mutable-OOM-Befund nicht.
Die neuen Nachweise liegen in
[md-cleanup-tests.log](evidence/2026-10-11/md-cleanup-tests.log) und
[md-cleanup-check.log](evidence/2026-10-11/md-cleanup-check.log).

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

## Befehle zur Fortsetzung

Die aktuellen Build-, Test-, Formatierungs-, OCaml-, OOC- und ARM-Gate-Befehle
stehen zentral in [TESTING.md](TESTING.md). Alle Läufe erhalten systemd-
Speicherlimits; relative Pfade brauchen ein korrektes WorkingDirectory.
Bestehende Prozesshandles vor einem Neustart prüfen.

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
unter `target/`; Logs und Befunde dieses Standes zusätzlich unter `evidence/`.

Weitere Details: [Architektur](ARCHITECTURE.md),
[Tests und Befehle](TESTING.md), [Fremdquellen](THIRD_PARTY.md).
