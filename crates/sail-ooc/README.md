# sail-ooc — Out-of-core-Gerüst (Lane 60, E4)

Nur Heißes in RAM (Shard-AST, Rewrite-State, heiße Memos), Warm per Datei
(fertige Shards, Prelude/Builtins), Kalt per Disk-Cache (Content-Hash-Key).
E3: keine Builds/Tests in Lane-Agents — Verifikation später zentral per Test-Rig.

## Bausteine

| Typ | Rolle |
|---|---|
| `ContentId` (`id.rs`) | Stabile Content-Hash-ID (blake3, längen-präfixiert) statt Zähler |
| `PhaseBarrier`/`HotToken`/`SealReceipt` (`barrier.rs`) | Phasen-Barriere: höchstens ein heißer Shard; `begin` → `seal` (serialisieren + verwerfen) / `evict` |
| `WarmLayer` (`warm.rs`) | Warmer Layer: `<hex>.blob`, atomare Spills, `Arc<[u8]>`-Reads (read-mostly), explizites Live-Budget, `evict`/`evict_all` |
| `MemoSpills` (`memo.rs`) | Kalte Memo-Spills (z. B. Z3-Ergebnisse) über `sail-cache` (Lane 36), run-übergreifend, Domänen-getrennt |
| `SummaryTable`/`ShardSummary`/`ExportedSymbols` (`summary.rs`) | Cross-Shard-Summaries für Lanes 08/09; max. 64 KiB pro Eintrag, Bodies nur per `ContentId` referenziert |
| `CallgraphSummary`/`MonoSummary`/`ConstPropSummary` + `export_*` (`summaries.rs`) | Lane 74: Summary-Typen + Export-Schnittstelle je interprozeduralem Pass (Callgraph/Mono/Const-Prop); Bodies bleiben auf Platte |

## Warum kein echtes mmap

`mmap` braucht `unsafe` für den Zeiger-Cast; das Crate steht unter
`#![forbid(unsafe_code)]` (E3/E4-bindend). `WarmLayer::get` lädt per
`fs::read` — der OS-Page-Cache paget selbst ein/aus; das Zugriffsprofil
(Shard-Streaming via `prefetch`, Punktzugriffe, read-mostly) ist dasselbe.
Ein mmap-Backend kann später hinter der gleichen API nachgerüstet werden.

## Treiber-Skizze

```text
for shard in plan {
    let hot = barrier.begin(shard.id.clone())?;      // heiß: typchecken/rewrite
    let bytes = typchecken_und_serialisieren(&hot);  // nur dieser Shard in RAM
    let receipt = barrier.seal(hot, &bytes, PASS_VERSION)?;
    warm.spill(&receipt.content, &bytes)?;           // warm ablegen
    summaries.insert(SummaryRecord::from_summary(&exports)?)?; // nur Summary
}
warm.evict_all(); // Drop-Punkt (Lane-37-Policy)
```

Auf Lane 11 (`sail-shard`: Shards/Fingerprints) und Lane 36 (`sail-cache`:
Disk-Cache) aufgesetzt — gelesen, nicht gedoppelt. Lane-37-Drop-Punkte
tragen `evict_all`/`SummaryTable::remove`.
