# Lane 48 — workspace-unify
Branch: `lane/48-workspace-unify` | Status: QUEUED

## Ziel
Ein Workspace, keine Duplikate, keine Stub-Reste.

## Aufgaben
- Nach Integrator: doppelte Minimal-Crates vereinen, Lane-15-Stubs entfernen wo echt existiert,
  `[lints] workspace=true` überall, `cargo test --workspace` grün als Gate.

## Done
- `cargo geiger --forbid-only` sauber, keine toten Member.
