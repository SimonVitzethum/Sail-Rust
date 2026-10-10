# Lane 101 — solver-Not (ConstraintKind)
Branch: `lane/101-solver-symbol` | Status: AKTIV

## Ziel
Rig-Fehler: `ConstraintKind::Not` fehlt (sail-solver/src/query.rs:51) + Folgefehler
(query.rs:135 u.a. — neuestes Rig-Log lesen). Variante ergänzen oder Call-Site auf
现有-Varianten abbilden (Semantik wahren!).

## Aufgaben
- Minimaler Fix, Tests als Code (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- query.rs fehlerfrei (Rig-Beweis offen).
