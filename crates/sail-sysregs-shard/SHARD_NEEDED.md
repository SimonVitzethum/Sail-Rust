# SHARD_NEEDED — was Lane 11 (`sail-shard`) liefern muss

`sail-shard` ist noch ein Platzhalter (`crates/sail-shard/src/lib.rs`).
Diese Lane definiert Shard-Typen **lokal** (`sail-sysregs-shard`) und nutzt
kein Gesamt-Doc. Migration, sobald `sail-shard` landet:

## Von `sail-shard` benoetigt

1. **Block-Typ mit konstantem Budget**: `struct ShardBlock { index, defs: Vec<Def>, fingerprint: u64 }`
   — Fingerprint-Algorithmus muss zentral festgelegt werden (diese Lane nutzt
   FNV-1a-64 ueber die Block-Bytes; `combined_fingerprint` ist grenzenabhaengig).
2. **Streaming-Splitter-Trait**: Zeilen-/Token-weises Splitten an
   Top-Level-Grenzen, das nie das Gesamt-Dokument haelt
   (entspricht `DefSplitter`/`BlockSplitter` hier).
3. **Per-Shard-Fingerprints + Skip-Protokoll**: Hash ueber (Input-Bytes +
   Pass-Version) je Shard, damit Zweitlaeufe unveraenderte Shards
   ueberspringen (Lane-11-`Done`: Zweitlauf == alles skipped).
4. **Fehlertyp mit Zeilenbezug** (`SplitError { line, message }`-aequivalent),
   damit Shard-Fehler auf die Quelldatei zurueckfuehren.

## Von `sail-lex` / `sail-parse` benoetigt (spaeter)

- Pro-Block-Lexen/Parsen statt Datei-als-Ganzes: `parse_block(&str)` hier ist
  nur Kopf-Pruefung (`val`/`function`-Paarung). Sobald `sail-parse` stabil ist,
  wird der Block-Inhalt durch den echten Def-Parser ersetzt — Blockgrenzen
  und Streaming-Rahmen bleiben unveraendert.
- Referenz: `crates/sail-parse/src/{lib,mini_ast,parser}.rs`,
  `crates/sail-lex/src/{lexer,lib}.rs` (nur gelesen, keine Pfad-Dependency,
  damit die Lane selbstgenuegsam bleibt).

## Nicht in dieser Lane

Full-Build, Typcheck, Lake-Projekt, Z3 — nur parsen + sharden + messen.
