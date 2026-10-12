# Audit offene Items (außer Lanes 07, 09, 14, 15) — 2026-10-12

Strikt lesendes Audit im Haupt-Worktree (`/home/simon/Dokumente/Sail/sail-rs`,
Basis `a972e3a`, darüber uncommittete Worktree-Änderungen, `git status` zeigt
~32 geänderte Dateien). Jede Aussage unten hat einen Dateibeleg mit Pfad:Zeile.
Ausgeschlossen (aktiv, nicht auditiert): Lane-07 (`lexp_bounds.rs` existiert im
Worktree nicht — läuft anderswo), Lane-09 (`pat.rs`/`canonical_literal`/`eq`),
Lane-14 (`sail-lean/exps.rs` existiert im Worktree nicht — `crates/sail-lean/src/`
enthält nur `conv.rs`, `lib.rs`, `pats.rs`, `tests.rs`), Lane-15 (Profiler/Quota).

Referenz: `WORK_STATE.md` Z.191–231 (Offene Arbeiten 1–10), `ARCHITECTURE.md`
(Reale Pipeline), `TESTING.md`, `evidence/2026-10-12/lane-01-oom-single.log`,
`evidence/2026-10-12/lane-05-ocaml-mutable.md`.

## 1. Abgedeckt-Verifikation (als erledigt betrachteter Worktree-Stand)

| Behauptung | Befund | Beleg |
| --- | --- | --- |
| Grammatik Block-let/var | OK (Worktree-diff) | `crates/sail-grammar/src/sail.lalrpop` (diff HEAD: `Exp::Block(b)` als Körper statt Geschwister + Unit) |
| Parse-Scope-Faltung | OK (Worktree-diff) | `crates/sail-parse/src/convert.rs` (diff: `is_unit_body`/`is_bare_decl`, faltet Altform, lässt Lane-03-Form) |
| Vektor-Primitive Modul | OK als Modul | `crates/sail-ty/src/vector_prims.rs:46-143` (`classify`/`normalize`/Arität, nur 4 Hash-Prims); Tests `crates/sail-ty/tests/canonical_program.rs:1313-1352` |
| Record-/Union-Tabellen | OK als Tabellen | `crates/sail-ty/src/program.rs` (diff: `RecordField`/`RecordDef`, OCaml-`Env.records`-Payloads); `crates/sail-ty/src/ast_env.rs` (diff: `record_fields_ast`/`variant_ctors_ast`/`accessor_scheme_ast`) |
| Inline-Header | OK als Logik | `crates/sail-ty/src/conv.rs` (diff, Lane-11-Kommentar: `typ_from_pat` + Alias-Expansion) |
| Nexp/Quant/Solver/Subsume | OK als Bausteine | `crates/sail-ty/src/nexp.rs` (diff: `normalize_nexp`, `normalize_constraint`, `solve_for_var`); `crates/sail-ty/src/subsume.rs` (diff: Range-Subsumption); `quant.rs`/`solver.rs`-Diffs vorhanden |
| TypedDef-Fehlerpfad CLI (Lane-12) | OK als Fehlerpfad | `crates/sail-cli/src/typecheck.rs:3-12,47-90` (`TypedProgramStatus::Missing`, `require_typed_program` → Exit 1; Solver-Zähler ehrlich 0) |
| Rewrites-Verdrahtung (Lane-13) | OK als Gerüst, s.u. | `crates/sail-rewrite/src/pipeline.rs:279-311` (`is_typed_implemented_pass`, 8 Pässe echt, Rest `TypedStubPass` Z.311-336) |
| Speicherfix canonical_body (Lane-02) | OK solo | `evidence/2026-10-12/lane-01-oom-single.log:66,102` (Tiefentest OK bei 128M, threads=1 und =4, Peak ~120M) |
| Mutable-OCaml-Vergleich (WS-2) | läuft (Lane-05 aktiv) | `evidence/2026-10-12/lane-05-ocaml-mutable.md` + Positiv/Negativ-Logs; kein neuer Lane nötig |

## 2. Offen-Tabelle (sortiert nach Abhängigkeit)

