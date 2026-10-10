#!/usr/bin/env bash
# limited-run.sh — schwere Befehle unter systemd-MemoryMax laufen lassen.
# Lane 51 (test-rig). Nur Wrapper, startet selbst nichts Schweres.
#
# Aufruf:
#   ./scripts/limited-run.sh [-m MEM] [-l LOG] -- <befehl...>
#
# Optionen:
#   -m MEM   MemoryMax, z.B. 20G (Default, Builder-Profil) oder 10G (Checks).
#   -l LOG   Logdatei (Default: logs/limited-run-<timestamp>-<pid>.log).
#   -h       Hilfe.
#
# Verhalten:
#   - Führt <befehl...> via `systemd-run --user --scope -p MemoryMax=<MEM>` aus.
#   - Spiegelt stdout/stderr per tee in die Logdatei.
#   - Gibt den Exit-Code des Befehls weiter (nicht den von tee).
set -euo pipefail
cd "$(dirname "$0")/.."

MEM="20G"
LOG=""

usage() {
    sed -n '2,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
}

while getopts ":m:l:h" opt; do
    case "$opt" in
        m) MEM="$OPTARG" ;;
        l) LOG="$OPTARG" ;;
        h) usage; exit 0 ;;
        :) echo "FEHLER: -$OPTARG braucht ein Argument" >&2; exit 2 ;;
        \?) echo "FEHLER: unbekannte Option -$OPTARG" >&2; exit 2 ;;
    esac
done
shift $((OPTIND - 1))

if [ "$#" -eq 0 ]; then
    echo "FEHLER: kein Befehl angegeben (erwartet: -- <befehl...>)" >&2
    usage >&2
    exit 2
fi

if ! [[ "$MEM" =~ ^[0-9]+[MG]$ ]]; then
    echo "FEHLER: -m MEM muss Format <Zahl>[M|G] haben (z.B. 20G, 512M), war: $MEM" >&2
    exit 2
fi

if [ -z "$LOG" ]; then
    LOG="logs/limited-run-$(date +%Y%m%d-%H%M%S)-$$.log"
fi
mkdir -p "$(dirname "$LOG")"

echo "limited-run: MemoryMax=$MEM log=$LOG cmd: $*"

# --scope laeuft synchron und liefert den Exit-Code des Befehls.
# PIPESTATUS[0] statt pipefail-Ergebnis, damit tee-Fehler den Code nicht
# verfremden und Befehls-Fehler nicht verloren gehen.
systemd-run --user --scope -p "MemoryMax=${MEM}" "$@" 2>&1 | tee "$LOG"
rc=${PIPESTATUS[0]}
echo "limited-run: exit=$rc log=$LOG" | tee -a "$LOG"
exit "$rc"
