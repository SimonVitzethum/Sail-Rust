# Lane 60 — out-of-core Umsetzung (E4)
Branch: `lane/60-ooc` | Status: AKTIV

## Ziel
E4 umsetzen: nur Heißes in RAM (Shard-AST, Rewrite-State, heiße Memos),
Warm per mmap (fertige Shards, Prelude/Builtins), Kalt per Disk-Cache (Content-Hash-Key).

## Aufgaben
- `crates/sail-ooc`: Phasen-Barrieren (typchecken → serialisieren + verwerfen → nächster),
  mmap-Layer (safe-API, read-mostly), Memo-Spills mit Content-Hash (Z3-Ergebnisse
  run-übergreifend), Cross-Shard nur als Summaries (Lanes 08/09 anpassen wo nötig).
- Auf Lane 11 (Shards), 36 (Disk-Cache), 37 (Arena/IDs) aufsetzen — lesen, nicht doppeln.
  Stabile Content-Hash-IDs statt Zähler (sonst Cache wertlos).
- E3: KEINE Builds/Tests ausführen. `#![forbid(unsafe_code)]`.

## Done
- OoC-Design implementiert + Doku, Verifikation via Test-Rig (später).
