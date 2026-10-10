# UNIFY_HARNESS.md — Lane 114 harness-unify (Analyse + Plan, kein Umbau)

Stand: 10.10.2026, Branch `lane/114-harness-unify`. E3 gilt: **kein
`cargo`/Build/Test durch Lane-Agents**; Verifikation läuft ausschließlich
beim Integrator. Diese Datei ist **nur Analyse + Migrationsplan**. Sie baut
keinen fremden Crate um, benennt nichts um, löscht nichts.

## 1. Ist-Befund (gelesen, nicht gebaut)

| Dimension | `sail-oracle` (Lane 13) | `sail-frontend-diff` (Lane 86) | `sail-smoke` (Lane 39) |
|---|---|---|---|
| Auftrag | Backend-Diff: OCaml-`sail --lean`-Output vs. Rust-Backend-Output | Frontend-Diff: OCaml-Parse-Bäume (normalisiert) vs. Rust-Frontend + Typ-Urteil-Stichprobe | Ausführungs-Parität: Rust-Lean-Output vs. OCaml-C-Output an 10 Decode/Execute-Fällen |
| Oracle-Anruf | `sail --lean-matchbv … --lean-output-dir` unter `ulimit -v` + `timeout` (`oracle.rs`: `build_oracle_command`, `stage_input`, `collect_oracle_output`, `classify_exit`) | `sail --just-check --ddump-initial-ast/--ddump-tc-ast` unter **gleicher** `ulimit+timeout`-Konvention (`harness.rs`: `build_dump_command`, `run_dump`; wiederverwendet `oracle::classify_exit`, `oracle::stage_input`) | Keiner (Skizze: `RunnerConfig`/`RunnerPlan` für zwei Binaries, noch kein Executor) |
| Normalisierung | `normalize.rs` (`normalize_with`: CRLF→LF, trailing-WS, Blank-Kollaps, `Sail <version>`-Banner-Drop, `<PATH>`-Redaktion) | Eigene `collapse_ws`/`strip_locations` in `norm_tree.rs` + `normalize_scheme` in `ty_sample.rs`; Oracle-Seite nutzt `sail-oracle`-Prädikat `is_volatile_line` | Eigenes `normalize_log` (CRLF→LF, trim, Blank-Drop, WS-Kollaps) — **dritter** Normalisierer, semantisch ≈ Teilmenge von `normalize` |
| Vergleich | `diff.rs` (eigener Unified-Diff, LCS + Fallback + Cap) | `tree_compare.rs` (positionaler Kind/Name/Header-Vergleich) + Volltext-Diff **via `sail-oracle`-Diff**; `ty_sample::compare_samples` | `compare_logs` (normalisierte Gleichheit, kein Diff) |
| Status-Taxonomie | `CaseStatus` (Pass/Mismatch/OracleFailed/Timeout/OomSuspected/Skipped/HarnessError) + `is_failure` | Eigenes `FrontendStatus` (ParseMatch/ParseMismatch/TySampleMismatch/… gleiche Reststufen) — **zweite** Taxonomie, gleiche Semantik | `Comparison` (Match/Mismatch) + `SuiteResult` — **dritte**, gröbere Taxonomie |
| Report | `report.rs` (Markdown: Summary/Failing/Skipped/Diffs/Passed) + Binary `sail-oracle-diff` (CLI, Exit 1 bei Fail) | Kein Renderer (nur `reason()`-Log + Detail-Strings; als Folgearbeit vermerkt) | Keiner (`run_all` nur Zähler) |
| Fixture-Katalog | Implizit: `*.sail` in `test_dir` + 8192-Byte-Cap (ARM → Nightly-Skip) | Explizit: `catalog.rs` (36 `sail/test/lean`-Fixtures, 8 Smoke / 25 Standard / 3 Nightly, `errors` = ParseOnly, Byte-Pins) | Explizit: 10 Fälle (`SMOKE_CASES`) über 5 `*.sail`-Dateien + `SMOKE.md` |
| Deps | std-only | `sail-parse`, `sail-oracle` | keine |
| Reife | Orakel-Seite fertig + verifiziert (Sail 0.20.3); Rust-Seite Stub (`MissingBackend`, ehrlicher Skip) | Code + Plan, E3 (Sifter-Heuristik **erst am Rig-Lauf** zu verifizieren) | Fixture-Tabelle + Vergleichslogik fertig; Executor-Skizze |

Redundanzen (Befund, keine Wertung): 2× Kommando-Konstruktor (`build_oracle_command`
vs. `build_dump_command`, gleiche `bash -c 'ulimit…; exec timeout…'`-Form);
3× Normalisierer (`normalize_with`, `collapse_ws`+`strip_locations`+`normalize_scheme`,
`normalize_log`); 3× Status-Typ (`CaseStatus`, `FrontendStatus`, `Comparison`);
2× Größen-Cap (Oracle-`DEFAULT_MAX_FILE_BYTES` vs. Katalog-`MAX_FILE_BYTES`, beide 8192);
2× Scratch-Helfer (`unique_dir` in beiden Harnessen). Geteilt wird bereits:
`classify_exit`, `stage_input`, `is_volatile_line`, `diff::unified_diff`
(alles `sail-oracle` → `sail-frontend-diff`).

