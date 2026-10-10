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
#   - Session existiert noch nicht  -> tmux new-session -d -s SESSION -n FENSTER.
#   - Session existiert bereits      -> tmux new-window -d -t SESSION -n FENSTER.
#   - Der Fenster-Befehl lautet: ./scripts/limited-run.sh -m MEM -- <befehl...>
#     (ohne <befehl...> öffnet das Fenster nur eine Shell).
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
    # Befehl shell-quoted für tmux (sendet exakt ein Kommando pro Fenster).
    quoted="$(printf ' %q' "$@")"
    WIN_CMD="./scripts/limited-run.sh -m ${MEM} --${quoted}"
else
    WIN_CMD="$SHELL"
fi

if [ "$DRY_RUN" -eq 1 ]; then
    if tmux has-session -t "$SESSION" 2>/dev/null; then
        echo "dry-run: tmux new-window -d -t ${SESSION} -n ${WINDOW} ${WIN_CMD}"
    else
        echo "dry-run: tmux new-session -d -s ${SESSION} -n ${WINDOW} ${WIN_CMD}"
    fi
    exit 0
fi

if ! command -v tmux >/dev/null 2>&1; then
    echo "FEHLER: tmux nicht gefunden (tmux -V prüfen, siehe docs/TEST-RIG.md)" >&2
    exit 1
fi

if tmux has-session -t "$SESSION" 2>/dev/null; then
    tmux new-window -d -t "$SESSION" -n "$WINDOW" "$WIN_CMD"
    echo "tmux-test: Fenster $SESSION:$WINDOW angelegt (cmd: $WIN_CMD)"
else
    # WICHTIG: tmux-Server in eigenem systemd-Scope starten. Der Aufrufer läuft in einer
    # transienten Scope-Unit, deren Cleanup beim Befehlsende alle Restprozesse (inkl. tmux-Server)
    # killen würde — das hat wiederholt Sessions samt Läufen zerstört. Eigener Scope überlebt.
    systemd-run --user --scope --unit="tmux-$SESSION" -- tmux new-session -d -s "$SESSION" -n "$WINDOW" "$WIN_CMD"
    echo "tmux-test: Session $SESSION mit Fenster $WINDOW angelegt (cmd: $WIN_CMD, scope: tmux-$SESSION)"
fi
echo "tmux-test: Reattach mit: tmux attach -t $SESSION   (Abhaengen: Ctrl-b d)"
