#!/usr/bin/env bash
# tmux-test.sh — Test-Runs in tmux, ein Fenster je Run, begrenzt via limited-run.sh.
# Lane 51 (test-rig), Lane 92 (rig-hygiene: Rotation + Cap-Weitergabe).
#
# Aufruf:
#   ./scripts/tmux-test.sh [-s SESSION] [-w FENSTER] [-m MEM] [-c CAP] [-S|--summary]
#       [--keep N] [--max-mb N] [--dry-run] -- <befehl...>
#   ./scripts/tmux-test.sh [-s SESSION] [--dry-run]     # nur Session mit Shell anlegen
#
# Optionen:
#   -s SESSION  tmux-Session-Name (Default: sail-test).
#   -w FENSTER  Fenster-Name für diesen Run (Default: run-<Zeit>).
#   -m MEM      MemoryMax für limited-run.sh (Default 20G, Checks 10G).
#   -c CAP      Warn-Cap für limited-run.sh (Default: $RIG_WARN_CAP oder 20).
#   -S, --summary   Summary-Mode für limited-run.sh (auch RIG_SUMMARY=1).
#   --no-summary    Summary-Mode explizit aus.
#   --keep N    Log-Rotation: neueste N limited-run-*.log behalten
#               (Default: $RIG_KEEP oder 10). Vor Fenster-Start geprunt.
#   --max-mb N  Log-Rotation: Logs auf max. N MiB deckeln, Aelteste zuerst
#               (Default: $RIG_MAX_MB oder 50; 0 = kein Groessen-Deckel).
#   --dry-run   nur zeigen, was getan wuerde (kein tmux-Aufruf).
#   -h          Hilfe.
#
# Verhalten:
#   - Session existiert noch nicht  -> eigene oneshot-Unit (remain-after-exit) + tmux new-session.
#   - Session existiert bereits      -> tmux new-window.
#   - Der Fenster-Befehl lautet: $SCRIPT_DIR/limited-run.sh -m MEM -c CAP [-S]
#     --keep N --max-mb N -- <befehl...> (ohne <befehl...> öffnet das Fenster
#     nur eine Shell; dann entfaellt limited-run inkl. Rotation/Caps).
#   - Vor jedem Fenster-Start rotiert tmux-test.sh alte limited-run-*.log
#     (Lane 92: --keep/--max-mb bzw. RIG_KEEP/RIG_MAX_MB); limited-run.sh prunt
#     beim Start zusaetzlich selbst, damit Direktaufrufe auch gedeckelt sind.
#   - "$@" wird als Array durchgereicht, nie re-quotet (ein %q-Join hat -c-Skripte
#     zerlegt -> Fenster starben instant, Server ohne Fenster beendet sich).
#
# Reattach / Bedienung (siehe auch TESTING.md):
#   tmux attach -t SESSION        # anhaengen
#   Ctrl-b d                      # abhaengen (Run laeuft weiter)
#   tmux ls                       # Sessions auflisten
#   tmux capture-pane -p -t SESSION:FENSTER   # Fenster-Inhalt ansehen
#   tmux kill-window -t SESSION:FENSTER       # einzelnen Run beenden
#   tmux kill-session -t SESSION              # ganze Session beenden
set -euo pipefail
cd "$(dirname "$0")/.."
SCRIPT_DIR="$(pwd)/scripts"
# HINWEIS: tmux-Server erbt NICHT unser CWD (Service startet in /) — daher überall
# absolute Pfade ($SCRIPT_DIR), sonst stirbt das Fenster instant (relatives ./scripts/...).

SESSION="sail-test"
WINDOW=""
MEM="20G"
WARN_CAP="${RIG_WARN_CAP:-20}"
SUMMARY="${RIG_SUMMARY:-0}"
KEEP="${RIG_KEEP:-10}"
MAX_MB="${RIG_MAX_MB:-50}"
DRY_RUN=0

usage() {
    sed -n '2,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        -s) SESSION="${2:?FEHLER: -s braucht einen Session-Namen}"; shift 2 ;;
        -w) WINDOW="${2:?FEHLER: -w braucht einen Fenster-Namen}"; shift 2 ;;
        -m) MEM="${2:?FEHLER: -m braucht MemoryMax}"; shift 2 ;;
        -c|--warn-cap) WARN_CAP="${2:?FEHLER: -c braucht eine Zahl}"; shift 2 ;;
        --warn-cap=*) WARN_CAP="${1#*=}"; shift ;;
        -S|--summary) SUMMARY=1; shift ;;
        --no-summary) SUMMARY=0; shift ;;
        --keep) KEEP="${2:?FEHLER: --keep braucht eine Zahl}"; shift 2 ;;
        --keep=*) KEEP="${1#*=}"; shift ;;
        --max-mb) MAX_MB="${2:?FEHLER: --max-mb braucht eine Zahl}"; shift 2 ;;
        --max-mb=*) MAX_MB="${1#*=}"; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        -h|--help) usage; exit 0 ;;
        --) shift; break ;;
        -*) echo "FEHLER: unbekannte Option $1" >&2; exit 2 ;;
        *) break ;;
    esac
