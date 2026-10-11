#!/usr/bin/env bash
# Build once under a systemd limit, then run each test binary under a tighter
# limit. Arguments are forwarded to cargo test (e.g. -p sail-rewrite).
# Does not include doctests; use the workspace cargo test gate for those.
set -euo pipefail
cd "$(dirname "$0")/.."
TASK_DIR="$(pwd)"
BUILD_MEMORY="${SAIL_BUILD_MEMORY:-4G}"
TEST_MEMORY="${SAIL_TEST_MEMORY:-128M}"
BUILD_JOBS="${SAIL_BUILD_JOBS:-2}"
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
    systemd-run --user --wait --pipe --collect \
        -p "MemoryMax=$TEST_MEMORY" -p MemorySwapMax=0 -p RuntimeMaxSec=180 \
        -p "WorkingDirectory=$TASK_DIR" \
        "$binary" --test-threads=4 || STATUS=1
done < "$BINARIES"
exit "$STATUS"
