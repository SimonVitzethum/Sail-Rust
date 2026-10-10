#!/usr/bin/env bash
# limited-run.sh — schwere Befehle unter systemd-MemoryMax laufen lassen.
# Lane 51 (test-rig), Lane 92 (rig-hygiene: Warn-Caps/Summary/Rotation).
#
# Aufruf:
#   ./scripts/limited-run.sh [-m MEM] [-l LOG] [-c CAP] [-S|--summary] [--keep N]
#       [--max-mb N] -- <befehl...>
#
# Optionen:
#   -m MEM   MemoryMax, z.B. 20G (Default, Builder-Profil) oder 10G (Checks).
#   -l LOG   Logdatei (Default: logs/limited-run-<timestamp>-<pid>.log).
#   -c CAP   Warn-Cap: max. so viele Warning-Zeilen landen im Output/Log,
#            Rest wird gezaehlt und als eine Summary-Zeile gemeldet
#            (Default: $RIG_WARN_CAP oder 20; 0 = alle unterdruecken).
#   -S, --summary     Summary-Mode: aufeinanderfolgende identische Zeilen
#            werden gekuerzt (erste 2 gezeigt, Rest gezaehlt gemeldet).
#            Auch via RIG_SUMMARY=1. Default: aus.
#   --no-summary      Summary-Mode explizit aus (sticht RIG_SUMMARY=1).
#   --keep N          Log-Rotation: so viele neueste limited-run-*.log im
#            Log-Verzeichnis behalten (Default: $RIG_KEEP oder 10).
#   --max-mb N        Log-Rotation: Gesamtgroesse der limited-run-*.log auf
#            max. N MiB deckeln, Aelteste zuerst loeschen
#            (Default: $RIG_MAX_MB oder 50; 0 = kein Groessen-Deckel).
#   -h, --help        Hilfe.
#
# Verhalten:
#   - Führt <befehl...> via `systemd-run --user --scope -p MemoryMax=<MEM>` aus.
#   - Filtert stdout/stderr erst durch Warn-Cap/Summary (Lane 92), damit keine
#     33MB-Warning-Fluten mehr geloggt werden; spiegelt das Ergebnis per tee
#     in die Logdatei. Der Cap gilt fuer Terminal UND Log (kein Voll-Log).
#   - Gibt den Exit-Code des Befehls weiter (nicht den von tee).
#   - Rotiert vorher alte limited-run-*.log im Log-Verzeichnis (--keep/--max-mb).
set -euo pipefail
cd "$(dirname "$0")/.."

MEM="20G"
LOG=""
WARN_CAP="${RIG_WARN_CAP:-20}"
SUMMARY="${RIG_SUMMARY:-0}"
KEEP="${RIG_KEEP:-10}"
MAX_MB="${RIG_MAX_MB:-50}"

usage() {
    sed -n '2,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        -m|--mem) MEM="${2:?FEHLER: -m braucht MemoryMax}"; shift 2 ;;
        --mem=*) MEM="${1#*=}"; shift ;;
        -m*) MEM="${1#-m}"; shift ;; # -m20G
        -l|--log) LOG="${2:?FEHLER: -l braucht eine Logdatei}"; shift 2 ;;
        --log=*) LOG="${1#*=}"; shift ;;
        -c|--warn-cap) WARN_CAP="${2:?FEHLER: -c braucht eine Zahl}"; shift 2 ;;
        --warn-cap=*) WARN_CAP="${1#*=}"; shift ;;
        -S|--summary) SUMMARY=1; shift ;;
        --no-summary) SUMMARY=0; shift ;;
        --keep) KEEP="${2:?FEHLER: --keep braucht eine Zahl}"; shift 2 ;;
        --keep=*) KEEP="${1#*=}"; shift ;;
        --max-mb) MAX_MB="${2:?FEHLER: --max-mb braucht eine Zahl}"; shift 2 ;;
        --max-mb=*) MAX_MB="${1#*=}"; shift ;;
        -h|--help) usage; exit 0 ;;
        --) shift; break ;;
        -*) echo "FEHLER: unbekannte Option $1" >&2; exit 2 ;;
        *) break ;;
    esac
done
# Restliche Args = Befehl (darf nichts mehr sein -> Fehler unten).

if [ "$#" -eq 0 ]; then
    echo "FEHLER: kein Befehl angegeben (erwartet: -- <befehl...>)" >&2
    usage >&2
    exit 2
fi

if ! [[ "$MEM" =~ ^[0-9]+[MG]$ ]]; then
    echo "FEHLER: -m MEM muss Format <Zahl>[M|G] haben (z.B. 20G, 512M), war: $MEM" >&2
    exit 2
fi