## 2. Entscheidung: welcher Harness bleibt

- **`sail-oracle` bleibt — als Kern (Infra).** Einziger Harness mit
  ausführbarem Runner (Binary + CLI-Flags + Exit-Code), Report-Renderer und
  verifizierter Orakel-Seite; seine Bausteine (`classify_exit`,
  `stage_input`, `normalize`, `diff`, `report`, Status-Semantik) werden
  bereits von `sail-frontend-diff` wiederverwendet. Der Name bleibt, der
  Inhalt wird zum Shared-Kernel (kein Umbau in dieser Lane).
- **`sail-frontend-diff` bleibt — als Frontend-Stufe auf dem Kern.**
  Sein Auftrag (Parse-Bäume + Typ-Stichprobe) ist disjunkt zum Backend-Diff;
  `catalog.rs` ist der einzige explizite Suite-Katalog und wird
  stufenübergreifend gebraucht. Langfristig eine Stufe/ein Subcommand des
  Einheitsharness, kein eigenes Paralleluniversum.
- **`sail-smoke` geht auf — Fixtures wandern, Harness-Logik nicht.**
  Der Crate löst sich als Harness auf: Die 10 dokumentierten
  Decode/Execute-Fälle (inkl. `SMOKE.md`-Erwartungen) sind wertvoll und
  wandern als **Exec/Semantik-Stufe** in den Einheitsharness; seine
  Eigenlogik (`normalize_log`, `compare_logs`, `RunnerPlan`) wandert
  **nicht** mit, sondern wird durch Kern-Bausteine ersetzt. Bis zur Migration
  bleibt der Crate unverändert bestehen (kein Löschen in dieser Lane).

Zielbild: **ein Harness, drei Stufen** — `frontend` (parse+ty, aus
`sail-frontend-diff`), `backend` (lean-output, aus `sail-oracle`),
`exec` (lean-vs-c-Verhalten, aus `sail-smoke`-Fixtures) — ein Katalog, ein
Executor-Protokoll (`ulimit+timeout`, Staging, `classify_exit`), ein
Statusmodell, ein Markdown-Report, ein Binary mit Subcommands.

## 3. Was wandert wohin (Ziel-Mapping, Folgearbeit)

| Quelle (heute) | Ziel (Einheitsharness) | Art |
|---|---|---|
| `sail-oracle/{normalize,diff}` | Shared-Kernel, unverändert (alle Stufen nutzen ihn; `normalize_log`/`collapse_ws`-Varianten entfallen) | behalten, kanonisieren |
| `sail-oracle/oracle.rs` (`OracleConfig`, `build_*_command`, `stage_input`, `classify_exit`, `CaseStatus`, `discover_tests`) | Executor-Protokoll + Statusmodell für alle Stufen (`FrontendConfig`/`build_dump_command`/`FrontendStatus`/`unique_dir`-Duplikate entfallen) | behalten, vereinheitlichen |
| `sail-oracle/report.rs` + `main.rs` (`sail-oracle-diff`) | Einziger Report-Renderer + einziges Binary (Subcommands `frontend`/`backend`/`exec`) | behalten, erweitern |
| `sail-frontend-diff/catalog.rs` | **Der** Suite-Katalog (Tiers, `ParseOnly`-Focus, Byte-Pins) für alle Stufen; Smoke-8 des Katalogs ≠ `sail-smoke`-10 (nebeneinander dokumentieren, nicht vermengen) | behalten, stufenübergreifend |
| `sail-frontend-diff/{norm_tree,tree_compare,ty_sample}` | Frontend-Stufe (Parse-Vergleich + Typ-Stichprobe), frisst Kern-Normalizer/Diff | behalten als Stufe |
| `sail-frontend-diff/harness.rs` (`run_frontend_case`) | Frontend-Stufenläufer auf Executor-Protokoll | behalten, auf Kern umstellen |
| `sail-smoke/SMOKE_CASES` + `SMOKE.md` (10 Fälle, Lean-/C-Erwartungen, Log-Schnipsel) | Exec-Stufe als Fixture-Tabelle (Fälle referenzieren Katalog-Stems: `bitvec_operation`, `bitfield`, `match_bv`, `register_vector`, `range`, `loop`) | übernehmen (Daten) |
| `sail-smoke/{normalize_log,compare_logs,RunnerPlan/RunnerConfig}` | Entfällt (ersetzt durch Kern-`normalize` + Kern-`diff` + Executor-Protokoll) | verwerfen bei Migration |
| `MissingBackend`-Skip-Semantik | Stufenübergreifende Konvention (fehlende Gegenseite = ehrlicher Skip, nie Fake-Pass) | behalten |

