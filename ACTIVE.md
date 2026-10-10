# Aktiv-Set (immer genau 15 Worktrees)

Aktiv: Lane 01–15 unter `sail-rs/worktrees/lane-01..15`.
Queued: Lane 16–40 nur als Spec + Branch, kein Worktree.

Rotation: Lane fertig → mergen → Worktree löschen → nächsten Queued-Branch aktivieren:

```
./scripts/lane-rotate.sh <fertig-lane-nr> <neu-lane-nr>
# z.B. ./scripts/lane-rotate.sh 01 16
```

Regeln:
- Max. 15 Verzeichnisse in `worktrees/` — Skript bricht bei mehr ab.
- Jeder Merge braucht Lane-13-Diff-Report; RAM-Lanes brauchen Lane-14-Budget.
