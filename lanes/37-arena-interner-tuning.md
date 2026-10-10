# Lane 37 — arena-interner-tuning
Branch: `lane/37-arena` | Status: QUEUED

## Ziel
Speicher-Tuning von Arena + Interner (ohne unsafe).

## Aufgaben
- Shard-größenabhängiges Sizing, `drop`-Punkte verifizieren, `cargo bloat`-Report.
- Verbietet eigene unsafe-Arenen; nur `slotmap`-artige Indizes.

## Done
- Peak-Delta dokumentiert (vorher/nachher aus Lane 14).
