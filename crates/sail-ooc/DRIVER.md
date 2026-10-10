# OoC-Driver-Schleife (Lane 83, E4-bindend)

Fahrtuechtige Schleife ueber [`sail-ooc`](src/lib.rs) (Lane 60: Barriere/Warm/Memo/Summaries),
den Lane-62-Hooks (`sail-cli::ooc`, gelesen) und den Lane-74-Summaries
(Callgraph/Mono/ConstProp, gelesen, nicht gedoppelt):

```text
Shard holen → Phase (heiss) → sealen → spillen → weiter
```

E3: keine Builds/Tests in Lane-Agents — Tests liegen als Code bei
(`src/driver.rs`, `#[cfg(test)]`); Verifikation spaeter zentral per Test-Rig.
`#![forbid(unsafe_code)]` (Crate-Root).

## Zustandsdiagramm

```text
                        ┌─────────────────────────────────────────────┐
                        │                                             │
                        ▼                                             │
┌──────┐ holen/begin ┌──────────────┐ seal ┌────────┐ spill ┌─────────┐ │weiter
│ Idle │ ──────────▶ │ Hot(fetch →  │ ───▶ │ Sealed │ ────▶ │ Spilled │ ─┘
│      │             │ typecheck →  │      │ (RAM   │       │ (warm + │
│      │ ◀────────── │ rewrite)     │      │ frei)  │       │ summary)│
└──────┘ abort/evict └──────────────┘      └────────┘       └─────────┘
  │  ▲                     │                                    │
  │  │                     │ abort/evict (kein Cache-Eintrag)   │ summary_insert
  │  │                     ▼                                    ▼
  │  │              (Evicted-Zaehler +1,                    nur Fakten in RAM
  │  │               Barriere sofort frei)                  (Bodies per ContentId)
  │  │
  └──┴── finish (evict_all: Warm-Referenzen frei, Dateien bleiben) ──▶ Finished
```

Zustaende in Code: [`OocDriverLoop::state`](src/driver.rs) (`Idle`, `Hot{shard,phase}`,
`Finished`). Besitz-Modell: Der heisse [`HotToken`](src/barrier.rs) lebt im Treiber —
solange er lebt, weist `begin` jeden zweiten Shard ab (`AlreadyHot`). `seal`
konsumiert das Token (serialisieren + verwerfen); faellt es ohne `seal`/`evict`
weg, gibt sein `Drop` die Barriere frei (Abbruchpfad, `evicted` +1).

Reihenfolge ist fix (**Seal-vor-Spill**): Schlaegt Warm-Spill oder Summary fehl,
ist die Barriere trotzdem frei — der Shard ist versiegelt, aber nicht gezaehlt.

## Schleife (eine Iteration)

| Schritt | Phase | RAM | Code |
|---|---|---|---|
| 1. holen | `Fetch` | Shard-Bytes | `fetch()` (kalt → heiss) |
| 2. Phase | `Typecheck`/`Rewrite` | Shard-AST + Rewrite-State (genau ein Shard) | `run_phase(&fetched)` |
| 3. sealen | `Seal` | nichts mehr (verworfen) | `barrier.seal(token, &bytes, PASS)` → `SealReceipt{content}` |
| 4. spillen | `Spill` | — | `warm.spill(&receipt.content, &bytes)` |
| 5. Summary | `Summary` | nur Fakten (≤ 64 KiB) | `summaries.insert(record)` (Body nur per `ContentId`) |
| 6. weiter | — | — | naechster Shard; am Ende `finish()` (`evict_all`) |

Memo-Spills laufen orthogonal dazu: `memo_prove(query, compute)` — Lookup zuerst,
sonst rechnen + spillen (run-uebergreifend, Domaene z. B. `"solver/z3-v1"`).

## Peak-Budgets je Phase (Lane-14-Format, deklariert — keine Messlaeufe)

Konstanten in [`src/driver.rs`](src/driver.rs) (`PHASE_BUDGETS`, `check_phase_budget`;
Format wie `sail-perf::BUDGETS`/`check_budget`, ohne neue Abhaengigkeit):

| Phase | Konstante | Limit | Wofuer |
|---|---|---|---|
| `fetch` | `PEAK_FETCH_KIB` | 64 MiB | Shard-Bytes holen |
| `typecheck` | `PEAK_TYPECHECK_KIB` | 512 MiB | Shard-AST heiss |
| `rewrite` | `PEAK_REWRITE_KIB` | 512 MiB | Rewrite-State heiss |
| `seal` | `PEAK_SEAL_KIB` | 128 MiB | Serialisierung (kurzlebig) |
| `spill` | `PEAK_SPILL_KIB` | 256 MiB | Warm-Spill (live Bytes) |
| `summary` | `PEAK_SUMMARY_KIB` | 4 MiB | Summary-Tabelle (Eintrag ≤ 64 KiB) |

Heiss ist hoechstens EIN Shard (~1,5 GiB Spitze); passt mit Abstand ins kleinste
Modellbudget (4 GiB, Lane 14: `aarch64_small` 4 GiB, `arm-v8.5-a` 16 GiB,
`arm-v9.4-a` 24 GiB). Bei Ueberschreitung: Shard verkleinern oder Pass in
Lane 07/09 zurueckweisen (Fehlertext traegt die Anweisung).

## Anbindung Lane 74 (nur referenziert)

Lane-74-Summaries (`CallgraphSummary`, `MonoSummary`, `ConstPropSummary`) sind
[`ShardSummary`](src/summary.rs)-Impls und passen als `summarize`-Schluss in
`run_shard`/`run_all` (2–3 Zeilen Mapping von der Pass-IR auf reine Daten):

```text
let summary = CallgraphSummary::new(shard.clone(), receipt.content, facts);
table.insert(callgraph_record(&summary)?)?; // Bodies bleiben auf Platte
```

Der Treiber kennt nur [`SummaryRecord`](src/summary.rs) — keine Abhaengigkeit
auf Pass-Crates, keine Umbauten fremder Crates.
