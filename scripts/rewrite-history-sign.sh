#!/bin/bash
# NACH dem Landen von lane-11 + lane-17 laufen lassen (worktrees clean, keine Agents aktiv).
# Schreibt ALLE sail-rs-Commits um: Autor SimonVitzethum + beide Trailer. Kein Push nötig (kein Remote).
set -euo pipefail
cd "$(dirname "$0")/.."
export FILTER_BRANCH_SQUELCH_WARNING=1
git filter-branch --force --msg-filter '
  grep -q "Co-authored-by: muse-spark-1.3" || cat - <<TRAILER

Co-authored-by: muse-spark-1.3 <muse-spark-1.3@noreply.local>
TRAILER' --env-filter '
  export GIT_AUTHOR_NAME="SimonVitzethum" GIT_AUTHOR_EMAIL="simon.vitzethum@icloud.com"
  export GIT_COMMITTER_NAME="SimonVitzethum" GIT_COMMITTER_EMAIL="simon.vitzethum@icloud.com"
' -- --all
echo "OK - danach: git worktree list pruefen, Agents auf neue SHAs hinweisen"
