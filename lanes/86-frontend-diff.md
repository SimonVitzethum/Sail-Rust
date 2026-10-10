# Lane 86 — frontend-diff (OCaml-Parität)
Branch: `lane/86-frontend-diff` | Status: AKTIV

## Ziel
Differential-Harness Frontend: OCaml-sail vs. Rust-Frontend (Parse-Bäume normalisiert
vergleichen, Typ-Urteile stichproben) als Code (NICHT ausführen, E3) +-diff-Plan.

## Aufgaben
- Normalisierer + Vergleichslogik + Fall-Katalog (sail/test als Fixtures).
  `#![forbid(unsafe_code)]`.

## Done
- Harness-Code + Plan, Rig-Lauf später.
- Geliefert (E3, ungetesteter Code): neues Crate `sail-frontend-diff`
  (Workspace-Member) mit `norm_tree` (Rust-`ParsedFile`-Normalisierung +
  OCaml-`--ddump-initial-ast`-Sifter), `tree_compare` (positionaler
  Kind/Name/Header-Vergleich + Seq-Diff), `ty_sample` (deterministische
  First-N-Stichprobe vs. `--ddump-tc-ast`-Sifter), `catalog` (36
  `sail/test/lean`-Fixtures: 8 smoke / 25 standard / 3 nightly,
  `errors` = ParseOnly), `harness` (`--just-check`-Dump-Anrufe unter
  ulimit+timeout, `run_frontend_case`), 4 Testdateien (nicht ausgeführt),
  `FRONTEND_DIFF_PLAN.md` (Diff-Plan). Sifter-Heuristik am ersten Rig-Lauf
  gegen Sail 0.20.3 verifizieren.
