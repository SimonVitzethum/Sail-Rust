# Lane 121 — symbol-threading (Lane-71-Fallout)
Branch: `lane/121-symbol-threading` | Status: AKTIV

## Ziel
22× E0308 aus Rig-Log limited-run-20261010-203328-129194.log (lesen!):
sail-parse/convert.rs + bridge.rs liefern String wo LitKind u.a. Symbol
wollen (Lane 71). Interner an Konversionsgrenze durchreichen (Signaturen
minimal erweitern) oder vor-internieren — kein Verhalten ändern.

## Aufgaben
- Fix an allen 22 Stellen, Tests als Code (NICHT ausführen, E3).
- `#![forbid(unsafe_code)]`.

## Done
- Alle 22 Fehler weg (Rig-Beweis offen).