done
# Restliche Args = Befehl für das Fenster (darf leer sein -> nur Shell).

if [ -z "$WINDOW" ]; then
    WINDOW="run-$(date +%H%M%S)"
fi

if [ "$#" -gt 0 ]; then
    HAVE_CMD=1
else
    HAVE_CMD=0
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

# limited-run-Aufruf fuer das Fenster: Caps/Rotation immer mitgeben, damit kein
# Run mehr 33MB-Logs schreibt (Lane 92). "$@" (der Run-Befehl) bleibt Array.
RIG_ARGS=(-m "$MEM" -c "$WARN_CAP" --keep "$KEEP" --max-mb "$MAX_MB")
if [ "$SUMMARY" -eq 1 ]; then
    RIG_ARGS+=(-S)
fi

# Log-Rotation vor dem Start (Lane 92): gleiche keep-Regel wie limited-run.sh.
# Nur bei echtem Start (kein dry-run): dry-run zeigt nur, was getan wuerde.
if [ "$HAVE_CMD" -eq 1 ] && [ "$DRY_RUN" -eq 0 ]; then
    if [ -d logs ]; then
        n=0
        while IFS=$'\t' read -r _mtime f; do
            n=$((n + 1))
            if [ "$n" -gt "$KEEP" ]; then
                rm -f -- "$f"
            fi
        done < <(find logs -maxdepth 1 -type f -name 'limited-run-*.log' \
            -printf '%T@\t%p\n' 2>/dev/null | sort -rn)
    fi
fi

if [ "$DRY_RUN" -eq 1 ]; then
    if tmux has-session -t "$SESSION" 2>/dev/null; then
        echo "dry-run: tmux new-window -d -t ${SESSION} -n ${WINDOW} [limited-run-args: ${RIG_ARGS[*]}] [cmd-args: $*]"
    else
        echo "dry-run: systemd-run unit tmux-${SESSION} -> tmux new-session -d -s ${SESSION} -n ${WINDOW} [limited-run-args: ${RIG_ARGS[*]}] [cmd-args: $*]"
    fi
    exit 0
fi

if ! command -v tmux >/dev/null 2>&1; then
    echo "FEHLER: tmux nicht gefunden (tmux -V prüfen, siehe TESTING.md)" >&2
    exit 1
fi

# WICHTIG: "$@" wird als Argument-Array durchgereicht, NIE re-quotet (%q-Join hat
# -c-Skripte zerlegt und Fenster instant gekillt). tmux execvp't Mehrfach-Args direkt.
if tmux has-session -t "$SESSION" 2>/dev/null; then
    if [ "$HAVE_CMD" -eq 1 ]; then
        tmux new-window -d -t "$SESSION" -n "$WINDOW" $SCRIPT_DIR/limited-run.sh "${RIG_ARGS[@]}" -- "$@"
    else
        tmux new-window -d -t "$SESSION" -n "$WINDOW"
    fi
    echo "tmux-test: Fenster $SESSION:$WINDOW angelegt"
else
    # Eigene oneshot-Unit mit remain-after-exit: überlebt Caller-Scope-Cleanup
    # (direkt/Scope/forking sterben reproduzierbar). MemoryMax deckt Rig gesamt ab.
    # Stoppen: systemctl --user stop tmux-$SESSION (killt Cgroup inkl. tmux).
    RIG_UNIT="tmux-${SESSION}"
    if [ "$HAVE_CMD" -eq 1 ]; then
        # shellcheck disable=SC2086
        systemd-run --user --unit="$RIG_UNIT" --service-type=oneshot --remain-after-exit \
            -p MemoryMax=24G -- tmux new-session -d -s "$SESSION" -n "$WINDOW" \
            $SCRIPT_DIR/limited-run.sh "${RIG_ARGS[@]}" -- "$@"
    else
        systemd-run --user --unit="$RIG_UNIT" --service-type=oneshot --remain-after-exit \
            -p MemoryMax=24G -- tmux new-session -d -s "$SESSION" -n "$WINDOW"
    fi
    echo "tmux-test: Session $SESSION mit Fenster $WINDOW angelegt (unit: $RIG_UNIT)"
fi
echo "tmux-test: Reattach mit: tmux attach -t $SESSION   (Abhaengen: Ctrl-b d)"
