# Lane 46 — arm-v8.5-a-Durchlaufplan (E2E, nur Plan — E3: kein Lauf)

Modell: `sail-arm/arm-v8.5-a` (außerhalb des Worktrees, nur gelesen).
Ziel: `arm-v8.5-a`-Durchlauf mit Budget **<16 GiB Peak-RSS, kein Swap** (Lane 14).
Status: **Plan + Messgerüst only** — Messung steht aus (E3-Test-Moratorium),
führt der Integrator zentral mit Memory-Limits aus.

## 1. Dateiliste (`model/`, `sail.json`-Reihenfolge) mit Zeilenzahlen

Quelle: `sail-arm/arm-v8.5-a/model/sail.json`
(Optionen: `-non_lexical_flow -no_lexp_bounds_check`; Eingabereihenfolge unten).
`wc -l -c`, Modellstand gelesen am 10.10.2026 (nur `ls`/`wc`, kein Build/Lauf):

| # | Datei | Zeilen | Bytes | Anteil Zeilen |
|---|-------|--------|-------|---------------|
| 1 | `model/prelude.sail` | 464 | 17 328 | 0,4 % |
| 2 | `model/no_devices.sail` | 78 | 4 443 | 0,1 % |
| 3 | `model/aarch_types.sail` | 683 | 15 717 | 0,5 % |
| 4 | `model/aarch_mem.sail` | 11 526 | 408 871 | 9,3 % |
| 5 | `model/aarch64.sail` | 30 205 | 1 692 365 | 24,3 % |
| 6 | `model/aarch64_float.sail` | 2 441 | 82 311 | 2,0 % |
| 7 | `model/aarch64_vector.sail` | 9 528 | 383 463 | 7,7 % |
| 8 | `model/aarch32.sail` | 46 913 | 2 105 099 | 37,7 % |
| 9 | `model/aarch_decode.sail` | 21 464 | 833 374 | 17,3 % |
| | **Summe (9 Dateien)** | **124 406** | **5 584 015** | 100 % |

Nicht in `sail.json` (nur Executable-Variante via `Makefile`, nicht Teil des
Lean-Emit-Durchlaufs): `model/elfmain.sail` (373), `model/main.sail` (432),
`model/devices.sail` (268, Alternativ-`DEVICES`), `translate_address.isail` (17).

Größte Einzelrisiken: `aarch32.sail` (37,7 %) und `aarch64.sail` (24,3 %)
zusammen **62 %** aller Zeilen — beide sind Extra-Split-Kandidaten (s. §2).

## 2. Shard-Einteilung (Lane-11-Regeln)

Shard-Grenzen aus `file_start/file_end` + Extra-Splits pro Instr-Datei
(`lanes/11-sharding-incremental.md`); Fingerprints pro Shard, bounded Queue
(8 Shards inflight), Parallel-Scheduler über Shards (Owner: Lane 11).

| Shard | Dateien | Zeilen ca. | Bemerkung |
|-------|---------|------------|-----------|
| S0-base | `prelude` + `no_devices` + `aarch_types` | ~1 200 | winzig, ein Shard |
| S1-mem | `aarch_mem` | ~11 500 | allein |
| S2-a64 | `aarch64` | ~30 200 | Extra-Split-Kandidat (z. B. Hälften nach Top-Level-Defs), exakte Schnittpunkte per `file_start/file_end`-Scan beim Lauf |
| S3-fpvec | `aarch64_float` + `aarch64_vector` | ~12 000 | zusammen |
| S4-a32 | `aarch32` | ~46 900 | größter Shard; Extra-Split-Kandidat Nr. 1 (instrs32-Hälften), exakte Schnitte beim Lauf |
| S5-decode | `aarch_decode` | ~21 500 | allein |

Reihenfolge = `sail.json`-Ordnung (S0→S5); kein Shard überlappt.
Fällt ein Shard aus dem Budget (§4), wird **nicht hier** repariert, sondern
per Fail-Meldung an die Owner-Lane zurückgemeldet (Lane 11 Sharding,
Lane 07 Rewrites, Lane 09 Lowering).

## 3. Phasen (Messpunkte, Lane-14-`PhaseRecord`)

Pro Shard × Phase je ein JSONL-Record (`perf/timings.jsonl`-Format):

