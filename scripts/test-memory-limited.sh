#!/usr/bin/env bash
# Build once under a systemd limit, then run each test binary sequentially
# under a tighter limit. Arguments are forwarded to cargo test (e.g. -p sail-rewrite).
# Does not include doctests; use the workspace cargo test gate for those.
#
# Lane-24 (O-10, 2026-10-12): 128M-Suite-Flakiness ist Test-Parallelitaet, kein Leak.
#   - Tiefentest solo (1 Test): Peak ~93.6M, ok (threads=1 und threads=4).
#   - 39-Test-Binaerdatei (canonical_program) sequenziell (--test-threads=1): Peak ~95.5M, gruen.
#   - gleiche Binaerdatei mit --test-threads=4 unter 128M: flaky (~126.6M ok vs. OOM-Kill).
# Diagnose statt Limiterhoehung: erst betroffene Binaerdatei / betroffenen Test
# einzeln unter dem Originallimit messen. Sequenzieller Re-Run:
#   SAIL_TEST_THREADS=1 scripts/test-memory-limited.sh <args>
# Optionaler Retry (nur Diagnose, Standard 0 = aus):
#   SAIL_TEST_RETRIES=1 scripts/test-memory-limited.sh <args>
# Ein Retry wiederholt fehlgeschlagene Binaerdateien einmal mit --test-threads=1.
#
# Lane-38 (Suite-Stabilitaet, 2026-10-13): Default SAIL_TEST_THREADS 4 -> 2.
# Messreihe canonical_program-Binaerdatei (35 Tests, Basis a972e3a) unter 128M,
# je 3x via Gate (Logs: evidence/2026-10-13/lane-38/canonical-threads{1,2,4}-try{1,2,3}):
#   - t1: 53.0M / 53.0M / 52.4M, je 35/35 ok (~0.6 s).
#   - t2: 75.1M / 65.6M / 74.5M, je 35/35 ok (~0.35 s), max 75.1M (~53M Abstand zur Decke).
#   - t4: 103.0M / 102.3M / 102.8M, je 35/35 ok (~0.20 s), nur ~25M Abstand zur
#     Decke; mit schwererem 39-Test-Inhalt kippelig (~126.6M ok vs. OOM-Kill).
# Entscheidung (a): Default 2 — Peak -27% gegenueber t4 bei 1.7x Tempo gegenueber
# t1, Limit bleibt 128M, Override via SAIL_TEST_THREADS erhalten.
# Verworfen (b) Binaerdatei-Split: braucht crates/-Aenderungen (Auftrag verbietet sie).
# Verworfen (c) 128M -> 160M: keine Budget-Doku-Rechtfertigung (OOC-Byte-Budget 32M
# vs. 128M-Gate-Workload betrifft Streaming-Fenster, nicht Testthreads) und
# verstoesst gegen TESTING.md ("erst messen, nicht erhoehen").
# SAIL_TEST_THREADS/SAIL_TEST_RETRIES sind additiv; uebrige Defaults (128M/4G,
# Jobs 2, Retries 0, gleiche Default-Pakete) erhalten das bisherige Verhalten.
set -euo pipefail
cd "$(dirname "$0")/.."
TASK_DIR="$(pwd)"
BUILD_MEMORY="${SAIL_BUILD_MEMORY:-4G}"
TEST_MEMORY="${SAIL_TEST_MEMORY:-128M}"
BUILD_JOBS="${SAIL_BUILD_JOBS:-2}"
TEST_THREADS="${SAIL_TEST_THREADS:-2}"
TEST_RETRIES="${SAIL_TEST_RETRIES:-0}"
MANIFEST="$(mktemp)"
BINARIES="$(mktemp)"
trap 'rm -f "$MANIFEST" "$BINARIES"' EXIT
if [ "$#" -eq 0 ]; then
    set -- -p sail-rewrite -p sail-shard -p sail-ooc -p sail-par-pipe
fi
systemd-run --user --wait --pipe --collect \
    -p "MemoryMax=$BUILD_MEMORY" -p MemorySwapMax=0 \
    -p "WorkingDirectory=$TASK_DIR" \
    "$(command -v cargo)" test --no-run --profile ci -j "$BUILD_JOBS" \
    --message-format=json "$@" > "$MANIFEST"
python3 - "$MANIFEST" > "$BINARIES" <<'PY'
import json, sys
binaries = set()
with open(sys.argv[1]) as stream:
    for line in stream:
        item = json.loads(line)
        if item.get('reason') == 'compiler-artifact' and item.get('profile', {}).get('test') and item.get('executable'):
            binaries.add(item['executable'])
for binary in sorted(binaries):
    print(binary)
if not binaries:
    sys.exit('No test binaries found')
PY
STATUS=0
while IFS= read -r binary; do
    ATTEMPT=0
    while true; do
        if [ "$ATTEMPT" -eq 0 ]; then
            THREADS="$TEST_THREADS"
        else
            THREADS=1
        fi
        if systemd-run --user --wait --pipe --collect \
            -p "MemoryMax=$TEST_MEMORY" -p MemorySwapMax=0 -p RuntimeMaxSec=180 \
            -p "WorkingDirectory=$TASK_DIR" \
            "$binary" --test-threads="$THREADS"; then
            break
        fi
        ATTEMPT=$((ATTEMPT + 1))
        if [ "$ATTEMPT" -gt "$TEST_RETRIES" ]; then
            echo "FAIL: $binary (threads=$THREADS; retries exhausted)" >&2
            STATUS=1
            break
        fi
        echo "RETRY: $binary failed with threads=$THREADS; retrying once with --test-threads=1 (parallel-flake diagnosis)" >&2
    done
done < "$BINARIES"
exit "$STATUS"
