#!/usr/bin/env bash
# Verify the elaborated gate against clean, opaque and imported proof bodies.
set -euo pipefail
TASK_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LEAN="${SAIL_LEAN_BIN:-$(command -v lean)}"
MEMORY="${SAIL_LEAN_TEST_MEMORY:-2G}"
mkdir -p "$TASK_ROOT/target/no-sorry-test"
OUT="$(mktemp -d "$TASK_ROOT/target/no-sorry-test/run.XXXXXXXX")"
SYSROOT="$(systemd-run --user --wait --pipe --collect \
    -p "MemoryMax=$MEMORY" -p MemorySwapMax=0 "$LEAN" --print-prefix)"
cat > "$OUT/Good.lean" <<'LEAN'
theorem good : True := True.intro
LEAN
cat > "$OUT/Bad.lean" <<'LEAN'
set_option warningAsError false
opaque bad : False := by sorry
LEAN
cat > "$OUT/Transitive.lean" <<'LEAN'
import Bad
def marker : Nat := 1
LEAN
for MODULE in Good Bad Transitive; do
    # Lean accepts Bad.olean; the separate gate must still reject it.
    systemd-run --user --wait --pipe --collect \
        -p "MemoryMax=$MEMORY" -p MemorySwapMax=0 -p "WorkingDirectory=$OUT" \
        -E "LEAN_PATH=$OUT" -E "LEAN_SYSROOT=$SYSROOT" \
        "$LEAN" -o "$MODULE.olean" "$MODULE.lean"
    printf '%s\n' "$MODULE" > "$OUT/modules.txt"
    STATUS=0
    systemd-run --user --wait --pipe --collect \
        -p "MemoryMax=$MEMORY" -p MemorySwapMax=0 -p "WorkingDirectory=$OUT" \
        -E "LEAN_PATH=$OUT" -E "LEAN_SYSROOT=$SYSROOT" \
        "$LEAN" --run "$TASK_ROOT/scripts/CheckLeanNoSorry.lean" "$OUT/modules.txt" \
        > "$OUT/$MODULE.log" 2>&1 || STATUS=$?
    cat "$OUT/$MODULE.log"
    if [ "$MODULE" = Good ]; then
        [ "$STATUS" -eq 0 ]
        rg -q 'Lean no-sorry check passed' "$OUT/$MODULE.log"
    else
        [ "$STATUS" -eq 1 ]
        rg -q 'Lean proof hole: bad' "$OUT/$MODULE.log"
    fi
done
printf 'Elaborated no-sorry gate verified under %s without swap: %s\n' "$MEMORY" "$OUT"
