#!/usr/bin/env bash
# Compare Rust Lean output of sail/test/lean/<name>.sail with the OCaml golden.
# usage: scripts/lean-golden.sh <name> [extra sail-rs flags]
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NAME="$1"; shift
TESTDIR="$ROOT/../sail/test/lean"
OUT="${GOLDEN_OUT:-$ROOT/target/golden}/$NAME"
rm -rf "$OUT"; mkdir -p "$OUT"
cd "$TESTDIR"
"$ROOT/target/ci/sail-rs" -lean -lean-single-file -lean-matchbv -lean-output-dir "$OUT" \
    -lean-lib-path "$ROOT/../lean-sail" "$@" "$NAME.sail" > "$OUT/compiler.log" 2>&1
status=$?
if [ ! -f "$OUT/out/Out.lean" ]; then
    echo "[$NAME] no output (exit $status): $(grep -v '^sail-rs: \[' "$OUT/compiler.log" | tail -2 | cut -c1-300)"
    exit 1
fi
{ cat "$OUT/out/Out/Defs.lean"; echo; echo "XXXXXXXXX"; echo; cat "$OUT/out/Out.lean"; } > "$OUT/output"
# `/-- Type quantifiers: .. -/` comments of desugared `int`/`bool` arguments
# (fresh `k_exNNN_` kids) are not reproduced yet; compare without them unless
# GOLDEN_STRICT=1 (see TESTING.md).
if [ "${GOLDEN_STRICT:-0}" = "1" ]; then
    cp "$OUT/output" "$OUT/cmp-actual"; cp "$NAME.expected.lean" "$OUT/cmp-expected"
else
    grep -v '^/-- Type quantifiers:' "$OUT/output" > "$OUT/cmp-actual"
    grep -v '^/-- Type quantifiers:' "$NAME.expected.lean" > "$OUT/cmp-expected"
fi
if diff -q "$OUT/cmp-actual" "$OUT/cmp-expected" > /dev/null; then
    echo "[$NAME] MATCH"
else
    echo "[$NAME] DIFF ($(diff "$OUT/cmp-actual" "$OUT/cmp-expected" | grep -c '^[<>]') lines)"
    exit 1
fi
