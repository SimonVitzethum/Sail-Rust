# Lane-39 Konsolidierungsnotiz — 2026-10-13

Nur lesend geprüft im Haupt-Worktree (`/home/simon/Dokumente/Sail/sail-rs`,
Basis `a972e3a`). Keine `crates/`- oder `scripts/`-Änderung durch Lane-39,
keine schweren Läufe, keine alten Logs/Fixtures angefasst.

## 1. Commit-/Branch-Befund (Beleg: `git log`, `git branch`, `git status`)

- `main` steht auf `a972e3a` (Doku-Konsolidierung, 2026-10-11). Kein neuer
  Commit seitdem auf `main`.
- Eigene Branch-Commits über `a972e3a` haben nur: `lane-07` (`34f79f1`),
  `lane-08` (`9a3b6a0`), `lane-14` (`9961e01`), `lane-20` (`755b5e3`),
  `lane-24` (`2955a2f`, `f5ce597`). Alle übrigen `lane-*`-Branches zeigen
  auf `a972e3a` (kein eigener Commit). `lane/41–44` sind ein separater
  Track (Multicore/Boot-Skizzen, kein ARM-Pfad).
- Der Haupt-Worktree enthält einen uncommitteten Sammelstand:
  gestagt nur Lane-08-Anteil (`program.rs`, `ast_env.rs`, 2 Testdateien),
  dazu ~60 weitere geänderte plus 12 ungetrackte Dateien (u. a. `typed_def.rs`,
  `vector_prims.rs`, `lexp_bounds.rs`, `exp_arms.rs`, `recursion.rs`,
  `budget.rs`, `quota.rs`-Dateien, neue Rewrite-Pässe, `lane-24`-Evidence).
  „Gemergt je Lane“ heißt daher unten: eigener Branch-Commit und/oder
  Inhalt im Sammel-Worktree mit Dateibeleg — **nicht** „in `main` committet“.

## 2. Verifizierte Zahlen (eigene Greps, Prozessausgang + Peak)

- Lane-01 (`evidence/2026-10-12/lane-01-oom-single.log`):
  Build `BUILD_EXIT=0`, Peak 199,7M; Tiefentest solo threads=1 `RUN_A_EXIT=0`,
  Peak 120,4M; derselbe Test threads=4 `RUN_B_EXIT=0`, Peak 120,7M.
  Je 1/1 bestanden, Swap 0.
- Lane-05 (`evidence/2026-10-12/lane-05-*.log`, `.md`):
  Positiv-Log 16× `ende rc=0` (15 Einzel + 1 kombiniert), Negativ-Log
  21× `ende rc=1`, 0 gekreuzte Ausgänge. Peaks 11,6–12,7M, Swap 0.
- Lane-24 (`evidence/2026-10-12/lane-24/`, REPORT.md, eigene Greps):
  Tiefentest solo 1/1, Peak 24,1M; `canonical_program` (35 Tests) threads=1
  35/35, Peak 53,5M; threads=4 Versuch 1+2 je 35/35, Peaks 102,7M/103,1M.
  Vollsuite sequenziell: 104 Suiten `ok`, Summe 1179 bestanden,
  104× `0 failed`, keine nicht-ok-Zeile, Gate `ende rc=0`.
  OOC 16 Worker: 4096 Definitionen, 1052672 Ausdrücke, 567919927 AST-Bytes,
  Typecheck 0,56 s, Compiler-Exit 1 (erwarteter Missing-Backend-Fehler),
  Lauf-Peak 128M (Kappe), Swap 0, Gate `ende rc=0`.
- Lane-15 (`evidence/2026-10-12/lane-15-profiler.log`, lane-berichtet, von
  Lane-39 nicht nachgemessen): Unit 13 Suiten/140 Tests, RC=0;
  Erstbuild-Peak 989,8M, gecacht 136,6M; OOC-Gate 16 Worker bestanden,
  Lauf-Peak 128M (Kappe), Swap 0.
- Befundlage schwerer Inhalt (39-Test-Binärdatei inkl. uncommittetem
  Mutable-Tiefentest, dokumentiert in `scripts/test-memory-limited.sh`-Header
  (Worktree) und Lane-24-REPORT §2): solo ~93,6M ok, sequenziell ~95,5M grün,
  threads=4 flaky (~126,6M ok vs. OOM-Kill). Von Lane-39 nicht neu gemessen.

## 3. OOM-Urteil (korrigiert)

Kein Leak-Befund: derselbe Tiefentest ist solo wie mit 4 Threads grün
(Lane-01: 120,4M vs. 120,7M), die 35-Test-Binärdatei skaliert ~1,9× mit der
Threadzahl (53,5M → ~103M). An der 128M-Decke kippt nur die schwere
39-Test-Kombination je nach Scheduling — Test-Parallelität, kein Leak.
Default bleibt 4 (Befund reproduzierbar), stabil ist `SAIL_TEST_THREADS=1`.

## 4. md-cleanup-sinngemäße Prüfung (Skript fehlt, manuell)

`scripts/md-cleanup-check` existiert nicht; Ersatzprüfung per Skript:
Root-Layout = exakt 5 Dateien (`ARCHITECTURE.md`, `README.md`, `TESTING.md`,
`THIRD_PARTY.md`, `WORK_STATE.md`), keine defekten relativen Links in
Root-Markdown (vorher und nachher: 0 defekt). Geprüft nach den Lane-39-Edits
erneut (siehe Abnahme-Tabelle in WORK_STATE.md).
