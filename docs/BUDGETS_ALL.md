# BUDGETS_ALL — konsolidierte Peak-Tabelle (Lane 55)

Rein lesend konsolidiert (E3: keine Messläufe, keine Builds).
Quellen: `perf/BUDGETS.md` + `perf/timings.jsonl` (Lane 14),
`crates/sail-sysregs-shard/SHARD_REPORT.md` (Lane 31),
`crates/sail-arena/ARENA_REPORT.md` (Lane 37, Stand Main-Repo —
im Lane-55-Worktree noch nicht gemergt, nur lesend referenziert),
`CORE_REPORT.md` (Lane 33),
`crates/sail-sve-sme/FAILURES_SVE_SME.md` (Lane 32),
Plan-Dateien `lanes/11/14/31/32/33/37`.
Alle Worktree-Kopien von `perf/BUDGETS.md` / `CORE_REPORT.md`
(Lanes 30, 41–54, 56–58) sind byte-identisch zum geprüften Stand.

Status: `ok` = gemessen und im Budget, `über` = Budget gerissen,
`ausstehend` = noch nicht gemessen.

## 1. Verbindliche Modell-Budgets (Lane 14, `sail-perf`: `BUDGETS`)

| Modell | Phase | Peak | Methode | Status |
|---|---|---|---|---|
| `aarch64_small` | alle (parse/resolve/typecheck/lean-emit) | Limit 4 GiB (4 194 304 KiB) | `check_budget()` | ausstehend (Lane-02/05-Pipeline nötig) |
| `arm-v8.5-a` | alle | Limit 16 GiB (16 777 216 KiB) | `check_budget()` | ausstehend |
| `arm-v9.4-a` | alle | Limit 24 GiB (25 165 824 KiB) | `check_budget()` | ausstehend |

Zielmaschine: 32 GiB RAM ohne Swap. Bei Überschreitung: Shard
verkleinern oder Pass in Lane 07/09 zurückweisen.

## 2. Gemessene Peaks (Infrastruktur/Shard-Ebene, kein Modell-Nachweis)

| Modell | Phase | Peak | Methode | Status |
|---|---|---|---|---|
| — (Fixture `tiny.sail`) | fixture-parse | 12 304 KiB | `perf/measure.sh`, Fallback `resource.getrusage` (kein `/usr/bin/time` auf Host) | ok (0,3 % des kleinsten Budgets; nur Ketten-Nachweis) |
| — (Fixture `tiny.sail`) | fixture-hash | 12 320 KiB | wie oben | ok (wie oben) |
| — (Fixture `tiny.sail`) | fixture-sleep010 | 12 452 KiB | wie oben | ok (wie oben) |
| `arm-v9.4-a`-Shard `sysregs_autogen.sail` (Lane 31) | shard-scan, Standard (`defs_per_block=64`, 88 Blöcke) | **3 108 KiB (3,04 MiB)** | In-Prozess-Peak `VmHWM` aus `/proc/self/status`, 2026-10-10 | ok |
| wie oben | shard-scan, Variante N=16 (352 Blöcke) | 2 872 KiB (2,80 MiB) | wie oben | ok |
| wie oben | shard-scan, Variante N=256 (22 Blöcke) | 3 520 KiB (3,44 MiB) | wie oben | ok |
| — (Fixture `phased-temps`, Lane 37) | arena vorher (1 Mono-Arena) | 11 664 KiB `VmHWM` / 200 000 Live-Einträge (6,4 MB) | `VmHWM` + deterministische Live-Zählung, `--release` | ok (Vorher-Wert, Regressionstest `tuned_peak_is_below_plain_peak` grün) |
| — (Fixture `phased-temps`, Lane 37) | arena nachher (`ShardArena`, Drop pro Phase) | 3 380 KiB `VmHWM` / 20 800 Live-Einträge (−89,6 %) | wie oben | ok |

## 3. Struktur-Maxima ohne RAM-Peak (Status: ausstehend für Budget-Check)

| Modell | Phase | Peak | Methode | Status |
|---|---|---|---|---|
| `arm-v9.4-a`-Kern (Lane 33: `instrs32`/`instrs64`/`v8_base`) | scan (kein Full-Build) | größte Lean-Funktion `__DecodeA64_SVE` 1 161 Arme; größte Einzel-Def `FPExpCoefficient_read` 500 Zeilen / 163 Arme | Streaming-Scan + Kreuzprüfung | ausstehend (kein RSS gemessen) |
| `arm-v9.4-a`-Shards `instrs64_sve` + `instrs64_sme` (Lane 32) | Katalog (kein Build) | 138 094 Zeilen, 5 328 `function`-Defs; 2 738 `foreach_inc`-Sites | read-only `grep -w` (E3) | ausstehend (kein RSS gemessen) |

## 4. Lücken (ausstehend)

- Echte Peak-RSS pro Modell × Phase (parse/resolve/typecheck/lean-emit):
  3 Modelle × 4 Phasen = 12 Messungen ausstehend, sobald die
  Lane-02/05/07/09-Pipeline terminiert (§Offen in `perf/BUDGETS.md`).
- Kein einziger `über`-Fall verzeichnet.
