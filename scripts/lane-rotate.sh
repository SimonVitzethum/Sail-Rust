#!/bin/bash
# lane-rotate.sh <done-nr> <new-nr> — z.B. lane-rotate.sh 01 16
set -euo pipefail
cd "$(dirname "$0")/.."
DONE="lane-$1"; NEW="lane-$2"
COUNT=$(ls worktrees/ | wc -l)
if [ "$COUNT" -gt 15 ]; then echo "mehr als 15 aktiv, erst mergen/loeschen"; exit 1; fi
echo "1) Lane $DONE fertigstellen, mergen, dann:"
echo "   git merge lane/<name> && git worktree remove worktrees/$DONE --force && git branch -d lane/<name>"
echo "2) Neue Lane aktivieren:"
echo "   git worktree add worktrees/$NEW lane/<name-neu>"
