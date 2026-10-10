#!/usr/bin/env bash
# Lane 46 — Peak-RSS-Messung pro Phase fuer den arm-v8.5-a-Durchlauf.
#
# /proc-Polling-Sampler: startet <cmd>, fragt waehrend der Laufzeit
# periodisch /proc/<pid>/status (VmHWM = Peak-RSS des Prozesses) ab und
# schreibt einen JSONL-Record im Lane-14-Format
# ({"phase","wall_ms","peak_rss_kb","note"}, vgl. sail_perf::PhaseRecord,
#  lesbar via read_records; siehe perf/BUDGETS.md "Timings-Log").
#
# Warum ein zweiter Wrapper neben perf/measure.sh?
# measure.sh misst exakt (GNU time -v / getrusage) den Peak EINES Kind-
# prozesses. Treibt ein Treiber mehrstufige Pipelines (ein Kind terminiert,
# naechstes startet), unterschaetzt ein einzelnes VmHWM den Gesamt-Peak.
# Dieser Sampler pollt deshalb den Prozessbaum (Kind + Nachkommen) und
# nimmt das Maximum ueber alle Samples und Prozesse.
#
# Nutzung (NICHT im Lane-Kontext laufen lassen — E3-Test-Moratorium;
# Lauf nur zentral durch den Integrator mit Memory-Limits):
#   SAIL_PERF_NOTE="model=arm-v8.5-a shard=S4-a32 host=$(uname -m)" \
#     bash perf/measure-armv85.sh --phase s4-a32-parse --log perf/timings.jsonl \
#       -- <cmd> [args...]
#   bash perf/measure-armv85.sh --phase <S>-<phase> -- <cmd>  # ohne Log
#
# Optionen:
#   --phase NAME   Phasenname, Schema <shard>-<phase>, z. B. s4-a32-parse (Pflicht)
#   --log FILE     JSONL-Log (angehaengt, mkdir -p); ohne --log nur stderr-Report
#   --note TEXT    ueberschreibt SAIL_PERF_NOTE
#   --interval SEC Poll-Intervall in Sekunden (Default 0.05)
#   -h|--help      Kurz-Hilfe
#
# Hinweise:
# - Erfordert Linux-/proc (VmHWM). Ohne /proc: Fehlerstatus 2, kein Record.
# - Der Sampler selbst erzeugt keine Last: nur ein sleep + wenige
#   /proc-Reads pro Intervall; gemessen wird AUSSCHLIESSLICH <cmd>.
# - Kind-Stdout/Stderr bleiben unangetastet; Exit-Status wird propagiert.
# - peak_rss_kb ist das Maximum aller VmHWM-Samples ueber Kind + Nachkommen
#   (Summe waere Doppelzaehlung bei Fork ohne exec — bewusst Maximum).
set -euo pipefail

PHASE=""
LOG=""
NOTE="${SAIL_PERF_NOTE:-}"
INTERVAL="0.05"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --phase) PHASE="$2"; shift 2 ;;
    --log) LOG="$2"; shift 2 ;;
    --note) NOTE="$2"; shift 2 ;;
    --interval) INTERVAL="$2"; shift 2 ;;
    -h|--help) sed -n '1,35p' "$0"; exit 0 ;;
    --) shift; break ;;
    *) echo "measure-armv85.sh: unknown arg: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$PHASE" ]]; then echo "measure-armv85.sh: missing --phase" >&2; exit 2; fi
if [[ $# -eq 0 ]]; then echo "measure-armv85.sh: missing command after --" >&2; exit 2; fi
if [[ ! -r /proc/self/status ]]; then
  echo "measure-armv85.sh: /proc not available — cannot sample VmHWM" >&2; exit 2
fi

# VmHWM (kB) eines PIDs, 0 bei fehlendem/unlesbarem Eintrag (Prozess schon weg).
vmhwm_of() {
  local pid="$1" v
  v=$(awk '/^VmHWM:/ {print $2; exit}' "/proc/$pid/status" 2>/dev/null || true)
  [[ "$v" =~ ^[0-9]+$ ]] && printf '%s' "$v" || printf '0'
}

# Alle Nachkommen-PIDs (rekursiv) ueber /proc-Scan (PPid-Abgleich).
descendants_of() {
  local root="$1" pid ppid
  for d in /proc/[0-9]*; do
    pid="${d#/proc/}"
    ppid=$(awk '/^PPid:/ {print $2; exit}' "$d/status" 2>/dev/null || true)
    if [[ "$ppid" == "$root" ]]; then
      printf '%s\n' "$pid"
      descendants_of "$pid"
    fi
  done
}

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT
PEAK_FILE="$TMP_DIR/peak.txt"
echo 0 > "$PEAK_FILE"

START_MS=$(date +%s%3N)

set +e
"$@" &
CHILD=$!
CHILD_RC=0

# Poll-Schleife: Maximum von VmHWM ueber Kind + lebende Nachkommen.
while kill -0 "$CHILD" 2>/dev/null; do
  cur_peak=$(cat "$PEAK_FILE")
  sample=$(vmhwm_of "$CHILD")
  while IFS= read -r desc; do
    [[ -z "$desc" ]] && continue
    v=$(vmhwm_of "$desc")
    if (( v > sample )); then sample=$v; fi
  done < <(descendants_of "$CHILD")
  if (( sample > cur_peak )); then echo "$sample" > "$PEAK_FILE"; fi
  sleep "$INTERVAL"
done
wait "$CHILD"
CHILD_RC=$?
set -e

END_MS=$(date +%s%3N)
WALL_MS=$((END_MS - START_MS))
RSS_KB=$(cat "$PEAK_FILE")

# Ein letzter Sample nach wait: VmHWM des (ggf. Zombie-)Kindes mitnehmen.
last=$(vmhwm_of "$CHILD")
if (( last > RSS_KB )); then RSS_KB=$last; fi

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
  echo "measure-armv85.sh: $REC -> $LOG" >&2
else
  echo "measure-armv85.sh: $REC" >&2
fi

exit "$CHILD_RC"
