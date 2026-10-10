# Lane 76 — grammar-harden (Frontend vollständig 1/7)
Branch: `lane/76-grammar-harden` | Status: AKTIV — Ziel: ARM in 20GiB, multithreaded.

## Ziel
LALRPOP-Generierung wirklich einschalten: restliche Konflikte in sail.lalrpop lösen
(E3c/E4-Designs aus Lane 59/GRAMMAR.md + CORPUS.md-Befunde), sodass `process_root`
durchläuft. build.rs wieder scharf stellen (mit klarem Fehler statt panic, falls doch).

## Aufgaben
- Konflikt-Fixes im .lalrpop, D1/D2 per CORPUS.md entscheiden. Tests als Code
  (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Grammatik konfliktfrei (Rig-Beweis offen), Generierung an.
