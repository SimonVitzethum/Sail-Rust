# Lane 115 — scanner-dedup (58-Hotspot-1)
Branch: `lane/115-scanner-dedup` | Status: AKTIV

## Ziel
58-Analyse-Hotspot 1: sail-parse ↔ sail-sve-sme Scanner-Duplikate (lexer/span/
mini_ast byte-identisch, parser 1 Doc-Zeile) vereinen — eine Wahrheit, ein Import.

## Aufgaben
- Dedup + Dep-Verdrahtung, Tests als Code (NICHT ausführen, E3).
  `#![forbid(unsafe_code)]`.

## Done
- Kein doppelter Scanner, Verifikation via Rig.
