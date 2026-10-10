# Lane 45 — e2e-tiny-arm
Branch: `lane/45-e2e-tiny` | Status: QUEUED

## Ziel
Erste durchgehende Pipeline Parse→Lean an `SailTinyArm.sail` + `lake build`.

## Aufgaben
- Alle Phase-1-Crates in einer Binary verdrahten (sail-cli als Treiber),
  `sail/test/lean/SailTinyArm.sail` durchlaufen lassen, Output per `lake build -K 2` prüfen.
- Lücken als Fail-Liste mit Owner-Lane (kein Fixieren fremder Crates).

## Done
- Tiny-Arm baut ODER Fail-Liste mit je max. 1 Owner.
