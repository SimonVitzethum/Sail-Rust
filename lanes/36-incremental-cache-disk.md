# Lane 36 — incremental-cache-disk
Branch: `lane/36-inccache` | Status: QUEUED

## Ziel
Disk-Cache für Shards (Second-Run = Skip).

## Aufgaben
- `target/sail-cache/<shard-hash>/` mit Output + Fingerprint, Atomar schreiben, GC alter Einträge.

## Done
- Zweitlauf ohne Änderung schreibt nichts neu (Test in Lane 13).