for v in "WARN_CAP $WARN_CAP" "KEEP $KEEP" "MAX_MB $MAX_MB"; do
    name="${v%% *}"; val="${v#* }"
    if ! [[ "$val" =~ ^[0-9]+$ ]]; then
        echo "FEHLER: $name muss eine nicht-negative Zahl sein, war: $val" >&2
        exit 2
    fi
done
if ! [[ "$SUMMARY" =~ ^[01]$ ]]; then
    echo "FEHLER: SUMMARY muss 0/1 sein (RIG_SUMMARY), war: $SUMMARY" >&2
    exit 2
fi

if [ -z "$LOG" ]; then
    LOG="logs/limited-run-$(date +%Y%m%d-%H%M%S)-$$.log"
fi
mkdir -p "$(dirname "$LOG")"

# Log-Rotation (Lane 92): neueste $KEEP limited-run-*.log behalten, Rest
# loeschen; danach Gesamtgroesse auf $MAX_MB MiB deckeln (Aelteste zuerst).
rig_prune_logs() {
    local dir="$1" keep="$2" max_mb="$3" f
    local -a files=()
    [ -d "$dir" ] || return 0
    # Nach mtime neueste zuerst; Namen mit Leerzeichen via TAB-Trennung sicher.
    while IFS=$'\t' read -r _mtime f; do
        files+=("$f")
    done < <(find "$dir" -maxdepth 1 -type f -name 'limited-run-*.log' \
        -printf '%T@\t%p\n' 2>/dev/null | sort -rn)
    local n="${#files[@]}" i=0
    for f in ${files[@]+"${files[@]}"}; do
        i=$((i + 1))
        if [ "$i" -gt "$keep" ]; then
            rm -f -- "$f"
        fi
    done
    if [ "$max_mb" -gt 0 ]; then
        local total total_mb oldest
        while true; do
            total=$(du -cb "$dir"/limited-run-*.log 2>/dev/null | tail -1 | cut -f1 || true)
            [ -z "${total:-}" ] && break
            [ "$total" = "0" ] && break
            total_mb=$((total / 1048576))
            [ "$total_mb" -le "$max_mb" ] && break
            oldest=$(find "$dir" -maxdepth 1 -type f -name 'limited-run-*.log' \
                -printf '%T@\t%p\n' 2>/dev/null | sort -n | head -1 | cut -f2-)
            [ -z "$oldest" ] && break
            # Das gerade gewaehlte Log nie wegrotieren.
            if [ "$oldest" = "$LOG" ]; then
                break
            fi
            rm -f -- "$oldest"
        done
    fi
}
rig_prune_logs "$(dirname "$LOG")" "$KEEP" "$MAX_MB"

echo "limited-run: MemoryMax=$MEM log=$LOG warn_cap=$WARN_CAP summary=$SUMMARY cmd: $*"

# Warn-Cap/Summary-Filter (Lane 92): Zeilen mit 'warning' (case-insensitiv)
# werden auf $WARN_CAP gedeckelt und gezaehlt; im Summary-Mode werden zusaetzlich
# aufeinanderfolgende identische Zeilen gekuerzt. Gilt vor tee, also fuer
# Terminal UND Log — volle Warning-Fluten landen nirgends mehr.
rig_filter() {
    awk -v cap="$WARN_CAP" -v summary="$SUMMARY" '
        /[Ww][Aa][Rr][Nn][Ii][Nn][Gg]/ {
            warn_total++
            if (warn_kept < cap) { print; warn_kept++ }
            else { warn_suppressed++ }
            next
        }
        {
            if (summary == 1) {
                if ($0 == prev) { repeat++; next }
                if (repeat > 0) {
                    printf "... (vorige Zeile %dx wiederholt, gekuerzt)\n", repeat + 1
                    repeat = 0
                }
                prev = $0
            }
            print
        }
        END {
            if (summary == 1 && repeat > 0) {
                printf "... (vorige Zeile %dx wiederholt, gekuerzt)\n", repeat + 1
            }
            if (warn_suppressed > 0) {
                printf "limited-run: %d Warning-Zeilen unterdrueckt (Cap %d, %d gezeigt)\n", \
                    warn_suppressed, cap, warn_kept
            }
        }
    '
}

# --scope laeuft synchron und liefert den Exit-Code des Befehls.
# PIPESTATUS[0] statt pipefail-Ergebnis, damit tee-/Filter-Fehler den Code nicht
# verfremden und Befehls-Fehler nicht verloren gehen.
systemd-run --user --scope -p "MemoryMax=${MEM}" "$@" 2>&1 | rig_filter | tee "$LOG"
rc=${PIPESTATUS[0]}
echo "limited-run: exit=$rc log=$LOG" | tee -a "$LOG"
exit "$rc"
