# Lane 08 — callgraph-effects
Branch: `lane/08-callgraph-effects` | Worktree: `sail-rs/worktrees/lane-08` (nach Merge löschen)

## Ziel
Callgraph + Effektanalyse (rein vs. `SailM` vs. `ExceptM`/Early-Return/Loops).

## Aufgaben
- `crates/sail-effects`: Callgraph-Aufbau, `function_is_pure`, `has_effect/has_loop/has_early_return`-Prädikate als reine Funktionen auf der AST.
- Liefert Flags, die Lane 10 für `do`-Blöcke/`←` braucht.

## Done
- Effektlabels stimmen mit OCaml an `test/lean`-Subset überein.
