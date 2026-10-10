# Lane 14 — perf-ram-profiling
Branch: `lane/14-perf-ram` | Worktree: `sail-rs/worktrees/lane-14` (nach Merge löschen)

## Ziel
RAM-Budgets beweisen (32GB ohne Swap).

## Aufgaben
- Peak-RSS pro Phase messen (`/usr/bin/time -v`, `bytehound`/`dhat`-optional), Flamegraphs, Shard-Größen-Tuning.
- Budgets: `aarch64_small` <4GB, `arm-v8.5-a` <16GB, `arm-v9.4-a` <24GB; bei Überschreitung Shard verkleinern oder Pass in Lane 07/09 zurückweisen.
- `-dprofile`-Äquivalent als Timings-Log.

## Done
- `perf/BUDGETS.md` mit gemessenen Peaks pro Modell, reproduzierbar.
