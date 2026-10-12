# Lane-70: Tiny-Modell Ende-zu-Ende — Pipe-Beleg + Integrationslücken

Stand: 2026-10-13. Worktree `worktrees/lane-70`, Branch `lane-70`, Basis
`a972e3a` (sauber, keine Bestands-Änderung). Haupt-Worktree nur gelesen.
Alle Läufe via `/tmp/opencode/sail-gate/gated-run.sh` (Peaks inkl. Swap 0
in den Gate-Logs). Kein Push.

## Methode (kein Bestands-Umbau)

Drei NEUE Harness-Dateien (einzige `crates/`-Schreibungen) jagen dasselbe
Modell per Hand durch die Stufen — nötig, weil am Base-Stand kein
CLI-Pfad über Rewrite/Emit existiert (`run_compiler` endet nach Typecheck;
kein `TypedPipeline`, kein `exps.rs`):

- `crates/sail-ty/tests/lane70_tiny.rs` — Parse→Declare→Check je Def,
  schreibt `tiny.sail` (Single Source of Truth) + `stage-typecheck.log`.
- `crates/sail-lean/tests/lane70_emit.rs` — `typ_from_ast` je Modelltyp,
  Signaturköpfe + `Lane70tiny/Tiny.lean`, lake-Layout, Token-NoSorry-Scan.
- `crates/sail-cli/tests/lane70_project.rs` — `LakeProject::layout`-Render
  (Default + `lib_path`), Skelett ohne Funktions-Defs (Lücke L5/L7).

Das Modell (`tiny.sail`, prelude-frei, 22 Defs): 6 Funktionen
(`if`/Tupel/`let`-frei, Vektor- + Bitvektor-Literale, Enum per `and`-Klauseln),
`struct`/`enum`/`union`-Deklarationen, dazu 3 rote Sonden
(Record-Konstruktion, Feldzugriff, `match`). OCaml-Referenz (`/home/simon/.opam/sail/bin/sail -just_check`): Voll **und** Grün je RC=0 —
das Modell ist gültiges Sail; jede Rust-Ablehnung unten ist eine
Compiler-Lücke, kein Modellfehler.

## Grün-Tabelle (was grün wurde, mit Beleg)

| # | Stufe | Ergebnis | Beleg |
|---|---|---|---|
| G1 | Parse (Harness) | 22/22 Defs, inkl. `match`/`struct T{…}`/`p.x`/`and`-Klauseln | `stage-typecheck.log` (`parse ok #0–#21`), Harness RC=0 |
| G2 | Declare (Harness) | 22/22 in Source-Order, inkl. Record/Enum/Union + `val`-Specs | `stage-typecheck.log` (`declare ok`), RC=0 |
| G3 | Check grüne Teilmenge | 6/6 Funktionen, 16 Exprs; Rot exakt 3 mit Catch-all-Meldung | `stage-typecheck.log` (`SUMMARY defs=22 exprs=16 green_funs=6/6 red_defs=3`), RC=0 |
| G4 | CLI Frontend+Declare (grün) | 36 Defs aus 2 Dateien (Core-Lib + Modell), 82548 AST-Bytes, 16 Worker | `harness-ty-cli-ocaml-final.log`, CLI-GREEN |
| G5 | CLI Typecheck (grün) | 36 Defs, 17 Exprs, danach **expliziter** Abbruch „compilation incomplete: … rewrite integration and Lean body emission are not implemented“, Exit 1 (kein Stub-Erfolg) | ebd. |
| G6 | Rewrite Shard-Pipeline | Bestehende Suite grün: 11+1+1+2+1 = 16 Tests, 0 failed | `harness-ty-rewrite-cli-build.log` |
| G7 | Emit Typsenkung+Signaturen | `Unit`, `(Int × Bool)`, `(Vector Int 2)`, `(BitVec 2)`, `Colour` (Named) senken; 6 Köpfe generiert; Token-Scan sauber | `lane70_emit` RC=0, `Tiny.lean` |
| G8 | `lake build` (generiertes Lean) | `Build completed successfully`, Lean 4.29.0, offline (keine Deps); Gate-Fenster-Peak 1,8G, Swap 0 | `harness-emit-cli-lake-nosorry.log` |
| G9 | NoSorry-Gate | `Lean no-sorry check passed for 2 modules and their imports` (`CheckLeanNoSorry.lean`, RC=0) | ebd. |
| G10 | OCaml-Parität Modell | Voll RC=0, Grün RC=0 | `harness-ty-cli-ocaml-final.log` |