| # | WORK_STATE-Punkt | Status | Härte-Beleg | Vorgeschlagene Lane (exklusive Dateien, Abnahme) |
| --- | --- | --- | --- | --- |
| O-1 | WS-4: Vektor-Prim-Normalisierung im kanonischen Pfad | OFFEN — Modul vorhanden, **Verdrahtung fehlt**: 0 Referenzen auf `vector_prims` außerhalb Modul+Tests | `grep vector_prims` trifft nur `lib.rs:40` (Mod-Decl) + Tests; `canonical_body.rs` enthält keinen Treffer; ARM-Blocker `to_bytes_le` weiter historisch (`WORK_STATE.md:31`) | **Lane-16**: `canonical_body.rs` (Call-Site: `App`-Arm) exklusiv + Tests; braucht Lane-07 (LHS) vorher für Update-Kette. Abnahme: `scripts/test-memory-limited.sh -p sail-ty --test canonical_program` + ARM-Gate bis über `to_bytes_le` hinaus |
| O-2 | WS-8: TypedDef-Emission | OFFEN — kein `TypedDef`-Typ, CLI bricht explizit ab | `crates/sail-cli/src/main.rs:117-131` (`run_compiler`: nach `typecheck::run` immer `Err(... compilation incomplete ...)`); `typecheck.rs:47` ("No elaborated TypedDef pipeline exists yet") | **Lane-17**: `sail-ty/src/typed_def.rs` (neu) + `sail-cli/src/typecheck.rs` (Emission statt Counts); hängt von nichts Aktivem ab, blockiert O-3/O-4. Abnahme: `typecheck-ooc-gate.sh` mit TypedDef statt Backend-Fehler |
| O-3 | WS-8: Rewrite-Anbindung an echten Pfad | OFFEN — 8/51 Pässe echt, Rest Stubs; läuft auf `sail_ast::Ast`, nicht auf TypedDef; CLI ruft nie Rewrites | `pipeline.rs:32-52` (`LEAN_REWRITES`, 51 Einträge), `:279-291` (8 implementiert), `:311` (`TypedStubPass` = Identity); kein `TypedPipeline`-Gebrauch in `sail-cli/` (grep leer) | **Lane-18**: `sail-cli` (Verdrahtung TypedDef→TypedPipeline) + je Pass eine Datei unter `sail-rewrite/src/passes/`; nach Lane-17. Abnahme: OOC-Gate mit Rewrite-Stufe grün |
| O-4 | WS-8: Lean-Körper + ARM-Lake-Build | OFFEN — `lean-emit`-Phase ist simuliert | `main.rs:175` (`profiler.phase("lean-emit", \|\| {})` = No-op); Lane-14 (exps) aktiv → danach Lean-Emit-Lane nötig | nach Lane-14 + Lane-18. Abnahme: `scripts/arm-lean-gate.sh` (16G, Ausweich 28G) |
| O-5 | WS-6: fehlende kanonische Exp-Regeln | OFFEN — 11 `ExpKind`-Varianten ohne Arm | `canonical_body.rs`: `Struct`/`StructUpdate`/`Field`/`Match`/`VectorAppend`/`Exit`/`Config`/`Ref`/`Throw`/`Try`/`Assert` je 0 Treffer; `pattern()` Catch-all `canonical_body.rs:1145-1151` (ctor/vector/list/disjunktiv abgewiesen); `termination-measure typing` unsupported (`:363`); Mapping-Expansion fehlt (`:659`) | **Lane-19**: `canonical_body.rs` (Exp-Arme) + `program.rs` (Mapping/scattered/outcome-Tabellen); Muster-Anteil gehört Lane-09 (aktiv) → Koordination, keine Dateiüberlappung (`pat.rs` bleibt Lane-09). Abnahme: `test-memory-limited.sh -p sail-ty -p sail-parse -p sail-grammar -p sail-cli -p sail-solver` |
| O-6 | WS-7: Existential-/Range-/arithm. Inferenz-Reste | OFFEN — explizite Lücken im Code | `canonical_body.rs:1535` ("cannot infer dependent call parameter (existential opening or arithmetic solving required"); synonym-Expansion unsupported (`:429`-Umgebung); SMT-Potenzen/allg. Constraint-Apps fehlen (ARCHITECTURE Z.165) | **Lane-20**: `subsume.rs` + `nexp.rs` + `solver.rs`; nach Lane-19. Abnahme: `solver_parity`-Tests + kanonische Suite unter 128M |
| O-7 | WS-9: Rekursions-Audit-Reste | OFFEN | 83× `.clone()` in `canonical_body.rs`, 44× in `program.rs`; ARCHITECTURE Z.123-124 bestätigt Clone-Reste ("Rekursionsfreiheit ... noch nicht nachgewiesen") | **Lane-21**: Audit-Report + iterative Ersetzung, Dateien beliebig lesend, schreibend nur `canonical_body.rs`/`program.rs`-Clone-Stellen + `conv.rs`; unabhängig, jederzeit. Abnahme: 10.000-Tiefentests + Stack-Limit-Nachweis |
| O-8 | WS-10: Bytebudget statt Definitionsfenster | OFFEN — kein Budget-Mechanismus | kein `budget`/`Budget` in `sail-ooc/src/*.rs`, `sail-shard/src/*.rs`; `typecheck.rs:32-38` zählt nur (`streamed_bytes`), begrenzt nichts; ARCHITECTURE Z.66-67 bestätigt Fenster statt Budget | **Lane-22**: `sail-ooc/` + `sail-shard/` (Scheduler-Credits nach Bytes) + `typecheck.rs` (Fenster→Budget); unabhängig. Abnahme: OOC-Teilmenge mit Byte-Limit kleiner als AST-Bytes |
| O-9 | WS-10: Z3-Quote + Z3-128M-Nachweis | OFFEN | `sail-solver/src/pool.rs:15` (`SolverPool` ohne Limit/Quote — keine Treffer für limit/quota/permit); ARCHITECTURE Z.69 ("keine adaptive Speicherquote"); "Z3-intensive 16-Worker-Prüfung bei 128 MiB ist nicht nachgewiesen" (WORK_STATE Z.229) | **Lane-23**: `sail-solver/src/pool.rs` + `cache.rs` (Quote) ; nach Lane-22. Abnahme: 16-Worker-Typecheck unter 128M-cgroup mit Z3-lastigen Beweisen |
| O-10 | WS-1/WS-3: Vollsuite-128M-Flakiness + OOC-Wiederholung | OFFEN (subsidiär) — Einzeltest ok, Suite nicht | lane-01-Log: Einzel-Tiefentest OK (threads 1+4), aber Vollsuite-OOM aus WORK_STATE Z.137 un widerlegt (parallele Testbinaries unter gemeinsamem 128M-cgroup); OOC-Log `dependent-calls-ooc-128M.log` veraltet (vor Mutable-Arbeit, WORK_STATE Z.138) | **Lane-24**: nur `scripts/test-memory-limited.sh` (Sequenzierung/Retry-Doku) + Re-Run-Nachweise; nach Lane-01-Fix. Abnahme: `test-memory-limited.sh` Vollsuite grün + `typecheck-ooc-gate.sh` neu geloggt |

