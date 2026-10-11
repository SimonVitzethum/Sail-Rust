#!/usr/bin/env bash
# Supported-subset gate, not ARM acceptance or a successful Lean compilation.
# Check substantially more AST data than the memory limit, including bodies.
set -euo pipefail
TASK_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$TASK_ROOT/target/typecheck-ooc-gate"
OUT="$(mktemp -d "$TASK_ROOT/target/typecheck-ooc-gate/run.XXXXXXXX")"
MEMORY="${SAIL_TYPECHECK_MEMORY:-128M}"
WORKERS="${SAIL_RS_THREADS:-8}"
python3 - "$OUT/model.sail" <<'PY'
import sys
with open(sys.argv[1], 'w') as output:
    block = '; '.join(['()'] * 512 + ['x'])
    for index in range(2048):
        output.write(f'val function_{index} : int -> int\n')
        output.write(f'function function_{index}(x) = {{ {block} }}\n')
PY
systemd-run --user --wait --pipe --collect \
    -p MemoryMax=4G -p MemorySwapMax=0 -p "WorkingDirectory=$TASK_ROOT" \
    "$(command -v cargo)" build --profile ci -j 2 -p sail-cli
STATUS=0
systemd-run --user --wait --pipe --collect \
    --setenv="SAIL_RS_THREADS=$WORKERS" \
    -p "MemoryMax=$MEMORY" -p MemorySwapMax=0 -p RuntimeMaxSec=180 \
    -p "WorkingDirectory=$TASK_ROOT" \
    "$TASK_ROOT/target/ci/sail-rs" --no-core --lean -dprofile "$OUT/model.sail" \
    > "$OUT/compiler.log" 2>&1 || STATUS=$?
cat "$OUT/compiler.log"
if [ "$STATUS" -ne 1 ]; then
    echo "Unexpected compiler status $STATUS; expected explicit missing-backend failure." >&2
    exit 1
fi
python3 - "$OUT/compiler.log" "$MEMORY" "$WORKERS" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
expected = f'typechecked 4096 definitions and 1052672 expressions using {sys.argv[3]} workers'
if expected not in text or 'compilation incomplete:' not in text:
    sys.exit('Supported-subset typecheck did not complete.')
match = re.search(r'(\d+) AST bytes spooled', text)
size = re.fullmatch(r'(\d+)([KMG])', sys.argv[2])
if not match or not size:
    sys.exit('Cannot establish AST size versus cgroup limit.')
limit = int(size[1]) * 1024 ** {'K': 1, 'M': 2, 'G': 3}[size[2]]
if int(match[1]) <= limit:
    sys.exit('AST workload does not exceed the memory limit.')
print(f'Subset typecheck gate passed: {int(match[1]):,} AST bytes under {sys.argv[2]}, no swap.')
print('This is not a successful compiler/Lean/ARM build.')
PY
printf 'Retained input and log: %s\n' "$OUT"
