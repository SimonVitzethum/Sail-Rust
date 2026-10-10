# Lane 43 — multicore-scheduler-replay
Branch: `lane/43-scheduler` | Status: QUEUED (Phase 2)

## Ziel
Core-Scheduler mit deterministischem Replay (kein Lockstep).

## Aufgaben
- `crates/sail-sched`: Cores als Threads, bounded Queues (Lane-11-Scheduler wiederverwenden),
  Event-Log über `event_clauses`-Semantik, Replay-Modus für Debugging.
- RAM-Regel: Scheduler-Overhead pro Core konstant, kein N×Modell.

## Done
- 2-Core-Replay-Demo an Fixture deterministisch, Peak protokolliert.
