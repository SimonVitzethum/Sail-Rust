# Lane 89 — ooc-types (E0608+E0506)
Branch: `lane/89-ooc-types` | Status: AKTIV

## Ziel
Zwei Typfehler (Rig-Log): summaries.rs:101 (Index in Result → match/unwrap_logik mit
Fehlerpfad) + barrier.rs:170 (self.generation-Borrow vs. lock_state — Reihenfolge/
Clone minimal ändern).

## Aufgaben
- Fixe, Tests als Code (NICHT ausführen, E3). `#![forbid(unsafe_code)]`, keine API-Brüche
  (Lane-70-Drop-Semantik erhalten).

## Done
- Beide Fehler weg (Rig-Beweis offen).
