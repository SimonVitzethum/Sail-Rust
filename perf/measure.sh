#!/usr/bin/env bash
# Lane 14 — Peak-RSS-Messung pro Phase.
# Wrapper um GNU `/usr/bin/time -v` mit Fallbacks:
#   1. /usr/bin/time -v (bevorzugt, exakte Peak-RSS)
#   2. python3 resource.getrusage(RUSAGE_CHILDREN) (Peak-RSS des Kindprozesses)
#   3. wall-clock only + Warnung (kein RSS)
#
# Nutzung:
#   perf/measure.sh --phase parse --log perf/timings.jsonl -- <cmd> [args...]
#   perf/measure.sh --phase parse -- <cmd>            # nur stderr-Report, kein Log
#
# Das Timings-Log ist JSONL (`-dprofile`-Äquivalent), eine Zeile pro Phase:
#   {"phase":"parse","wall_ms":12,"peak_rss_kb":3456,"note":"..."}
set -euo pipefail

PHASE=""
LOG=""
NOTE="${SAIL_PERF_NOTE:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --phase) PHASE="$2"; shift 2 ;;
    --log) LOG="$2"; shift 2 ;;
    --note) NOTE="$2"; shift 2 ;;
    -h|--help) sed -n '1,20p' "$0"; exit 0 ;;
    --) shift; break ;;
    *) echo "measure.sh: unknown arg: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$PHASE" ]]; then echo "measure.sh: missing --phase" >&2; exit 2; fi
if [[ $# -eq 0 ]]; then echo "measure.sh: missing command after --" >&2; exit 2; fi

# type -P liefert nur echte Pfade (kein bash-keyword `time`).
TIME_BIN=""
if [[ -x /usr/bin/time ]]; then TIME_BIN=/usr/bin/time; else
  P=$(type -P time 2>/dev/null || true)
  if [[ -n "$P" && -x "$P" ]]; then TIME_BIN="$P"; fi
fi

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT
ERR_FILE="$TMP_DIR/stderr.txt"
NUM_FILE="$TMP_DIR/nums.txt"

WALL_MS=0
RSS_KB=0
CHILD_RC=0

set +e
if [[ -n "$TIME_BIN" ]]; then
  START_MS=$(date +%s%3N)
  "$TIME_BIN" -v "$@" 2>"$ERR_FILE"
  CHILD_RC=$?
  END_MS=$(date +%s%3N)
  WALL_MS=$((END_MS - START_MS))
  RSS_KB=$(grep "Maximum resident set size" "$ERR_FILE" | awk '{print $6}' | head -n1)
  RSS_KB=${RSS_KB:-0}
  cat "$ERR_FILE" >&2
elif command -v python3 >/dev/null 2>&1; then
  python3 - "$NUM_FILE" "$@" <<'PYEOF'
import subprocess, sys, time, resource
out = sys.argv[1]; cmd = sys.argv[2:]
t0 = time.monotonic()
p = subprocess.run(cmd)
wall_ms = int(round((time.monotonic() - t0) * 1000))
rss_kb = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss  # Linux: KiB
with open(out, "w") as f:
    f.write("%d %d %d\n" % (wall_ms, rss_kb, p.returncode))
PYEOF
  CHILD_RC=$?
  if [[ -f "$NUM_FILE" ]]; then
    read -r WALL_MS RSS_KB MEASURED_RC < "$NUM_FILE"
    CHILD_RC=${MEASURED_RC:-$CHILD_RC}
  fi
else
  START_MS=$(date +%s%3N)
  "$@"
  CHILD_RC=$?
  END_MS=$(date +%s%3N)
  WALL_MS=$((END_MS - START_MS))
  echo "measure.sh: WARN: no GNU time and no python3 — RSS unknown (0)" >&2
fi
set -e

# JSON-Escaping ohne Dependencies (Phase/Note sind einfache Strings).
json_escape() {
  local s="$1"
  s=${s//\\/\\\\}
  s=${s//\"/\\\"}
  s=${s//$'\n'/\\n}
  s=${s//$'\t'/\\t}
  printf '"%s"' "$s"
}

REC=$(printf '{"phase":%s,"wall_ms":%s,"peak_rss_kb":%s,"note":%s}' \
  "$(json_escape "$PHASE")" "${WALL_MS:-0}" "${RSS_KB:-0}" "$(json_escape "$NOTE")")

if [[ -n "$LOG" ]]; then
  mkdir -p "$(dirname "$LOG")"
  printf '%s\n' "$REC" >> "$LOG"
  echo "measure.sh: $REC -> $LOG" >&2
else
  echo "measure.sh: $REC" >&2
fi

exit "$CHILD_RC"
