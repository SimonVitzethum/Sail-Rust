# Lane 79 — typechecker-full (Frontend vollständig 4/7)
Branch: `lane/79-typechecker-full` | Status: AKTIV

## Ziel
Vollständiger bidirektionaler Checker: alle Sail-Typregeln (NExp-Kinds, Typquants,
Register-Typen, Subsumption, coercions), Z3 nur via Solver-Trait (Lane 06/80).

## Aufgaben
- sail-ty vervollständigen, Regelabdeckung als Tabelle (Regel → Status → Test).
  Tests als Code (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Regel-Tabelle vollständig, keine infer/check-Lücken.
