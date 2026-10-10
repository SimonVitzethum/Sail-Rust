# Lane 09 — mono-anf-lowering
Branch: `lane/09-mono-anf` | Worktree: `sail-rs/worktrees/lane-09` (nach Merge löschen)

## Ziel
Monomorphisierung + ANF-Lowering speicherschonend (Shard für Shard, kein Gesamt-AST-Clone).

## Aufgaben
- `crates/sail-lower`: `-auto_mono`-Äquivalent mit Split-Limits, `internal_lets`, `remove_blocks`, Pattern-Exhaustiveness nur pro Funktion.
- Frühes Droppen: nach Lowering wird Original-Def freigegeben.

## RAM-Regel
- `sysregs_autogen.sail` (160k Zeilen) muss in konstanter Shard-Größe durchlaufen, Peak wird von Lane 14 gemessen.

## Done
- Mono-Output an `instrs32`-Subset äquivalent zu OCaml (Diff-Test Lane 13).
