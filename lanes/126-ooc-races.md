# Lane 126 — ooc-races (Bug-Hunt-Fixes)
Branch: `lane/126-ooc-races` | Status: AKTIV

## Ziel
Concurrency-Befunde: driver-Barriere-Leck (begin→Err ohne evict, finish mit hot),
DiskCache-Race (Mutex um check-then-act + Lookup-Touch + gc), Decode-Caps
(u32-Längen!), sealed-Vec + SummaryTable deckeln, Thundering-Herd-Doku/Singleflight,
warm-Ersetzungspfad, saturating-Drift, HashMap-Ordnung sortieren, memo-store-Err
nicht schlucken, u32-Trunkierung.

## Aufgaben
- Fixe minimal (Semantik: kein Deadlock, kein Leak, kein falscher Treffer).
  Tests als Code (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Alle Punkte behoben oder begründet abgewiesen (Rig-Beweis offen).
- Lane 126 (E3, kein Build/Test-Lauf): je Fundstelle Fix + Test als Code.
  - Barriere-Leck: `run_shard` wirft nach `begin`-Fehlern per `fail_hot` ab
    (`sail-ooc/driver.rs`, `sail-cli/ooc.rs` Seal-Fehler → `abort_hot`);
    `finish` mit heissem Shard verwirft (`evicted`) statt zu lecken.
  - DiskCache-Race: prozessweite `Mutex`-Sperre (`Arc<Mutex<()>>`, `Clone`
    teilt sie) um Check-then-act (`store`), Lookup-Touch (`lookup`) und `gc`;
    Vergiftung-tolerant; prozessübergreifend dokumentiert Best-Effort.
  - Decode-Caps: Gesamtdeckel `MAX_SUMMARY_BYTES`, Count-Deckel
    `MAX_DECODE_ENTRIES` (16_384), String-Deckel je Eintrag — in allen vier
    `decode`-Pfaden (`summary.rs`, `summaries.rs` via `Cursor::count`/`str`).
  - sealed/summary-Deckel: `MAX_SEALED_RECEIPTS` (65_536, `SealedFull` wie
    Evict + `clear_sealed`) und `MAX_SUMMARY_ENTRIES` (65_536, `TooMany`;
    `insert` → `Result`, `export_*`/Driver propagieren).
  - Thundering-Herd: kein Singleflight — dokumentiert in `DiskCache`,
    `MemoSpills`, beiden `memo_prove`; Erstling gewinnt, extern dedup.
  - warm-Pfad: `spill` mit live-fremdem Inhalt → `ContentMismatch` (kein
    stilles Ersetzen mehr); Plattenpfad unverändert.
  - saturating: `live_bytes` über `checked_add`/`checked_sub` (`expect` statt
    Sättigung) + `debug_check_accounting` (Debug, O(n)); Budgetvergleiche mit
    `checked_add(...).unwrap_or(MAX)`.
  - Ordnung: `SummaryTable::shards()` sortiert (Name, Chunk); HashMap-Iteration
    sonst nirgends beobachtbar.
  - store-Err: `memo_prove` zählt `memo_store_errors` (Best-Effort dokumentiert),
    neu `memo_prove_result` → `DriverError::Io`.
  - u32-Trunk: `check_u32_len`/`checked_u32`/`try_write_str`, `try_encode` je
    Summary-Typ (Default `Ok(encode)`), `from_summary` nutzt Prüfpfad;
    `encode` panict statt zu kappen; `write_u32`/`write_str` entfernt.
- `#![forbid(unsafe_code)]` überall eingehalten (nur `std::sync::Mutex`).
