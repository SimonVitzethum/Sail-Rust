# sail-par-pipe (Lane 69)

Shard-parallele Pipeline-Orchestrierung **Parse → Check → Emit** als
Code-Skizze + Doku. `#![forbid(unsafe_code)]`, nur std + `crossbeam-channel`
+ `rayon` (+ `sail-shard` aus Lane 11).

## Was diese Crate ist

Orchestrierungs-Code, **kein Treiber-Umbau**: Die drei Stufen sind injizierte
Closures, sodass echte `sail-parse`- / `sail-ty`- / `sail-lean`-Treiber später
verdrahtet werden, ohne fremde Crates umzubauen. Tests liegen als Code bei
und werden unter E3 **nicht ausgeführt** (Verifikation via Test-Rig, später).

## Architektur

```text
shards (kanonische Ordnung, z. B. ARMv9_SHARD_ORDER)
  │  ingress: (order, Shard)
  ▼
┌─────────┐ bounded(8) ┌─────────┐ bounded(8) ┌────────┐ unbounded ┌─────────┐
│  PARSE  │ ─────────▶ │  CHECK  │ ─────────▶ │  EMIT  │ ────────▶ │  MERGE  │
│ n Work. │ Backpress. │ n Work. │ Backpress. │ n Work.│ deadlock- │ sort by │
└─────────┘            └─────────┘            └────────┘ frei      │  order  │
                                                                  └─────────┘
```

Zwei Ausführungsvarianten (`src/pipeline.rs`):

| Variante | Funktion | Überlapp | Basis |
|---|---|---|---|
| Gestuft | `run_pipeline` | Parse(N+1) neben Check(N) neben Emit(N−1) | eigene Pools + bounded Kanäle |
| Ein-poolig | `run_shard_parallel` | keiner (ganze Kette pro Shard) | `sail_shard::Scheduler::run_with_index` |

## Regeln

- **Bounded Queues / Backpressure:** Stufen-Kanäle mit Kapazität
  `max_inflight` (Default 8, Lane-11-Konvention). Voller Kanal blockiert den
  Sender — wartende Eingaben bleiben O(`max_inflight`), egal wie viele Shards.
- **Ergebniskanal unbegrenzt:** wie im Lane-11-Scheduler; begrenzt +
  erst-nach-Produktion-lesen verklemmt bei mehreren Workern.
- **Ein Shard pro Worker:** ein Worker bearbeitet stets genau einen Shard.
- **Deterministischer Merge:** Eingangsindex `order` wird am Ingress
  vergeben; Konsument/Merge sortiert danach (`src/merge.rs`: `merge_ordered`,
  `merge_ordered_par`, `manifest`, `concat_ok`). Ausgabe bit-identisch,
  unabhängig vom Scheduling.
- **Fehlerisolation:** `StageError { stage, shard, message }` wird nach unten
  durchgereicht; Nachbar-Shards laufen weiter. `concat_ok` sammelt *alle*
  Fehler (kein stilles Überspringen).
- **Panics sind fail-fast** (Scope propagiert); erwartete Fehler sind
  pro-Shard-`Result`s.

## Lane-Bezug (nur gelesen, nichts gedoppelt)

- Lane 11 (`sail-shard`, Branch-Stand): `Shard`/`ShardId`, `Scheduler`
  (bounded Queue, `max_inflight` 8), `IncrementalCache` — wiederverwendet,
  nicht nachimplementiert. Eigene Queue-Logik nur dort, wo die Stufung sie
  erzwingt (drei Kanäle statt einem), mit derselben Schließ-Kaskade.
- Lane 43 (Scheduler/Replay, QUEUED): bounded-Queue-Prinzip übernommen; der
  `order`-Index + `manifest` liefern die deterministische Ordnung, auf der
  ein späterer Replay-Modus aufsetzen kann.
- Lane 62: im Branch-Stand kein Crate vorhanden (geprüft 10.10.2026).

## Weiteres (Später, nicht hier)

- Echte Treiber-Verdrahtung (`sail-parse` → `sail-ty` → `sail-lean`) als
  Closures; `IncrementalCache::plan` davor (nur `Dirty`-Shards einspeisen),
  `mark_built` danach.
- Test-Rig: Ordnungs-Test mit streuenden Sleep-Zeiten, Backpressure-Test
  (`max_inflight = 1` seriell), Fehlerisolations-Test, Paritäts-Test
  gestuft vs. ein-poolig.
