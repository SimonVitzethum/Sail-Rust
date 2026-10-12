#!/usr/bin/env bash
# Lane-01 OOM-Repro-Messung (nur messen, kein src-Change).
# Baut das kanonische Testprogramm unter 4G und fuehrt danach EINEN
# benannten Test unter 128MiB ohne Swap aus. Der cgroup-Peak steht in der
# "Memory peak"-Zeile der systemd-run-Abschlussmeldung.
# Aufruf: scripts/lane-01-oom-repro.sh <sourcedir> <testfilter> <threads> <logfile>
set -u
SRC_DIR="${1:?sourcedir fehlt}"
FILTER="${2:?testfilter fehlt}"
THREADS="${3:?threads fehlt}"
LOGFILE="${4:?logfile fehlt}"
JOBS="${SAIL_BUILD_JOBS:-2}"
echo "### build --no-run (4G) in $SRC_DIR"
systemd-run --user --wait --pipe --collect \
  -p MemoryMax=4G -p MemorySwapMax=0 -p "WorkingDirectory=$SRC_DIR" \
  cargo test -p sail-ty --test canonical_program --no-run --profile ci -j "$JOBS"
echo "### single test run (128M, threads=$THREADS, filter=$FILTER)"
systemd-run --user --wait --pipe --collect \
  -p MemoryMax=128M -p MemorySwapMax=0 -p "WorkingDirectory=$SRC_DIR" \
  cargo test -p sail-ty --test canonical_program "$FILTER" \
  --profile ci -j "$JOBS" -- --nocapture --test-threads="$THREADS"
