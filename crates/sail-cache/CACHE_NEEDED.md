# CACHE_NEEDED.md — sail-cache ↔ Lane 11 (sail-shard)

Stand: Lane 11 (`sail-shard`) ist noch Platzhalter (nur `lib.rs`-Stub).
`sail-cache` definiert deshalb **lokale Fingerprint-Minimaltypen**
(`ShardFingerprint`, `fingerprint()`), damit Lane 36 unabhängig
testbar ist.

## Was Lane 11 liefern muss (Integrationsvertrag)

1. **Shard-Identität** — stabiler String pro Shard für `<shard-hash>/`
   (nur `[A-Za-z0-9._-]`, max. 128 Zeichen; sonst `InvalidInput`).
   Vorschlag: aus Datei-Shard-Namen ableiten (z. B. `instrs32-<fp16>`).
2. **Shard-Input-Bytes** — kanonische Bytes, über die `fingerprint()`
   läuft (Sail-Quelltext des Shards + relevante Config).
3. **Pass-Version** — String, der sich bei jeder Pass-Änderung ändert
   (Compiler-Version + Pass-Flags). Invalidiert den Cache implizit.
4. **Ablauf pro Shard** (Second-Run-Skip):
   `is_fresh()` → bei Treffer `lookup()` (Output nutzen),
   sonst kompilieren → `store()` (gibt `false` zurück, falls
   zwischenzeitlich schon frisch — dann nichts geschrieben).

## Was NICHT nötig ist

- Bounded Queue / Parallel-Scheduler bleiben Lane 11 (bleibt dort).
- `sail-cache` ist pass-agnostisch: jeder Output (`Vec<u8>`) cachbar.
- LRU-Limit pro `DiskCache`-Instanz (`with_max_entries`); Default 128.

## Ablösung der Minimaltypen

Sobald `sail-shard` echte Fingerprints hat: `ShardFingerprint` durch
den Lane-11-Typ ersetzen (beide sind 16-Hex-FNV — Migration = Type-Alias
+ Re-Export, kein Layout-Wechsel auf Disk nötig).