Nicht in 16–24 (bereits vergeben/aktiv, nicht anfassen): 02 (Speicherfix, solo ok),
03 (Grammatik/Convert-Faltung), 05 (OCaml-Vergleich, misst), 07/09/14/15 (aktiv),
12 (TypedDef-Fehlerpfad), 13 (Rewrite-Gerüst). Hinweis: `e2e.rs` erwähnt Lanes
29/38/45/47/116 (älteres Nummernsystem, TinyArm-E2E-Skizze, nicht ARM-Pfad) —
16–24 kollidieren damit nicht.

## 3. Gate-Urteile (explizit)

| Gate | Urteil | Beleg |
| --- | --- | --- |
| ARM-Gate Ende-zu-Ende (frontend→typecheck→rewrites→lean→lake) | OFFEN | `main.rs:117-131` bricht vor Rewrites/Lean ab; letzter ARM-Lauf historisch bei `to_bytes_le` (WORK_STATE Z.31); O-1→O-4 Kette |
| NoSorry-Gate | OFFEN (blockiert, nicht defekt) | `scripts/CheckLeanNoSorry.lean` + `scripts/test-lean-no-sorry.sh` vorhanden, aber ohne Compiler-Output nicht ausführbar; kein Gegenbeleg |
| OOC-128M-Nachweis nach allen Merges | OFFEN | `scripts/typecheck-ooc-gate.sh:36-38` erwartet weiter den Backend-Fehler; seit Mutable-Arbeit nicht wiederholt (WORK_STATE Z.138); Lane-24 |

## 4. Top-3 nächste Lanes

1. **Lane-16** (Vektor-Prim-Verdrahtung, nach Lane-07): löst den dokumentierten ARM-Blocker `to_bytes_le` auf und ist die einzige offene Abhängigkeit vor dem ersten echten ARM-Typecheck.
2. **Lane-17** (TypedDef-Emission): beendet die "Counts statt Programm"-Ära (`typecheck.rs`, `main.rs`); Voraussetzung für jede Rewrite-/Lean-Anbindung (O-3/O-4).
3. **Lane-19** (kanonische Exp-Regeln: Match/Struct/Field/VectorAppend/Try/usw.): ohne sie scheitert jeder ARM-Typecheck nach `to_bytes_le` an der nächsten `unsupported`-Stelle; Muster-Anteil bei Lane-09 lassen.
