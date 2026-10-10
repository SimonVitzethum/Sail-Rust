# Lane 12 — lake-project-gen
Branch: `lane/12-lake-gen` | Worktree: `sail-rs/worktrees/lane-12` (nach Merge löschen)

## Ziel
Lake-Projekt-Generierung wie `sail_plugin_lean.ml` (`start_lean_output`, `create_lake_project`).

## Aufgaben
- `lean-toolchain` (pin, Default wie OCaml `lean4:v4.29.0`), `lakefile.toml` (`lean-sail rev v6`, optional mathlib nur bei `real-numbers`), `Defs.lean`-Prelude (`maxHeartbeats/maxRecDepth`), `Specialization`-Import, `ArmExtras`-Import, Register/Instanzen-Blöcke.
- Kein `sorry`-Einfügen; `decreasing_trivial`-Taktik nur aus `ArmExtras` übernehmen.

## Done
- Generiertes Projekt `lake build -K 2` fähig an kleinem Modell.
