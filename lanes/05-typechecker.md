# Lane 05 — typechecker
Branch: `lane/05-typechecker` | Worktree: `sail-rs/worktrees/lane-05` (nach Merge löschen)

## Ziel
Sail2-Typchecker ohne Z3-Kern (Z3 kommt aus Lane 06 als Trait).

## Aufgaben
- `crates/sail-ty`: Bidirektionales Checken, Kind-Checking (`K_int/K_bool/K_type`), Typquants, Register-Typen.
- Definiert `Solver`-Trait (`prove(nc) -> bool/unknown`); Default-Fallback ohne Solver muss terminieren.

## Abgrenzung
- Kein Z3-Code hier, nur Trait + Caching-Schnittstelle.

## Done
- `aarch64_small` + `test/lean`-Subset checken mit Stub-Solver.