`parse` → `resolve` → `typecheck` → `lean-emit`
(Phasennamen wie in `perf/BUDGETS.md`, Abschnitt „Offen“).

Messkette: `perf/measure.sh` (GNU-`time -v` bevorzugt, Fallbacks eingebaut)
für Einzelprozesse; `perf/measure-armv85.sh` (dieser Lane, `/proc`-Polling
von `VmHWM`, s. §5) für pipelines/mehrstufige Treiber, bei denen kein
einzelnes Kind den Peak trägt. Beide schreiben dasselbe
`{"phase","wall_ms","peak_rss_kb","note"}`-Format (`sail_perf::PhaseRecord`,
lesbar via `read_records`).

Namensschema: `<shard>-<phase>`, z. B. `s4-a32-parse`, …,
`note`: `model=arm-v8.5-a shard=S4-a32 host=<arch>`.

## 4. Budgettabelle (<16 GiB, Lane-14-Format)

Verbindliches Limit aus `crates/sail-perf/src/lib.rs`
(`BUDGET_ARM_V85A_GB` = 16, `BUDGETS`-Array, `check_budget()`):

| Modell | Limit | Limit (KiB) | Status |
|--------|-------|-------------|--------|
| `arm-v8.5-a` | 16 GiB | 16 777 216 | ⏳ Messung ausstehend (E3-Moratorium; misst Integrator) |

Geplante Messmatrix (24 Records: 6 Shards × 4 Phasen), jeweils Peak-RSS
gegen dieselben 16 777 216 KiB zu prüfen (`check_budget()`-Semantik:
`peak <= limit` ✅, sonst ❌ + Eskalation):

| Shard | parse | resolve | typecheck | lean-emit |
|-------|-------|---------|-----------|-----------|
| S0-base | ⏳ | ⏳ | ⏳ | ⏳ |
| S1-mem | ⏳ | ⏳ | ⏳ | ⏳ |
| S2-a64 | ⏳ | ⏳ | ⏳ | ⏳ |
| S3-fpvec | ⏳ | ⏳ | ⏳ | ⏳ |
| S4-a32 | ⏳ | ⏳ | ⏳ | ⏳ |
| S5-decode | ⏳ | ⏳ | ⏳ | ⏳ |

**Bei Budgetriss:** Shard verkleinern (Extra-Split, Owner Lane 11) oder Pass
in Lane 07/09 zurückweisen — Wortlaut wie `check_budget()`-Fehlertext;
Fail-Liste mit je max. 1 Owner an die Owner-Lane zurückmelden (Lane-46-Spec).
Danach Peaks in `perf/BUDGETS.md`-Tabelle („Gemessene Peaks“) + `perf/timings.jsonl`
nachtragen und Status oben auf ✅/❌ setzen.

## 5. Messgerüst (`perf/measure-armv85.sh`, NICHT laufen lassen)

`/proc`-Polling-Sampler (E3-konform: nur Skript + Doku, hier kein Lauf):

```sh
SAIL_PERF_NOTE="model=arm-v8.5-a shard=S4-a32 host=$(uname -m)" \
  bash perf/measure-armv85.sh --phase s4-a32-parse --log perf/timings.jsonl \
    -- sail-cli --shard S4-a32 --phase parse -- <modelldateien>
```

Details siehe Skript-Header. Nach dem Lauf (Integrator):
`peak_of`/`check_budget()`-Auswertung pro Record, Protokoll in
`perf/BUDGETS.md`-Format (§4-Tabelle mit ✅/❌ füllen).

## 6. Reproduzieren (Integrator, später, mit Memory-Limits)

```sh
# Je Shard x Phase (Schema; konkrete Treiber-Flags folgen aus Lane 38/45):
SAIL_PERF_NOTE="model=arm-v8.5-a shard=<S> host=$(uname -m)" \
  bash perf/measure-armv85.sh --phase <S>-<phase> --log perf/timings.jsonl -- <cmd>
# Alternativ pro Einzelprozess:
SAIL_PERF_NOTE="model=arm-v8.5-a shard=<S> host=$(uname -m)" \
  bash perf/measure.sh --phase <S>-<phase> --log perf/timings.jsonl -- <cmd>
```
