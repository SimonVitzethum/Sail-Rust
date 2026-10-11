#!/usr/bin/env bash
# Acceptance gate: real ARMv9.4 Sail -> Lean -> lake build, never stub success.
# SAIL_ARM_MEMORY=28G is the permitted fallback; default is the 16 GiB target.
set -euo pipefail
TASK_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODEL="${SAIL_ARM_MODEL:-$TASK_ROOT/../sail-arm/arm-v9.4-a}"
MEMORY="${SAIL_ARM_MEMORY:-16G}"
case "$MEMORY" in
    16G|28G) ;;
    *) echo 'SAIL_ARM_MEMORY must be 16G or 28G' >&2; exit 1 ;;
esac
OUT_ROOT="${SAIL_ARM_OUTPUT:-$TASK_ROOT/target/arm-lean-gate}"
MODEL="$(realpath "$MODEL")"
mkdir -p "$OUT_ROOT"
# Never accept a stale project from an earlier or failed compiler run.
OUT="$(mktemp -d "$OUT_ROOT/run.XXXXXXXX")"
OUT="$(realpath "$OUT")"
cd "$TASK_ROOT"
systemd-run --user --wait --pipe --collect \
    -p MemoryMax=4G -p MemorySwapMax=0 -p "WorkingDirectory=$TASK_ROOT" \
    "$(command -v cargo)" build --profile ci -j 2 -p sail-cli
# Read the canonical source list from the model's Makefile, including its
# device selection. Avoid maintaining a second, drifting list of ARM inputs.
make --no-print-directory -s -C "$MODEL" -f Makefile -f - \
    SAIL_DIR="$TASK_ROOT/../sail" sail_rs_source_manifest > "$OUT/sources.txt" <<'MAKE'
.PHONY: sail_rs_source_manifest
sail_rs_source_manifest:
	@printf '%s\n' $(SAIL_SRCS) src/termination_measures.sail
MAKE
mapfile -t SOURCES < "$OUT/sources.txt"
if [ "${#SOURCES[@]}" -eq 0 ]; then
    echo 'ARM source manifest is empty' >&2
    exit 1
fi
printf 'ARM gate: model=%s memory=%s sources=%s output=%s\n' \
    "$MODEL" "$MEMORY" "${#SOURCES[@]}" "$OUT"
systemd-run --user --wait --pipe --collect \
    -p "MemoryMax=$MEMORY" -p MemorySwapMax=0 -p "WorkingDirectory=$MODEL" \
    "$TASK_ROOT/target/ci/sail-rs" -dprofile -verbose 1 -memo_z3 \
    -lean -lean-force-output -lean_output_dir "$OUT/lean" -o armv9 \
    -undefined_gen -lean-noncomputable -lean-line-width 100000 \
    -lean-import-file lean/ArmExtras.lean "${SOURCES[@]}" \
    2>&1 | tee "$OUT/compiler.log"
if rg -n 'frontend stub|Full Lean emission lands|only proves the.*CLI' "$OUT/lean"; then
    echo 'ARM gate failed: compiler emitted a frontend stub; no full model was compiled.' >&2
    exit 1
fi
if [ ! -f "$OUT/lean/lakefile.toml" ] && [ ! -f "$OUT/lean/lakefile.lean" ]; then
    echo 'ARM gate failed: generated Lean project has no lakefile.' >&2
    exit 1
fi
systemd-run --user --wait --pipe --collect \
    -p "MemoryMax=$MEMORY" -p MemorySwapMax=0 -p "WorkingDirectory=$OUT/lean" \
    "$(command -v lake)" build 2>&1 | tee "$OUT/lake.log"
printf 'ARM gate passed: full compiler output and lake build completed under %s.\n' "$MEMORY"
