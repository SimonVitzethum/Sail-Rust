# Perf-Budgets (Lane 14)

Zielmaschine: **32 GiB RAM ohne Swap**. Verifiziert auf dieser Maschine:

```
MemTotal:  32570984 kB  (~31 GiB)
SwapTotal:        0 kB
```

## Budgets (Peak-RSS pro Modell, verbindlich)

| Modell          | Limit | Limit (KiB) | Status |
|-----------------|-------|-------------|--------|
| `aarch64_small` | 4 GiB | 4 194 304   | ⏳ Messung ausstehend (Lane 02/05-Pipeline nötig) |
| `arm-v8.5-a`    | 16 GiB| 16 777 216  | ⏳ Messung ausstehend |
| `arm-v9.4-a`    | 24 GiB| 25 165 824  | ⏳ Messung ausstehend |

Konstanten: `BUDGET_AARCH64_SMALL_GB` / `BUDGET_ARM_V85A_GB` / `BUDGET_ARM_V94A_GB`
in `crates/sail-perf/src/lib.rs` (`BUDGETS`-Array, `check_budget()`).

**Bei Überschreitung:** Shard verkleinern oder Pass in Lane 07/09 zurückweisen
(`check_budget()` gibt genau diese Anweisung im Fehlertext zurück).

## Messmethode

Wrapper: `perf/measure.sh --phase <phase> --log <file> -- <cmd> [args...]`

1. Bevorzugt GNU `/usr/bin/time -v` (exakte Peak-RSS, `Maximum resident set size`).
   Auf Hosts ohne GNU time (z. B. hier: kein `/usr/bin/time` installiert):
2. Fallback `python3` + `resource.getrusage(RUSAGE_CHILDREN).ru_maxrss` (Linux: KiB).
3. Letztes Fallback: nur Wall-Clock, RSS=0 + Warnung.

Der Wrapper lässt Kind-Stdout unangetastet und propagiert den Exit-Status.
Parser für GNU-`time -v`-Stderr: `sail_perf::parse_time_v_output`
(inkl. `Elapsed`-Formate `M:SS.cc` / `H:MM:SS`, Unit-Tests mit echten Samples).

## Timings-Log (`-dprofile`-Äquivalent)

JSONL, eine Zeile pro Phase (`perf/timings.jsonl`):

```json
{"phase":"fixture-parse","wall_ms":1,"peak_rss_kb":12304,"note":"host=x86_64 fixture=tiny"}
```

API: `PhaseRecord::new` / `from_report`, `append_record`, `read_records`, `peak_of`.

## Gemessene Peaks (Infrastruktur-Fixture, kein Modell)

`perf/fixtures/tiny.sail` (3-Zeilen-Dummy) — beweist nur, dass Kette
Wrapper → JSONL → Parser funktioniert; **kein Modell-Budget-Nachweis**:

| Phase           | wall_ms | peak_rss_kb | vs. kleinstes Budget (4 GiB) |
|----------------|---------|-------------|------------------------------|
| fixture-parse  | 1       | 12 304      | 0,3 % ✅                      |
| fixture-hash   | 2       | 12 320      | 0,3 % ✅                      |
| fixture-sleep010| 101    | 12 452      | 0,3 % ✅                      |

Log: `perf/timings.jsonl` (valides JSONL, maschinell lesbar via `read_records`).

## Reproduzieren

```sh
cargo test -p sail-perf   # Parser + Log-Roundtrip + Budgets (8 Tests)
SAIL_PERF_NOTE="host=$(uname -m) fixture=tiny" \
  bash perf/measure.sh --phase <phase> --log perf/timings.jsonl -- <cmd>
```

## Offen (Folge-Arbeit nach Lane 02/05/07/09)

- Echte Peaks pro Modell + Phase messen (parse/resolve/typecheck/lean-emit),
  sobald die Pipeline terminiert; Tabelle oben auf ✅/❌ setzen.
- Optional: `bytehound`/`dhat`, Flamegraphs, Shard-Größen-Tuning.
