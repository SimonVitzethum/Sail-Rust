# Lane 06 — z3-bridge-memo
Branch: `lane/06-z3-bridge` | Worktree: `sail-rs/worktrees/lane-06` (nach Merge löschen)

## Ziel
Z3-Anbindung mit begrenztem Speicher (Ersatz für OCaml `-memo_z3` unbegrenzt).

## Aufgaben
- `crates/sail-solver`: implementiert Lane-05-`Solver`-Trait via `z3`-Crate.
- Bounded LRU (Größe/TTL konfigurierbar, Default klein), Hit/Miss-Metriken, Solver-Pool (1 Solver pro Thread, kein Sharing).
- `unsafe` nur in der Dep, eigener Code `forbid(unsafe_code)`.

## RAM-Regel
- Cache-Limit einhaltbar, bei ARMv9 messbar (Lane 14 verifiziert).

## Done
- Gleiche Prove-Ergebnisse wie OCaml an `test/lean`-Subset bei <512MB Cache.
