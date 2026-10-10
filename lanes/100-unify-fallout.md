# Lane 100 — unify-fallout (Rig-Errors Welle 5+)
Branch: `lane/100-unify-fallout` | Status: AKTIV

## Ziel
Rig-Fehler aus Unify/Payload-Wellen schließen (exakte Meldungen im neuesten
sail-rs/logs/limited-run-*.log lesen!):
- `Symbol: Display` fehlt (sail-ty/env.rs, sail-solver/query.rs, sail-bitvec/conv.rs) —
  Display-Impl an zentraler Stelle (sail-ast interner) oder lokale Bridges.
- `SharedTables: Serialize/Deserialize/Eq` + `==` auf Arc (sail-ty/env.rs).
- `PatKind::Typ` hat kein Feld `inner` (sail-ty/conv.rs:194) — an neue Shape anpassen.

## Aufgaben
- Minimale Fixes, Verhalten identisch. Tests als Code (NICHT ausführen, E3).
- `#![forbid(unsafe_code)]`.

## Done
- Alle obigen Fehler weg (Rig-Beweis offen).
