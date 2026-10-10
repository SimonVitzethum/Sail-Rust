# Lane 04 — nexp-constraint
Branch: `lane/04-nexp-constraint` | Worktree: `sail-rs/worktrees/lane-04` (nach Merge löschen)

## Ziel
Numerische Typausdrücke (NExp) + Constraints als eigene, testbare Einheit.

## Aufgaben
- `crates/sail-nexp`: `Nexp_*`-Algebra (constant/var/if/times/sum/minus/neg/exp/div/mod/abs), Vereinfachung, `provably_nneg`-Äquivalent.
- Reine Funktionen, kein Solver-Backend.

## Input/Output
- Input: NExp/Constraint. Output: normalisierte Form + `Nat-vs-Int`-Entscheidung für Lean (`Nat`/`Int`-Abbildung).

## Done
- Property-Tests (proptest) für Arithmetik-Identitäten.