Hand-Anteile (markiert, nicht als Compiler-Leistung ausgegeben): alle 6
Lean-Bodies sind manuelle 1:1-Übersetzungen (`-- MANUAL BODY`), `inductive
Colour` ist manuell. Generiert sind Typsenkung, Köpfe, Layout.

## Lücken-Tabelle (Stufe → Fehler → zuständige Lane, kein Umbau)

| # | Stufe | Fehler (Laufzeit-belegt) | Owner |
|---|---|---|---|
| L1 | Typecheck `E_match` (`describe`) | `typechecking incomplete …: expression rule is not yet implemented in the canonical checker` (Catch-all `canonical_body.rs` `run()`) | **Lane 02** (full-`Exp`-Port; `sail-ty/src/pat.rs:6`). Vgl. Lane-31 B3 |
| L2 | Typecheck `E_struct` (`mk_point`, CLI-Abbruch `file1:596-653`) | dito | **Lane 02** (dito; Record-Konstruktion) |
| L3 | Typecheck Feldzugriff (`get_x`) | dito | **Lane 02** (dito) |
| L4 | Rewrite kanonisch | Kein `TypedPipeline`/`rewrite_stage.rs` am Base-Stand; nur Shard-Mini-AST (3 echte Pässe, Rest Stubs) | **lane-47** (`ast-unify`; `e2e.rs`: blockiert `lower_shard`/`realize_mapping`) + Lane-07-Notiz (`sail-rewrite/src/ast.rs:24`) |
| L5 | Lean-Typedefs | `struct`/`enum`/`union` haben keine Emission | **lane-30** (`lean-typedefs`, `e2e.rs`) |
| L6 | Lean-Bodies | Kein Drucker (`exps.rs` nur Branch `9961e01`, ungemergt) | **lane-14** (Branch existiert, ungemergt) |
| L7 | Emit-Treiber/CLI-Anbindung | `run_compiler` endet nach Typecheck (G5-Beleg) | **ohne dokumentierten Owner** (E2E-Skizze: lane-45 per `e2e.rs`-Header) |
| L8 | TypedDef-Handoff | `Checked` zählt nur Defs/Exprs (kein elaboriertes Programm für Rewrite/Lean) | **ohne dokumentierten Owner** am Base-Stand |
| L9 | `sail-ty` Laxheit (Sonde) | Wiederholte `function`-Zeilen werden akzeptiert (`DUP-PROBE accepted=true`), obwohl OCaml (`has already been declared`) und Resolver (`duplicate function`, `env.rs:389`) ablehnen | **ohne dokumentierten Owner** (`sail-ty` declare-Pfad) |

Triage-Notiz (kein Fix nötig): Der CLI-Frontend-Resolver bricht früher ab
als der kanonische Checker (L2 zuerst: `file1:596-653` statt L1/L3) —
Schicht-Reihenfolge beim Lesen von Gate-Abbrüchen beachten.

## Korrigierte Zwischenbehauptung

Erste Modellfassung nutzte wiederholte `function`-Zeilen als Mehrklauseln;
OCaml + Resolver lehnen das zu Recht ab (korrekte Form: `and`-Klauseln,
vgl. `sail/test/typecheck/pass/funcl_guard.sail`). Beleg: `ocaml-first-run.log`.
Die Harness-Akzeptanz der alten Form ist als L9-Sonde erhalten, nicht als
Freigabe.

## Dateien dieser Lane (nur `evidence/2026-10-13/lane-70/` + 3 Harness-Dateien)

- `tiny.sail`, `tiny_green.sail` — Modell / Grün-Teilmenge (OCaml RC=0/0).
- `stage-typecheck.log` — je-Def Parse/Declare/Check + Scheme-Snapshots + DUP-PROBE.
- `Tiny.lean`, `Lane70tiny.lean`, `lakefile.toml`, `lean-toolchain` — gebautes Projekt (G8/G9).
- `lake-skeleton/` — `sail-lake`-Render ohne Funktions-Defs (L5/L7-Beleg).
- `harness-ty-rewrite-cli-build.log`, `harness-emit-cli-lake-nosorry.log`, `harness-ty-cli-ocaml-final.log`, `ocaml-first-run.log` — Rohlogs aller Gate-Läufe.
- `REPORT.md` (diese Datei).
