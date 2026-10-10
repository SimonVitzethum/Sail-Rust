#!/usr/bin/env bash
# tmux-test.sh — Test-Runs in tmux, ein Fenster je Run, begrenzt via limited-run.sh.
# Lane 51 (test-rig). Legt nur Sessions/Fenster an, startet selbst nichts Schweres.
#
# Aufruf:
#   ./scripts/tmux-test.sh [-s SESSION] [-w FENSTER] [-m MEM] [--dry-run] -- <befehl...>
#   ./scripts/tmux-test.sh [-s SESSION] [--dry-run]     # nur Session mit Shell anlegen
#
# Optionen:
#   -s SESSION  tmux-Session-Name (Default: sail-test).
#   -w FENSTER  Fenster-Name für diesen Run (Default: run-<Zeit>).
#   -m MEM      MemoryMax für limited-run.sh (Default 20G, Checks 10G).
#   --dry-run   nur zeigen, was getan wuerde (kein tmux-Aufruf).
#   -h          Hilfe.
#
# Verhalten:
#   - Session existiert noch nicht  -> eigene oneshot-Unit (remain-after-exit) + tmux new-session.
#   - Session existiert bereits      -> tmux new-window.
#   - Der Fenster-Befehl lautet: $SCRIPT_DIR/limited-run.sh -m MEM -- <befehl...>
#     (ohne <befehl...> öffnet das Fenster nur eine Shell).
#   - "$@" wird als Array durchgereicht, nie re-quotet (ein %q-Join hat -c-Skripte
#     zerlegt -> Fenster starben instant, Server ohne Fenster beendet sich).
#
# Reattach / Bedienung (siehe auch docs/TEST-RIG.md):
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
DRY_RUN=0

usage() {
    sed -n '2,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        -s) SESSION="${2:?FEHLER: -s braucht einen Session-Namen}"; shift 2 ;;
        -w) WINDOW="${2:?FEHLER: -w braucht einen Fenster-Namen}"; shift 2 ;;
        -m) MEM="${2:?FEHLER: -m braucht MemoryMax}"; shift 2 ;;
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

if [ "$DRY_RUN" -eq 1 ]; then
    if tmux has-session -t "$SESSION" 2>/dev/null; then
        echo "dry-run: tmux new-window -d -t ${SESSION} -n ${WINDOW} [cmd-args: $*]"
    else
        echo "dry-run: systemd-run unit tmux-${SESSION} -> tmux new-session -d -s ${SESSION} -n ${WINDOW} [cmd-args: $*]"
    fi
    exit 0
fi

if ! command -v tmux >/dev/null 2>&1; then
    echo "FEHLER: tmux nicht gefunden (tmux -V prüfen, siehe docs/TEST-RIG.md)" >&2
    exit 1
fi

# WICHTIG: "$@" wird als Argument-Array durchgereicht, NIE re-quotet (%q-Join hat
# -c-Skripte zerlegt und Fenster instant gekillt). tmux execvp't Mehrfach-Args direkt.
if tmux has-session -t "$SESSION" 2>/dev/null; then
    if [ "$HAVE_CMD" -eq 1 ]; then
        tmux new-window -d -t "$SESSION" -n "$WINDOW" $SCRIPT_DIR/limited-run.sh -m "$MEM" -- "$@"
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
            $SCRIPT_DIR/limited-run.sh -m "$MEM" -- "$@"
    else
        systemd-run --user --unit="$RIG_UNIT" --service-type=oneshot --remain-after-exit \
            -p MemoryMax=24G -- tmux new-session -d -s "$SESSION" -n "$WINDOW"
    fi
    echo "tmux-test: Session $SESSION mit Fenster $WINDOW angelegt (unit: $RIG_UNIT)"
fi
echo "tmux-test: Reattach mit: tmux attach -t $SESSION   (Abhaengen: Ctrl-b d)"
