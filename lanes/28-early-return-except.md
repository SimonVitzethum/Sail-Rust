# Lane 28 — early-return-except
Branch: `lane/28-early-return` | Status: QUEUED

## Ziel
`early_return`/`throw`/`try-catch`/`exit`/`assert`.

## Aufgaben
- `SailME`/`ExceptM`-Abbildung, `sailTryCatch(E)`, `E_exit → throw Error.Exit`.
- `early_return.sail`, `errors.sail`-Tests.

## Done
- Monaden-Schachtelung korrekt, keine falschen `←`.