Bewusst **nicht** vereinheitlicht: die Vergleichskerne selbst (Backend-Output-Diff
vs. positionaler Def-Vergleich vs. Verhaltens-Log-Vergleich sind drei verschiedene
Aussagen und bleiben drei Stufen); die Sifter-Heuristiken (`sift_oracle_dump`,
`sift_tc_dump`) bleiben frontend-lokal bis zur Rig-Verifikation.

## 4. Migrationsplan (Folge-Lanes, Reihenfolge bindend)

- **Phase 0 — Freeze (sofort, kein Code):** Keine neuen Harness-Features in
  einem der drei Crates außerhalb dieses Plans; Duplikate werden nicht weiter
  kopiert, sondern per Re-Export aus `sail-oracle` genutzt. Diese Datei ist
  der Referenzpunkt für Reviews.
- **Phase 1 — Kern kanonisieren:** `sail-oracle`-Bausteine als dokumentierten
  Shared-Kernel festschreiben (Re-Export-Modul, keine API-Brüche);
  `sail-frontend-diff` stellt `build_dump_command`/`FrontendConfig::memory_kb`/
  `unique_dir` auf Kern-Helfer um; `MAX_FILE_BYTES`-Doppel (beide 8192) zu
  einer Konstante mit zwei Re-Exports. Akzeptanz: kein Verhaltensdelta,
  vorhandene Tests grün (Integrator-Gate, E3 beachtet).
- **Phase 2 — Smoke migrieren:** Exec-Stufe neu (Fixtures aus `SMOKE_CASES`,
  Vergleich über Kern-`normalize`+`diff`, Ausführung über Executor-Protokoll
  statt `RunnerPlan`-Skizze); `sail-smoke` wird danach zum reinen
  Fixture-Datenlieferanten degradiert und bei grüner Exec-Stufe entfernt
  (Crate + Member-Zeile). Akzeptanz: 10/10 Exec-Fälle laufen über den
  Einheitsharness, `SMOKE.md`-Erwartungen als Stufen-Doku erhalten.
- **Phase 3 — Frontend als Stufe:** `run_frontend_case` wird Stufenläufer
  (`FrontendStatus` auf Kern-Status abgebildet: ParseMatch→Pass,
  ParseMismatch/TySampleMismatch→Mismatch, Rest 1:1); fehlender
  Markdown-Renderer wird Kern-`report.rs`-Instanz (kein zweiter Renderer).
  Sifter-Verifikation am Rig-Lauf **vor** Phase 3 (Heuristik-Risiko, siehe §5).
- **Phase 4 — Ein Binary, ein Katalog:** `sail-oracle-diff` → Subcommands
  (`frontend`/`backend`/`exec`, `--only`, `--report-out` geerbt); Katalog
  (`catalog.rs`) steuert alle Stufen (Smoke-Tier = schnelles Tor,
  `ParseOnly`-Focus nur für Frontend-Stufe relevant). Akzeptanz: ein
  `scripts/check`-konformer Workspace, ein Report-Format, keine toten
  Harness-Duplikate (`grep` nach `normalize_log|RunnerPlan|FrontendStatus|
  build_dump_command` findet nur noch Weiterleitungen bzw. nichts).
- **Nicht tun:** kein Zusammenwerfen der drei Vergleichsaussagen in einen
  Topf; keine Normalisierung, die echte Mismatches weglächelt (Skelett-Lücken
  wie Bodies/`Type`-`Let`-Bridge bleiben sichtbare Mismatches);
  keine `errors`-tc-Dumps erzwingen (ParseOnly bleibt); keine parallelen
  Orakel-Läufe ohne Memory-Limits (E3/Rig-Regeln aus Lane 51 gelten weiter).

## 5. Risiken

- Sifter-Heuristiken (`DEF_`-Token, tc-Marker) sind E3-Code — erste Rig-Läufe
  validieren, **dann** erst Phase 3 (sonst zementiert die Migration falsche
  Annahmen; Eskalation: `HarnessError`-Pfad, kein Aufweichen).
- Rust-Header (Sail-Quelltext) vs. OCaml-Dump-Header (Pretty-Print) sind
  verschiedene Sprachen — initiale `HeadMismatch`-Rate kann hoch sein;
  Eskalation = bewusste Reduktion auf Kind+Name (dokumentiert), nicht still.
- `guardedmatch`-Sibling-`.json` und `errors`-ParseOnly sind Staging-Sonderfälle,
  die der Einheits-Executor übernehmen muss (heute in beiden Harnessen korrekt,
  beim Zusammenlegen leicht zu verlieren).
